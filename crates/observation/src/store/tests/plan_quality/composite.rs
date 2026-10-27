//! Joint quality survives real receipt/reopen without becoming a model score.
use super::*;
use hiroute_domain::*;

#[test]
fn composite_score_roundtrips_membership_and_never_credits_one_model() {
    let directory = tempfile::tempdir().unwrap();
    let first = Fixture::new("composite-stage");
    let mut next = Fixture::new("composite-trigger");
    next.session = first.session.clone();
    next.fact_stream = first.fact_stream.clone();
    let membership = HistoryMembershipV1 {
        version: 1,
        branch_id: "history/composite".into(),
        revision: 2,
        request_ids: vec![first.request.to_string()],
        clipped: false,
        relation: HistoryRelationV1::Contextual,
        source_unverified: true,
        association_unverified: true,
        capture_partial: false,
    };
    let mut policy = JudgmentSettingsV1::default().execution_policy("Task");
    policy.criteria_digest = Some(CanonicalDigest::of_bytes(b"frozen-rubric"));
    let score = ExecutionFactV1::StageAssessmentRecorded(StageAssessmentRecordedV1 {
        scope: StageEvaluationScopeV1 {
            version: 1,
            executed_branch_id: "smart_saving".into(),
            policy,
        },
        history_membership: membership.clone(),
        segment_id: "stage/composite".into(),
        plan_id: AgentPlanId::parse(PLAN).unwrap(),
        plan_revision: 17,
        trigger_request_id: next.request.clone(),
        target_from_turn_id: TurnId::parse("turn/joint").unwrap(),
        target_through_turn_id: TurnId::parse("turn/joint").unwrap(),
        target_from_ordinal: 1,
        target_through_ordinal: 1,
        assessed_at_ms: 2000,
        score: 0.94,
        partial: false,
        reason: Some("joint actual outcome".into()),
    });
    {
        let store = open_store(directory.path());
        let channel = fact_channel(&first, 256 * 1024);
        let writer = writer(&store);
        let mut completed = finished(
            &first,
            "stage/composite",
            "turn/joint",
            1,
            "smart_saving",
            1000,
        );
        if let ExecutionFactV1::AgentTurnFinished {
            attribution,
            executed_branch_id,
            model_configuration_id,
            profile_digest,
            history_membership,
            ..
        } = &mut completed
        {
            *attribution = AgentTurnAttributionV1::Mixed;
            *executed_branch_id = None;
            *model_configuration_id = None;
            *profile_digest = None;
            *history_membership = Some(membership);
        }
        for (sequence, fixture, fact, at) in [
            (1, &first, super::super::support::route_decision(PLAN), 1000),
            (2, &first, completed, 1100),
            (3, &next, super::super::support::route_decision(PLAN), 1900),
            (4, &next, score.clone(), 2000),
        ] {
            assert!(matches!(
                offer_fact(&writer, &channel, fixture.fact(sequence, fact, at)),
                WriterCycleOutcome::Ack(_)
            ));
        }
        // Idempotent delivery does not create another quality sample.
        assert!(matches!(
            offer_fact(&writer, &channel, next.fact(4, score, 2000)),
            WriterCycleOutcome::Ack(_)
        ));
    }
    let store = open_store(directory.path());
    let samples = store
        .observed_plan_quality_samples(&reader(), &query(Some(17)), 5000)
        .unwrap();
    let joint = samples
        .samples
        .iter()
        .find(|s| s.segment_id == "stage/composite")
        .unwrap();
    assert_eq!(joint.attribution, AgentTurnAttributionV1::Mixed);
    assert!(joint.model_configuration_id.is_none());
    assert!(!joint.history_partial);
    assert_eq!(joint.assessment.as_ref().unwrap().score, 0.94);
    assert!(!joint.assessment.as_ref().unwrap().partial);
    assert!(
        samples
            .summary
            .models
            .iter()
            .all(|m| m.average_score.is_none())
    );
    assert_eq!(
        samples.summary.scored_stage_count, 1,
        "joint quality remains a real stage score"
    );
    assert_eq!(samples.summary.unrated_stage_count, 0);
    let timeline = store
        .observed_routing_timeline(
            &reader(),
            &RoutingTimelineQueryV1 {
                session_id: first.session.to_string(),
                after_watermark: None,
                limit: 50,
            },
            5000,
        )
        .unwrap();
    let joint = timeline
        .turns
        .iter()
        .find(|t| t.agent_turn_id.as_deref() == Some("turn/joint"))
        .unwrap();
    assert_eq!(joint.assessment.as_ref().unwrap().score, 0.94);
    assert!(!joint.assessment.as_ref().unwrap().partial);
    assert!(joint.model_configuration_id.is_none());
}
