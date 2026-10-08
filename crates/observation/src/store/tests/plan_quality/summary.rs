use super::*;

#[test]
fn branch_competence_keeps_tiers_and_frozen_floor_and_excludes_partial_scores() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("quality-branch-tiers");
    let channel = fact_channel(&fixture, 256 * 1024);
    let writer = writer(&store);
    let mut sequence = 0;
    for (i, primary, score, partial) in [(0, 0, 0.49, false), (1, 1, 0.5, false), (2, 1, 0.0, true)]
    {
        let segment = format!("stage-{i}");
        let turn = format!("turn-{i}");
        let mut fact = finished(&fixture, &segment, &turn, i + 1, "writing", 1000 + i * 100);
        if let ExecutionFactV1::AgentTurnFinished {
            branch_execution, ..
        } = &mut fact
        {
            *branch_execution = Some(hiroute_domain::BranchExecutionV1 {
                policy: hiroute_domain::BranchExecutionPolicyV1 {
                    name: "发布时的写作分支".into(),
                    floor_millis: 500,
                    criteria_digest: None,
                },
                group: if primary == 0 {
                    hiroute_domain::ExecutionGroupV1::Regular
                } else {
                    hiroute_domain::ExecutionGroupV1::Primary
                },
                candidate_index: 0,
            });
        }
        accept(
            &writer,
            &channel,
            &fixture,
            &mut sequence,
            fact,
            1000 + i * 100,
        );
        let mut scored = assessment(
            &fixture,
            &segment,
            (&turn, &turn, i + 1, i + 1),
            1011 + i * 100,
            score,
        );
        if let ExecutionFactV1::BranchAssessmentRecorded { partial: value, .. } = &mut scored {
            *value = partial;
        }
        accept(
            &writer,
            &channel,
            &fixture,
            &mut sequence,
            scored,
            1011 + i * 100,
        );
    }
    let mut q = query(Some(17));
    let page = store
        .observed_plan_quality_samples(&reader(), &q, 5000)
        .unwrap();
    assert_eq!(page.summary.models.len(), 2);
    let upgraded = page
        .summary
        .models
        .iter()
        .find(|m| m.execution.group == Some(hiroute_domain::ExecutionGroupV1::Primary))
        .unwrap();
    assert_eq!(upgraded.average_score, Some(0.5));
    assert_eq!(upgraded.unrated_stage_count, 1);
    assert_eq!(
        upgraded.branch_policy.as_ref().unwrap().name,
        "发布时的写作分支"
    );
    q.competence = Some(hiroute_domain::PlanCompetenceFilter::MeetsFloor);
    let page = store
        .observed_plan_quality_samples(&reader(), &q, 5000)
        .unwrap();
    assert_eq!(page.samples.len(), 1);
    assert_eq!(page.samples[0].assessment.as_ref().unwrap().score, 0.5);
    assert_eq!(page.summary.scored_stage_count, 2);
    q.competence = Some(hiroute_domain::PlanCompetenceFilter::BelowFloor);
    let page = store
        .observed_plan_quality_samples(&reader(), &q, 5000)
        .unwrap();
    assert_eq!(page.samples.len(), 1);
    assert_eq!(
        page.samples[0].branch_execution.as_ref().unwrap().group,
        hiroute_domain::ExecutionGroupV1::Regular
    );
    q.competence = None;
    q.unrated_only = true;
    let page = store
        .observed_plan_quality_samples(&reader(), &q, 5000)
        .unwrap();
    assert_eq!(page.samples.len(), 1);
    assert!(page.samples[0].assessment.as_ref().unwrap().partial);
}

