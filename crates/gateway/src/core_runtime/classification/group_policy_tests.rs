use super::*;
use crate::agent_turn_history::{AssessmentTarget, PlanSnapshot};
use crate::server::core_runtime::classification::BoundAssessment;
use crate::server::core_runtime::profiles::planner::tests::branches::fixture;

fn history() -> AgentTurnHistorySnapshot {
    AgentTurnHistorySnapshot {
        previous_decision: None,
        visible_conversation: vec![],
        history_partial: false,
        assessment_from: None,
        assessment_target: None,
        _pin: None,
    }
}
fn outcome(
    input: &PlannerInputV1,
    probability: Option<f64>,
    score: Option<f64>,
) -> ClassificationOutcome {
    let mut decision = input.classification_decision.clone().unwrap();
    decision.simple_probability = probability.and_then(serde_json::Number::from_f64);
    let MaterializedRouteV1::Branches { branches, .. } = &input.policy.route else {
        unreachable!()
    };
    let policy = execution_policy(
        input,
        &branches[0].id,
        branches[0].judgment.as_ref().unwrap(),
        &branches[0].name,
    );
    ClassificationOutcome {
        diagnostic_failure: None,
        decision,
        facts: input.classification_facts.clone().unwrap(),
        assessment: score.map(|score| BoundAssessment {
            target: AssessmentTarget {
                evaluation_scope: None,
                history_membership: None,
                branch_execution: Some(hiroute_domain::BranchExecutionV1 {
                    policy,
                    group: ExecutionGroupV1::Regular,
                    candidate_index: 0,
                }),
                segment_id: "stage-1".into(),
                first_turn_id: "turn-1".into(),
                through_turn_id: "turn-2".into(),
                first_ordinal: 1,
                through_ordinal: 2,
                target_partial: false,
                plan: PlanSnapshot {
                    plan_id: "plan-test".into(),
                    plan_revision: 7,
                },
                attribution: ExecutionAttribution::Single {
                    selected_branch_id: "code".into(),
                    executed_branch_id: "code".into(),
                    model_configuration_id: "model-a".into(),
                    profile_digest: "profile-a".into(),
                },
            },
            score,
            partial: false,
            reason: None,
        }),
    }
}
#[test]
fn probability_and_competence_boundaries_select_the_current_group() {
    let input = fixture();
    for (p, score, expected, reason) in [
        (
            Some(0.8),
            Some(0.5),
            ExecutionGroupV1::Regular,
            ModelGroupReasonV1::SimpleTask,
        ),
        (
            Some(0.799),
            Some(1.0),
            ExecutionGroupV1::Primary,
            ModelGroupReasonV1::ComplexTask,
        ),
        (
            Some(0.8),
            Some(0.499),
            ExecutionGroupV1::Primary,
            ModelGroupReasonV1::LowCompetence,
        ),
        (
            None,
            Some(1.0),
            ExecutionGroupV1::Primary,
            ModelGroupReasonV1::DegreeUnavailable,
        ),
    ] {
        let mut result = outcome(&input, p, score);
        apply(&input, &history(), &mut result);
        assert_eq!(result.decision.execution_group, expected);
        assert_eq!(result.decision.selection_reason, reason);
    }
}
#[test]
fn new_turn_redecides_instead_of_resetting_or_preserving_the_old_group() {
    let input = fixture();
    let mut low = outcome(&input, Some(0.95), Some(0.1));
    apply(&input, &history(), &mut low);
    let mut previous = history();
    previous.previous_decision = Some((
        PlanSnapshot {
            plan_id: "plan-test".into(),
            plan_revision: 7,
        },
        low.decision,
    ));
    for (p, score, expected) in [
        (0.1, None, ExecutionGroupV1::Primary),
        (0.9, Some(0.1), ExecutionGroupV1::Primary),
        (0.9, None, ExecutionGroupV1::Regular),
        (0.9, Some(1.0), ExecutionGroupV1::Regular),
    ] {
        let mut current = outcome(&input, Some(p), score);
        apply(&input, &previous, &mut current);
        assert_eq!(current.decision.execution_group, expected);
    }
}
#[test]
fn partial_other_category_changed_version_or_standard_cannot_apply_low_score() {
    let input = fixture();
    for variant in 0..5 {
        let mut result = outcome(&input, Some(0.9), Some(0.0));
        let assessment = result.assessment.as_mut().unwrap();
        match variant {
            0 => assessment.partial = true,
            1 => assessment.target.plan.plan_revision = 6,
            2 => {
                assessment
                    .target
                    .branch_execution
                    .as_mut()
                    .unwrap()
                    .policy
                    .criteria_digest = None
            }
            3 => assessment.target.attribution = ExecutionAttribution::Mixed,
            _ => {
                if let ExecutionAttribution::Single {
                    executed_branch_id, ..
                } = &mut assessment.target.attribution
                {
                    *executed_branch_id = "writing".into();
                }
            }
        }
        apply(&input, &history(), &mut result);
        assert_eq!(result.decision.execution_group, ExecutionGroupV1::Regular);
        assert!(result.decision.competence_trigger.is_none());
    }
}
#[test]
fn single_group_and_inherited_turn_have_no_new_group_decision() {
    let input = fixture();
    let mut result = outcome(&input, None, Some(0.0));
    result.decision.branch_id = "writing".into();
    apply(&input, &history(), &mut result);
    assert_eq!(result.decision.execution_group, ExecutionGroupV1::Regular);
    assert_eq!(
        result.decision.selection_reason,
        ModelGroupReasonV1::SingleGroup
    );
    result.decision.decision_source = ComplexityDecisionSourceV1::Inherited;
    result.decision.execution_group = ExecutionGroupV1::Primary;
    let before = result.decision.clone();
    apply(&input, &history(), &mut result);
    assert_eq!(result.decision, before);
}

