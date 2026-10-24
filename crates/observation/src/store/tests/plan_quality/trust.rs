use super::*;
use hiroute_domain::TrustPopulationQueryV1;

#[test]
fn assessment_reasons_are_bound_to_the_trigger_request() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let writer = writer(&store);
    let fixtures = [
        Fixture::new("assessment-owner"),
        Fixture::new("other-request"),
    ];
    for (index, fixture) in fixtures.iter().enumerate() {
        let channel = fact_channel(fixture, 256 * 1024);
        let stage = format!("stage-{index}");
        assert!(matches!(
            offer_fact(
                &writer,
                &channel,
                fixture.fact(
                    1,
                    finished(fixture, &stage, "turn", 1, "smart_saving", 1000),
                    1000
                )
            ),
            WriterCycleOutcome::Ack(_)
        ));
        // Event IDs are scoped to their producer stream and can repeat in
        // another request. Its assessment must not duplicate or relabel a stage.
        assert!(matches!(
            offer_fact(
                &writer,
                &channel,
                fixture.fact(
                    2,
                    assessment(
                        fixture,
                        &stage,
                        ("turn", "turn", 1, 1),
                        1100,
                        if index == 0 { 0.9 } else { 0.2 }
                    ),
                    1100
                )
            ),
            WriterCycleOutcome::Ack(_)
        ));
    }
    let page = store
        .observed_plan_quality_samples(&reader(), &query(Some(17)), 5000)
        .unwrap();
    assert_eq!(page.samples.len(), 2);
    for (index, reason) in ["score-0.9", "score-0.2"].into_iter().enumerate() {
        let sample = page
            .samples
            .iter()
            .find(|s| s.segment_id == format!("stage-{index}"))
            .unwrap();
        let assessment = sample.assessment.as_ref().unwrap();
        assert_eq!(
            assessment.trigger_request_id,
            fixtures[index].request.as_str()
        );
        assert_eq!(assessment.reason.as_deref(), Some(reason));
    }
    let population = store
        .observed_trust_population(
            &reader(),
            &TrustPopulationQueryV1 {
                active_revision: 17,
                plan_id: PLAN.into(),
                branch_id: None,
                revision: Some(17),
                from_ms: Some(0),
                to_ms: Some(10_000),
            },
            5000,
        )
        .unwrap();
    assert_eq!(population.stages.len(), 2);
    assert_eq!(population.active_stages.len(), 2);
}

#[test]
fn trust_drilldown_filters_exact_identity_without_changing_scope_summary() {
    use hiroute_domain::{
        BranchExecutionPolicyV1, BranchExecutionV1, ExecutionGroupV1, PlanTrustRowFilterV1,
    };
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("trust-detail");
    let channel = fact_channel(&fixture, 256 * 1024);
    let writer = writer(&store);
    for (i, group) in [ExecutionGroupV1::Regular, ExecutionGroupV1::Primary]
        .into_iter()
        .enumerate()
    {
        let mut fact = finished(
            &fixture,
            &format!("stage-{i}"),
            &format!("turn-{i}"),
            i as u64 + 1,
            "smart_saving",
            1000 + i as u64 * 100,
        );
        if let ExecutionFactV1::AgentTurnFinished {
            branch_execution, ..
        } = &mut fact
        {
            *branch_execution = Some(BranchExecutionV1 {
                policy: BranchExecutionPolicyV1 {
                    name: "Trial".into(),
                    extension: None,
                    floor_millis: Some(500),
                    criteria_digest: None,
                },
                group,
                candidate_index: 0,
            });
        }
        assert!(matches!(
            offer_fact(
                &writer,
                &channel,
                fixture.fact(i as u64 + 1, fact, 1100 + i as i64)
            ),
            WriterCycleOutcome::Ack(_)
        ));
    }
    let mut q = query(None);
    q.trust_row = Some(PlanTrustRowFilterV1 {
        branch_id: "smart_saving".into(),
        group: Some(ExecutionGroupV1::Regular),
        model_configuration_id: Some(MODEL.into()),
        reasoning_profile_id: None,
        attribution: AgentTurnAttributionV1::Single,
        group_by_executor: false,
        executor: None,
    });
    let page = store
        .observed_plan_quality_samples(&reader(), &q, 5000)
        .unwrap();
    assert_eq!(page.samples.len(), 1);
    assert_eq!(page.samples[0].segment_id, "stage-0");
    assert_eq!(page.summary.unrated_stage_count, 2);
    q.trust_row.as_mut().unwrap().reasoning_profile_id = Some("high".into());
    assert!(
        store
            .observed_plan_quality_samples(&reader(), &q, 5000)
            .unwrap()
            .samples
            .is_empty()
    );
    q.trust_row = None;
    q.segment_ids = vec!["stage-1".into()];
    q.execution_group = Some(ExecutionGroupV1::Primary);
    assert_eq!(
        store
            .observed_plan_quality_samples(&reader(), &q, 5000)
            .unwrap()
            .samples[0]
            .segment_id,
        "stage-1"
    );
    q.executor = Some("not-this-executor".into());
    assert!(
        store
            .observed_plan_quality_samples(&reader(), &q, 5000)
            .unwrap()
            .samples
            .is_empty()
    );
}

