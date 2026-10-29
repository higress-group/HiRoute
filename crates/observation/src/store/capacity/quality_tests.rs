//! Logical retirement is the read barrier even while physical cleanup is pending.
use super::*;
use crate::WriterCycleOutcome;
use crate::query_v2::ObservationReaderContext;
use crate::store::tests::support::*;
use hiroute_domain::{
    AgentPlanId, AgentTurnAttributionV1, AgentTurnStatusV1, CanonicalDigest, DeletionDataClass,
    ExecutionFactV1, ObservationPrincipalV1, ObservedProfileDigestV1, PlanQualitySamplesQuery,
    SessionDeletionSpecV1, TrustPopulationQueryV1, TurnId,
};

fn stage(fixture: &Fixture, segment: &str) -> ExecutionFactV1 {
    ExecutionFactV1::AgentTurnFinished {
        history_membership: None,
        opening_cause: None,
        branch_execution: None,
        agent_turn_id: segment.into(),
        segment_id: segment.into(),
        ordinal: 1,
        plan_id: AgentPlanId::parse("plan/codex-daily").unwrap(),
        plan_revision: 17,
        selected_branch_id: "regular".into(),
        executed_branch_id: Some("regular".into()),
        model_configuration_id: Some("model-a".into()),
        profile_digest: Some(ObservedProfileDigestV1::from(CanonicalDigest::of_bytes(
            b"profile-a",
        ))),
        attribution: AgentTurnAttributionV1::Single,
        started_at_ms: 1000,
        finished_at_ms: 1100,
        status: AgentTurnStatusV1::Completed,
        history_partial: false,
        first_request_id: Some(fixture.request.clone()),
        last_request_id: Some(fixture.request.clone()),
    }
}

fn assessment(fixture: &Fixture, segment: &str, score: f64) -> ExecutionFactV1 {
    ExecutionFactV1::BranchAssessmentRecorded {
        history_membership: None,
        segment_id: segment.into(),
        plan_id: AgentPlanId::parse("plan/codex-daily").unwrap(),
        plan_revision: 17,
        model_configuration_id: "model-a".into(),
        profile_digest: ObservedProfileDigestV1::from(CanonicalDigest::of_bytes(b"profile-a")),
        trigger_request_id: fixture.request.clone(),
        target_from_turn_id: TurnId::parse(segment).unwrap(),
        target_through_turn_id: TurnId::parse(segment).unwrap(),
        target_from_ordinal: 1,
        target_through_ordinal: 1,
        assessed_at_ms: 1200,
        score,
        partial: false,
        reason: Some(format!("score-{score}")),
    }
}

