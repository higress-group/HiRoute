//! Nonblocking disclosure of main executions that optional history could not admit.
use super::*;

impl AgentTurnHistoryStore {
    pub(crate) fn note_capture_gap(&self, key: &AgentTurnHistoryKey) {
        if let Some(mut inner) = self.inner.try_lock() {
            inner.note_capture_gap(key);
            return;
        }
        // Never make model output wait for optional history. Contention prevents
        // a scoped write, so a fixed-size epoch conservatively invalidates other
        // retained assessment segments at their next admission as well.
        let _ = self
            .capture_gap_epoch
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |epoch| {
                epoch.checked_add(1)
            });
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
