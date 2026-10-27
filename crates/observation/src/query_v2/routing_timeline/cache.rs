use super::*;
use std::{
    collections::{BTreeSet, VecDeque},
    sync::Arc,
};

// Bound both the session count and retained bodies. An evicted frozen page must
// restart with Stale; a completed cursor can rebuild from current retained facts.
#[derive(Default)]
pub(crate) struct TimelineCache {
    pub(super) entries: BTreeMap<String, Snapshot>,
    order: VecDeque<(String, std::time::Instant)>,
}
impl TimelineCache {
    pub(super) fn take(&mut self, key: &str) -> Option<Snapshot> {
        self.prune_idle();
        self.order.retain(|(k, _)| k != key);
        self.entries.remove(key)
    }
    fn prune_idle(&mut self) {
        while self
            .order
            .front()
            .is_some_and(|(_, at)| at.elapsed() >= std::time::Duration::from_secs(60))
        {
            if let Some((key, _)) = self.order.pop_front() {
                self.entries.remove(&key);
            }
        }
    }
    pub(super) fn insert(&mut self, key: String, value: Snapshot) {
        self.prune_idle();
        self.order.retain(|(k, _)| k != &key);
        self.entries.remove(&key);
        while self.entries.len() >= 2 {
            if let Some((old, _)) = self.order.pop_front() {
                self.entries.remove(&old);
            } else {
                self.entries.clear();
            }
        }
        self.order
            .push_back((key.clone(), std::time::Instant::now()));
        self.entries.insert(key, value);
    }
}
#[derive(Default)]
pub(super) struct Snapshot {
    pub position: Option<Position>,
    pub read_requests: usize,
    pub projected_turns: usize,
    pub visibility: u64,
    pub expires_at: i64,
    pub requests: BTreeMap<String, Request>,
    pub facts: Vec<Fact>,
    pub turns: Vec<Arc<Projected>>,
    pub bases: BTreeMap<String, Arc<Projected>>,
}
impl Snapshot {
    pub fn refresh(
        &mut self,
        tx: &rusqlite::Transaction<'_>,
        reader: &ObservationReaderContext,
        query: &RoutingTimelineQueryV1,
        target: Position,
        now: i64,
        changed: Option<&BTreeSet<String>>,
    ) -> Result<(), ObservationV2Error> {
        let session = query.session_id.as_str();
        if changed.is_some_and(BTreeSet::is_empty) {
            self.position = Some(target);
            return Ok(());
        }
        if changed.is_some() {
            read::check_row_bounds(tx, reader, session, target)?;
        }
        let (requests, facts) = read::read(tx, reader, session, target, now, changed)?;
        self.read_requests += requests.len();
        if changed.is_some()
            && self
                .requests
                .keys()
                .any(|id| changed.is_some_and(|c| c.contains(id)) && !requests.contains_key(id))
        {
            return Err(ObservationV2Error::Stale);
        }
        let previous = if let Some(changed) = changed {
            self.requests.retain(|id, _| !changed.contains(id));
            self.requests.extend(requests);
            self.facts.retain(|f| !changed.contains(&f.request_id));
            self.facts.extend(facts);
            self.facts.sort_unstable_by_key(|f| f.position);
            self.position.unwrap_or_default()
        } else {
            self.requests = requests;
            self.facts = facts;
            self.bases.clear();
            self.turns.clear();
            Position::default()
        };
        if self.requests.len() > 10_000
            || self
                .requests
                .values()
                .map(|r| r.request_bytes)
                .sum::<usize>()
                > 16 * 1024 * 1024
            || self.requests.values().map(|r| r.fact_count).sum::<usize>() > 50_000
            || self.requests.values().map(|r| r.usage_count).sum::<usize>() > 50_000
            || self.requests.values().map(|r| r.body_bytes).sum::<usize>() > 32 * 1024 * 1024
        {
            return Err(ObservationV2Error::ScopeTooLarge);
        }
        self.expires_at = self
            .requests
            .values()
            .map(|r| {
                r.view
                    .started_at_ms
                    .saturating_add(crate::managed_text::RETENTION_MS)
            })
            .min()
            .unwrap_or(i64::MAX);
        let turns = projection::project(
            session,
            &self.requests,
            &self.facts,
            previous,
            &mut self.bases,
            &self.turns,
            query.include_presentation,
        )?;
        let old: BTreeMap<_, _> = self.turns.iter().map(|t| (&t.view.turn_id, t)).collect();
        self.projected_turns += turns
            .iter()
            .filter(|t| {
                old.get(&t.view.turn_id)
                    .is_none_or(|old| !Arc::ptr_eq(old, t))
            })
            .count();
        self.turns = turns;
        self.position = Some(target);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_snapshots_are_retired_and_replacing_a_key_keeps_other_readers() {
        let mut cache = TimelineCache::default();
        cache.insert("a".into(), Snapshot::default());
        cache.insert("b".into(), Snapshot::default());
        cache.insert("a".into(), Snapshot::default());
        assert_eq!(cache.entries.len(), 2);
        cache.order.front_mut().unwrap().1 =
            std::time::Instant::now() - std::time::Duration::from_secs(61);
        cache.prune_idle();
        assert!(!cache.entries.contains_key("b"));
        assert!(cache.entries.contains_key("a"));
    }
}
