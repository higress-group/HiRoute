use super::*;
use std::sync::mpsc::{self, RecvTimeoutError};

#[test]
fn contended_admission_waits_for_metadata_and_keeps_the_actual_turn() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, Duration::from_secs(60));
    let key = store.scope_key("workspace", "contended").unwrap();
    let now = Instant::now();
    let input = request(vec![text(MessageRole::User, "actual input")]);
    let (ticket, history) = std::thread::scope(|threads| {
        let lock = store.inner.lock();
        let (started, entrance) = mpsc::sync_channel(0);
        let (finished, completion) = mpsc::channel();
        let store = &store;
        let key = &key;
        let replay = &replay;
        let worker = threads.spawn(move || {
            // This path enters admission directly, independently of checkpoint
            // preparation's lock, so both entrances have a regression.
            let analyzed = AgentTurnHistoryStore::prepare_request(&input, replay).unwrap();
            started.send(()).unwrap();
            let result = store.begin_with_admission(
                key.clone(),
                plan(1),
                analyzed,
                TurnDecisionInputs {
                    native_turn: None,
                    history_continuity: crate::context_hold::HistoryContinuity::Continuing,
                    reselect_on_user_message: true,
                },
                now,
                || Some(()),
            );
            finished.send(()).unwrap();
            result
        });
        entrance.recv().unwrap();
        let while_locked = completion.recv_timeout(Duration::from_millis(50));
        drop(lock); // Release even if the old nonblocking implementation returned.
        let result = worker.join().unwrap();
        assert!(matches!(while_locked, Err(RecvTimeoutError::Timeout)));
        new_turn(result.unwrap().unwrap().0)
    });
    assert!(!history.history_partial);
    assert!(history.assessment_target.is_none());
    accepted_output::text_output(&store, &ticket, "actual accepted answer");
    close_turn(
        &store, &ticket, "regular", "model", "profile", "actual", now,
    );
    drop(history);
    let next = request(vec![
        text(MessageRole::User, "actual input"),
        text(MessageRole::Assistant, "actual accepted answer"),
        text(MessageRole::User, "next input"),
    ]);
    let (_, history) = new_turn(store.begin(key, plan(1), &next, &replay, now).unwrap());
    let target = history.assessment_target.as_ref().unwrap();
    assert_eq!(target.through_turn_id, ticket.agent_turn_id);
    assert!(!target.target_partial);
}

#[test]
fn expiry_visits_are_bounded_and_current_scope_expires_outside_the_window() {
    let (_directory, replay) = replay();
    let ttl = Duration::from_secs(60);
    let store = AgentTurnHistoryStore::new(128 * 1024 * 1024, ttl);
    let now = Instant::now();
    let input = request(vec![text(MessageRole::User, "small retained task")]);
    for index in 0..512 {
        let key = store.scope_key("workspace", &index.to_string()).unwrap();
        let (ticket, history) = new_turn(store.begin(key, plan(1), &input, &replay, now).unwrap());
        drop(history);
        close_turn(&store, &ticket, "regular", "model", "profile", "r", now);
    }
    let mut inner = store.inner.lock();
    assert_eq!(inner.entries.len(), 512);
    inner.expiry_cursor = None;
    let current = inner.entries.last_key_value().unwrap().1.scope.clone();
    let expired = now + ttl;
    let visits = inner.purge_expired(expired, ttl, &current);
    assert_eq!(visits, maintenance::EXPIRY_BATCH + 1);
    assert!(!inner.entries.contains_key(&current));
    assert_eq!(inner.entries.len(), 512 - visits);
    let absent = store.scope_key("workspace", "absent").unwrap();
    for _ in 0..16 {
        let visits = inner.purge_expired(expired, ttl, &absent);
        assert!(visits <= maintenance::EXPIRY_BATCH);
    }
    assert!(inner.entries.is_empty());
    assert!(inner.scopes.is_empty());
    assert_eq!(inner.accounted_bytes, store_fixed_bytes());
}

#[test]
fn expiry_preserves_pending_requests_and_pinned_snapshots_then_reclaims_them() {
    let (_directory, replay) = replay();
    let ttl = Duration::from_secs(60);
    let store = AgentTurnHistoryStore::new(2 * 1024 * 1024, ttl);
    let key = store.scope_key("workspace", "protected").unwrap();
    let now = Instant::now();
    let first = request(vec![text(MessageRole::User, "first")]);
    let (ticket, history) = new_turn(
        store
            .begin(key.clone(), plan(1), &first, &replay, now)
            .unwrap(),
    );
    drop(history);
    let later = now + ttl;
    {
        let mut inner = store.inner.lock();
        inner.purge_expired(later, ttl, &key);
        assert!(inner.entries.contains_key(&key));
    }
    accepted_output::text_output(&store, &ticket, "answer");
    close_turn(&store, &ticket, "regular", "model", "profile", "r1", now);
    let next = request(vec![
        text(MessageRole::User, "first"),
        text(MessageRole::Assistant, "answer"),
        text(MessageRole::User, "second"),
    ]);
    let (ticket, history) = new_turn(
        store
            .begin(key.clone(), plan(1), &next, &replay, now)
            .unwrap(),
    );
    close_turn(&store, &ticket, "regular", "model", "profile", "r2", now);
    {
        let mut inner = store.inner.lock();
        assert!(inner.entries[&key].snapshot_pins > 0);
        inner.purge_expired(later, ttl, &key);
        assert!(inner.entries.contains_key(&key));
    }
    drop(history);
    {
        let mut inner = store.inner.lock();
        inner.purge_expired(later, ttl, &key);
        assert!(!inner.entries.contains_key(&key));
        assert_eq!(inner.accounted_bytes, store_fixed_bytes());
    }
}
