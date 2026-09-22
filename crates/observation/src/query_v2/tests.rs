use super::*;
use crate::WriterCycleOutcome;
use crate::{DigestAuthority, LocalObservationStore};
use hiroute_domain::{ObservationRoutingContextNameStateV1, ObservationRoutingContextStateV1};

fn open(root: &std::path::Path) -> LocalObservationStore {
    LocalObservationStore::open(root, DigestAuthority::new([3; 32])).unwrap()
}

fn seed(store: &LocalObservationStore, id: &str, time: i64) {
    let connection = store.connection.lock();
    connection.execute(
        "INSERT OR IGNORE INTO sessions(workspace_id,session_id,started_at_ms,updated_at_ms) VALUES(?1,'session',1,1)",
        [WorkspaceId::DEFAULT],
    ).unwrap();
    connection.execute(
        "INSERT INTO logical_requests(workspace_id,request_id,session_id,turn_id,traffic_kind,started_at_ms) VALUES(?1,?2,'session','generated-turn','unknown',?3)",
        rusqlite::params![WorkspaceId::DEFAULT,id,time],
    ).unwrap();
}

fn link(request: &str, run: &str) -> RunObservationLink {
    RunObservationLink {
        workspace_id: WorkspaceId::default(),
        request_id: LogicalRequestId::parse(request).unwrap(),
        task_id: format!("task-{run}"),
        run_id: run.into(),
        producer_epoch: "epoch".into(),
        source_event_id: format!("event-{request}"),
        plan_id: "plan/test".into(),
        plan_revision: "1".into(),
        publication_ref: "publication".into(),
        harness_id: "codex".into(),
        protocol_kind: "acp".into(),
        native_session_id: None,
        native_turn_id: None,
        parent_context_ref: None,
        continued_from_run_id: None,
    }
}

fn reader(run: Option<&str>) -> ObservationReaderContext {
    match run {
        Some(run) => ObservationReaderContext::run_scoped(
            WorkspaceId::default(),
            "worker".into(),
            1,
            10000,
            [run.to_owned()].into_iter().collect(),
            false,
            false,
        )
        .unwrap(),
        None => ObservationReaderContext::local_user(
            WorkspaceId::default(),
            "user".into(),
            1,
            10000,
            false,
            false,
        )
        .unwrap(),
    }
}

fn query(limit: u16) -> ObservationRequestQuery {
    ObservationRequestQuery {
        from_ms: 0,
        to_ms: 1000,
        session_id: None,
        request_id: None,
        limit,
        cursor: None,
        agent_id: None,
        plan_id: None,
        native_model: None,
        outcome: None,
        only_model_switch: false,
    }
}

#[test]
fn request_pages_are_bounded_and_freeze_insertion_watermark() {
    let directory = tempfile::tempdir().unwrap();
    let store = open(directory.path());
    for index in 0..251 {
        seed(&store, &format!("request-{index:03}"), 100);
    }
    let first = store
        .observed_requests(&reader(None), &query(200), 500)
        .unwrap();
    assert_eq!(first.requests.len(), 200);
    assert!(
        first
            .requests
            .iter()
            .all(|request| request.native_turn_id.is_none())
    );
    assert!(first.requests.iter().all(|request| {
        request.routing_context.state == ObservationRoutingContextStateV1::Unavailable
            && request.routing_context.plan_id.is_none()
            && request.routing_context.display_name.is_none()
    }));
    seed(&store, "late-insertion", 100);
    let mut next = query(200);
    next.cursor = first.next_cursor;
    let second = store.observed_requests(&reader(None), &next, 500).unwrap();
    assert_eq!(second.requests.len(), 51);
    assert!(second.next_cursor.is_none());
    assert!(
        !second
            .requests
            .iter()
            .any(|request| request.request_id == "late-insertion")
    );
    assert_eq!(
        store.observed_requests(&reader(None), &query(201), 500),
        Err(ObservationV2Error::Invalid)
    );
}

#[test]
fn exact_request_locator_is_independent_of_timeline_page_position() {
    let directory = tempfile::tempdir().unwrap();
    let store = open(directory.path());
    for index in 0..80 {
        seed(&store, &format!("request-{index:03}"), 100 + index);
    }
    let mut exact = query(1);
    exact.session_id = Some("session".into());
    exact.request_id = Some("request-073".into());
    let page = store.observed_timeline(&reader(None), &exact, 500).unwrap();
    assert_eq!(page.requests.len(), 1);
    assert_eq!(page.requests[0].request_id, "request-073");
    assert!(page.next_cursor.is_none());
}

