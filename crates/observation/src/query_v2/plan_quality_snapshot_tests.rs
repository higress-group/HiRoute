//! Concurrent production ingestion is not a loss of read authority.
use super::*;
use crate::WriterCycleOutcome;
use crate::store::tests::support::{Fixture, fact_channel, offer_fact, open_store, writer};
use hiroute_domain::*;

fn append(store: &std::sync::Arc<LocalObservationStore>, name: &str) {
    let f = Fixture::new(name);
    let channel = fact_channel(&f, 256 * 1024);
    let fact = ExecutionFactV1::AgentTurnFinished {
        history_membership: None,
        opening_cause: None,
        branch_execution: None,
        agent_turn_id: name.into(),
        segment_id: name.into(),
        ordinal: 1,
        plan_id: AgentPlanId::parse("plan/test").unwrap(),
        plan_revision: 1,
        selected_branch_id: "regular".into(),
        executed_branch_id: Some("regular".into()),
        model_configuration_id: Some("model/test".into()),
        profile_digest: Some(ObservedProfileDigestV1::from(CanonicalDigest::of_bytes(
            b"profile",
        ))),
        attribution: AgentTurnAttributionV1::Single,
        started_at_ms: 1000,
        finished_at_ms: 1100,
        status: AgentTurnStatusV1::Completed,
        history_partial: false,
        first_request_id: Some(f.request.clone()),
        last_request_id: Some(f.request.clone()),
    };
    assert!(matches!(
        offer_fact(
            &writer(store),
            &channel,
            f.fact_for_plan(1, fact, 1100, "plan/test", None)
        ),
        WriterCycleOutcome::Ack(_)
    ));
}

fn reader() -> ObservationReaderContext {
    ObservationReaderContext::local_user(
        WorkspaceId::default(),
        "reader".into(),
        1,
        10000,
        false,
        false,
    )
    .unwrap()
}

fn query() -> PlanQualitySamplesQuery {
    serde_json::from_value(serde_json::json!({"plan_id":"plan/test", "limit":1})).unwrap()
}

#[test]
fn fresh_quality_read_returns_one_consistent_snapshot_despite_concurrent_ingestion() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(root.path());
    append(&store, "first");
    append(&store, "second");
    let before = store.observed_trust_generation(&reader(), 5000).unwrap();
    let page = store
        .plan_quality_samples_inner(&reader(), &query(), 5000, || {
            append(&store, "third");
        })
        .unwrap();
    let after = store.observed_trust_generation(&reader(), 5000).unwrap();
    assert!(
        after.0 > before.0,
        "the real writer advanced quality generation"
    );
    assert_eq!(after.1, before.1, "ordinary ingestion preserves visibility");
    assert_eq!(page.summary.unrated_stage_count, 2);
    assert_eq!(page.samples.len(), 1);
    assert_ne!(page.samples[0].segment_id, "third");
    let mut continuation = query();
    continuation.cursor = page.next_cursor;
    assert!(continuation.cursor.is_some());
    assert!(matches!(
        store.observed_plan_quality_samples(&reader(), &continuation, 5000),
        Err(ObservationV2Error::Stale)
    ));
    assert_eq!(
        store
            .observed_plan_quality_samples(&reader(), &query(), 5000)
            .unwrap()
            .summary
            .unrated_stage_count,
        3
    );
}

#[test]
fn quality_cursor_remains_strict_when_ingestion_finishes_during_the_next_page() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(root.path());
    append(&store, "first");
    append(&store, "second");
    let mut continuation = query();
    continuation.cursor = store
        .observed_plan_quality_samples(&reader(), &query(), 5000)
        .unwrap()
        .next_cursor;
    assert!(continuation.cursor.is_some());
    let result = store.plan_quality_samples_inner(&reader(), &continuation, 5000, || {
        append(&store, "third");
    });
    assert!(matches!(result, Err(ObservationV2Error::Stale)));
}

#[test]
fn fresh_quality_read_never_returns_a_snapshot_after_content_or_fact_deletion() {
    for data_class in [
        DeletionDataClass::ContentOnly,
        DeletionDataClass::FactsAndContent,
    ] {
        let root = tempfile::tempdir().unwrap();
        let store = open_store(root.path());
        append(&store, "first");
        let f = Fixture::new("first");
        let principal = ObservationPrincipalV1::local_user(f.workspace.clone());
        let spec = SessionDeletionSpecV1 {
            workspace_id: f.workspace,
            session_id: f.session,
            data_class,
            delete_rollups: false,
        };
        let preview = store.preview_session_deletion(&principal, &spec).unwrap();
        let result = store.plan_quality_samples_inner(&reader(), &query(), 5000, || {
            store
                .apply_session_deletion(
                    &principal,
                    &spec,
                    preview.store_revision,
                    &preview.change_digest,
                    5000,
                )
                .unwrap();
        });
        assert!(matches!(result, Err(ObservationV2Error::Stale)));
    }
}

#[test]
fn fresh_quality_read_keeps_reader_scope_and_deadline_checks() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(root.path());
    append(&store, "first");
    let expired = reader().with_query_deadline(std::time::Instant::now());
    assert!(matches!(
        store.observed_plan_quality_samples(&expired, &query(), 5000),
        Err(ObservationV2Error::Timeout)
    ));
    assert!(matches!(
        store.observed_plan_quality_samples(&reader(), &query(), 10001),
        Err(ObservationV2Error::Unauthorized)
    ));
    let scoped = ObservationReaderContext::run_scoped(
        WorkspaceId::default(),
        "worker".into(),
        1,
        10000,
        ["different-run".into()].into_iter().collect(),
        false,
        false,
    )
    .unwrap();
    let page = store
        .observed_plan_quality_samples(&scoped, &query(), 5000)
        .unwrap();
    assert!(page.samples.is_empty());
    assert_eq!(page.summary.unrated_stage_count, 0);
}
