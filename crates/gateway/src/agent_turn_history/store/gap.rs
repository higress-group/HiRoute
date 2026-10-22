//! Nonblocking disclosure of main executions that optional history could not admit.
use super::*;

impl AgentTurnHistoryStore {
    pub(super) fn capture_gap_epoch(&self, key: &AgentTurnHistoryKey) -> &AtomicU64 {
        // Scope keys are HMAC outputs. Fixed stripes bound contention bookkeeping
        // without a global gap that makes every unrelated session partial.
        &self.capture_gap_epochs[usize::from(key.0[0])]
    }

    pub(crate) fn note_capture_gap(&self, key: &AgentTurnHistoryKey) {
        if let Some(mut inner) = self.inner.try_lock() {
            inner.note_capture_gap(key);
            return;
        }
        // A collision discloses uncertainty conservatively. It cannot assert a
        // missing execution was captured, and stabilizes for later complete stages.
        let _ = self.capture_gap_epoch(key).fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |epoch| epoch.checked_add(1),
        );
    }

    pub(super) fn observe_capture_gap_epoch(
        &self,
        inner: &mut StoreInner,
        key: &AgentTurnHistoryKey,
        epoch: u64,
    ) {
        if inner
            .entries
            .get(key)
            .is_some_and(|entry| epoch == u64::MAX || entry.capture_gap_epoch < epoch)
        {
            inner.note_capture_gap(key);
            if let Some(entry) = inner.entries.get_mut(key) {
                entry.capture_gap_epoch = epoch;
            }
        }
    }
}
