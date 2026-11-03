use super::tests::{link, open, query, reader, seed};
use super::*;
use hiroute_domain::{
    ContentCompleteness, FactsCompleteness, ObservationQueryPort,
    ObservationSessionCorrelationKindV1,
};
use rusqlite::params;

#[test]
fn sessions_name_verified_executors_for_every_harness_without_inventing_agent_ownership() {
    for harness in ["codex", "claude", "dsh", "pi", "qoder", "opencode"] {
        let root = tempfile::tempdir().unwrap();
        let store = open(root.path());
        seed(&store, "first", 100);
        let mut relation = link("first", "run-one");
        relation.harness_id = harness.into();
        store.link_observed_request(&relation).unwrap();
        let page = store
            .observed_sessions(&reader(None), &query(50), 500)
            .unwrap();
        assert_eq!(page.sessions[0].executor.as_deref(), Some(harness));
        assert!(page.sessions[0].agent_id.is_empty());
        // Executor proof does not upgrade request-scoped native history evidence.
        assert_ne!(
            page.sessions[0].correlation_kind,
            ObservationSessionCorrelationKindV1::VerifiedWorker
        );
        drop(store);
        let reopened = open(root.path());
        let retained = reopened
            .observed_sessions(&reader(None), &query(50), 500)
            .unwrap();
        assert_eq!(retained.sessions, page.sessions);
    }
}

#[test]
fn continued_runs_keep_one_executor_while_scoped_reads_do_not_leak_other_harnesses() {
    let root = tempfile::tempdir().unwrap();
    let store = open(root.path());
    seed(&store, "first", 100);
    seed(&store, "second", 200);
    let first = link("first", "run-one");
    let mut second = link("second", "run-two");
    second.task_id = first.task_id.clone();
    second.continued_from_run_id = Some(first.run_id.clone());
    store.link_observed_request(&first).unwrap();
    store.link_observed_request(&second).unwrap();
    assert_eq!(
        store
            .observed_sessions(&reader(None), &query(50), 500)
            .unwrap()
            .sessions[0]
            .executor
            .as_deref(),
        Some("codex")
    );

    // A later Continue can use a different executor. No one executor owns the
    // combined scope; each authorized/time-filtered population keeps its own fact.
    seed(&store, "third", 300);
    let mut third = link("third", "run-three");
    third.task_id = first.task_id.clone();
    third.continued_from_run_id = Some(second.run_id.clone());
    third.harness_id = "dsh".into();
    store.link_observed_request(&third).unwrap();
    assert!(
        store
            .observed_sessions(&reader(None), &query(50), 500)
            .unwrap()
            .sessions[0]
            .executor
            .is_none()
    );
    for (run, harness) in [
        ("run-one", "codex"),
        ("run-two", "codex"),
        ("run-three", "dsh"),
    ] {
        let page = store
            .observed_sessions(&reader(Some(run)), &query(50), 500)
            .unwrap();
        assert_eq!(page.sessions[0].request_count, 1);
        assert_eq!(page.sessions[0].executor.as_deref(), Some(harness));
    }
    let early = ObservationRequestQuery {
        to_ms: 150,
        ..query(50)
    };
    assert_eq!(
        store
            .observed_sessions(&reader(None), &early, 500)
            .unwrap()
            .sessions[0]
            .executor
            .as_deref(),
        Some("codex")
    );
}

