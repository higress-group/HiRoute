//! Bounded, source/revision-specific identity resolution. The cache guards never
//! span network I/O. Waiters own their deadlines, not the initiating caller's.
use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};

use crate::{CpaLifecycleError, request_context};

const CAPACITY: usize = 8;
const BACKOFF: Duration = Duration::from_secs(2);

thread_local! { static OWNER_DEPTH: Cell<usize> = const { Cell::new(0) }; }
pub(super) struct NoProfileIo(std::marker::PhantomData<*const ()>);
impl NoProfileIo {
    pub(super) fn enter() -> Self {
        OWNER_DEPTH.with(|depth| depth.set(depth.get() + 1));
        Self(std::marker::PhantomData)
    }
}
impl Drop for NoProfileIo {
    fn drop(&mut self) {
        OWNER_DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}

#[derive(Default)]
pub(super) struct ProfileCache {
    entries: Mutex<VecDeque<(String, Arc<Flight>)>>,
}

struct Flight {
    state: Mutex<Resolution>,
    changed: Condvar,
}

enum Resolution {
    Running,
    Ready(String),
    Failed(Instant),
    Abandoned,
}

impl ProfileCache {
    pub(super) fn resolve(
        &self,
        source: &str,
        revision: &str,
        fetch: impl FnOnce() -> Result<String, CpaLifecycleError>,
    ) -> Result<String, CpaLifecycleError> {
        let key = format!("{source}/{revision}");
        // An owner may consume only already proved identities. A token that
        // rotated after preparation must return to a new prepare operation.
        if OWNER_DEPTH.with(|depth| depth.get() > 0) {
            let entries = request_context::lock(&self.entries)?;
            return entries
                .iter()
                .find(|(cached, _)| cached == &key)
                .and_then(|(_, flight)| match &*flight.state.lock() {
                    Resolution::Ready(account) => Some(account.clone()),
                    _ => None,
                })
                .ok_or(CpaLifecycleError::BorrowedClaudeAuthSourceChanged);
        }
        let (flight, mut owner) = {
            let mut entries = request_context::lock(&self.entries)?;
            if let Some((_, flight)) = entries.iter().find(|(cached, _)| cached == &key) {
                (Arc::clone(flight), false)
            } else {
                if entries.len() == CAPACITY {
                    let removable = entries.iter().position(|(_, flight)| {
                        !matches!(*flight.state.lock(), Resolution::Running)
                    });
                    entries
                        .remove(removable.ok_or(CpaLifecycleError::BorrowedClaudeAuthUnavailable)?);
                }
                let flight = Arc::new(Flight {
                    state: Mutex::new(Resolution::Running),
                    changed: Condvar::new(),
                });
                entries.push_back((key, Arc::clone(&flight)));
                (flight, true)
            }
        };
        if !owner {
            let mut state = request_context::lock(&flight.state)?;
            loop {
                request_context::check()?;
                match &*state {
                    Resolution::Ready(account) => return Ok(account.clone()),
                    Resolution::Failed(until) if Instant::now() < *until => {
                        return Err(CpaLifecycleError::BorrowedClaudeAuthUnavailable);
                    }
                    Resolution::Failed(_) | Resolution::Abandoned => {
                        *state = Resolution::Running;
                        owner = true;
                        break;
                    }
                    Resolution::Running => {
                        let wait = request_context::remaining(Duration::from_millis(20))?;
                        flight.changed.wait_for(&mut state, wait);
                    }
                }
            }
        }
        debug_assert!(owner);
        // Publish a terminal state even if the caller unwinds, so waiters cannot
        // inherit a permanently stuck single-flight slot.
        let completion = CompleteFlight {
            flight: &flight,
            completed: false,
        };
        let result = fetch();
        completion.finish(match &result {
            Ok(account) => Resolution::Ready(account.clone()),
            Err(_) if request_context::check().is_err() => Resolution::Abandoned,
            Err(_) => Resolution::Failed(Instant::now() + BACKOFF),
        });
        request_context::check()?;
        result
    }

    #[cfg(test)]
    pub(super) fn seed(&self, source: &str, revision: &str, account: &str) {
        self.entries.lock().push_back((
            format!("{source}/{revision}"),
            Arc::new(Flight {
                state: Mutex::new(Resolution::Ready(account.into())),
                changed: Condvar::new(),
            }),
        ));
    }
}

struct CompleteFlight<'a> {
    flight: &'a Flight,
    completed: bool,
}

impl CompleteFlight<'_> {
    fn finish(mut self, resolution: Resolution) {
        let mut state = self.flight.state.lock();
        *state = resolution;
        // Disarm while publishing under the same lock. A waiter may become the
        // next owner as soon as this guard unlocks; our Drop must not abandon it.
        self.completed = true;
        self.flight.changed.notify_all();
    }
}

impl Drop for CompleteFlight<'_> {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        let mut state = self.flight.state.lock();
        if matches!(*state, Resolution::Running) {
            *state = Resolution::Abandoned;
        }
        self.flight.changed.notify_all();
    }
}
