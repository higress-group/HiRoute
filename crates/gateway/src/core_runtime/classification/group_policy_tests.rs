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
    let policy = branches[0].execution_policy();
    ClassificationOutcome {
        diagnostic_failure: None,
        decision,
        facts: input.classification_facts.clone().unwrap(),
        assessment: score.map(|score| BoundAssessment {
            target: AssessmentTarget {
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