#[test]
fn quality_summary_resolves_retained_execution_names_and_exact_reasoning_profiles() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("quality-summary-metadata");
    let writer = writer(&store);
    let channel = fact_channel(&fixture, 256 * 1024);
    let mut facts = super::super::support::request_facts(
        &fixture,
        "attempt-quality",
        super::super::support::finished("attempt-quality"),
        1_000,
    );
    if let ExecutionFactV1::RouteDecision(route) = &mut facts[0].fact {
        let selection = route.complexity.as_mut().unwrap();
        selection.execution_group = hiroute_domain::ExecutionGroupV1::Primary;
        selection.simple_probability = Some(serde_json::Number::from_f64(0.95).unwrap());
        selection.simple_threshold_millis = Some(800);
        selection.selection_reason = hiroute_domain::ModelGroupReasonV1::LowCompetence;
        selection.competence_trigger = Some(hiroute_domain::CompetenceProtectionV1 {
            segment_id: "previous-stage".into(),
            score: serde_json::Number::from_f64(0.2).unwrap(),
            floor_millis: 500,
            from_group: hiroute_domain::ExecutionGroupV1::Regular,
        });
    } else {
        panic!("request fixture must begin with its route decision");
    }
    if let ExecutionFactV1::CandidateDecision(candidate) = &mut facts[1].fact {
        candidate.model_configuration_id = MODEL.into();
        candidate.reasoning_profile_id = Some("high".into());
    }
    if let ExecutionFactV1::AttemptStarted {
        model_configuration_id,
        ..
    } = &mut facts[4].fact
    {
        *model_configuration_id = MODEL.into();
    }
    let request_finished = facts.pop().unwrap().fact;
    for envelope in facts {
        assert!(matches!(
            offer_fact(&writer, &channel, envelope),
            WriterCycleOutcome::Ack(_)
        ));
    }
    let mut sequence = 9;
    accept(
        &writer,
        &channel,
        &fixture,
        &mut sequence,
        finished(
            &fixture,
            "segment-metadata",
            "agent-turn-1",
            1,
            "smart_saving",
            1_000,
        ),
        1_010,
    );
    for (at, score) in [(1_100, 0.3), (1_200, 0.9)] {
        accept(
            &writer,
            &channel,
            &fixture,
            &mut sequence,
            assessment(
                &fixture,
                "segment-metadata",
                ("agent-turn-1", "agent-turn-1", 1, 1),
                at,
                score,
            ),
            at,
        );
    }
    accept(
        &writer,
        &channel,
        &fixture,
        &mut sequence,
        request_finished,
        1_211,
    );
    let result = store
        .observed_plan_quality_samples(&reader(), &query(Some(17)), 5_000)
        .unwrap();
    assert_eq!(result.samples[0].native_model.as_deref(), Some("native-a"));
    let selection = result.samples[0]
        .selection
        .as_ref()
        .expect("stage opening selection retained");
    assert_eq!(
        selection.simple_probability.as_ref().unwrap().as_f64(),
        Some(0.95)
    );
    assert_eq!(selection.simple_threshold_millis, Some(800));
    assert_eq!(
        selection
            .competence_trigger
            .as_ref()
            .unwrap()
            .score
            .as_f64(),
        Some(0.2)
    );
    assert_eq!(
        result.samples[0].assessment.as_ref().unwrap().score,
        0.9,
        "the latest stage score must not rewrite the opening selection's low-score trigger"
    );
    assert_eq!(
        result.samples[0].reasoning_profile_id.as_deref(),
        Some("high")
    );
    assert_eq!(
        result.summary.models[0].native_model.as_deref(),
        Some("native-a")
    );
    assert_eq!(
        result.summary.models[0].reasoning_profile_id.as_deref(),
        Some("high")
    );
}

fn accept(
    writer: &crate::LocalObservationWriter,
    channel: &crate::FactChannel,
    fixture: &Fixture,
    sequence: &mut u64,
    fact: ExecutionFactV1,
    at: u64,
) {
    *sequence += 1;
    let outcome = offer_fact(
        writer,
        channel,
        fixture.fact(*sequence, fact, i64::try_from(at).unwrap()),
    );
    assert!(
        matches!(outcome, WriterCycleOutcome::Ack(_)),
        "sequence {sequence}: {outcome:?}"
    );
}