#[test]
fn missing_or_conflicted_links_cannot_borrow_a_sibling_executor() {
    let root = tempfile::tempdir().unwrap();
    let store = open(root.path());
    seed(&store, "first", 100);
    seed(&store, "second", 200);
    let first = link("first", "run-one");
    let second = link("second", "run-one");
    store.link_observed_request(&first).unwrap();
    let missing = store
        .observed_sessions(&reader(None), &query(50), 500)
        .unwrap();
    assert!(missing.sessions[0].executor.is_none());
    assert!(
        serde_json::to_value(&missing.sessions[0])
            .unwrap()
            .get("executor")
            .is_none()
    );
    store.link_observed_request(&second).unwrap();
    let mut conflict = second.clone();
    conflict.harness_id = "claude".into();
    conflict.source_event_id = "conflict".into();
    assert!(matches!(
        store.link_observed_request(&conflict),
        Err(ObservationV2Error::RelationshipConflict)
    ));
    assert!(
        store
            .observed_sessions(&reader(None), &query(50), 500)
            .unwrap()
            .sessions[0]
            .executor
            .is_none()
    );
    let early = ObservationRequestQuery {
        to_ms: 150,
        ..query(50)
    };
    assert_eq!(
        store
            .observed_sessions(&reader(None), &early, 500)
            .unwrap()
            .sessions[0]
            .executor
            .as_deref(),
        Some("codex")
    );
}

#[test]
fn executor_presentation_does_not_rewrite_a_recorded_native_agent() {
    let root = tempfile::tempdir().unwrap();
    let store = open(root.path());
    seed(&store, "first", 100);
    store
        .connection
        .lock()
        .execute("UPDATE sessions SET agent_id='agent_hermes_default'", [])
        .unwrap();
    let native = store
        .observed_sessions(&reader(None), &query(50), 500)
        .unwrap();
    assert_eq!(native.sessions[0].agent_id, "agent_hermes_default");
    assert!(native.sessions[0].executor.is_none());
    store
        .link_observed_request(&link("first", "run-one"))
        .unwrap();
    let delegated = store
        .observed_sessions(&reader(None), &query(50), 500)
        .unwrap();
    assert_eq!(delegated.sessions[0].agent_id, native.sessions[0].agent_id);
    assert_eq!(delegated.sessions[0].executor.as_deref(), Some("codex"));
}

fn retire_during_session_read(store: &crate::LocalObservationStore) {
    let mut db = store.connection.lock();
    let transaction = db.transaction().unwrap();
    transaction
        .execute("DELETE FROM logical_requests WHERE request_id='first'", [])
        .unwrap();
    invalidate_visibility(&transaction).unwrap();
    transaction.commit().unwrap();
}

#[test]
fn a_fresh_session_page_restarts_after_retirement_without_returning_expired_membership() {
    let directory = tempfile::tempdir().unwrap();
    let store = open(directory.path());
    seed(&store, "first", 100);
    seed(&store, "second", 200);
    *store.session_read_test_hook.lock() = Some(Box::new(retire_during_session_read));
    let page = store
        .observed_sessions(&reader(None), &query(50), 500)
        .unwrap();
    assert_eq!(page.sessions.len(), 1);
    assert_eq!(page.sessions[0].request_count, 1);
}

#[test]
fn a_paged_session_read_stays_stale_after_retirement_and_keeps_its_cursor_contract() {
    let directory = tempfile::tempdir().unwrap();
    let store = open(directory.path());
    seed(&store, "first", 100);
    seed(&store, "second", 200);
    {
        let db = store.connection.lock();
        db.execute("INSERT INTO sessions(workspace_id,session_id,started_at_ms,updated_at_ms) VALUES(?1,'other-session',200,200)", [WorkspaceId::DEFAULT]).unwrap();
        db.execute(
            "UPDATE logical_requests SET session_id='other-session' WHERE request_id='second'",
            [],
        )
        .unwrap();
    }
    let first = store
        .observed_sessions(&reader(None), &query(1), 500)
        .unwrap();
    assert!(first.next_cursor.is_some());
    *store.session_read_test_hook.lock() = Some(Box::new(retire_during_session_read));
    let next = ObservationRequestQuery {
        cursor: first.next_cursor,
        ..query(1)
    };
    assert!(matches!(
        store.observed_sessions(&reader(None), &next, 500),
        Err(ObservationV2Error::Stale)
    ));
}