fn c_input() -> PlannerInputV1 {
    let mut input = fixture();
    let MaterializedRouteV1::Branches {
        branches,
        native_protocol,
        default_branch_id,
        ..
    } = &mut input.policy.route
    else {
        unreachable!()
    };
    let protocol = hiroute_domain::NativeProtocolV1::default();
    for branch in branches.iter_mut() {
        protocol.stock_assessment(branch.judgment.as_mut().unwrap());
    }
    *native_protocol = Some(
        hiroute_domain::FrozenNativeProtocolV1::freeze(
            &protocol,
            hiroute_domain::branch_decision_definition(branches, default_branch_id),
            branches
                .iter()
                .map(|b| (b.id.clone(), b.judgment.clone().unwrap())),
        )
        .unwrap(),
    );
    input
}
fn with_applicability(
    input: &PlannerInputV1,
    p: Option<f64>,
    score: Option<f64>,
    requested: bool,
    a: Option<f64>,
) -> ClassificationOutcome {
    let mut result = outcome(input, p, score);
    let protocol = native_protocol(input).unwrap();
    let mut observation = hiroute_domain::RoutingObservationV1::not_dispatched(protocol, "code");
    observation.target_segment = Some("stage-1".into());
    observation.target_complete = true;
    observation.applicability_requested = requested;
    observation.applicability_probability = a.and_then(serde_json::Number::from_f64);
    observation.applicability_state = if a.is_some() {
        hiroute_domain::ApplicabilityStateV1::Valid
    } else if requested {
        hiroute_domain::ApplicabilityStateV1::Unavailable
    } else {
        hiroute_domain::ApplicabilityStateV1::NotRequested
    };
    observation.applicability_reason = if a.is_some() {
        hiroute_domain::ApplicabilityReasonV1::Answered
    } else {
        hiroute_domain::ApplicabilityReasonV1::InvalidOutput
    };
    result.decision.routing_observation = Some(observation);
    result
}