#[test]
fn quality_summary_uses_latest_raw_scores_over_full_scope_and_ignores_detail_filters() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("quality-summary-full-scope");
    let channel = fact_channel(&fixture, 256 * 1024);
    let writer = writer(&store);
    let mut sequence = 0;
    for i in 0..26 {
        let segment = format!("segment-{i}");
        let turn = format!("agent-turn-{}", i + 1);
        let at = 1_000 + i * 20;
        accept(
            &writer,
            &channel,
            &fixture,
            &mut sequence,
            finished(&fixture, &segment, &turn, i + 1, "smart_saving_simple", at),
            at,
        );
        if i < 25 {
            let score = if i == 0 {
                0.0
            } else if i == 24 {
                0.4
            } else {
                0.5
            };
            accept(
                &writer,
                &channel,
                &fixture,
                &mut sequence,
                assessment(
                    &fixture,
                    &segment,
                    (&turn, &turn, i + 1, i + 1),
                    at + 11,
                    score,
                ),
                at + 11,
            );
        }
    }
    accept(
        &writer,
        &channel,
        &fixture,
        &mut sequence,
        assessment(
            &fixture,
            "segment-24",
            ("agent-turn-25", "agent-turn-25", 25, 25),
            2_000,
            0.8,
        ),
        2_000,
    );

    let mut q = query(Some(17));
    q.limit = 20;
    let first = store
        .observed_plan_quality_samples(&reader(), &q, 5_000)
        .unwrap();
    assert_eq!(first.samples.len(), 20);
    assert!(first.next_cursor.is_some());
    assert_eq!(first.summary.scored_stage_count, 25);
    assert_eq!(first.summary.unrated_stage_count, 1);
    assert_eq!(first.summary.session_count, 1);
    assert_eq!(first.summary.available_revisions, [17]);
    let model = &first.summary.models[0];
    assert_eq!(model.scored_stage_count, 25);
    assert_eq!(model.unrated_stage_count, 1);
    assert!((model.average_score.unwrap() - 0.492).abs() < 1e-12);

    q.cursor = first.next_cursor.clone();
    let next = store
        .observed_plan_quality_samples(&reader(), &q, 5_000)
        .unwrap();
    assert_eq!(next.samples.len(), 6);
    assert_eq!(next.summary, first.summary);
    q.cursor = None;
    q.score_lt = Some(0.5);
    q.execution = Some(model.execution.clone());
    let low = store
        .observed_plan_quality_samples(&reader(), &q, 5_000)
        .unwrap();
    assert_eq!(low.samples.len(), 1);
    assert_eq!(low.samples[0].assessment.as_ref().unwrap().score, 0.0);
    assert_eq!(low.summary, first.summary);
    q.score_lt = None;
    q.unrated_only = true;
    let unrated = store
        .observed_plan_quality_samples(&reader(), &q, 5_000)
        .unwrap();
    assert_eq!(unrated.samples.len(), 1);
    assert!(unrated.samples[0].assessment.is_none());
    assert_eq!(unrated.summary, first.summary);
    q.score_lt = Some(0.5);
    assert!(matches!(
        store.observed_plan_quality_samples(&reader(), &q, 5_000),
        Err(crate::query_v2::ObservationV2Error::Invalid)
    ));
}

