use super::support::{Fixture, open_store, route_decision};
use crate::query_v2::ObservationReaderContext;
use crate::writer::ObservationCommitPort;
use hiroute_domain::{ExecutionFactV1, ObservationRequestQuery, ObservedRequestSummary};
use serde_json::Value;

fn reader(fixture: &Fixture) -> ObservationReaderContext {
    ObservationReaderContext::local_user(
        fixture.workspace.clone(),
        "local".into(),
        1,
        20_000,
        false,
        false,
    )
    .unwrap()
}

fn query(fixture: &Fixture) -> ObservationRequestQuery {
    ObservationRequestQuery {
        from_ms: 0,
        to_ms: 10_000,
        session_id: Some(fixture.session.to_string()),
        request_id: None,
        limit: 1,
        cursor: None,
        agent_id: None,
        plan_id: None,
        native_model: None,
        outcome: None,
        only_model_switch: false,
    }
}

fn auxiliary(fixture: &Fixture) -> hiroute_domain::ExecutionFactEnvelopeV1 {
    let mut fact = route_decision("plan/codex-daily");
    if let ExecutionFactV1::RouteDecision(route) = &mut fact {
        route.native_auxiliary = true;
    }
    fixture.fact(1, fact, 1000)
}

fn marked(request: &ObservedRequestSummary) -> bool {
    serde_json::to_value(request)
        .unwrap()
        .get("native_auxiliary")
        == Some(&Value::Bool(true))
}

#[test]
fn request_purpose_marks_auxiliary_without_hiding_raw_paginated_requests() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let title = Fixture::new("first-title");
    let mut main = Fixture::new("later-main");
    main.session = title.session.clone();
    store.ingest_fact(&auxiliary(&title), &[]).unwrap();
    store
        .ingest_fact(&main.fact(1, route_decision("plan/codex-daily"), 2000), &[])
        .unwrap();
    let mut query = query(&title);
    let first = store
        .observed_timeline(&reader(&title), &query, 5000)
        .unwrap();
    assert_eq!(first.requests.len(), 1);
    assert_eq!(first.requests[0].request_id, title.request.as_str());
    assert!(
        marked(&first.requests[0]),
        "trusted native auxiliary purpose must reach the raw request view"
    );
    query.cursor = first.next_cursor;
    assert!(query.cursor.is_some());
    let second = store
        .observed_timeline(&reader(&title), &query, 5000)
        .unwrap();
    assert_eq!(second.requests.len(), 1);
    assert_eq!(second.requests[0].request_id, main.request.as_str());
    assert!(!marked(&second.requests[0]));
    assert!(second.next_cursor.is_none());
    query.cursor = None;
    query.request_id = Some(title.request.to_string());
    let exact = store
        .observed_requests(&reader(&title), &query, 5000)
        .unwrap();
    assert_eq!(
        exact.requests.len(),
        1,
        "auxiliary request remains independently navigable"
    );
    assert!(marked(&exact.requests[0]));
}

#[test]
fn request_purpose_missing_or_oversized_carriers_remain_unknown_and_visible() {
    for missing in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let store = open_store(directory.path());
        let title = Fixture::new("unavailable-title-purpose");
        store.ingest_fact(&auxiliary(&title), &[]).unwrap();
        if missing {
            store
                .connection
                .lock()
                .execute(
                    "DELETE FROM observation_sensitive_payloads_v2 WHERE request_id=?1",
                    [title.request.as_str()],
                )
                .unwrap();
        } else {
            let huge = serde_json::json!({"fact":{"kind":"route_decision","native_auxiliary":true},"unknown":"x".repeat(1024 * 1024)});
            store
                .connection
                .lock()
                .execute(
                    "UPDATE observation_sensitive_payloads_v2 SET body_json=?2 WHERE request_id=?1",
                    rusqlite::params![title.request.as_str(), huge.to_string()],
                )
                .unwrap();
        }
        let page = store
            .observed_timeline(&reader(&title), &query(&title), 5000)
            .unwrap();
        assert_eq!(page.requests.len(), 1);
        assert!(
            !marked(&page.requests[0]),
            "no proof must not become an affirmative auxiliary marker"
        );
    }
}

#[test]
fn request_purpose_ambiguous_route_carriers_remain_unknown() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let title = Fixture::new("ambiguous-title-purpose");
    store.ingest_fact(&auxiliary(&title), &[]).unwrap();
    // A damaged duplicate index row is not a second trusted purpose witness.
    store.connection.lock().execute(
        "INSERT INTO observation_safe_facts_v2(workspace_id,request_id,original_digest,body_json) SELECT workspace_id,request_id,original_digest||'-ambiguous',body_json FROM observation_safe_facts_v2 WHERE request_id=?1",
        [title.request.as_str()],
    ).unwrap();
    let page = store
        .observed_timeline(&reader(&title), &query(&title), 5000)
        .unwrap();
    assert_eq!(page.requests.len(), 1);
    assert!(!marked(&page.requests[0]));
}

#[test]
fn request_purpose_absence_keeps_legacy_request_dtos_readable() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let legacy = Fixture::new("legacy-purpose-absent");
    store
        .ingest_fact(
            &legacy.fact(1, route_decision("plan/codex-daily"), 1000),
            &[],
        )
        .unwrap();
    let page = store
        .observed_timeline(&reader(&legacy), &query(&legacy), 5000)
        .unwrap();
    let mut bytes = serde_json::to_value(&page.requests[0]).unwrap();
    bytes.as_object_mut().unwrap().remove("native_auxiliary");
    let old: ObservedRequestSummary = serde_json::from_value(bytes).unwrap();
    assert!(!marked(&old));
}