#[test]
fn complex_work_still_records_applicable_low_score_protection() {
    use hiroute_domain::HistoricalProtectionActionV1 as A;
    let input = c_input();
    let mut result = with_applicability(&input, Some(0.7), Some(0.24), true, Some(0.55));
    result
        .assessment
        .as_mut()
        .unwrap()
        .target
        .branch_execution
        .as_mut()
        .unwrap()
        .group = ExecutionGroupV1::Primary;

    apply(&input, &history(), &mut result);

    assert_eq!(result.decision.execution_group, ExecutionGroupV1::Primary);
    assert_eq!(
        result.decision.selection_reason,
        ModelGroupReasonV1::LowCompetence
    );
    let trigger = result.decision.competence_trigger.as_ref().unwrap();
    assert_eq!(trigger.segment_id, "stage-1");
    assert_eq!(trigger.from_group, ExecutionGroupV1::Primary);
    assert_eq!(
        result
            .decision
            .routing_observation
            .as_ref()
            .unwrap()
            .protection_action,
        A::Applied
    );
}
#[test]
fn c_gate_boundaries_and_missing_required_question_preserve_independent_signals() {
    use hiroute_domain::{ApplicabilityReasonV1 as R, HistoricalProtectionActionV1 as A};
    let input = c_input();
    for (p, s, requested, a, group, action) in [
        (
            Some(0.8),
            Some(0.499),
            true,
            Some(0.499),
            ExecutionGroupV1::Regular,
            A::Suppressed,
        ),
        (
            Some(0.8),
            Some(0.499),
            true,
            Some(0.5),
            ExecutionGroupV1::Primary,
            A::Applied,
        ),
        (
            Some(0.8),
            Some(0.499),
            true,
            Some(0.501),
            ExecutionGroupV1::Primary,
            A::Applied,
        ),
        (
            Some(0.8),
            Some(0.5),
            true,
            Some(1.0),
            ExecutionGroupV1::Regular,
            A::NotNeeded,
        ),
        (
            Some(0.799),
            Some(0.1),
            true,
            Some(0.1),
            ExecutionGroupV1::Primary,
            A::Suppressed,
        ),
        (
            Some(0.9),
            Some(0.1),
            true,
            None,
            ExecutionGroupV1::Primary,
            A::RetainedUnavailable,
        ),
        (
            Some(0.9),
            Some(0.1),
            false,
            None,
            ExecutionGroupV1::Primary,
            A::RetainedUnavailable,
        ),
        (
            None,
            Some(0.1),
            true,
            Some(0.0),
            ExecutionGroupV1::Primary,
            A::NotConsidered,
        ),
    ] {
        let mut result = with_applicability(&input, p, s, requested, a);
        apply(&input, &history(), &mut result);
        assert_eq!(result.decision.execution_group, group);
        let o = result.decision.routing_observation.as_ref().unwrap();
        assert_eq!(o.protection_action, action);
        assert!(o.validate(), "{o:?}");
        if !requested {
            assert_eq!(o.applicability_reason, R::RequiredQuestionMissing);
            assert!(!o.applicability_requested);
        }
    }
}

#[test]
fn historical_gate_is_independent_of_valid_difficulty_and_preserves_base_on_release() {
    use hiroute_domain::HistoricalProtectionActionV1 as A;
    let input = c_input();
    for (p, score, a, reason, action, trigger) in [
        (
            0.9,
            0.0,
            Some(0.5),
            ModelGroupReasonV1::LowCompetence,
            A::Applied,
            true,
        ),
        (
            0.7,
            0.24,
            Some(0.55),
            ModelGroupReasonV1::LowCompetence,
            A::Applied,
            true,
        ),
        (
            0.7,
            0.24,
            Some(0.49),
            ModelGroupReasonV1::ComplexTask,
            A::Suppressed,
            false,
        ),
        (
            0.9,
            0.24,
            Some(0.49),
            ModelGroupReasonV1::SimpleTask,
            A::Suppressed,
            false,
        ),
        (
            0.7,
            0.24,
            None,
            ModelGroupReasonV1::LowCompetence,
            A::RetainedUnavailable,
            true,
        ),
        (
            0.7,
            0.5,
            Some(0.55),
            ModelGroupReasonV1::ComplexTask,
            A::NotNeeded,
            false,
        ),
    ] {
        let mut result = with_applicability(&input, Some(p), Some(score), true, a);
        apply(&input, &history(), &mut result);
        assert_eq!(result.decision.selection_reason, reason, "p={p}, a={a:?}");
        assert_eq!(result.decision.competence_trigger.is_some(), trigger);
        assert_eq!(
            result
                .decision
                .simple_probability
                .as_ref()
                .unwrap()
                .as_f64(),
            Some(p)
        );
        let observation = result.decision.routing_observation.as_ref().unwrap();
        assert_eq!(observation.protection_action, action);
        assert!(observation.validate(), "{observation:?}");
    }
}

