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
            inner.note_scope_gap(key);
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
        let affected = inner
            .scope_entries(key)
            .filter(|(_, entry)| epoch == u64::MAX || entry.capture_gap_epoch < epoch)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for branch in affected {
            inner.note_capture_gap(&branch);
            inner
                .entries
                .get_mut(&branch)
                .expect("retained branch")
                .capture_gap_epoch = epoch;
        }
    }
}

impl StoreInner {
    pub(super) fn note_scope_gap(&mut self, scope: &AgentTurnHistoryKey) {
        let keys = self
            .scope_entries(scope)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in keys {
            self.note_capture_gap(&key);
        }
    }
}
