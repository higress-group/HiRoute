use hiroute_domain::{
    AgentPlanId, AgentTurnAttributionV1, AgentTurnStatusV1, CanonicalDigest, ExecutionFactV1,
    ObservationNackDetailV1, ObservedProfileDigestV1, PlanQualitySamplesQuery, SEVEN_DAYS_MILLIS,
    TurnId,
};

use crate::WriterCycleOutcome;
use crate::query_v2::ObservationReaderContext;

use super::support::{Fixture, fact_channel, offer_fact, open_store, writer};

mod summary;

const PLAN: &str = "plan/codex-daily";
const MODEL: &str = "model/config-a";

fn profile() -> ObservedProfileDigestV1 {
    ObservedProfileDigestV1::from(CanonicalDigest::of_bytes(b"profile-a"))
}

fn finished(
    fixture: &Fixture,
    segment: &str,
    turn: &str,
    ordinal: u64,
    branch: &str,
    started_at_ms: u64,
) -> ExecutionFactV1 {
    finished_for_model(
        fixture,
        segment,
        turn,
        ordinal,
        branch,
        started_at_ms,
        MODEL,
    )
}

fn finished_for_model(
    fixture: &Fixture,
    segment: &str,
    turn: &str,
    ordinal: u64,
    branch: &str,
    started_at_ms: u64,
    model: &str,
) -> ExecutionFactV1 {
    finished_for_model_with_status(
        fixture,
        segment,
        turn,
        ordinal,
        branch,
        started_at_ms,
        model,
        AgentTurnStatusV1::Completed,
    )
}

#[allow(clippy::too_many_arguments)]
fn finished_for_model_with_status(
    fixture: &Fixture,
    segment: &str,
    turn: &str,
    ordinal: u64,
    branch: &str,
    started_at_ms: u64,
    model: &str,
    status: AgentTurnStatusV1,
) -> ExecutionFactV1 {
    ExecutionFactV1::AgentTurnFinished {
        branch_execution: None,
        agent_turn_id: turn.into(),
        segment_id: segment.into(),
        ordinal,
        plan_id: AgentPlanId::parse(PLAN).unwrap(),
        plan_revision: 17,
        selected_branch_id: branch.into(),
        executed_branch_id: Some(branch.into()),
        model_configuration_id: Some(model.into()),
        profile_digest: Some(profile()),
        attribution: AgentTurnAttributionV1::Single,
        started_at_ms,
        finished_at_ms: started_at_ms + 10,
        status,
        history_partial: false,
        first_request_id: Some(fixture.request.clone()),
        last_request_id: Some(fixture.request.clone()),
    }
}

fn assessment(
    fixture: &Fixture,
    segment: &str,
    target: (&str, &str, u64, u64),
    assessed_at_ms: u64,
    score: f64,
) -> ExecutionFactV1 {
    assessment_for_model(fixture, segment, target, assessed_at_ms, score, MODEL)
}

fn assessment_for_model(
    fixture: &Fixture,
    segment: &str,
    target: (&str, &str, u64, u64),
    assessed_at_ms: u64,
    score: f64,
    model: &str,
) -> ExecutionFactV1 {
    ExecutionFactV1::BranchAssessmentRecorded {
        segment_id: segment.into(),
        plan_id: AgentPlanId::parse(PLAN).unwrap(),
        plan_revision: 17,
        model_configuration_id: model.into(),
        profile_digest: profile(),
        trigger_request_id: fixture.request.clone(),
        target_from_turn_id: TurnId::parse(target.0).unwrap(),
        target_through_turn_id: TurnId::parse(target.1).unwrap(),
        target_from_ordinal: target.2,
        target_through_ordinal: target.3,
        assessed_at_ms,
        score,
        partial: false,
        reason: Some(format!("score-{score}")),
    }
}

fn query(plan_revision: Option<u64>) -> PlanQualitySamplesQuery {
    PlanQualitySamplesQuery {
        competence: None,
        plan_id: Some(PLAN.into()),
        session_id: None,
        segment_id: None,
        plan_revision,
        model_configuration_id: None,
        execution: None,
        unrated_only: false,
        from_ms: Some(0),
        to_ms: Some(10_000),
        score_gt: None,
        score_lt: None,
        limit: 50,
        cursor: None,
    }
}

