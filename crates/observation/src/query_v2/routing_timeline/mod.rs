//! A bounded projection of existing facts. Cursors retain a snapshot while paging and
//! advance only after every changed turn has been delivered.
mod admission;
pub(crate) mod cache;
mod projection;
mod read;
mod transitions;

use super::{ObservationReaderContext, ObservationV2Error, QueryDeadline, identifier};
use hiroute_domain::*;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Position {
    facts: i64,
    values: i64,
    links: i64,
}
impl Position {
    fn newer_than(self, other: Self) -> bool {
        self.facts > other.facts || self.values > other.values || self.links > other.links
    }
    fn include(&mut self, other: Self) {
        self.facts = self.facts.max(other.facts);
        self.values = self.values.max(other.values);
        self.links = self.links.max(other.links);
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    schema: u8,
    binding: String,
    visibility: u64,
    delivered: Position,
    snapshot: Option<Position>,
    after: Option<(i64, String)>,
    #[serde(default)]
    expires_at: i64,
}
struct Fact {
    position: i64,
    event_id: String,
    producer_epoch: String,
    request_id: String,
    value: ExecutionFactV1,
}
struct Request {
    view: RoutingTimelineRequestV1,
    position: Position,
    first_fact: i64,
    native_turn: Option<String>,
    run_id: Option<String>,
    decision: Option<RouteDecisionFactV1>,
    candidate: Option<(String, Option<String>)>,
    identities: std::collections::BTreeSet<(String, String)>,
    identity_gap: bool,
    request_finished: bool,
    stage_finished: bool,
    models: BTreeMap<u32, String>,
    credentials: BTreeMap<u32, (String, String)>,
    partial: bool,
    body_bytes: usize,
    request_bytes: usize,
    fact_count: usize,
    usage_count: usize,
}
#[derive(Clone)]
struct Projected {
    view: RoutingTimelineTurnV1,
    position: Position,
}

impl crate::LocalObservationStore {
    #[cfg(test)]
    pub(crate) fn timeline_work(&self) -> (usize, usize) {
        let cache = self.timeline_cache.lock();
        (
            cache.entries.values().map(|s| s.read_requests).sum(),
            cache.entries.values().map(|s| s.projected_turns).sum(),
        )
    }

    pub fn observed_routing_timeline(
        &self,
        reader: &ObservationReaderContext,
        query: &RoutingTimelineQueryV1,
        now_ms: i64,
    ) -> Result<RoutingTimelinePageV1, ObservationV2Error> {
        self.routing_timeline_inner(reader, query, now_ms, || {})
    }

