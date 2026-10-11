//! A synchronous compatibility operation's caller-owned lifetime. Never execute these
//! operations on an async worker. Scoped thread-local propagation is confined to one
//! blocking job; a child thread must explicitly carry `capture()` with it.
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::{Mutex, MutexGuard};

use crate::CpaLifecycleError;

const WAIT_SLICE: Duration = Duration::from_millis(20);

#[derive(Clone)]
pub struct CpaRequestContext {
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
    revision: Option<(Arc<AtomicU64>, u64)>,
    external_cancellation: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}

impl CpaRequestContext {
    pub fn new(deadline: Instant) -> Self {
        Self {
            deadline,
            cancelled: Arc::new(AtomicBool::new(false)),
            revision: None,
            external_cancellation: None,
        }
    }

    /// Hosts can expose their cancellation source without coupling the bridge to
    /// a particular async runtime. This predicate must be an atomic, nonblocking read.
    pub fn with_cancellation_check(mut self, check: Arc<dyn Fn() -> bool + Send + Sync>) -> Self {
        self.external_cancellation = Some(check);
        self
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn ensure_active(&self) -> Result<(), CpaLifecycleError> {
        if self.cancelled.load(Ordering::Acquire)
            || Instant::now() >= self.deadline
            || self
                .external_cancellation
                .as_ref()
                .is_some_and(|check| check())
            || self
                .revision
                .as_ref()
                .is_some_and(|(revision, expected)| revision.load(Ordering::Acquire) != *expected)
        {
            Err(CpaLifecycleError::OperationCancelled)
        } else {
            Ok(())
        }
    }

    pub fn run<T>(&self, operation: impl FnOnce() -> T) -> T {
        let _scope = self.enter();
        operation()
    }

    pub(crate) fn observing(mut self, revision: Arc<AtomicU64>, expected: u64) -> Self {
        self.revision = Some((revision, expected));
        self
    }

    pub(crate) fn enter(&self) -> ContextGuard {
        ACTIVE.with(|active| active.borrow_mut().push(self.clone()));
        ContextGuard {
            _thread_bound: std::marker::PhantomData,
        }
    }
}

thread_local! {
    static ACTIVE: RefCell<Vec<CpaRequestContext>> = const { RefCell::new(Vec::new()) };
}

pub(crate) struct ContextGuard {
    _thread_bound: std::marker::PhantomData<*const ()>,
}
impl Drop for ContextGuard {
    fn drop(&mut self) {
        ACTIVE.with(|active| {
            active.borrow_mut().pop();
        });
    }
}

pub(crate) fn check() -> Result<(), CpaLifecycleError> {
    ACTIVE.with(|active| {
        active
            .borrow()
            .iter()
            .try_for_each(CpaRequestContext::ensure_active)
    })
}

pub(crate) fn remaining(limit: Duration) -> Result<Duration, CpaLifecycleError> {
    check()?;
    Ok(ACTIVE.with(|active| {
        active.borrow().iter().fold(limit, |left, context| {
            left.min(context.deadline.saturating_duration_since(Instant::now()))
        })
    }))
}

pub(crate) fn capture() -> Vec<CpaRequestContext> {
    ACTIVE.with(|active| active.borrow().clone())
}

pub(crate) fn with_captured<T>(contexts: &[CpaRequestContext], operation: impl FnOnce() -> T) -> T {
    match contexts.split_first() {
        Some((context, rest)) => context.run(|| with_captured(rest, operation)),
        None => operation(),
    }
}

/// Cleanup owns a separate stop-only budget; a cancelled request cannot prevent
/// stopping a child that it has already launched. Never use this to grant work.
pub(crate) fn cleanup<T>(budget: Duration, operation: impl FnOnce() -> T) -> T {
    struct Restore(Vec<CpaRequestContext>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ACTIVE.with(|active| *active.borrow_mut() = std::mem::take(&mut self.0));
        }
    }
    let previous = ACTIVE.with(|active| std::mem::take(&mut *active.borrow_mut()));
    let _restore = Restore(previous);
    CpaRequestContext::new(Instant::now() + budget).run(operation)
}

/// Only blocking executors or synchronous control owners may wait here.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, CpaLifecycleError> {
    loop {
        let budget = remaining(WAIT_SLICE)?;
        if let Some(guard) = mutex.try_lock_for(budget) {
            check()?;
            return Ok(guard);
        }
    }
}

pub(crate) fn sleep(duration: Duration) -> Result<(), CpaLifecycleError> {
    let end = Instant::now() + duration;
    while Instant::now() < end {
        std::thread::sleep(remaining(
            end.saturating_duration_since(Instant::now())
                .min(WAIT_SLICE),
        )?);
    }
    check()
}

pub(crate) fn io_check() -> std::io::Result<()> {
    check().map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "CPA operation expired"))
}