fn reader() -> ObservationReaderContext {
    ObservationReaderContext::local_user(
        hiroute_domain::WorkspaceId::default(),
        "user".into(),
        1,
        20_000,
        false,
        false,
    )
    .unwrap()
}

#[test]
fn assessment_before_execution_survives_restart_and_is_bound_to_the_segment() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let first = Fixture::new("quality-assessment-first");
    let mut execution = Fixture::new("quality-execution-second");
    execution.session = first.session.clone();
    execution.fact_stream = first.fact_stream.clone();
    let channel = fact_channel(&first, 256 * 1024);
    let writer = writer(&store);

    assert!(matches!(
        offer_fact(
            &writer,
            &channel,
            first.fact(
                1,
                assessment(
                    &first,
                    "segment-a",
                    ("agent-turn-1", "agent-turn-1", 1, 1),
                    1200,
                    0.6
                ),
                1200,
            ),
        ),
        WriterCycleOutcome::Ack(_)
    ));
    assert!(matches!(
        offer_fact(
            &writer,
            &channel,
            execution.fact(
                2,
                finished(
                    &execution,
                    "segment-a",
                    "agent-turn-1",
                    1,
                    "smart_saving_simple",
                    1000
                ),
                1000,
            ),
        ),
        WriterCycleOutcome::Ack(_)
    ));
    drop(writer);
    drop(channel);
    drop(store);

    let reopened = open_store(directory.path());
    let page = reopened
        .observed_plan_quality_samples(&reader(), &query(Some(17)), 5_000)
        .unwrap();
    assert_eq!(page.samples.len(), 1);
    let sample = &page.samples[0];
    assert_eq!(sample.segment_id, "segment-a");
    assert_eq!(sample.first_turn_id, "agent-turn-1");
    assert_eq!(sample.model_configuration_id.as_deref(), Some(MODEL));
    assert_eq!(sample.assessment.as_ref().unwrap().score, 0.6);
}

#[test]
fn unknown_boundary_round_is_assessable_and_the_new_model_starts_unscored_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let writer = writer(&store);
    let first = Fixture::new("quality-unknown-boundary");
    let mut assessed = Fixture::new("quality-unknown-assessment");
    assessed.session = first.session.clone();
    assessed.fact_stream = first.fact_stream.clone();
    let mut switched = Fixture::new("quality-boundary-switch");
    switched.session = first.session.clone();
    switched.fact_stream = first.fact_stream.clone();
    let channel = fact_channel(&first, 256 * 1024);

    for fact in [
        first.fact(
            1,
            finished_for_model_with_status(
                &first,
                "segment-a",
                "agent-turn-1",
                1,
                "smart_saving_simple",
                1_000,
                "model/config-a",
                AgentTurnStatusV1::Unknown,
            ),
            1_010,
        ),
        assessed.fact(
            2,
            assessment_for_model(
                &assessed,
                "segment-a",
                ("agent-turn-1", "agent-turn-1", 1, 1),
                1_200,
                0.4,
                "model/config-a",
            ),
            1_200,
        ),
        switched.fact(
            3,
            finished_for_model(
                &switched,
                "segment-b",
                "agent-turn-2",
                2,
                "smart_saving_complex",
                1_300,
                "model/config-b",
            ),
            1_310,
        ),
    ] {
        assert!(matches!(
            offer_fact(&writer, &channel, fact),
            WriterCycleOutcome::Ack(_)
        ));
    }
    drop(writer);
    drop(channel);
    drop(store);

    let reopened = open_store(directory.path());
    let page = reopened
        .observed_plan_quality_samples(&reader(), &query(Some(17)), 5_000)
        .unwrap();
    assert_eq!(page.samples.len(), 2);
    let old = page
        .samples
        .iter()
        .find(|sample| sample.segment_id == "segment-a")
        .unwrap();
    assert_eq!(
        old.model_configuration_id.as_deref(),
        Some("model/config-a")
    );
    assert_eq!(old.assessment.as_ref().unwrap().score, 0.4);
    assert_eq!(old.assessment.as_ref().unwrap().target_through_ordinal, 1);
    let new = page
        .samples
        .iter()
        .find(|sample| sample.segment_id == "segment-b")
        .unwrap();
    assert_eq!(
        new.model_configuration_id.as_deref(),
        Some("model/config-b")
    );
    assert!(new.assessment.is_none());
}