#[test]
fn a_fresh_session_page_never_extends_an_existing_query_deadline() {
    let directory = tempfile::tempdir().unwrap();
    let store = open(directory.path());
    seed(&store, "first", 100);
    let scope = reader(None).with_query_deadline(std::time::Instant::now());
    assert!(matches!(
        store.observed_sessions(&scope, &query(50), 500),
        Err(ObservationV2Error::Timeout)
    ));
}

#[test]
fn session_content_completeness_covers_all_visible_requests_and_recovers() {
    let root = tempfile::tempdir().unwrap();
    let store = open(root.path());
    seed(&store, "first", 100);
    seed(&store, "second", 200);
    store
        .link_observed_request(&link("first", "allowed"))
        .unwrap();
    store
        .link_observed_request(&link("second", "other"))
        .unwrap();
    {
        let db = store.connection.lock();
        db.execute(
            "UPDATE logical_requests SET outcome='accepted',finished_at_ms=300",
            [],
        )
        .unwrap();
        db.execute("UPDATE sessions SET content_completeness='complete'", [])
            .unwrap();
        for direction in ["request_input", "response_delivered"] {
            db.execute("INSERT INTO transcript_roots_v2 VALUES(?1,?2,'session','first',?3,'fork',NULL,'finish',100000000)", params![WorkspaceId::DEFAULT,format!("first-{direction}"),direction]).unwrap();
        }
    }
    let state = |scope: ObservationReaderContext, q: ObservationRequestQuery| {
        store.observed_sessions(&scope, &q, 500).unwrap().sessions[0].content_completeness
    };
    assert_eq!(state(reader(None), query(50)), ContentCompleteness::Partial);
    // A complete title request cannot establish completeness for its sibling,
    // but neither may that sibling leak into a narrower authorized/query scope.
    assert_eq!(
        state(reader(Some("allowed")), query(50)),
        ContentCompleteness::Complete
    );
    assert_eq!(
        state(
            reader(None),
            ObservationRequestQuery {
                to_ms: 150,
                ..query(50)
            }
        ),
        ContentCompleteness::Complete
    );
    assert_eq!(
        state(
            reader(None),
            ObservationRequestQuery {
                request_id: Some("first".into()),
                ..query(50)
            }
        ),
        ContentCompleteness::Complete
    );
    {
        let db = store.connection.lock();
        for direction in ["request_input", "response_delivered"] {
            db.execute("INSERT INTO transcript_roots_v2 VALUES(?1,?2,'session','second',?3,'fork',NULL,'finish',200000000)", params![WorkspaceId::DEFAULT,format!("second-{direction}"),direction]).unwrap();
        }
    }
    assert_eq!(
        state(reader(None), query(50)),
        ContentCompleteness::Complete
    );
    store
        .connection
        .lock()
        .execute("UPDATE sessions SET content_completeness='deleted'", [])
        .unwrap();
    assert_eq!(state(reader(None), query(50)), ContentCompleteness::Deleted);
}

#[test]
fn status_truncates_gap_details_without_losing_health_or_full_scope_completeness() {
    let root = tempfile::tempdir().unwrap();
    let store = open(root.path());
    seed(&store, "first", 100);
    {
        let db = store.connection.lock();
        db.execute(
            "UPDATE sessions SET facts_completeness='complete',content_completeness='complete'",
            [],
        )
        .unwrap();
        for n in 0..201 {
            db.execute("INSERT INTO observation_gaps(channel,producer_id,producer_epoch,stream_id,first_sequence,last_sequence,known_loss,workspace_id,session_id) VALUES(?1,'producer','epoch','stream',?2,?2,1,?3,'session')", params![if n == 200 { "fact" } else { "content" }, n, WorkspaceId::DEFAULT]).unwrap();
        }
    }
    let status = store.get_status(&WorkspaceId::default()).unwrap();
    assert_eq!(status.gaps.len(), 200);
    assert!(status.gaps_truncated);
    assert_eq!(status.content_completeness, ContentCompleteness::Partial);
    assert_eq!(
        status.facts_completeness,
        FactsCompleteness::Partial,
        "gap outside the detail window still affects the aggregate"
    );
}