#[test]
fn invalid_difficulty_and_ineligible_scores_never_claim_protection() {
    use hiroute_domain::HistoricalProtectionActionV1 as A;
    let input = c_input();
    for p in [None, Some(-0.1), Some(1.1)] {
        let mut result = with_applicability(&input, p, Some(0.1), true, Some(0.9));
        apply(&input, &history(), &mut result);
        assert_eq!(
            result.decision.selection_reason,
            ModelGroupReasonV1::DegreeUnavailable
        );
        assert!(result.decision.competence_trigger.is_none());
        assert_eq!(
            result
                .decision
                .routing_observation
                .unwrap()
                .protection_action,
            A::NotConsidered
        );
    }
    for score in [-0.1, 0.5, 1.1, f64::NAN] {
        let mut result = with_applicability(&input, Some(0.7), Some(score), true, Some(0.9));
        apply(&input, &history(), &mut result);
        assert_eq!(
            result.decision.selection_reason,
            ModelGroupReasonV1::ComplexTask
        );
        assert!(result.decision.competence_trigger.is_none());
    }
}

#[test]
fn invalid_applicability_retains_a_valid_complex_trigger_and_marks_unavailable() {
    use hiroute_domain::{
        ApplicabilityReasonV1 as R, ApplicabilityStateV1 as S, HistoricalProtectionActionV1 as A,
    };
    let input = c_input();
    for invalid in [-0.1, 1.1] {
        let mut result = with_applicability(&input, Some(0.7), Some(0.1), true, Some(invalid));
        apply(&input, &history(), &mut result);
        assert_eq!(
            result.decision.selection_reason,
            ModelGroupReasonV1::LowCompetence
        );
        assert!(result.decision.competence_trigger.is_some());
        let observation = result.decision.routing_observation.unwrap();
        assert_eq!(
            (
                observation.applicability_state,
                observation.applicability_reason,
                observation.protection_action
            ),
            (S::Unavailable, R::InvalidOutput, A::RetainedUnavailable)
        );
        assert!(observation.applicability_probability.is_none());
        assert!(observation.validate());
    }
}
#[test]
fn c_applicability_never_overrides_attribution_or_a_partial_target() {
    let input = c_input();
    for (variant, p) in (0..8).flat_map(|variant| [0.7, 0.9].map(|p| (variant, p))) {
        let mut result = with_applicability(&input, Some(p), Some(0.1), true, Some(1.0));
        let a = result.assessment.as_mut().unwrap();
        match variant {
            0 => {
                a.partial = true;
                result
                    .decision
                    .routing_observation
                    .as_mut()
                    .unwrap()
                    .target_complete = false;
            }
            1 => a.target.plan.plan_revision = 6,
            2 => {
                a.target
                    .branch_execution
                    .as_mut()
                    .unwrap()
                    .policy
                    .criteria_digest = None
            }
            3 => a.target.attribution = ExecutionAttribution::Mixed,
            4 => {
                if let ExecutionAttribution::Single {
                    executed_branch_id, ..
                } = &mut a.target.attribution
                {
                    *executed_branch_id = "docs".into();
                }
            }
            5 => a.target.target_partial = true,
            6 => a.target.plan.plan_id = "another-plan".into(),
            _ => a.target.branch_execution = None,
        }
        apply(&input, &history(), &mut result);
        assert_eq!(
            result.decision.execution_group,
            if p < 0.8 {
                ExecutionGroupV1::Primary
            } else {
                ExecutionGroupV1::Regular
            }
        );
        assert!(result.decision.competence_trigger.is_none());
        assert_eq!(
            result
                .decision
                .routing_observation
                .unwrap()
                .protection_action,
            hiroute_domain::HistoricalProtectionActionV1::NotConsidered
        );
    }
}

