//! Request-critical-path timing. Only the request owner is timed; spawned readers,
//! observation workers and overlapping transport spans are deliberately excluded.

use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum LocalPhase {
    Ingress,
    Replay,
    Parse,
    Externalize,
    Prevalidate,
    History,
    Decision,
    Plan,
    Prepare,
    Stream,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum WaitKind {
    ClientRead,
    Decision,
    Connect,
    UpstreamWrite,
    UpstreamRead,
    DownstreamWrite,
    LocalWork,
    Other,
}

#[derive(Clone, Debug, Default)]
pub struct TimingSnapshot {
    pub elapsed_us: u64,
    pub local_us: [u64; 11],
    pub wait_us: [u64; 8],
}

#[derive(Clone, Debug)]
pub struct RequestClock(Arc<Mutex<Clock>>);

#[derive(Debug)]
struct Clock {
    started: Instant,
    changed: Instant,
    active: bool,
    phase: LocalPhase,
    pending: Option<WaitKind>,
    pending_revision: u64,
    poll_generation: u64,
    local_ns: [u64; 11],
    wait_ns: [u64; 8],
    finished: Option<TimingSnapshot>,
}

tokio::task_local! {
    static CURRENT: RequestClock;
}

impl RequestClock {
    pub fn new() -> Self {
        Self::started_at(Instant::now())
    }

    pub fn started_at(now: Instant) -> Self {
        Self(Arc::new(Mutex::new(Clock {
            started: now,
            changed: now,
            active: true,
            phase: LocalPhase::Ingress,
            pending: None,
            pending_revision: 0,
            poll_generation: 0,
            local_ns: [0; 11],
            wait_ns: [0; 8],
            finished: None,
        })))
    }

    pub async fn run<F: Future>(&self, future: F) -> F::Output {
        let mut future = pin!(future);
        CURRENT
            .scope(
                self.clone(),
                poll_fn(|cx| {
                    {
                        let mut clock = self.0.lock();
                        clock.settle(Instant::now());
                        clock.active = true;
                        clock.pending = None;
                        clock.poll_generation = clock.poll_generation.wrapping_add(1);
                    }
                    let result = future.as_mut().poll(cx);
                    let mut clock = self.0.lock();
                    clock.settle(Instant::now());
                    // Ready remains local until the owner freezes the clock. Pending
                    // gaps include executor scheduling delay, not just network time.
                    clock.active = result.is_ready();
                    result
                }),
            )
            .await
    }

    /// Freeze once at HTTP completion/cancellation, independently of later sinks.
    pub fn finish(&self) -> TimingSnapshot {
        let mut clock = self.0.lock();
        if let Some(snapshot) = &clock.finished {
            return snapshot.clone();
        }
        let now = Instant::now();
        clock.settle(now);
        let elapsed_us = micros(now.duration_since(clock.started).as_nanos());
        let mut snapshot = TimingSnapshot {
            elapsed_us,
            local_us: clock.local_ns.map(|ns| ns / 1_000),
            wait_us: clock.wait_ns.map(|ns| ns / 1_000),
        };
        // Round once per bucket; keep the partition exact at microsecond precision.
        let accounted = snapshot
            .local_us
            .iter()
            .chain(&snapshot.wait_us)
            .sum::<u64>();
        snapshot.local_us[LocalPhase::Other as usize] += elapsed_us.saturating_sub(accounted);
        clock.finished = Some(snapshot.clone());
        snapshot
    }
}

impl Default for RequestClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock {
    fn settle(&mut self, now: Instant) {
        if self.finished.is_some() {
            return;
        }
        let ns = now
            .duration_since(self.changed)
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64;
        let bucket = if self.active {
            &mut self.local_ns[self.phase as usize]
        } else {
            &mut self.wait_ns[self.pending.unwrap_or(WaitKind::Other) as usize]
        };
        *bucket = bucket.saturating_add(ns);
        self.changed = now;
    }
}

pub fn set_phase(phase: LocalPhase) {
    let _ = CURRENT.try_with(|current| {
        let mut clock = current.0.lock();
        clock.settle(Instant::now());
        clock.phase = phase;
    });
}

/// Label only time during which this future is Pending. Polling its codec or
/// protocol code stays in the local bucket. Nested leaf waits take precedence;
/// conflicting concurrently pending operations remain unclassified.
pub async fn wait<F: Future>(kind: WaitKind, future: F) -> F::Output {
    let mut future = pin!(future);
    let mut marker = None;
    poll_fn(|cx| {
        let revision = CURRENT
            .try_with(|current| current.0.lock().pending_revision)
            .ok();
        let result = future.as_mut().poll(cx);
        if result.is_pending() {
            marker = CURRENT
                .try_with(|current| {
                    let mut clock = current.0.lock();
                    if Some(clock.pending_revision) != revision {
                        return None;
                    }
                    let kind = if clock.phase == LocalPhase::Decision
                        && matches!(
                            kind,
                            WaitKind::Connect | WaitKind::UpstreamRead | WaitKind::UpstreamWrite
                        ) {
                        WaitKind::Decision
                    } else {
                        kind
                    };
                    clock.pending = Some(match clock.pending {
                        Some(previous) if previous != kind => WaitKind::Other,
                        _ => kind,
                    });
                    clock.pending_revision = clock.pending_revision.wrapping_add(1);
                    Some(PendingMarker {
                        clock: current.clone(),
                        generation: clock.poll_generation,
                        revision: clock.pending_revision,
                    })
                })
                .ok()
                .flatten();
        } else {
            marker.take();
        }
        result
    })
    .await
}

/// A select/timeout can discard a Pending I/O future without suspending the
/// request on it. Retire its label with that future; never let it label later
/// cleanup. Revisions prevent an old marker from erasing a newer leaf/sibling.
struct PendingMarker {
    clock: RequestClock,
    generation: u64,
    revision: u64,
}

impl Drop for PendingMarker {
    fn drop(&mut self) {
        let mut clock = self.clock.0.lock();
        if clock.poll_generation == self.generation && clock.pending_revision == self.revision {
            clock.settle(Instant::now());
            clock.pending = None;
            clock.pending_revision = clock.pending_revision.wrapping_add(1);
        }
    }
}

fn micros(ns: u128) -> u64 {
    (ns / 1_000).min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests;