#[test]
fn run_scope_and_cursor_signature_prevent_scope_expansion() {
    let directory = tempfile::tempdir().unwrap();
    let store = open(directory.path());
    for (id, run) in [("a1", "a"), ("a2", "a"), ("b1", "b")] {
        seed(&store, id, 100);
        store.link_observed_request(&link(id, run)).unwrap();
    }
    let first = store
        .observed_requests(&reader(Some("a")), &query(1), 500)
        .unwrap();
    assert_eq!(first.requests[0].run_id.as_deref(), Some("a"));
    let mut next = query(1);
    next.cursor = first.next_cursor.clone();
    assert_eq!(
        store.observed_requests(&reader(Some("b")), &next, 500),
        Err(ObservationV2Error::Stale)
    );
    assert_eq!(
        store.observed_request_link(
            &reader(Some("a")),
            &LogicalRequestId::parse("b1").unwrap(),
            500
        ),
        Err(ObservationV2Error::Unauthorized)
    );
    let cursor = next.cursor.as_mut().unwrap();
    cursor.replace_range(..1, "z");
    assert_eq!(
        store.observed_requests(&reader(Some("a")), &next, 500),
        Err(ObservationV2Error::Invalid)
    );
    assert_eq!(
        store.observed_requests(&reader(Some("a")), &query(1), 10000),
        Err(ObservationV2Error::Unauthorized)
    );
}

