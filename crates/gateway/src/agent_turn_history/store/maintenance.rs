//! Bounded idle expiry; current-scope TTL is checked before branch selection.
use std::ops::Bound::{Excluded, Unbounded};

use super::*;

pub(super) const EXPIRY_BATCH: usize = 32;

impl StoreInner {
    pub(super) fn purge_expired(
        &mut self,
        now: Instant,
        ttl: Duration,
        scope: &AgentTurnHistoryKey,
    ) -> usize {
        // A single rotating key replaces the former full-store scan. Clone keys
        // before removal so that no retained entry/reference crosses mutation.
        let mut keys = self.expiry_keys();
        if keys.is_empty() {
            self.expiry_cursor = None;
            keys = self.expiry_keys();
        }
        self.expiry_cursor = keys.last().cloned();
        let mut visits = keys.len();
        for key in keys {
            self.remove_if_expired(&key, now, ttl);
        }
        // The global window may not reach this namespace yet. Its bounded heads
        // still obey TTL immediately, rather than granting expired continuity.
        let keys = self
            .scope_entries(scope)
            .take(branches::MAX_HEADS)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        visits += keys.len();
        for key in keys {
            self.remove_if_expired(&key, now, ttl);
        }
        visits
    }

    fn expiry_keys(&self) -> Vec<AgentTurnHistoryKey> {
        let start = self.expiry_cursor.as_ref().map_or(Unbounded, Excluded);
        self.entries
            .range::<AgentTurnHistoryKey, _>((start, Unbounded))
            .take(EXPIRY_BATCH)
            .map(|(key, _)| key.clone())
            .collect()
    }

    fn remove_if_expired(&mut self, key: &AgentTurnHistoryKey, now: Instant, ttl: Duration) {
        if self.entries.get(key).is_some_and(|entry| {
            entry.pending_request.is_none()
                && entry.snapshot_pins == 0
                && now.saturating_duration_since(entry.last_access) >= ttl
        }) {
            self.remove(key);
        }
    }
}
