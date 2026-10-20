use super::support::*;
use crate::{LocalObservationStore, WriterCycleOutcome};
use hiroute_domain::{ContentMode, ObservationQueryPort, RunObservationLink};
use std::sync::Arc;

fn ingest(store: &Arc<LocalObservationStore>, fixture: &Fixture, grant: &str, generation: u64) {
    let mut event = fixture.fact(1, route_decision("plan/codex-daily"), 100);
    event.trust.grant_id = grant.into();
    event.trust.grant_generation = generation;
    assert!(matches!(
        offer_fact(&writer(store), &fact_channel(fixture, 64 * 1024), event),
        WriterCycleOutcome::Ack(_)
    ));
}
fn agent(store: &LocalObservationStore, fixture: &Fixture) -> String {
    store
        .get_session(&fixture.workspace, &fixture.session, ContentMode::None)
        .unwrap()
        .summary
        .agent_id
}

#[test]
fn authenticated_grant_attribution_survives_rotation_reopen_and_does_not_backfill() {
    let directory = tempfile::tempdir().unwrap();
    let old = Fixture::new("old-unproven");
    let current = Fixture::new("managed");
    {
        let store = open_store(directory.path());
        ingest(&store, &old, "grant-a", 1);
        store
            .register_agent_grant(
                &current.workspace,
                "grant-a",
                1,
                "connection-a",
                "agent_claude_default",
            )
            .unwrap();
        assert_eq!(
            agent(&store, &old),
            "",
            "registration never rewrites old observations"
        );
        ingest(&store, &old, "grant-a", 1);
        assert_eq!(
            agent(&store, &old),
            "",
            "replayed old facts do not backfill attribution"
        );
        ingest(&store, &current, "grant-a", 1);
        assert_eq!(agent(&store, &current), "agent_claude_default");
        store
            .register_agent_grant(
                &current.workspace,
                "grant-a",
                2,
                "connection-a",
                "agent_claude_default",
            )
            .unwrap();
        assert!(
            store
                .register_agent_grant(
                    &current.workspace,
                    "grant-a",
                    1,
                    "connection-b",
                    "agent_codex_default"
                )
                .is_err(),
            "a stored grant cannot be reassigned"
        );
    }
    let store = open_store(directory.path());
    assert_eq!(agent(&store, &current), "agent_claude_default");
    assert_eq!(agent(&store, &old), "");
    for (suffix, generation, expected) in [
        ("rotated", 2, "agent_claude_default"),
        ("old-in-flight", 1, "agent_claude_default"),
        ("unmapped-generation", 3, ""),
    ] {
        let fixture = Fixture::new(suffix);
        ingest(&store, &fixture, "grant-a", generation);
        assert_eq!(agent(&store, &fixture), expected);
    }
}

#[test]
fn conflicting_agent_grants_quarantine_the_session_across_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let first = Fixture::new("conflict-a");
    {
        let store = open_store(directory.path());
        store
            .register_agent_grant(
                &first.workspace,
                "grant-a",
                1,
                "connection-a",
                "agent_claude_default",
            )
            .unwrap();
        store
            .register_agent_grant(
                &first.workspace,
                "grant-b",
                1,
                "connection-b",
                "agent_codex_default",
            )
            .unwrap();
        ingest(&store, &first, "grant-a", 1);
        let mut second = Fixture::new("conflict-b");
        second.session = first.session.clone();
        ingest(&store, &second, "grant-b", 1);
        assert_eq!(agent(&store, &first), "");
    }
    let store = open_store(directory.path());
    let mut again = Fixture::new("conflict-again");
    again.session = first.session.clone();
    ingest(&store, &again, "grant-a", 1);
    assert_eq!(
        agent(&store, &first),
        "",
        "later matching evidence cannot erase the conflict"
    );
    // Erasing all session facts must remove the attribution evidence, too.
    store.connection.lock().execute("UPDATE sessions SET agent_id='',correlation='unproven',tombstone_reason='user_deleted' WHERE session_id=?1",[first.session.as_str()]).unwrap();
    let count: u64 = store
        .connection
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM observation_session_agents_v1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn worker_run_relation_remains_the_authority_for_unmanaged_grants() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("worker");
    let link = RunObservationLink {
        workspace_id: fixture.workspace.clone(),
        request_id: fixture.request.clone(),
        task_id: "task-worker".into(),
        run_id: "run-worker".into(),
        producer_epoch: "epoch".into(),
        source_event_id: "worker-event".into(),
        plan_id: "plan/codex-daily".into(),
        plan_revision: "17".into(),
        publication_ref: "publication".into(),
        harness_id: "claude".into(),
        protocol_kind: "acp".into(),
        native_session_id: Some("worker-native-session".into()),
        native_turn_id: None,
        parent_context_ref: None,
        continued_from_run_id: None,
    };
    store.link_observed_request(&link).unwrap();
    ingest(&store, &fixture, "worker-grant", 1);
    assert_eq!(agent(&store, &fixture), "");
    assert_eq!(
        store
            .observed_request_link(
                &hiroute_domain::ObservationReaderContext::local_user(
                    fixture.workspace.clone(),
                    "attribution-test".into(),
                    1,
                    10000,
                    false,
                    false
                )
                .unwrap(),
                &fixture.request,
                1000
            )
            .unwrap(),
        Some(link)
    );
}

#[test]
fn existing_format_five_store_adds_provenance_tables_without_rewriting_sessions() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new("existing-format-five");
    {
        let store = open_store(directory.path());
        ingest(&store, &fixture, "grant-old", 1);
        store.connection.lock().execute_batch("DROP TRIGGER observation_session_agent_erasure_v1; DROP TABLE observation_session_agents_v1; DROP TABLE observation_agent_grants_v1;").unwrap();
    }
    let store = open_store(directory.path());
    store
        .register_agent_grant(
            &fixture.workspace,
            "grant-old",
            1,
            "connection",
            "agent_claude_default",
        )
        .unwrap();
    assert_eq!(agent(&store, &fixture), "");
    let schema: String = store
        .connection
        .lock()
        .query_row(
            "SELECT value FROM observation_meta WHERE key='schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(schema, "5");
}