#[test]
fn relation_replay_is_idempotent_and_conflicts_are_quarantined() {
    let directory = tempfile::tempdir().unwrap();
    let store = open(directory.path());
    seed(&store, "request", 1);
    let original = link("request", "a");
    store.link_observed_request(&original).unwrap();
    store.link_observed_request(&original).unwrap();
    let mut duplicate = original.clone();
    duplicate.source_event_id = "second-event".into();
    store.link_observed_request(&duplicate).unwrap();
    let mut conflicting = original.clone();
    conflicting.run_id = "b".into();
    assert_eq!(
        store.link_observed_request(&conflicting),
        Err(ObservationV2Error::RelationshipConflict)
    );
    assert_eq!(
        store
            .observed_requests(&reader(Some("a")), &query(50), 500)
            .unwrap()
            .requests
            .len(),
        0
    );
    assert_eq!(
        store
            .observed_requests(&reader(Some("b")), &query(50), 500)
            .unwrap()
            .requests
            .len(),
        0
    );
    let local = store
        .observed_requests(&reader(None), &query(50), 500)
        .unwrap();
    assert!(local.requests[0].relation_conflicted);
    assert!(local.requests[0].run_id.is_none());
    let traffic: String = store
        .connection
        .lock()
        .query_row(
            "SELECT traffic_kind FROM logical_requests WHERE workspace_id=?1 AND request_id='request'",
            [WorkspaceId::DEFAULT],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(traffic, "unknown");
}

#[test]
fn timeline_exposes_bounded_parent_navigation_only_for_a_verified_relation() {
    let directory = tempfile::tempdir().unwrap();
    let store = open(directory.path());
    seed(&store, "request", 1);
    let mut relation = link("request", "run");
    relation.parent_context_ref = Some("parent/navigation".into());
    store.link_observed_request(&relation).unwrap();
    let traffic: String = store
        .connection
        .lock()
        .query_row(
            "SELECT traffic_kind FROM logical_requests WHERE workspace_id=?1 AND request_id='request'",
            [WorkspaceId::DEFAULT],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(traffic, "normal");
    let mut timeline = query(50);
    timeline.session_id = Some("session".into());
    let page = store
        .observed_timeline(&reader(None), &timeline, 500)
        .unwrap();
    assert_eq!(page.requests.len(), 1);
    assert_eq!(page.requests[0].run_id.as_deref(), Some("run"));
    assert_eq!(
        page.requests[0].parent_context_ref.as_deref(),
        Some("parent/navigation")
    );
    assert_eq!(
        page.requests[0].routing_context.state,
        ObservationRoutingContextStateV1::Recorded
    );
    assert_eq!(
        page.requests[0].routing_context.plan_id.as_deref(),
        Some("plan/test")
    );
    assert_eq!(
        page.requests[0].routing_context.plan_revision.as_deref(),
        Some("1")
    );
    assert!(page.requests[0].routing_context.display_name.is_none());
    assert_eq!(
        page.requests[0].routing_context.name_state,
        ObservationRoutingContextNameStateV1::Unavailable
    );
}

#[test]
fn malformed_receipt_downgrades_routing_context_without_exposing_its_body() {
    let directory = tempfile::tempdir().unwrap();
    let store = open(directory.path());
    seed(&store, "damaged-receipt", 100);
    store
        .connection
        .lock()
        .execute(
            "INSERT INTO routing_receipts(workspace_id,receipt_id,session_id,turn_id,request_id,body_json,body_digest,frozen_at_ms) VALUES(?1,'receipt-damaged','session','generated-turn','damaged-receipt','{\"managed_sensitive_ref\":\"receipt:sha256:invalid\",\"extra\":true}','sha256:invalid',100)",
            [WorkspaceId::DEFAULT],
        )
        .unwrap();
    let mut timeline = query(50);
    timeline.session_id = Some("session".into());
    let page = store
        .observed_timeline(&reader(None), &timeline, 500)
        .unwrap();
    assert_eq!(page.requests.len(), 1);
    assert_eq!(
        page.requests[0].routing_context,
        hiroute_domain::ObservationRoutingContextV1::unavailable()
    );
}

#[test]
fn deletion_invalidates_cursor_and_expired_detail_never_reappears() {
    let directory = tempfile::tempdir().unwrap();
    let store = open(directory.path());
    seed(&store, "a", 10);
    seed(&store, "b", 10);
    let first = store
        .observed_requests(&reader(None), &query(1), 500)
        .unwrap();
    super::invalidate_visibility(&store.connection.lock()).unwrap();
    let mut next = query(1);
    next.cursor = first.next_cursor;
    assert_eq!(
        store.observed_requests(&reader(None), &next, 500),
        Err(ObservationV2Error::Stale)
    );
    let long_reader = ObservationReaderContext::local_user(
        WorkspaceId::default(),
        "user".into(),
        1,
        i64::MAX,
        false,
        false,
    )
    .unwrap();
    assert!(
        store
            .observed_requests(
                &long_reader,
                &query(50),
                10 + crate::managed_text::RETENTION_MS
            )
            .unwrap()
            .requests
            .is_empty()
    );
}

#[test]
fn query_workers_are_bounded_and_released_on_drop() {
    let directory = tempfile::tempdir().unwrap();
    let store = open(directory.path());
    let permits = (0..4)
        .map(|_| store.query_permit().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        store.observed_requests(&reader(None), &query(50), 500),
        Err(ObservationV2Error::Busy)
    );
    drop(permits);
    // Prove that all worker slots were returned, independently of SQLite/OS query
    // execution and its separate deadline. Public query tests cover successful reads.
    let mut reacquired = (0..4)
        .map(|_| {
            store
                .query_permit()
                .expect("dropped worker slot must be reusable")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        store.observed_requests(&reader(None), &query(50), 500),
        Err(ObservationV2Error::Busy)
    );
    drop(reacquired.pop());
    let replacement = store
        .query_permit()
        .expect("one released slot must be reusable");
    assert!(matches!(
        store.query_permit(),
        Err(ObservationV2Error::Busy)
    ));
    drop(replacement);
    drop(reacquired);
}

#[test]
fn legacy_reads_use_independent_connections_and_share_worker_limits() {
    use hiroute_domain::ObservationQueryPort;
    let directory = tempfile::tempdir().unwrap();
    let store = open(directory.path());
    let writer_guard = store.connection.lock();
    assert!(store.get_status(&WorkspaceId::default()).is_ok());
    drop(writer_guard);
    let permits = (0..4)
        .map(|_| store.query_permit().unwrap())
        .collect::<Vec<_>>();
    assert!(matches!(
        store.get_status(&WorkspaceId::default()),
        Err(hiroute_domain::ObservationQueryError::Unavailable)
    ));
    drop(permits);
    assert!(store.get_status(&WorkspaceId::default()).is_ok());
}

#[test]
fn legacy_list_refuses_an_unbounded_projection_instead_of_silently_truncating() {
    use hiroute_domain::{ObservationQueryError, ObservationQueryPort, SessionListQueryV1};
    let directory = tempfile::tempdir().unwrap();
    let store = open(directory.path());
    {
        let connection = store.connection.lock();
        for index in 0..201 {
            connection.execute("INSERT INTO sessions(workspace_id,session_id,started_at_ms,updated_at_ms) VALUES(?1,?2,100,100)",rusqlite::params![WorkspaceId::DEFAULT,format!("session-{index}")]).unwrap();
        }
    }
    let query = SessionListQueryV1 {
        include_unlinked: true,
        ..Default::default()
    };
    assert!(matches!(
        store.list_sessions(&WorkspaceId::default(), &query),
        Err(ObservationQueryError::InvalidQuery)
    ));
}

#[test]
fn model_filter_uses_started_native_models_and_switch_scope_is_one_request() {
    let directory = tempfile::tempdir().unwrap();
    let store = open(directory.path());
    for id in ["single-a", "single-b", "fallback"] {
        seed(&store, id, 100);
    }
    {
        let connection = store.connection.lock();
        for (request, ordinal, model) in [
            ("single-a", 1, "a"),
            ("single-b", 1, "b"),
            ("fallback", 1, "a"),
            ("fallback", 2, "a"),
            ("fallback", 3, "b"),
        ] {
            connection.execute("INSERT INTO observation_attempt_models_v2(workspace_id,request_id,ordinal,model_id) VALUES(?1,?2,?3,?4)",rusqlite::params![WorkspaceId::DEFAULT,request,ordinal,model]).unwrap();
        }
    }
    let mut filter = query(100);
    filter.only_model_switch = true;
    let page = store
        .observed_requests(&reader(None), &filter, 500)
        .unwrap();
    assert_eq!(page.requests.len(), 1);
    assert_eq!(page.requests[0].request_id, "fallback");
    assert_eq!(page.requests[0].attempted_model_count, 2);
    filter.only_model_switch = false;
    filter.native_model = Some("b".into());
    let page = store
        .observed_requests(&reader(None), &filter, 500)
        .unwrap();
    assert_eq!(page.requests.len(), 2);
    assert!(
        page.requests
            .iter()
            .all(|request| request.final_native_model.is_none())
    );
}

#[test]
fn query_deadline_interrupts_long_sql_without_a_writer_dependency() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    let started = std::time::Instant::now();
    let deadline = super::QueryDeadline::start(&connection).unwrap();
    let result=connection.query_row("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n",[],|row|row.get::<_,i64>(0));
    assert!(result.is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    drop(deadline);
    assert_eq!(
        connection
            .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn session_counts_and_chronological_pages_share_authorized_scope() {
    let directory = tempfile::tempdir().unwrap();
    let store = open(directory.path());
    for (id, time, run) in [
        ("later", 300, "a"),
        ("earlier", 100, "a"),
        ("hidden", 900, "b"),
    ] {
        seed(&store, id, time);
        store.link_observed_request(&link(id, run)).unwrap();
    }
    let scope = reader(Some("a"));
    let sessions = store.observed_sessions(&scope, &query(50), 500).unwrap();
    assert_eq!(sessions.sessions.len(), 1);
    assert_eq!(sessions.sessions[0].request_count, 2);
    assert_eq!(sessions.sessions[0].last_request_at_ms, 300);
    assert_eq!(
        sessions.sessions[0].correlation_kind,
        hiroute_domain::ObservationSessionCorrelationKindV1::Unknown
    );
    let mut timeline = query(1);
    timeline.session_id = Some("session".into());
    let first = store.observed_timeline(&scope, &timeline, 500).unwrap();
    assert_eq!(first.requests[0].request_id, "earlier");
    timeline.cursor = first.next_cursor;
    assert_eq!(
        store.observed_requests(&scope, &timeline, 500),
        Err(ObservationV2Error::Stale)
    );
    let second = store.observed_timeline(&scope, &timeline, 500).unwrap();
    assert_eq!(second.requests[0].request_id, "later");
    assert!(second.next_cursor.is_none());
}

#[test]
fn timeline_uses_each_frozen_route_and_safe_projection_survives_sensitive_deletion() {
    use crate::store::tests::support::{
        Fixture, fact_channel, finished_with_value, offer_fact, request_facts, writer,
    };

    let directory = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(open(directory.path()));
    let first = Fixture::new("route-alpha");
    let mut second = Fixture::new("route-beta");
    second.session = first.session.clone();
    let mut legacy = Fixture::new("route-legacy");
    legacy.session = first.session.clone();
    let mut conflicted = Fixture::new("route-conflicted");
    conflicted.session = first.session.clone();

    for (fixture, plan_id, display_name, occurred_at_ms) in [
        (&first, "plan/alpha", Some("Alpha route"), 100),
        (&second, "plan/beta", Some("Beta route"), 200),
        (&legacy, "plan/legacy", None, 300),
        (&conflicted, "plan/receipt", Some("Receipt route"), 400),
    ] {
        let writer = writer(&store);
        let channel = fact_channel(fixture, 512 * 1024);
        let attempt_id = format!("attempt-{}", plan_id.replace('/', "-"));
        let mut facts = request_facts(
            fixture,
            &attempt_id,
            finished_with_value(&attempt_id, plan_id, "USD", Some(100), Some(90), Some(80)),
            occurred_at_ms,
        );
        for fact in &mut facts {
            fact.trust.plan_display_name = display_name.map(str::to_owned);
        }
        for fact in facts {
            assert!(matches!(
                offer_fact(&writer, &channel, fact),
                WriterCycleOutcome::Ack(_)
            ));
        }
    }

    let mut conflicting_link = link(conflicted.request.as_str(), "run-conflicted");
    conflicting_link.plan_id = "plan/other".into();
    conflicting_link.plan_revision = "17".into();
    store.link_observed_request(&conflicting_link).unwrap();

    // Receipt and raw fact bodies are content-bearing and may be deleted. The independently
    // projected safe routing identity must continue to serve the exact historical context.
    store
        .connection
        .lock()
        .execute("DELETE FROM observation_sensitive_payloads_v2", [])
        .unwrap();

    let mut timeline = query(50);
    timeline.session_id = Some(first.session.to_string());
    let page = store
        .observed_timeline(&reader(None), &timeline, 500)
        .unwrap();
    assert_eq!(page.requests.len(), 4);
    for (request, expected_plan, expected_name) in [
        (&page.requests[0], "plan/alpha", Some("Alpha route")),
        (&page.requests[1], "plan/beta", Some("Beta route")),
    ] {
        assert_eq!(
            request.routing_context.state,
            ObservationRoutingContextStateV1::Recorded
        );
        assert_eq!(
            request.routing_context.plan_id.as_deref(),
            Some(expected_plan)
        );
        assert_eq!(
            request.routing_context.display_name.as_deref(),
            expected_name
        );
        assert_eq!(
            request.routing_context.name_state,
            ObservationRoutingContextNameStateV1::Recorded
        );
    }
    assert_eq!(
        page.requests[2].routing_context.state,
        ObservationRoutingContextStateV1::Recorded
    );
    assert_eq!(
        page.requests[2].routing_context.plan_id.as_deref(),
        Some("plan/legacy")
    );
    assert!(page.requests[2].routing_context.display_name.is_none());
    assert_eq!(
        page.requests[2].routing_context.name_state,
        ObservationRoutingContextNameStateV1::Unavailable
    );
    assert_eq!(
        page.requests[3].routing_context.state,
        ObservationRoutingContextStateV1::Conflicted
    );
    assert!(page.requests[3].routing_context.plan_id.is_none());
    assert!(page.requests[3].routing_context.display_name.is_none());

    let public_facts = store
        .observed_facts(
            &reader(None),
            &hiroute_domain::ObservationFactsQueryV2 {
                request_id: first.request.clone(),
                limit: 50,
                cursor: None,
            },
            500,
        )
        .unwrap();
    assert!(
        public_facts
            .facts
            .iter()
            .all(|fact| fact.routing_context.is_none())
    );
}

#[test]
fn session_correlation_kind_uses_only_visible_request_facts() {
    use hiroute_domain::ObservationSessionCorrelationKindV1 as Kind;

    let directory = tempfile::tempdir().unwrap();
    let store = open(directory.path());
    let insert = |session: &str, request: &str, scope: &str, provenance: &str| {
        let connection = store.connection.lock();
        connection
            .execute(
                "INSERT OR IGNORE INTO sessions(workspace_id,session_id,started_at_ms,updated_at_ms) VALUES(?1,?2,1,1)",
                rusqlite::params![WorkspaceId::DEFAULT, session],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO logical_requests(workspace_id,request_id,session_id,turn_id,session_scope,correlation_provenance,traffic_kind,started_at_ms) VALUES(?1,?2,?3,?4,?5,?6,'normal',100)",
                rusqlite::params![
                    WorkspaceId::DEFAULT,
                    request,
                    session,
                    format!("turn-{request}"),
                    scope,
                    provenance
                ],
            )
            .unwrap();
    };
    insert("agent", "agent-request", "conversation", "agent_supplied");
    insert("worker", "worker-request", "conversation", "protocol_state");
    insert(
        "inferred",
        "inferred-request",
        "conversation",
        "gateway_generated",
    );
    insert("request", "request-request", "request_scoped", "unproven");
    insert("mixed", "mixed-a", "conversation", "agent_supplied");
    insert("mixed", "mixed-b", "conversation", "gateway_generated");
    insert(
        "conflicted",
        "conflicted-request",
        "conversation",
        "protocol_state",
    );
    store
        .link_observed_request(&link("worker-request", "worker-run"))
        .unwrap();
    let original = link("conflicted-request", "run-a");
    store.link_observed_request(&original).unwrap();
    let mut conflicting = original;
    conflicting.run_id = "run-b".into();
    assert_eq!(
        store.link_observed_request(&conflicting),
        Err(ObservationV2Error::RelationshipConflict)
    );

    let page = store
        .observed_sessions(&reader(None), &query(50), 500)
        .unwrap();
    let kinds = page
        .sessions
        .into_iter()
        .map(|session| (session.session_id, session.correlation_kind))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(kinds["agent"], Kind::AgentSupplied);
    assert_eq!(kinds["worker"], Kind::VerifiedWorker);
    assert_eq!(kinds["inferred"], Kind::Inferred);
    assert_eq!(kinds["request"], Kind::RequestScoped);
    assert_eq!(kinds["mixed"], Kind::Unknown);
    assert_eq!(kinds["conflicted"], Kind::Unknown);

    let worker_only = store
        .observed_sessions(&reader(Some("worker-run")), &query(50), 500)
        .unwrap();
    assert_eq!(worker_only.sessions.len(), 1);
    assert_eq!(
        worker_only.sessions[0].correlation_kind,
        Kind::VerifiedWorker
    );
}

#[test]
fn cancelled_reader_rejects_new_queries_without_cancelling_store() {
    let directory = tempfile::tempdir().unwrap();
    let store = open(directory.path());
    seed(&store, "request", 100);
    let cancelled = reader(None);
    cancelled.cancel();
    assert_eq!(
        store.observed_sessions(&cancelled, &query(10), 500),
        Err(ObservationV2Error::Unavailable)
    );
    assert_eq!(
        store
            .observed_sessions(&reader(None), &query(10), 500)
            .unwrap()
            .sessions
            .len(),
        1
    );
}

#[test]
fn unified_preview_clears_managed_run_content_and_retries_same_job() {
    use crate::managed_text::*;
    use hiroute_domain::*;
    let directory = tempfile::tempdir().unwrap();
    let store = open(directory.path());
    seed(&store, "request", 100);
    store.link_observed_request(&link("request", "a")).unwrap();
    let scope = ManagedTextScope {
        workspace_id: WorkspaceId::default(),
        task_id: "task-a".into(),
        run_id: "a".into(),
    };
    let reference = store
        .managed_text_put(
            &ManagedTextInput {
                scope: scope.clone(),
                purpose: ManagedTextPurpose::Result,
                source_event_id: "result".into(),
                source_revision: 1,
                original_created_at_ms: 100,
                import_origin: None,
            },
            200,
        )
        .unwrap();
    let principal = ObservationPrincipalV1::local_user(WorkspaceId::default());
    let spec = SessionDeletionSpecV1 {
        workspace_id: WorkspaceId::default(),
        session_id: SessionId::parse("session").unwrap(),
        data_class: DeletionDataClass::ContentOnly,
        delete_rollups: false,
    };
    assert_eq!(
        store.preview_session_deletion(&principal, &spec),
        Err(ObservationQueryError::InvalidQuery)
    );
    let preview = store
        .preview_session_deletion_v2(&principal, &spec, 200)
        .unwrap();
    assert_eq!(preview.managed_scopes.len(), 1);
    assert_eq!(preview.managed_scopes[0].reference_count, 1);
    let outcome = store
        .apply_session_deletion_v2(&principal, &preview, &preview.change_digest, 201)
        .unwrap();
    assert!(outcome.managed_native_cleanup_pending);
    assert_eq!(
        store
            .managed_text_resolve(&scope, &reference, 201)
            .unwrap()
            .state,
        ManagedTextState::Deleted
    );
    let again = store
        .apply_session_deletion_v2(&principal, &preview, &preview.change_digest, 202)
        .unwrap();
    assert_eq!(
        again.session.new_store_revision,
        outcome.session.new_store_revision
    );
}

#[test]
fn cancellation_interrupts_an_active_query_and_later_statements() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    let scope = reader(None);
    let deadline = super::QueryDeadline::start_for(&connection, &scope).unwrap();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        scope.cancel();
    });
    for _ in 0..2 {
        let started = std::time::Instant::now();
        let result = connection.query_row("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n", [], |r| r.get::<_, i64>(0));
        assert!(result.is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }
    canceller.join().unwrap();
    drop(deadline);
}

#[test]
fn adjacent_native_turn_models_survive_pages_and_filters_without_guessing_gaps() {
    let directory = tempfile::tempdir().unwrap();
    let store = open(directory.path());
    for (request, time, turn, model) in [
        ("a", 100, "turn-a", "model-a"),
        ("b-first", 200, "turn-b", "model-a"),
        ("b-final", 250, "turn-b", "model-b"),
        ("c", 300, "turn-c", "model-b"),
    ] {
        seed(&store, request, time);
        let mut relation = link(request, "a");
        relation.native_session_id = Some("native".into());
        relation.native_turn_id = Some(turn.into());
        store.link_observed_request(&relation).unwrap();
        let db = store.connection.lock();
        db.execute("INSERT INTO valuation_requests_v2(workspace_id,request_id,session_id,plan_id,started_ms,input_revision,input_digest,terminal,accepted_ordinal,partial) VALUES(?1,?2,'session','plan',?3,1,'digest',1,1,0)",rusqlite::params![WorkspaceId::DEFAULT,request,time]).unwrap();
        db.execute("INSERT INTO observation_attempt_models_v2(workspace_id,request_id,ordinal,model_id) VALUES(?1,?2,1,?3)",rusqlite::params![WorkspaceId::DEFAULT,request,model]).unwrap();
    }
    let mut q = query(1);
    q.session_id = Some("session".into());
    let first = store
        .observed_timeline(&reader(Some("a")), &q, 500)
        .unwrap();
    assert_eq!(first.requests[0].between_turn_model_change, None);
    q.cursor = first.next_cursor;
    let second = store
        .observed_timeline(&reader(Some("a")), &q, 500)
        .unwrap();
    assert_eq!(second.requests[0].request_id, "b-first");
    assert_eq!(second.requests[0].between_turn_model_change, Some(true));
    assert_eq!(
        second.requests[0].turn_final_native_model.as_deref(),
        Some("model-b")
    );
    assert_eq!(second.requests[0].within_request_fallback, Some(false));
    q.cursor = None;
    q.limit = 50;
    q.only_model_switch = true;
    q.native_model = Some("model-b".into());
    let filtered = store
        .observed_timeline(&reader(Some("a")), &q, 500)
        .unwrap();
    assert_eq!(filtered.requests.len(), 1);
    assert_eq!(filtered.requests[0].request_id, "b-final");
    assert_eq!(
        filtered.requests[0].previous_native_turn_id.as_deref(),
        Some("turn-a")
    );
    assert_eq!(
        store
            .observed_sessions(&reader(Some("a")), &q, 500)
            .unwrap()
            .sessions
            .len(),
        1
    );
    assert!(
        store
            .observed_timeline(&reader(Some("other")), &q, 500)
            .unwrap()
            .requests
            .is_empty()
    );
    // A missing identity in the observed interval breaks adjacency rather than
    // silently joining two otherwise complete turns across the unknown request.
    seed(&store, "unlinked", 150);
    q.native_model = None;
    assert!(
        store
            .observed_timeline(&reader(None), &q, 500)
            .unwrap()
            .requests
            .is_empty()
    );
    store
        .connection
        .lock()
        .execute(
            "UPDATE valuation_requests_v2 SET partial=1 WHERE request_id='b-final'",
            [],
        )
        .unwrap();
    assert!(
        store
            .observed_timeline(&reader(Some("a")), &q, 500)
            .unwrap()
            .requests
            .is_empty()
    );
}