    fn routing_timeline_inner(
        &self,
        reader: &ObservationReaderContext,
        query: &RoutingTimelineQueryV1,
        now_ms: i64,
        after_take: impl FnOnce(),
    ) -> Result<RoutingTimelinePageV1, ObservationV2Error> {
        reader.check(now_ms, false, false)?;
        if !identifier(&query.session_id) || query.limit == 0 || query.limit > 200 {
            return Err(ObservationV2Error::Invalid);
        }
        let _permit = self.query_permit()?;
        let started = std::time::Instant::now();
        let mut connection =
            Connection::open_with_flags(&self.activity_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let _deadline = QueryDeadline::start_for(&connection, reader)?;
        connection.pragma_update(None, "cache_size", -16_384)?;
        let tx = connection.transaction()?;
        let visibility: u64 = tx.query_row("SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='query_visibility_generation'", [], |r| r.get(0))?;
        let binding = if query.include_presentation {
            CanonicalDigest::of(&(
                reader.binding()?,
                &query.session_id,
                query.limit,
                "presentation/v1",
            ))
        } else {
            CanonicalDigest::of(&(reader.binding()?, &query.session_id, query.limit))
        }
        .map_err(|_| ObservationV2Error::Invalid)?
        .to_string();
        let mut cursor = match &query.after_watermark {
            Some(value) => {
                let cursor: Cursor = self.decode_observation_cursor(value)?;
                if cursor.schema != 2
                    || cursor.binding != binding
                    || cursor.visibility != visibility
                    || now_ms >= cursor.expires_at
                {
                    return Err(ObservationV2Error::Stale);
                }
                cursor
            }
            None => Cursor {
                schema: 2,
                binding,
                visibility,
                delivered: Position::default(),
                snapshot: None,
                after: None,
                expires_at: i64::MAX,
            },
        };
        let target = match cursor.snapshot {
            Some(value) => value,
            None => read::position(&tx)?,
        };
        let key = cursor.binding.clone();
        let cached = self
            .timeline_cache
            .lock()
            .take(&key)
            .filter(|c| c.visibility == visibility && now_ms < c.expires_at);
        after_take();
        let mut state = cached.unwrap_or_default();
        let frozen = cursor.snapshot.is_some();
        if frozen && state.position != Some(target) {
            // Mutable request/link projections cannot reconstruct an old frozen
            // page after eviction or restart. Re-admit a fresh snapshot instead.
            return Err(ObservationV2Error::Stale);
        }
        // A frozen page must never use the no-change shortcut: it still owes rows.
        if state.position != Some(target) || query.after_watermark.is_none() {
            let can_update = query.after_watermark.is_some()
                && !frozen
                && state.position.is_some_and(|p| p == cursor.delivered);
            let changed = if can_update {
                Some(read::changed_requests(
                    &tx,
                    reader,
                    &query.session_id,
                    cursor.delivered,
                    target,
                )?)
            } else {
                None
            };
            state.refresh(&tx, reader, query, target, now_ms, changed.as_ref())?;
        }
        state.visibility = visibility;
        cursor.expires_at = state.expires_at;
        if started.elapsed() > hiroute_domain::ObservationReaderContext::QUERY_BUDGET {
            return Err(ObservationV2Error::Timeout);
        }
        let start = cursor.after.as_ref().map_or(0, |after| {
            state.turns.partition_point(|turn| {
                (turn.view.started_at_ms, &turn.view.turn_id) <= (after.0, &after.1)
            })
        });
        let mut changed: Vec<_> = state.turns[start..]
            .iter()
            .filter(|turn| turn.position.newer_than(cursor.delivered))
            .take(usize::from(query.limit) + 1)
            .cloned()
            .collect();
        let complete = changed.len() <= usize::from(query.limit);
        changed.truncate(usize::from(query.limit));
        if complete {
            cursor.delivered = target;
            cursor.snapshot = None;
            cursor.after = None;
        } else {
            let last = changed.last().ok_or(ObservationV2Error::Unavailable)?;
            cursor.snapshot = Some(target);
            cursor.after = Some((last.view.started_at_ms, last.view.turn_id.clone()));
        }
        let turns = changed
            .into_iter()
            .map(|turn| {
                let mut view = turn.view.clone();
                if !query.include_presentation {
                    view.presentation = None;
                }
                view
            })
            .collect();
        let result = RoutingTimelinePageV1 {
            turns,
            watermark: self.encode_observation_cursor(&cursor)?,
            complete,
        };
        if serde_json::to_vec(&result)
            .map_err(|_| ObservationV2Error::Unavailable)?
            .len()
            > 1024 * 1024
        {
            return Err(ObservationV2Error::ScopeTooLarge);
        }
        tx.commit()?;
        super::check_visibility(&connection, visibility)?;
        self.timeline_cache.lock().insert(key, state);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_readers_do_not_hold_the_shared_cache_lock_during_projection() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::query_v2::tests::open(dir.path());
        let (entered, receive) = std::sync::mpsc::channel();
        let gate = std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for id in ["session/a", "session/b"] {
                let store = &store;
                let entered = entered.clone();
                let gate = gate.clone();
                handles.push(scope.spawn(move || {
                    let reader = ObservationReaderContext::local_user(
                        WorkspaceId::default(),
                        "user".into(),
                        1,
                        10_000,
                        false,
                        false,
                    )
                    .unwrap();
                    let query = RoutingTimelineQueryV1 {
                        include_presentation: false,
                        session_id: id.into(),
                        after_watermark: None,
                        limit: 50,
                    };
                    store.routing_timeline_inner(&reader, &query, 5000, || {
                        let probe = std::time::Instant::now();
                        loop {
                            if store.timeline_cache.try_lock().is_some() {
                                break;
                            }
                            assert!(
                                probe.elapsed() < std::time::Duration::from_millis(10),
                                "SQL and projection must not hold the cache mutex"
                            );
                            std::thread::yield_now();
                        }
                        entered.send(()).unwrap();
                        let (lock, wake) = &*gate;
                        let held = lock.lock().unwrap();
                        let (ready, _) = wake
                            .wait_timeout_while(
                                held,
                                std::time::Duration::from_millis(300),
                                |ready| !*ready,
                            )
                            .unwrap();
                        assert!(*ready, "the second reader did not enter concurrently");
                    })
                }));
            }
            receive
                .recv_timeout(std::time::Duration::from_millis(250))
                .unwrap();
            receive
                .recv_timeout(std::time::Duration::from_millis(250))
                .unwrap();
            *gate.0.lock().unwrap() = true;
            gate.1.notify_all();
            for handle in handles {
                assert!(handle.join().unwrap().is_ok());
            }
        });
    }

