use super::*;
use hiroute_domain::delegation::RunStateV1;
use serde_json::json;

#[test]
fn executor_task_outcomes_do_not_depend_on_recorded_model_stages() {
    let tasks = [
        TrustTaskOutcomeV1 {
            task_id: "task/empty".into(),
            executor: "dsh".into(),
            ordinal: 1,
            state: RunStateV1::Accepted,
        },
        TrustTaskOutcomeV1 {
            task_id: "task/continued".into(),
            executor: "dsh".into(),
            ordinal: 2,
            state: RunStateV1::Succeeded,
        },
    ];
    let groups = executor_outcomes(&tasks);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].task_outcomes["submitted"], 2);
    assert_eq!(groups[0].task_outcomes["running"], 1);
    assert_eq!(groups[0].task_outcomes["succeeded"], 1);
    assert_eq!(groups[0].task_outcomes["continued"], 1);
}

fn stage(id: &str, assessment: Option<(f64, bool)>, later: bool) -> TrustObservedStageV1 {
    let sample = serde_json::from_value(json!({
        "selection": null, "branch_execution": {"policy":{"name":"Trial", "floor_millis":500, "criteria_digest":null}, "group":"regular", "candidate_index":0}, "upgrade":null,
        "segment_id":id, "session_id":"session/a", "plan_id":"plan/a", "plan_revision":1,
        "selected_branch_id":"smart_saving", "executed_branch_id":"smart_saving", "model_configuration_id":"model/trial", "profile_digest":null, "native_model":"trial", "reasoning_profile_id":null,
        "attribution":"single", "first_turn_id":id, "first_turn_ordinal":1, "last_observed_turn_id":id, "last_observed_turn_ordinal":1,
        "first_at_ms":1000, "last_at_ms":1010, "history_partial":false, "first_request_id":"request/1", "last_request_id":"request/1", "execution_evidence_available":true,
        "task_id":"task/a", "run_id":"run/a",
        "assessment": assessment.map(|(score, partial)| json!({"event_id":"assessment/1", "trigger_request_id":"request/2", "assessed_at_ms":1100, "target_from_turn_id":id, "target_through_turn_id":id, "target_from_ordinal":1, "target_through_ordinal":1, "score":score, "partial":partial, "reason":null, "evidence_available":true}))
    })).unwrap();
    TrustObservedStageV1 {
        sample,
        executor: Some("deepseek_harness".into()),
        has_later_turn: later,
        request_durations_ms: vec![],
        output_tokens_per_second: vec![],
    }
}
#[test]
fn score_states_are_exclusive_and_terminal_completion_does_not_mean_competence() {
    let stages = [
        stage("complete", Some((0.5, false)), false),
        stage("partial", Some((1.0, true)), false),
        stage("waiting", None, false),
        stage("unscored", None, true),
    ];
    let refs = stages.iter().collect::<Vec<_>>();
    let measured = counts(&refs, &BTreeMap::new());
    assert_eq!(
        measured,
        TrustCountsV1 {
            executed: 4,
            complete: 1,
            competence_eligible: 1,
            meets_floor: 1,
            partial: 1,
            awaiting_assessment: 1,
            unscored: 1,
            sessions: 1
        }
    );
    assert_eq!(measured.scoring_coverage(), Some(0.25));
    assert_eq!(measured.competence_rate(), Some(1.0));
    let runs = BTreeMap::from([(
        "run/a".into(),
        TrustRunV1 {
            run_id: "run/a".into(),
            task_id: "task/a".into(),
            state: RunStateV1::Succeeded,
            ordinal: 1,
        },
    )]);
    let terminal = counts(&refs, &runs);
    assert_eq!(terminal.awaiting_assessment, 0);
    assert_eq!(terminal.unscored, 2);
    assert_eq!(terminal.complete, 1);
    assert_eq!(terminal.partial, 1);
}
fn selection(probability: f64, reason: &str, group: &str) -> BranchDecisionV1 {
    serde_json::from_value(json!({"policy":null,"competence_trigger":null,"execution_group":group,"simple_probability":probability,"simple_threshold_millis":800,"selection_reason":reason,"strategy_id":"test", "schema_version":"test", "payload_digest":CanonicalDigest::of_bytes(b"test"),"branch_id":"smart_saving", "complexity_score":null,"threshold":null,"decision_source":"external_classifier","reason_codes":[],"matched_user_phrase_ids":[],"fallback_used":false,"classification_duration_micros":100,"fallback_reason":null})).unwrap()
}
#[test]
fn replay_keeps_equality_protection_and_actual_relay_separate() {
    let mut equality = stage("equal", None, false);
    equality.sample.selection = Some(selection(0.8, "simple_task", "regular"));
    let mut protected = stage("protected", None, false);
    protected.sample.selection = Some(selection(0.7, "low_competence", "primary"));
    protected
        .sample
        .selection
        .as_mut()
        .unwrap()
        .competence_trigger = Some(CompetenceProtectionV1 {
        segment_id: "previous-primary".into(),
        score: serde_json::Number::from_f64(0.24).unwrap(),
        floor_millis: 500,
        from_group: ExecutionGroupV1::Primary,
    });
    protected.sample.branch_execution.as_mut().unwrap().group = ExecutionGroupV1::Primary;
    let mut newly_regular = stage("new", None, false);
    newly_regular.sample.selection = Some(selection(0.72, "complex_task", "primary"));
    newly_regular
        .sample
        .branch_execution
        .as_mut()
        .unwrap()
        .group = ExecutionGroupV1::Primary;
    let mut relayed = equality.clone();
    relayed.sample.branch_execution.as_mut().unwrap().group = ExecutionGroupV1::Primary;
    let result = replay(&[equality, protected, newly_regular, relayed], &[800, 650]);
    assert_eq!(result.included, 3);
    assert_eq!(
        result.excluded_by_reason.get("availability_relay"),
        Some(&1)
    );
    assert_eq!(result.thresholds[0].regular, 1);
    assert_eq!(result.thresholds[1].regular, 2);
    assert_eq!(result.thresholds[1].newly_regular, 1);
    assert_eq!(result.thresholds[1].protected_as_primary, 1);
}