#[test]
fn trust_population_keeps_full_active_history_beside_the_selected_window() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("trust-population");
    let channel = fact_channel(&fixture, 256 * 1024);
    let writer = writer(&store);
    for (index, at) in [1000, 2000].into_iter().enumerate() {
        let fact = finished(
            &fixture,
            &format!("stage-{index}"),
            &format!("turn-{index}"),
            index as u64 + 1,
            "smart_saving",
            at,
        );
        assert!(matches!(
            offer_fact(
                &writer,
                &channel,
                fixture.fact(index as u64 + 1, fact, at as i64)
            ),
            WriterCycleOutcome::Ack(_)
        ));
    }
    let query = TrustPopulationQueryV1 {
        active_revision: 17,
        plan_id: PLAN.into(),
        branch_id: None,
        revision: Some(17),
        from_ms: Some(1500),
        to_ms: Some(3000),
    };
    let population = store
        .observed_trust_population(&reader(), &query, 5000)
        .unwrap();
    assert_eq!(population.stages.len(), 1);
    assert_eq!(population.active_stages.len(), 2);
    assert!(population.active_stages[0].has_later_turn);
    assert!(!population.active_stages[1].has_later_turn);
    assert_eq!(population.from_ms, 1500);
    assert_eq!(population.active_stages[0].sample.assessment, None);
    assert!(matches!(
        offer_fact(
            &writer,
            &channel,
            fixture.fact(
                3,
                assessment(&fixture, "stage-0", ("turn-0", "turn-0", 1, 1), 2100, 0.4),
                2100
            )
        ),
        WriterCycleOutcome::Ack(_)
    ));
    let updated = store
        .observed_trust_population(&reader(), &query, 5000)
        .unwrap();
    assert!(updated.generation > population.generation);
    assert_eq!(
        updated.active_stages[0]
            .sample
            .assessment
            .as_ref()
            .unwrap()
            .score,
        0.4
    );
    assert_eq!(updated.stages.len(), 1);
}
#[test]
fn trust_population_cannot_expand_a_run_scoped_reader() {
    let directory = tempfile::tempdir().unwrap();
    let store = open_store(directory.path());
    let fixture = Fixture::new("trust-scoped");
    let channel = fact_channel(&fixture, 256 * 1024);
    let writer = writer(&store);
    assert!(matches!(
        offer_fact(
            &writer,
            &channel,
            fixture.fact(
                1,
                finished(&fixture, "stage", "turn", 1, "smart_saving", 1000),
                1000
            )
        ),
        WriterCycleOutcome::Ack(_)
    ));
    let reader = ObservationReaderContext::run_scoped(
        hiroute_domain::WorkspaceId::default(),
        "test".into(),
        1,
        6000,
        std::collections::BTreeSet::from(["unrelated-run".into()]),
        false,
        false,
    )
    .unwrap();
    let query = TrustPopulationQueryV1 {
        active_revision: 17,
        plan_id: PLAN.into(),
        branch_id: None,
        revision: None,
        from_ms: None,
        to_ms: None,
    };
    let population = store
        .observed_trust_population(&reader, &query, 5000)
        .unwrap();
    assert!(population.stages.is_empty());
    assert!(population.active_stages.is_empty());
}

