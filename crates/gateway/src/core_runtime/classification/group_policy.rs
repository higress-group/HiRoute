//! Per-turn two-group policy. No persistent upgrade cursor or parallel execution state.
use hiroute_domain::{ExecutionGroupV1, ModelGroupReasonV1, SMART_SAVING_SCOPE_ID};

use super::ClassificationOutcome;
use crate::agent_turn_history::{AgentTurnHistorySnapshot, ExecutionAttribution};
use crate::server::core_runtime::profiles::{
    ComplexityDecisionSourceV1, MaterializedRouteV1, PlannerInputV1, PlannerRouteIdentityV2,
};

pub(in crate::server::core_runtime) fn apply(
    input: &PlannerInputV1,
    _history: &AgentTurnHistorySnapshot,
    outcome: &mut ClassificationOutcome,
) {
    if outcome.decision.decision_source == ComplexityDecisionSourceV1::Inherited {
        return;
    }
    let (judgment, name, scope_id, dual) = match &input.policy.route {
        MaterializedRouteV1::SmartSaving { judgment, .. } => (
            judgment.as_ref(),
            "Smart saving",
            SMART_SAVING_SCOPE_ID,
            true,
        ),
        MaterializedRouteV1::Branches { branches, .. } => {
            let Some(branch) = branches.iter().find(|b| b.id == outcome.decision.branch_id) else {
                return;
            };
            (
                &branch.judgment,
                branch.name.as_str(),
                branch.id.as_str(),
                branch.primary_group.is_some(),
            )
        }
        _ => return,
    };
    let policy = judgment.execution_policy(name);
    outcome.decision.policy = Some(policy.clone());
    outcome.decision.competence_trigger = None;
    outcome.decision.simple_threshold_millis = None;
    if outcome.decision.decision_source != ComplexityDecisionSourceV1::ExternalClassifier {
        if matches!(input.policy.route, MaterializedRouteV1::Branches { .. }) {
            outcome.decision.execution_group = if dual {
                ExecutionGroupV1::Primary
            } else {
                ExecutionGroupV1::Regular
            };
        }
        outcome.decision.selection_reason = if outcome.decision.fallback_used {
            ModelGroupReasonV1::DecisionFallback
        } else {
            ModelGroupReasonV1::Heuristic
        };
        return;
    }
    if !dual {
        outcome.decision.execution_group = ExecutionGroupV1::Regular;
        outcome.decision.selection_reason = ModelGroupReasonV1::SingleGroup;
        return;
    }
    outcome.decision.simple_threshold_millis = Some(judgment.degree.simple_threshold_millis);
    let Some(probability) = outcome
        .decision
        .simple_probability
        .as_ref()
        .and_then(serde_json::Number::as_f64)
    else {
        outcome.decision.execution_group = ExecutionGroupV1::Primary;
        outcome.decision.selection_reason = ModelGroupReasonV1::DegreeUnavailable;
        return;
    };
    if probability < f64::from(judgment.degree.simple_threshold_millis) / 1000.0 {
        outcome.decision.execution_group = ExecutionGroupV1::Primary;
        outcome.decision.selection_reason = ModelGroupReasonV1::ComplexTask;
        return;
    }
    outcome.decision.execution_group = ExecutionGroupV1::Regular;
    outcome.decision.selection_reason = ModelGroupReasonV1::SimpleTask;
    let Some(assessment) = &outcome.assessment else {
        return;
    };
    if assessment.partial
        || !assessment.score.is_finite()
        || assessment.score < 0.0
        || assessment.score >= f64::from(judgment.competence.floor_millis) / 1000.0
    {
        return;
    }
    if !matches!(&input.policy.identity, PlannerRouteIdentityV2::Plan { plan_id, revision }
        if *plan_id == assessment.target.plan.plan_id && *revision == assessment.target.plan.plan_revision)
    {
        return;
    }
    let ExecutionAttribution::Single {
        selected_branch_id,
        executed_branch_id,
        ..
    } = &assessment.target.attribution
    else {
        return;
    };
    let Some(previous) = &assessment.target.branch_execution else {
        return;
    };
    if selected_branch_id != scope_id || executed_branch_id != scope_id || previous.policy != policy
    {
        return;
    }
    outcome.decision.execution_group = ExecutionGroupV1::Primary;
    outcome.decision.selection_reason = ModelGroupReasonV1::LowCompetence;
    outcome.decision.competence_trigger =
        serde_json::Number::from_f64(assessment.score).map(|score| {
            hiroute_domain::CompetenceProtectionV1 {
                segment_id: assessment.target.segment_id.clone(),
                score,
                floor_millis: judgment.competence.floor_millis,
                from_group: previous.group,
            }
        });
}

/// Attribute the actual frozen position, never infer it from model-list membership.
pub(in crate::server::core_runtime) fn execution_position(
    input: &PlannerInputV1,
    candidate: &crate::server::core_runtime::profiles::FrozenCandidateV1,
) -> (String, Option<hiroute_domain::BranchExecutionV1>) {
    let position = match &input.policy.route {
        MaterializedRouteV1::SmartSaving {
            judgment,
            simple_group_id,
            complex_group_id,
            ..
        } => {
            let group = if candidate.group_id == *simple_group_id {
                ExecutionGroupV1::Regular
            } else if candidate.group_id == *complex_group_id {
                ExecutionGroupV1::Primary
            } else {
                return ("unknown".into(), None);
            };
            Some((
                SMART_SAVING_SCOPE_ID,
                judgment.execution_policy("Smart saving"),
                group,
            ))
        }
        MaterializedRouteV1::Branches { branches, .. } => branches.iter().find_map(|branch| {
            let group = if branch.group.as_name() == candidate.group_id {
                ExecutionGroupV1::Regular
            } else if branch
                .primary_group
                .is_some_and(|g| g.as_name() == candidate.group_id)
            {
                ExecutionGroupV1::Primary
            } else {
                return None;
            };
            Some((branch.id.as_str(), branch.execution_policy(), group))
        }),
        _ => None,
    };
    let Some((scope, policy, group)) = position else {
        return ("unknown".into(), None);
    };
    let candidate_index = input
        .policy
        .groups
        .iter()
        .find(|g| g.group_id == candidate.group_id)
        .and_then(|g| {
            g.candidate_ids
                .iter()
                .position(|id| *id == candidate.candidate_id)
        })
        .unwrap_or(0) as u16;
    (
        scope.into(),
        Some(hiroute_domain::BranchExecutionV1 {
            policy,
            group,
            candidate_index,
        }),
    )
}

#[cfg(test)]
#[path = "group_policy_tests.rs"]
mod tests;