    #[test]
    fn expired_timeline_deadline_remains_transient_and_a_fresh_read_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::query_v2::tests::open(dir.path());
        let reader = ObservationReaderContext::local_user(
            WorkspaceId::default(),
            "user".into(),
            1,
            10_000,
            false,
            false,
        )
        .unwrap()
        .with_query_deadline(std::time::Instant::now() - std::time::Duration::from_millis(1));
        let query = RoutingTimelineQueryV1 {
            include_presentation: false,
            session_id: "session/test".into(),
            after_watermark: None,
            limit: 50,
        };
        assert_eq!(
            store
                .observed_routing_timeline(&reader, &query, 5000)
                .unwrap_err(),
            ObservationV2Error::Timeout
        );
        let fresh = ObservationReaderContext::local_user(
            WorkspaceId::default(),
            "user".into(),
            1,
            10_000,
            false,
            false,
        )
        .unwrap();
        assert!(
            store
                .observed_routing_timeline(&fresh, &query, 5000)
                .is_ok()
        );
    }

    #[test]
    fn elapsed_timeline_budget_remains_transient_and_a_fresh_read_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::query_v2::tests::open(dir.path());
        let reader = ObservationReaderContext::local_user(
            WorkspaceId::default(),
            "user".into(),
            1,
            10_000,
            false,
            false,
        )
        .unwrap();
        let query = RoutingTimelineQueryV1 {
            include_presentation: false,
            session_id: "session/test".into(),
            after_watermark: None,
            limit: 50,
        };
        assert_eq!(
            store
                .routing_timeline_inner(&reader, &query, 5000, || {
                    std::thread::sleep(
                        ObservationReaderContext::QUERY_BUDGET
                            + std::time::Duration::from_millis(10),
                    );
                })
                .unwrap_err(),
            ObservationV2Error::Timeout
        );
        assert!(
            store
                .observed_routing_timeline(&reader, &query, 5000)
                .is_ok()
        );
    }

    #[test]
    fn old_signed_cursor_requires_a_fresh_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::query_v2::tests::open(dir.path());
        let reader = ObservationReaderContext::local_user(
            WorkspaceId::default(),
            "user".into(),
            1,
            10_000,
            false,
            false,
        )
        .unwrap();
        let mut query = RoutingTimelineQueryV1 {
            include_presentation: false,
            session_id: "session/test".into(),
            after_watermark: None,
            limit: 50,
        };
        let page = store
            .observed_routing_timeline(&reader, &query, 5000)
            .unwrap();
        let mut cursor: serde_json::Value =
            store.decode_observation_cursor(&page.watermark).unwrap();
        cursor["schema"] = serde_json::json!(1);
        cursor.as_object_mut().unwrap().remove("expires_at");
        query.after_watermark = Some(store.encode_observation_cursor(&cursor).unwrap());
        assert_eq!(
            store
                .observed_routing_timeline(&reader, &query, 5000)
                .unwrap_err(),
            ObservationV2Error::Stale
        );
    }
}
