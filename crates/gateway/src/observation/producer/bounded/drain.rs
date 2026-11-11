//! Shutdown-only ACK frontier; foreground publishing never waits for a sink.
use super::*;

#[derive(Default)]
pub(super) struct DrainProgress {
    through: AtomicU64,
    gate: Mutex<()>,
    changed: Condvar,
}

impl DrainProgress {
    pub(super) fn settled(&self, through: u64) {
        if through <= self.through.load(Ordering::Acquire) {
            return;
        }
        let _guard = lock(&self.gate);
        self.through.store(through, Ordering::Release);
        self.changed.notify_all();
    }
}

impl ByteBoundedObservationProducer {
    /// Wait for the admitted sequence prefix to receive terminal sink feedback.
    /// The caller first quiesces producers. This uses no event-queue slot and
    /// preserves the existing per-record ACK/NACK and declared-loss handling.
    pub(crate) fn drain(&self, timeout: Duration) -> Result<(), ObservationError> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(ObservationError::Unavailable)?;
        let through = self
            .shared
            .next_sequence
            .load(Ordering::Acquire)
            .saturating_sub(1);
        let progress = &self.shared.drain;
        let mut guard = lock(&progress.gate);
        while progress.through.load(Ordering::Acquire) < through {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ObservationError::Unavailable);
            }
            guard = progress
                .changed
                .wait_timeout(guard, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "drain_tests.rs"]
mod tests;
