use super::*;

#[test]
fn complete_turn_scores_survive_a_later_interrupted_turn() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let first = Fixture::new("independent-quality");
    let channel = fact_channel(&first, 256 * 1024);
    let writer = writer(&store);
    for i in 1..=3 {
        let mut fixture = Fixture::new(&format!("independent-quality-{i}"));
        fixture.session = first.session.clone();
        fixture.fact_stream = first.fact_stream.clone();
        let unit = format!("unit-{i}");
        let turn = format!("turn-{i}");
        let at = 1000 + i * 100;
        let mut done = finished_for_model_with_status(
            &fixture,
            &unit,
            &turn,
            i,
            "code",
            at,
            MODEL,
            if i == 3 {
                AgentTurnStatusV1::Interrupted
            } else {
                AgentTurnStatusV1::Completed
            },
        );
        if let ExecutionFactV1::AgentTurnFinished {
            history_partial, ..
        } = &mut done
        {
            *history_partial = i == 3;
        }
        let mut score = assessment(
            &fixture,
            &unit,
            (&turn, &turn, i, i),
            at + 20,
            if i == 1 {
                0.9
            } else if i == 2 {
                0.8
            } else {
                0.3
            },
        );
        if let ExecutionFactV1::BranchAssessmentRecorded { partial, .. } = &mut score {
            *partial = i == 3;
        }
        for (sequence, fact) in [(i * 2 - 1, done), (i * 2, score)] {
            assert!(matches!(
                offer_fact(
                    &writer,
                    &channel,
                    fixture.fact(sequence, fact, at as i64 + 30)
                ),
                WriterCycleOutcome::Ack(_)
            ));
        }
    }
    let page = store
        .observed_plan_quality_samples(&reader(), &query(None), 5000)
        .unwrap();
    assert_eq!(page.samples.len(), 3);
    assert_eq!(page.summary.scored_stage_count, 2);
    assert_eq!(page.summary.unrated_stage_count, 1);
    assert!((page.summary.models[0].average_score.unwrap() - 0.85).abs() < 1e-9);
    for unit in ["unit-1", "unit-2"] {
        let sample = page.samples.iter().find(|s| s.segment_id == unit).unwrap();
        assert!(!sample.history_partial);
        assert!(!sample.assessment.as_ref().unwrap().partial);
    }
    let interrupted = page
        .samples
        .iter()
        .find(|s| s.segment_id == "unit-3")
        .unwrap();
    assert!(interrupted.history_partial);
    assert!(interrupted.assessment.as_ref().unwrap().partial);
}

#[test]
fn legacy_multi_turn_scores_are_not_migrated_or_counted_and_raw_facts_survive_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let first = Fixture::new("legacy-score-no-migration");
    let mut second = Fixture::new("legacy-score-second-turn");
    let mut feedback = Fixture::new("legacy-score-feedback");
    for fixture in [&mut second, &mut feedback] {
        fixture.session = first.session.clone();
        fixture.fact_stream = first.fact_stream.clone();
    }
    let preserved;
    {
        let store = open_store(directory.path());
        let channel = fact_channel(&first, 256 * 1024);
        let writer = writer(&store);
        let facts = [
            (
                &first,
                finished(&first, "legacy-stage", "turn-1", 1, "code", 1000),
            ),
            (
                &second,
                finished(&second, "legacy-stage", "turn-2", 2, "code", 1100),
            ),
            (
                &feedback,
                assessment(
                    &feedback,
                    "legacy-stage",
                    ("turn-1", "turn-2", 1, 2),
                    1200,
                    0.2,
                ),
            ),
        ];
        for (i, (fixture, fact)) in facts.into_iter().enumerate() {
            assert!(matches!(
                offer_fact(&writer, &channel, fixture.fact(i as u64 + 1, fact, 1300)),
                WriterCycleOutcome::Ack(_)
            ));
        }
        preserved = super::history_membership::fact_digests(&store.connection.lock());
        assert_eq!(preserved.len(), 3);
        let page = store
            .observed_plan_quality_samples(&reader(), &query(None), 5000)
            .unwrap();
        assert!(page.samples.is_empty());
        assert_eq!(page.summary.scored_stage_count, 0);
        let timeline = store
            .observed_routing_timeline(
                &reader(),
                &hiroute_domain::RoutingTimelineQueryV1 {
                    include_presentation: false,
                    session_id: first.session.to_string(),
                    after_watermark: None,
                    limit: 50,
                },
                5000,
            )
            .unwrap();
        assert_eq!(
            timeline.turns.len(),
            3,
            "both completed turns and the assessment-trigger request remain visible"
        );
        assert!(timeline.turns.iter().all(|turn| turn.assessment.is_none()));
    }
    let reopened = open_store(directory.path());
    assert_eq!(
        super::history_membership::fact_digests(&reopened.connection.lock()),
        preserved
    );
    let page = reopened
        .observed_plan_quality_samples(&reader(), &query(None), 5000)
        .unwrap();
    assert!(page.samples.is_empty());
}

#[test]
fn a_reopened_turn_with_changed_attribution_has_one_current_quality_sample() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let first = Fixture::new("reopened-turn-quality");
    let channel = fact_channel(&first, 256 * 1024);
    let writer = writer(&store);
    let mut mixed = finished(&first, "mixed-unit", "turn-1", 1, "code", 1200);
    if let ExecutionFactV1::AgentTurnFinished {
        attribution,
        executed_branch_id,
        model_configuration_id,
        profile_digest,
        ..
    } = &mut mixed
    {
        *attribution = AgentTurnAttributionV1::Mixed;
        *executed_branch_id = None;
        *model_configuration_id = None;
        *profile_digest = None;
    }
    for (i, fact) in [
        finished(&first, "single-unit", "turn-1", 1, "code", 1000),
        assessment(&first, "single-unit", ("turn-1", "turn-1", 1, 1), 1100, 0.9),
        mixed,
    ]
    .into_iter()
    .enumerate()
    {
        assert!(matches!(
            offer_fact(&writer, &channel, first.fact(i as u64 + 1, fact, 1300)),
            WriterCycleOutcome::Ack(_)
        ));
    }
    let page = store
        .observed_plan_quality_samples(&reader(), &query(None), 5000)
        .unwrap();
    assert_eq!(page.samples.len(), 1);
    assert_eq!(page.samples[0].segment_id, "mixed-unit");
    assert_eq!(page.samples[0].attribution, AgentTurnAttributionV1::Mixed);
    assert!(page.samples[0].assessment.is_none());
    assert_eq!(page.summary.scored_stage_count, 0);
}