#[test]
fn assessment_and_execution_cannot_rebind_a_segment_to_another_model() {
    for assessment_first in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let store = open_store(directory.path());
        let first = Fixture::new(if assessment_first {
            "quality-conflict-assessment-first"
        } else {
            "quality-conflict-execution-first"
        });
        let mut second = Fixture::new(if assessment_first {
            "quality-conflict-execution-second"
        } else {
            "quality-conflict-assessment-second"
        });
        second.session = first.session.clone();
        second.fact_stream = first.fact_stream.clone();
        let channel = fact_channel(&first, 256 * 1024);
        let writer = writer(&store);
        let assessment = first.fact(
            1,
            assessment_for_model(
                &first,
                "segment-conflict",
                ("agent-turn-1", "agent-turn-1", 1, 1),
                1_200,
                0.6,
                "model/config-a",
            ),
            1_200,
        );
        let execution = second.fact(
            2,
            finished_for_model(
                &second,
                "segment-conflict",
                "agent-turn-1",
                1,
                "smart_saving_simple",
                1_000,
                "model/config-b",
            ),
            1_000,
        );
        let (first_fact, conflicting_fact) = if assessment_first {
            (assessment, execution)
        } else {
            let mut execution = execution;
            execution.sequence = 1;
            let mut assessment = assessment;
            assessment.sequence = 2;
            (execution, assessment)
        };
        assert!(matches!(
            offer_fact(&writer, &channel, first_fact),
            WriterCycleOutcome::Ack(_)
        ));
        assert!(matches!(
            offer_fact(&writer, &channel, conflicting_fact),
            WriterCycleOutcome::Nack(ref nack)
                if matches!(
                    nack.detail,
                    ObservationNackDetailV1::ImmutableProjectionConflict { .. }
                )
        ));
    }
}

#[test]
fn latest_assessment_overwrites_without_extending_itself_and_branch_switch_starts_unscored() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let writer = writer(&store);
    let first = Fixture::new("quality-flow-1");
    let channel = fact_channel(&first, 1024 * 1024);
    let fixtures = (2..=6)
        .map(|index| {
            let mut fixture = Fixture::new(&format!("quality-flow-{index}"));
            fixture.session = first.session.clone();
            fixture.fact_stream = first.fact_stream.clone();
            fixture
        })
        .collect::<Vec<_>>();

    let facts = vec![
        first.fact(
            1,
            finished(
                &first,
                "segment-a",
                "agent-turn-1",
                1,
                "smart_saving_simple",
                1000,
            ),
            1010,
        ),
        fixtures[0].fact(
            2,
            assessment(
                &fixtures[0],
                "segment-a",
                ("agent-turn-1", "agent-turn-1", 1, 1),
                1200,
                0.6,
            ),
            1200,
        ),
        fixtures[1].fact(
            3,
            finished(
                &fixtures[1],
                "segment-a",
                "agent-turn-2",
                2,
                "smart_saving_simple",
                1300,
            ),
            1310,
        ),
        fixtures[2].fact(
            4,
            assessment(
                &fixtures[2],
                "segment-a",
                ("agent-turn-1", "agent-turn-2", 1, 2),
                1500,
                0.3,
            ),
            1500,
        ),
        fixtures[3].fact(
            5,
            assessment(
                &fixtures[3],
                "segment-a",
                ("agent-turn-1", "agent-turn-2", 1, 2),
                1600,
                0.2,
            ),
            1600,
        ),
        fixtures[4].fact(
            6,
            finished(
                &fixtures[4],
                "segment-b",
                "agent-turn-3",
                3,
                "smart_saving_complex",
                1700,
            ),
            1710,
        ),
    ];
    for fact in facts {
        assert!(matches!(
            offer_fact(&writer, &channel, fact),
            WriterCycleOutcome::Ack(_)
        ));
    }

    let page = store
        .observed_plan_quality_samples(&reader(), &query(None), 5_000)
        .unwrap();
    assert_eq!(page.samples.len(), 2);
    let old = page
        .samples
        .iter()
        .find(|sample| sample.segment_id == "segment-a")
        .unwrap();
    assert_eq!(old.last_observed_turn_ordinal, 2);
    let score = old.assessment.as_ref().unwrap();
    assert_eq!(score.score, 0.2);
    assert_eq!(score.target_through_ordinal, 2);
    let new = page
        .samples
        .iter()
        .find(|sample| sample.segment_id == "segment-b")
        .unwrap();
    assert!(new.assessment.is_none());

    let mut low = query(None);
    low.score_lt = Some(0.2);
    assert!(
        store
            .observed_plan_quality_samples(&reader(), &low, 5_000)
            .unwrap()
            .samples
            .is_empty()
    );
    low.score_lt = Some(0.200_001);
    assert_eq!(
        store
            .observed_plan_quality_samples(&reader(), &low, 5_000)
            .unwrap()
            .samples
            .len(),
        1
    );
    let mut high = query(None);
    high.score_gt = Some(0.2);
    assert!(
        store
            .observed_plan_quality_samples(&reader(), &high, 5_000)
            .unwrap()
            .samples
            .is_empty()
    );
}