#[test]
fn quality_summary_separates_actual_groups_profiles_and_revisions_and_exactly_drills_down() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("quality-summary-identities");
    let channel = fact_channel(&fixture, 256 * 1024);
    let writer = writer(&store);
    let mut sequence = 0;
    for (i, branch, other_profile, revision) in [
        (1, "smart_saving_simple", false, 17),
        (2, "smart_saving_complex", false, 17),
        (3, "smart_saving_complex", true, 17),
        (4, "smart_saving_complex", true, 18),
    ] {
        let segment = format!("segment-{i}");
        let turn = format!("agent-turn-{i}");
        let mut fact = finished(&fixture, &segment, &turn, i, branch, 1_000 + i * 20);
        if let ExecutionFactV1::AgentTurnFinished {
            profile_digest,
            plan_revision,
            selected_branch_id,
            ..
        } = &mut fact
        {
            *plan_revision = revision;
            if other_profile {
                *profile_digest = Some(CanonicalDigest::of_bytes(b"profile-b").into());
            }
            // A fallback executes the complex group after selection of simple.
            if i == 2 {
                *selected_branch_id = "smart_saving_simple".into();
            }
        }
        accept(
            &writer,
            &channel,
            &fixture,
            &mut sequence,
            fact,
            1_000 + i * 20,
        );
    }
    let all = store
        .observed_plan_quality_samples(&reader(), &query(None), 5_000)
        .unwrap();
    assert_eq!(all.summary.models.len(), 4);
    assert_eq!(all.summary.available_revisions, [18, 17]);
    for model in &all.summary.models {
        let mut q = query(None);
        q.execution = Some(model.execution.clone());
        let page = store
            .observed_plan_quality_samples(&reader(), &q, 5_000)
            .unwrap();
        assert_eq!(page.samples.len(), 1);
        assert_eq!(page.summary, all.summary);
        assert_eq!(
            page.samples[0].profile_digest,
            model.execution.profile_digest
        );
        assert_eq!(page.samples[0].plan_revision, model.execution.plan_revision);
        assert_eq!(
            page.samples[0].executed_branch_id,
            model.execution.executed_branch_id
        );
    }
    let current = store
        .observed_plan_quality_samples(&reader(), &query(Some(17)), 5_000)
        .unwrap();
    assert_eq!(current.summary.models.len(), 3);
    assert_eq!(current.summary.available_revisions, [18, 17]);
    let mut narrow = query(Some(17));
    narrow.from_ms = Some(1_051);
    let narrow = store
        .observed_plan_quality_samples(&reader(), &narrow, 5_000)
        .unwrap();
    assert_eq!(narrow.summary.models.len(), 1);
}

#[test]
fn quality_summary_does_not_attribute_mixed_execution_and_respects_run_authority() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("quality-summary-mixed");
    let channel = fact_channel(&fixture, 256 * 1024);
    let writer = writer(&store);
    let mut sequence = 0;
    let mut fact = finished(
        &fixture,
        "mixed-segment",
        "agent-turn-1",
        1,
        "smart_saving_simple",
        1_000,
    );
    if let ExecutionFactV1::AgentTurnFinished {
        executed_branch_id,
        model_configuration_id,
        profile_digest,
        attribution,
        ..
    } = &mut fact
    {
        *executed_branch_id = None;
        *model_configuration_id = None;
        *profile_digest = None;
        *attribution = AgentTurnAttributionV1::Mixed;
    }
    accept(&writer, &channel, &fixture, &mut sequence, fact, 1_000);
    let page = store
        .observed_plan_quality_samples(&reader(), &query(Some(17)), 5_000)
        .unwrap();
    assert_eq!(page.summary.models.len(), 1);
    let mixed = &page.summary.models[0];
    assert_eq!(mixed.execution.attribution, AgentTurnAttributionV1::Mixed);
    assert!(mixed.execution.model_configuration_id.is_none());
    assert!(mixed.average_score.is_none());
    let scoped_reader = ObservationReaderContext::run_scoped(
        hiroute_domain::WorkspaceId::default(),
        "worker".into(),
        1,
        20_000,
        ["other-run".into()].into_iter().collect(),
        false,
        false,
    )
    .unwrap();
    let hidden = store
        .observed_plan_quality_samples(&scoped_reader, &query(Some(17)), 5_000)
        .unwrap();
    assert!(hidden.samples.is_empty());
    assert!(hidden.summary.models.is_empty());
    assert_eq!(hidden.summary.session_count, 0);
    assert!(hidden.summary.available_revisions.is_empty());
}