pub(crate) fn trial_context(threshold: u16) -> TrustContextV1 {
    let legacy = crate::compiler::test_fixtures::custom_desired();
    let candidate = CandidateSelectionV1 {
        binding_id: "binding/trial".into(),
        reasoning: None,
    };
    let mut judgment = JudgmentSettingsV1::default();
    judgment.degree.simple_threshold_millis = threshold;
    TrustContextV1 {
        active: PlanExecutionRef {
            workspace_id: WorkspaceId::default(),
            plan_id: AgentPlanId::parse("plan/a").unwrap(),
            content_revision: 1,
            content_digest: CanonicalDigest::of_bytes(b"plan"),
        },
        enabled: true,
        configuration: AgentPlanAuthoringV2 {
            schema: PLAN_AUTHORING_SCHEMA_V2.into(),
            display_name: legacy.display_name,
            purpose: legacy.purpose,
            mode: PlanEditorMode::SmartSaving,
            requirements: legacy.requirements,
            limits: legacy.limits,
            strategy: AgentPlanStrategyV2::SmartSaving {
                economy: vec![candidate.clone()],
                primary: vec![candidate],
                judgment,
                native_protocol: None,
                reselect_on_user_message: false,
                complex_keywords: vec![],
                classifier: ComplexityClassifierModeV1::DecisionService {
                    service: Box::new(DecisionServiceV1 {
                        id: "decision/test".into(),
                        revision: 1,
                        name: "Test".into(),
                        connection: DecisionConnectionV1::SystemOne {
                            provider: "compatible".into(),
                            model: "synthetic".into(),
                            endpoint: "https://decision.example/choose".into(),
                            timeout_ms: 3000,
                            auth_header: ClassifierAuthHeaderV1 {
                                name: "Authorization".into(),
                                value_secret_ref: "synthetic".into(),
                            },
                        },
                    }),
                },
            },
            delegation_enabled: false,
            work: None,
        },
        binding_models: BTreeMap::from([("binding/trial".into(), "model/trial".into())]),
        identity_stability: BTreeMap::from([("model/trial".into(), "rolling-alias".into())]),
        announcements: vec![],
        generation: CanonicalDigest::of_bytes(b"context"),
    }
}
pub(crate) fn trial_population(count: usize, score: f64) -> TrustPopulationV1 {
    let stages: Vec<_> = (0..count)
        .map(|i| {
            let mut s = stage(&format!("stage/{i}"), Some((score, false)), true);
            s.sample.session_id = format!("session/{}", i % 3);
            s.sample.run_id = None;
            s.sample.selection = Some(selection(0.94, "simple_task", "regular"));
            s
        })
        .collect();
    TrustPopulationV1 {
        active_stages: stages.clone(),
        stages,
        retained_from_ms: 0,
        from_ms: 0,
        to_ms: 5000,
        generation: 1,
    }
}
#[test]
fn live_trust_suggestion_counts_protected_primary_stages() {
    let context = trial_context(900);
    let mut population = trial_population(40, 0.9);
    let mut protected = stage("protected", Some((0.9, false)), true);
    protected.sample.branch_execution.as_mut().unwrap().group = ExecutionGroupV1::Primary;
    let mut decision = selection(0.7, "low_competence", "primary");
    decision.competence_trigger = Some(
        serde_json::from_value(json!({
            "segment_id":"stage/0", "score":0.1, "floor_millis":500, "from_group":"primary"
        }))
        .unwrap(),
    );
    protected.sample.selection = Some(decision);
    population.stages.push(protected.clone());
    population.active_stages.push(protected);
    let query = PlanTrustQueryV1 {
        plan_id: "plan/a".into(),
        branch_id: None,
        revision: Default::default(),
        from_ms: None,
        to_ms: None,
        group_by: Default::default(),
    };
    let evidence = suggestion_evidence(&query, &context, &population, &BTreeMap::new());
    assert_eq!(evidence["branches"][0]["protection_count"], 1);
    assert_eq!(evidence["branches"][0]["upward_cohort"]["complete"], 40);
    let full = report(&query, &context, &population, &BTreeMap::new(), &[]);
    let protections = full
        .rows
        .iter()
        .flat_map(|row| &row.protections)
        .collect::<Vec<_>>();
    assert_eq!(protections.len(), 1);
    assert_eq!(protections[0].segment_id, "protected");
    assert_eq!(protections[0].trigger_segment_id, "stage/0");
}