#[test]
fn c_omission_reasons_survive_final_group_policy_without_creating_protection() {
    use hiroute_domain::ProtectionEligibilityV1 as E;
    for reason in [E::Unrated, E::DifferentCategory, E::DifferentPolicy] {
        let input = c_input();
        let mut result = outcome(&input, Some(0.9), None);
        let p = native_protocol(&input).unwrap();
        let mut observation = hiroute_domain::RoutingObservationV1::not_dispatched(p, "code");
        observation.target_segment = Some("old-stage".into());
        observation.eligibility = reason;
        result.decision.routing_observation = Some(observation);
        apply(&input, &history(), &mut result);
        assert_eq!(
            result.decision.routing_observation.unwrap().eligibility,
            reason
        );
        assert_eq!(result.decision.execution_group, ExecutionGroupV1::Regular);
    }
}

#[test]
fn not_dispatched_is_unrated_while_observed_no_target_remains_no_target() {
    use hiroute_domain::{
        ApplicabilityReasonV1 as R, HistoricalProtectionActionV1 as A, ProtectionEligibilityV1 as E,
    };
    let input = c_input();
    for (eligibility, reason) in [
        (E::Unrated, R::DispatchNotStarted),
        (E::NoTarget, R::NoTarget),
    ] {
        let mut result = outcome(&input, Some(0.9), None);
        let mut observation = hiroute_domain::RoutingObservationV1::not_dispatched(
            native_protocol(&input).unwrap(),
            "code",
        );
        observation.eligibility = eligibility;
        observation.applicability_reason = reason;
        result.decision.routing_observation = Some(observation);
        apply(&input, &history(), &mut result);
        assert_eq!(result.decision.execution_group, ExecutionGroupV1::Regular);
        let observed = result.decision.routing_observation.unwrap();
        assert_eq!(
            (
                observed.eligibility,
                observed.applicability_reason,
                observed.protection_action
            ),
            (eligibility, reason, A::NotConsidered)
        );
        assert!(observed.score.is_none() && !observed.applicability_requested);
    }
}

#[test]
fn extension_group_is_authoritative_even_with_a_low_complete_assessment() {
    let mut input = fixture();
    for selected in [ExecutionGroupV1::Regular, ExecutionGroupV1::Primary] {
        let mut result = outcome(&input, Some(0.99), Some(0.0));
        let MaterializedRouteV1::Branches { extension, .. } = &mut input.policy.route else {
            unreachable!()
        };
        *extension = Some(hiroute_domain::DecisionExtensionIdentityV1 {
            id: "extension/test".into(),
            revision: 1,
        });
        result.decision.execution_group = selected;
        result.decision.selection_reason = ModelGroupReasonV1::ExtensionDecision;
        apply(&input, &history(), &mut result);
        assert_eq!(result.decision.execution_group, selected);
        assert_eq!(
            result.decision.selection_reason,
            ModelGroupReasonV1::ExtensionDecision
        );
        assert!(result.decision.simple_probability.is_none());
        assert!(result.decision.simple_threshold_millis.is_none());
        assert!(result.decision.competence_trigger.is_none());
        assert!(
            result
                .decision
                .policy
                .as_ref()
                .unwrap()
                .floor_millis
                .is_none()
        );
        assert_eq!(result.assessment.as_ref().unwrap().score, 0.0);
    }
}