#[test]
fn request_retention_drops_expired_score_and_marks_a_continuing_segment_partial() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let writer = writer(&store);
    let first = Fixture::new("quality-retention-first");
    let mut assessed = Fixture::new("quality-retention-assessed");
    assessed.session = first.session.clone();
    assessed.fact_stream = first.fact_stream.clone();
    let mut recent = Fixture::new("quality-retention-recent");
    recent.session = first.session.clone();
    recent.fact_stream = first.fact_stream.clone();
    let channel = fact_channel(&first, 1024 * 1024);

    for fact in [
        first.fact(
            1,
            finished(
                &first,
                "segment-retained",
                "agent-turn-old",
                1,
                "smart_saving_simple",
                1_000,
            ),
            1_010,
        ),
        assessed.fact(
            2,
            assessment(
                &assessed,
                "segment-retained",
                ("agent-turn-old", "agent-turn-old", 1, 1),
                1_200,
                0.2,
            ),
            1_200,
        ),
        recent.fact(
            3,
            finished(
                &recent,
                "segment-retained",
                "agent-turn-recent",
                2,
                "smart_saving_simple",
                u64::try_from(SEVEN_DAYS_MILLIS + 10_000).unwrap(),
            ),
            SEVEN_DAYS_MILLIS + 10_010,
        ),
    ] {
        assert!(matches!(
            offer_fact(&writer, &channel, fact),
            WriterCycleOutcome::Ack(_)
        ));
    }

    let now = SEVEN_DAYS_MILLIS * 2 + 5_000;
    assert_eq!(store.expire_request_details(now, 16).unwrap(), 2);
    let mut quality_query = query(None);
    quality_query.to_ms = Some(now + 1);
    let retention_reader = ObservationReaderContext::local_user(
        hiroute_domain::WorkspaceId::default(),
        "user".into(),
        1,
        now + 1_000,
        false,
        false,
    )
    .unwrap();
    let page = store
        .observed_plan_quality_samples(&retention_reader, &quality_query, now)
        .unwrap();
    assert_eq!(page.samples.len(), 1);
    let sample = &page.samples[0];
    assert_eq!(sample.segment_id, "segment-retained");
    assert!(sample.history_partial);
    assert!(sample.first_request_id.is_none());
    assert_eq!(
        sample.last_request_id.as_deref(),
        Some(recent.request.as_str())
    );
    assert!(!sample.execution_evidence_available);
    assert!(sample.assessment.is_none());
}