#[test]
fn suggestion_projection_matches_report_for_every_direction_without_report_rows() {
    for (threshold, count, score) in [
        (800, 40, 0.9),
        (650, 40, 0.4),
        (500, 40, 0.9),
        (900, 40, 0.4),
        (800, 10, 0.9),
    ] {
        let context = trial_context(threshold);
        let population = trial_population(count, score);
        let query = PlanTrustQueryV1 {
            plan_id: "plan/a".into(),
            branch_id: None,
            revision: Default::default(),
            from_ms: None,
            to_ms: None,
            group_by: Default::default(),
        };
        let full = report(&query, &context, &population, &BTreeMap::new(), &[]);
        let lean = suggestion_evidence(&query, &context, &population, &BTreeMap::new());
        assert!(lean.get("rows").is_none());
        assert!(lean.get("sessions").is_none());
        let expected: Vec<_> = full
            .branches
            .iter()
            .filter(|b| !matches!(b.suggestion, TrustSuggestionV1::Keep { .. }))
            .collect();
        assert_eq!(lean["branches"].as_array().unwrap().len(), expected.len());
        for (actual, expected) in lean["branches"].as_array().unwrap().iter().zip(expected) {
            assert_eq!(
                actual["suggestion"],
                serde_json::to_value(&expected.suggestion).unwrap()
            );
            assert_eq!(
                actual["full_history"],
                serde_json::to_value(&expected.full_history).unwrap()
            );
            assert_eq!(
                actual["upward_cohort"],
                serde_json::to_value(&expected.upward_cohort).unwrap()
            );
            assert_eq!(actual["identity_stabilities"], json!(["rolling-alias"]));
            assert!(actual["replay"]["thresholds"].as_array().unwrap().len() <= 1);
        }
    }
}

#[test]
fn extension_assessments_count_as_scored_without_a_competence_denominator() {
    let mut extension = stage("extension", Some((1.0, false)), false);
    extension.sample.branch_execution.as_mut().unwrap().policy =
        hiroute_domain::DecisionExtensionIdentityV1 {
            id: "extension/test".into(),
            revision: 1,
        }
        .execution_policy("Extension");
    let extension_only = counts(&[&extension], &BTreeMap::new());
    assert_eq!(extension_only.complete, 1);
    assert_eq!(extension_only.unscored, 0);
    assert_eq!(extension_only.competence_rate(), None);
    let native = stage("native", Some((0.5, false)), false);
    let mixed = counts(&[&extension, &native], &BTreeMap::new());
    assert_eq!(mixed.complete, 2);
    assert_eq!(mixed.scoring_coverage(), Some(1.0));
    assert_eq!(mixed.competence_rate(), Some(1.0));
    let mut old_extension = native;
    old_extension.sample.selection = Some(selection(0.8, "simple_task", "regular"));
    old_extension
        .sample
        .selection
        .as_mut()
        .unwrap()
        .decision_protocol = Some(hiroute_domain::DecisionProtocolV1::CustomExtensionV1);
    assert_eq!(replay(&[old_extension], &[700]).included, 0);
}