fn retirement_barrier(retire_trigger: bool) {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let execution = Fixture::new("quality-retirement-execution");
    let mut trigger = Fixture::new("quality-retirement-trigger");
    let mut control = Fixture::new("quality-retirement-control");
    trigger.session = execution.session.clone();
    control.session = execution.session.clone();
    let writer = writer(&store);
    for fixture in [&execution, &trigger, &control] {
        let channel = fact_channel(fixture, 512 * 1024);
        let attempt = format!("attempt-{}", fixture.request);
        let mut events = request_facts(fixture, &attempt, finished(&attempt), 100);
        let request_finished = events.pop().unwrap().fact;
        for event in events {
            assert!(matches!(
                offer_fact(&writer, &channel, event),
                WriterCycleOutcome::Ack(_)
            ));
        }
        let mut facts = if fixture.request == execution.request {
            vec![stage(fixture, "retiring-stage")]
        } else if fixture.request == trigger.request {
            vec![assessment(fixture, "retiring-stage", 0.6)]
        } else {
            vec![
                stage(fixture, "retained-stage"),
                assessment(fixture, "retained-stage", 0.9),
            ]
        };
        facts.push(request_finished);
        for (index, fact) in facts.into_iter().enumerate() {
            let outcome = offer_fact(
                &writer,
                &channel,
                fixture.fact(10 + index as u64, fact, 1200 + index as i64),
            );
            assert!(matches!(outcome, WriterCycleOutcome::Ack(_)), "{outcome:?}");
        }
    }
    drop(writer);
    let reader = || {
        ObservationReaderContext::local_user(
            execution.workspace.clone(),
            "reader".into(),
            1,
            10_000,
            false,
            false,
        )
        .unwrap()
    };
    let query: PlanQualitySamplesQuery = serde_json::from_value(serde_json::json!({
        "plan_id":"plan/codex-daily", "from_ms":0, "to_ms":10000, "limit":50
    }))
    .unwrap();
    let before = store
        .observed_plan_quality_samples(&reader(), &query, 5000)
        .unwrap();
    assert_eq!(before.samples.len(), 2);
    assert_eq!(before.summary.scored_stage_count, 2);
    let mut paged = query.clone();
    paged.limit = 1;
    let cursor = store
        .observed_plan_quality_samples(&reader(), &paged, 5000)
        .unwrap()
        .next_cursor
        .unwrap();
    let retired = if retire_trigger { &trigger } else { &execution };
    store
        .capacity_retire(
            &mut store.connection.lock(),
            retired.workspace.as_str(),
            retired.session.as_str(),
            retired.request.as_str(),
            1500,
        )
        .unwrap();
    // An expired slice performs no physical work. The committed barrier must
    // already govern fresh reads, including after the process is reopened.
    assert!(
        !store
            .capacity_cleanup(
                &mut store.connection.lock(),
                &mut State::default(),
                Instant::now() - Duration::from_millis(1),
            )
            .unwrap()
    );
    let verify = |store: &LocalObservationStore| {
        let page = store
            .observed_plan_quality_samples(&reader(), &query, 5000)
            .unwrap();
        assert_eq!(page.summary.scored_stage_count, 1);
        assert_eq!(page.summary.unrated_stage_count, u64::from(retire_trigger));
        assert_eq!(page.samples.len(), 1 + usize::from(retire_trigger));
        let retained = page
            .samples
            .iter()
            .find(|sample| sample.segment_id == "retained-stage")
            .unwrap();
        assert_eq!(retained.assessment.as_ref().unwrap().score, 0.9);
        assert_eq!(
            retained,
            before
                .samples
                .iter()
                .find(|sample| sample.segment_id == "retained-stage")
                .unwrap()
        );
        assert_eq!(page.summary.models[0].average_score, Some(0.9));
        if retire_trigger {
            assert!(
                page.samples
                    .iter()
                    .find(|sample| sample.segment_id == "retiring-stage")
                    .unwrap()
                    .assessment
                    .is_none()
            );
        }
        let mut scored = query.clone();
        scored.score_gt = Some(0.5);
        let scored = store
            .observed_plan_quality_samples(&reader(), &scored, 5000)
            .unwrap();
        assert_eq!(scored.samples.len(), 1);
        assert_eq!(scored.samples[0].segment_id, "retained-stage");
        let trust_query = TrustPopulationQueryV1 {
            active_revision: 17,
            plan_id: "plan/codex-daily".into(),
            branch_id: None,
            revision: Some(17),
            from_ms: Some(0),
            to_ms: Some(10_000),
        };
        for lean in [false, true] {
            let population = if lean {
                store.observed_trust_suggestion_population(&reader(), &trust_query, 5000)
            } else {
                store.observed_trust_population(&reader(), &trust_query, 5000)
            }
            .unwrap();
            assert_eq!(population.stages.len(), page.samples.len());
            assert_eq!(
                population
                    .stages
                    .iter()
                    .filter(|stage| stage.sample.assessment.is_some())
                    .count(),
                1
            );
        }
        let db = store.connection.lock();
        assert_eq!(
            db.query_row(
                "SELECT step FROM observation_capacity_pending_v5",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM plan_quality_segments WHERE score IS NOT NULL",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            2,
            "quality projection cleanup has not run"
        );
    };
    verify(&store);
    paged.cursor = Some(cursor);
    assert!(matches!(
        store.observed_plan_quality_samples(&reader(), &paged, 5000),
        Err(crate::query_v2::ObservationV2Error::Stale)
    ));
    drop(store);
    let store = open_store(directory.path());
    verify(&store);
    store.set_capacity_limit(None).unwrap();
    for _ in 0..30 {
        store.capacity.lock().next_due = Instant::now();
        store.capacity_cycle(5000).unwrap();
    }
    assert_eq!(
        store
            .connection
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM observation_capacity_pending_v5",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    let after = store
        .observed_plan_quality_samples(&reader(), &query, 5000)
        .unwrap();
    assert_eq!(after.summary.scored_stage_count, 1);
    assert_eq!(after.summary.unrated_stage_count, u64::from(retire_trigger));
    assert_eq!(after.samples.len(), 1 + usize::from(retire_trigger));
}

#[test]
fn retired_last_request_is_hidden_before_cleanup_and_after_reopen() {
    retirement_barrier(false);
}

#[test]
fn retired_assessment_trigger_is_unrated_before_cleanup_and_after_reopen() {
    retirement_barrier(true);
}

#[test]
fn content_only_tombstone_preserves_quality_and_trust_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("quality-content-only");
    let writer = writer(&store);
    let channel = fact_channel(&fixture, 512 * 1024);
    let mut events = request_facts(&fixture, "content-only", finished("content-only"), 100);
    let request_finished = events.pop().unwrap().fact;
    events.extend([
        fixture.fact(10, stage(&fixture, "retained-stage"), 1100),
        fixture.fact(11, assessment(&fixture, "retained-stage", 0.6), 1200),
        fixture.fact(12, request_finished, 1201),
    ]);
    for event in events {
        let outcome = offer_fact(&writer, &channel, event);
        assert!(matches!(outcome, WriterCycleOutcome::Ack(_)), "{outcome:?}");
    }
    install_content(&store, &fixture, b"delete only these bytes", 1300);
    drop(writer);
    let reader = || {
        ObservationReaderContext::local_user(
            fixture.workspace.clone(),
            "reader".into(),
            1,
            10_000,
            false,
            false,
        )
        .unwrap()
    };
    let query: PlanQualitySamplesQuery = serde_json::from_value(serde_json::json!({
        "plan_id":"plan/codex-daily", "from_ms":0, "to_ms":10000, "limit":50
    }))
    .unwrap();
    let before = store
        .observed_plan_quality_samples(&reader(), &query, 5000)
        .unwrap();
    assert_eq!(before.summary.scored_stage_count, 1);
    let principal = ObservationPrincipalV1::local_user(fixture.workspace.clone());
    let spec = SessionDeletionSpecV1 {
        workspace_id: fixture.workspace.clone(),
        session_id: fixture.session.clone(),
        data_class: DeletionDataClass::ContentOnly,
        delete_rollups: false,
    };
    let preview = store.preview_session_deletion(&principal, &spec).unwrap();
    store
        .apply_session_deletion(
            &principal,
            &spec,
            preview.store_revision,
            &preview.change_digest,
            2000,
        )
        .unwrap();
    // Content-backed reasons and selectors disappear; frozen quality facts,
    // score, identity and execution evidence keep their existing meaning.
    let mut expected = before.clone();
    expected.samples[0].selection = None;
    expected.samples[0].reasoning_profile_id = None;
    expected.samples[0].assessment.as_mut().unwrap().reason = None;
    expected.summary.models[0].reasoning_profile_id = None;
    let verify = |store: &LocalObservationStore| {
        let page = store
            .observed_plan_quality_samples(&reader(), &query, 5000)
            .unwrap();
        assert_eq!(page, expected);
        let mut scored = query.clone();
        scored.score_gt = Some(0.5);
        assert_eq!(
            store
                .observed_plan_quality_samples(&reader(), &scored, 5000)
                .unwrap()
                .samples
                .len(),
            1
        );
        let trust_query = TrustPopulationQueryV1 {
            active_revision: 17,
            plan_id: "plan/codex-daily".into(),
            branch_id: None,
            revision: Some(17),
            from_ms: Some(0),
            to_ms: Some(10_000),
        };
        for lean in [false, true] {
            let population = if lean {
                store.observed_trust_suggestion_population(&reader(), &trust_query, 5000)
            } else {
                store.observed_trust_population(&reader(), &trust_query, 5000)
            }
            .unwrap();
            assert_eq!(population.stages.len(), 1);
            assert_eq!(
                population.stages[0]
                    .sample
                    .assessment
                    .as_ref()
                    .unwrap()
                    .score,
                0.6
            );
        }
        let db = store.connection.lock();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM content_instances_v2", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM logical_requests", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM observation_request_tombstones_v2",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
    };
    verify(&store);
    drop(channel);
    drop(store);
    let store = open_store(directory.path());
    verify(&store);
}