#[test]
fn trust_suggestion_population_preserves_count_inputs_without_performance_details() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let f = Fixture::new("lean-trust");
    let channel = fact_channel(&f, 256 * 1024);
    let w = writer(&store);
    assert!(matches!(
        offer_fact(
            &w,
            &channel,
            f.fact(
                1,
                finished(&f, "stage", "turn", 1, "smart_saving", 1000),
                1000
            )
        ),
        WriterCycleOutcome::Ack(_)
    ));
    let before = store.observed_trust_generation(&reader(), 5000).unwrap();
    assert!(matches!(
        offer_fact(
            &w,
            &channel,
            f.fact(
                2,
                assessment(&f, "stage", ("turn", "turn", 1, 1), 1100, 0.9),
                1100
            )
        ),
        WriterCycleOutcome::Ack(_)
    ));
    assert_ne!(
        before,
        store.observed_trust_generation(&reader(), 5000).unwrap()
    );
    let q = TrustPopulationQueryV1 {
        active_revision: 17,
        plan_id: PLAN.into(),
        branch_id: None,
        revision: Some(17),
        from_ms: None,
        to_ms: None,
    };
    let full = store
        .observed_trust_population(&reader(), &q, 5000)
        .unwrap();
    let lean = store
        .observed_trust_suggestion_population(&reader(), &q, 5000)
        .unwrap();
    assert_eq!(lean.active_stages.len(), full.active_stages.len());
    for (a, b) in lean.active_stages.iter().zip(&full.active_stages) {
        assert_eq!(a.sample.assessment, b.sample.assessment);
        assert_eq!(a.sample.selection, b.sample.selection);
        assert_eq!(a.sample.branch_execution, b.sample.branch_execution);
        assert_eq!(a.has_later_turn, b.has_later_turn);
        assert_eq!(a.sample.run_id, b.sample.run_id);
        assert!(a.request_durations_ms.is_empty());
    }
}

#[test]
#[ignore = "large retained-population timing evidence; run explicitly in release mode"]
fn trust_suggestion_population_read_cost() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let f = Fixture::new("large-trust");
    let channel = fact_channel(&f, 256 * 1024);
    let w = writer(&store);
    assert!(matches!(
        offer_fact(
            &w,
            &channel,
            f.fact(
                1,
                finished(&f, "seed", "turn", 1, "smart_saving", 1000),
                1000
            )
        ),
        WriterCycleOutcome::Ack(_)
    ));
    // Expand a writer-produced stage using the exact current persisted columns.
    // This measures the real bounded SQL projection, not Gateway ingestion speed.
    {
        let mut connection = store.connection.lock();
        let tx = connection.transaction().unwrap();
        let columns: Vec<String> = tx
            .prepare("PRAGMA table_info(plan_quality_segments)")
            .unwrap()
            .query_map([], |r| r.get(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let select: Vec<String> = columns
            .iter()
            .map(|c| match c.as_str() {
                "segment_id" => "'bench-'||?1||'-'||?2".into(),
                "plan_id" => "?1".into(),
                "last_observed_turn_ordinal" => "?2".into(),
                _ => c.clone(),
            })
            .collect();
        let sql = format!(
            "INSERT INTO plan_quality_segments({}) SELECT {} FROM plan_quality_segments WHERE workspace_id=?3 AND segment_id='seed'",
            columns.join(","),
            select.join(",")
        );
        {
            let mut insert = tx.prepare(&sql).unwrap();
            for plan in ["plan/large-a", "plan/large-b"] {
                for index in 1..=49_999 {
                    insert
                        .execute(rusqlite::params![plan, index, f.workspace.as_str()])
                        .unwrap();
                }
            }
        }
        tx.commit().unwrap();
    }
    for plan in ["plan/large-a", "plan/large-b"] {
        let q = TrustPopulationQueryV1 {
            active_revision: 17,
            plan_id: plan.into(),
            branch_id: None,
            revision: Some(17),
            from_ms: None,
            to_ms: None,
        };
        let start = std::time::Instant::now();
        let result = store.observed_trust_suggestion_population(&reader(), &q, 5000);
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        match result {
            Ok(p) => {
                assert_eq!(p.active_stages.len(), 49_999);
                eprintln!(
                    "SUGGESTION_POPULATION plan={plan} stages=49999 cold_ms={elapsed:.3} status=available"
                );
            }
            Err(crate::query_v2::ObservationV2Error::ScopeTooLarge) => eprintln!(
                "SUGGESTION_POPULATION plan={plan} stages=49999 cold_ms={elapsed:.3} status=scope_too_large"
            ),
            Err(e) => panic!("unexpected {e:?}"),
        }
    }
}
