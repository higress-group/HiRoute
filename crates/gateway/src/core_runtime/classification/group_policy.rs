//! Per-turn two-group policy. No persistent upgrade cursor or parallel execution state.
use hiroute_domain::{ExecutionGroupV1, ModelGroupReasonV1, SMART_SAVING_SCOPE_ID};

use super::ClassificationOutcome;
use crate::agent_turn_history::{AgentTurnHistorySnapshot, ExecutionAttribution};
use crate::server::core_runtime::profiles::{
    ComplexityDecisionSourceV1, MaterializedRouteV1, PlannerInputV1, PlannerRouteIdentityV2,
};

pub(in crate::server::core_runtime) fn apply(
    input: &PlannerInputV1,
    history: &AgentTurnHistorySnapshot,
    outcome: &mut ClassificationOutcome,
) {
    if outcome.decision.decision_source != ComplexityDecisionSourceV1::Inherited
        && outcome.decision.routing_observation.is_none()
        && let Some(p) = native_protocol(input)
    {
        let mut o =
            hiroute_domain::RoutingObservationV1::not_dispatched(p, &outcome.decision.branch_id);
        if let Some(a) = &outcome.assessment {
            o.target_segment = Some(a.target.segment_id.clone());
            o.target_complete = !a.partial;
        }
        outcome.decision.routing_observation = Some(o);
    }
    reduce(input, history, outcome);
    finalize_observation(input, outcome);
}

fn reduce(
    input: &PlannerInputV1,
    _history: &AgentTurnHistorySnapshot,
    outcome: &mut ClassificationOutcome,
) {
    if outcome.decision.decision_source == ComplexityDecisionSourceV1::Inherited {
        return;
    }
    if let Some(extension) = extension_identity(input) {
        let (name, dual) = match &input.policy.route {
            MaterializedRouteV1::SmartSaving { .. } => ("Smart saving", true),
            MaterializedRouteV1::Branches { branches, .. } => {
                let Some(branch) = branches
                    .iter()
                    .find(|branch| branch.id == outcome.decision.branch_id)
                else {
                    return;
                };
                (branch.name.as_str(), branch.primary_group.is_some())
            }
            _ => return,
        };
        outcome.decision.policy = Some(extension.execution_policy(name));
        outcome.decision.simple_threshold_millis = None;
        outcome.decision.simple_probability = None;
        outcome.decision.competence_trigger = None;
        if outcome.decision.fallback_used {
            if matches!(input.policy.route, MaterializedRouteV1::Branches { .. }) {
                outcome.decision.execution_group = if dual {
                    ExecutionGroupV1::Primary
                } else {
                    ExecutionGroupV1::Regular
                };
            }
            outcome.decision.selection_reason = ModelGroupReasonV1::DecisionFallback;
        } else if !dual {
            outcome.decision.execution_group = ExecutionGroupV1::Regular;
        }
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
                branch
                    .judgment
                    .as_ref()
                    .expect("validated built-in judgment"),
                branch.name.as_str(),
                branch.id.as_str(),
                branch.primary_group.is_some(),
            )
        }
        _ => return,
    };
    let policy = execution_policy(input, scope_id, judgment, name);
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
        .filter(|p| p.is_finite() && (0.0..=1.0).contains(p))
    else {
        outcome.decision.execution_group = ExecutionGroupV1::Primary;
        outcome.decision.selection_reason = ModelGroupReasonV1::DegreeUnavailable;
        return;
    };
    if probability < f64::from(judgment.degree.simple_threshold_millis) / 1000.0 {
        outcome.decision.execution_group = ExecutionGroupV1::Primary;
        outcome.decision.selection_reason = ModelGroupReasonV1::ComplexTask;
    } else {
        outcome.decision.execution_group = ExecutionGroupV1::Regular;
        outcome.decision.selection_reason = ModelGroupReasonV1::SimpleTask;
    }
    let Some(assessment) = &outcome.assessment else {
        return;
    };
    if assessment.partial
        || assessment.target.target_partial
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
    if native_protocol(input).is_some() {
        use hiroute_domain::{
            ApplicabilityReasonV1 as R, ApplicabilityStateV1 as S,
            HistoricalProtectionActionV1 as A,
        };
        if let Some(o) = outcome.decision.routing_observation.as_mut() {
            if !o.applicability_requested {
                o.applicability_reason = R::RequiredQuestionMissing;
                o.protection_action = A::RetainedUnavailable;
            } else if let Some(a) = o
                .applicability_probability
                .as_ref()
                .and_then(serde_json::Number::as_f64)
                .filter(|a| {
                    o.applicability_state == S::Valid && a.is_finite() && (0.0..=1.0).contains(a)
                })
            {
                if a < 0.5 {
                    o.protection_action = A::Suppressed;
                    return;
                }
                o.protection_action = A::Applied;
            } else {
                if o.applicability_probability.take().is_some()
                    || o.applicability_state != S::Unavailable
                {
                    o.applicability_state = S::Unavailable;
                    o.applicability_reason = R::InvalidOutput;
                }
                o.protection_action = A::RetainedUnavailable;
            }
        }
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

fn extension_identity(
    input: &PlannerInputV1,
) -> Option<&hiroute_domain::DecisionExtensionIdentityV1> {
    match &input.policy.route {
        MaterializedRouteV1::SmartSaving { extension, .. }
        | MaterializedRouteV1::Branches { extension, .. } => extension.as_ref(),
        _ => None,
    }
}

fn native_protocol(input: &PlannerInputV1) -> Option<&hiroute_domain::FrozenNativeProtocolV1> {
    match &input.policy.route {
        MaterializedRouteV1::SmartSaving {
            native_protocol, ..
        }
        | MaterializedRouteV1::Branches {
            native_protocol, ..
        } => native_protocol.as_ref(),
        _ => None,
    }
}

fn execution_policy(
    input: &PlannerInputV1,
    scope: &str,
    judgment: &hiroute_domain::JudgmentSettingsV1,
    name: &str,
) -> hiroute_domain::BranchExecutionPolicyV1 {
    if let Some(extension) = extension_identity(input) {
        return extension.execution_policy(name);
    }
    native_protocol(input).map_or_else(
        || judgment.execution_policy(name),
        |p| p.execution_policy(scope, judgment, name),
    )
}

fn finalize_observation(input: &PlannerInputV1, outcome: &mut ClassificationOutcome) {
    use hiroute_domain::{HistoricalProtectionActionV1 as A, ProtectionEligibilityV1 as E};
    if outcome.decision.decision_source == ComplexityDecisionSourceV1::Inherited {
        return;
    }
    let Some(p) = native_protocol(input) else {
        return;
    };
    let Some(o) = outcome.decision.routing_observation.as_mut() else {
        return;
    };
    let Some(scope) = p.scopes.get(&outcome.decision.branch_id) else {
        return;
    };
    o.standard = scope.standard.clone();
    o.simple_probability = outcome.decision.simple_probability.clone();
    o.simple_threshold_millis = scope.simple_threshold_millis;
    o.competence_floor_millis = scope.competence_floor_millis;
    o.score = outcome
        .assessment
        .as_ref()
        .and_then(|a| serde_json::Number::from_f64(a.score));
    let dual = match &input.policy.route {
        MaterializedRouteV1::Branches { branches, .. } => branches
            .iter()
            .find(|b| b.id == outcome.decision.branch_id)
            .is_some_and(|b| b.primary_group.is_some()),
        _ => true,
    };
    o.eligibility = if !dual {
        E::SingleGroup
    } else if !o.target_complete && !matches!(o.eligibility, E::NoTarget | E::PartialTarget) {
        o.eligibility
    } else if o.target_segment.is_none() {
        E::NoTarget
    } else if !o.target_complete {
        E::PartialTarget
    } else if let Some(a) = &outcome.assessment {
        if a.partial
            || a.target.target_partial
            || !a.score.is_finite()
            || !(0.0..=1.0).contains(&a.score)
        {
            E::Unrated
        } else if !matches!(&input.policy.identity, PlannerRouteIdentityV2::Plan { plan_id, revision } if *plan_id == a.target.plan.plan_id && *revision == a.target.plan.plan_revision)
        {
            E::DifferentPlan
        } else if let ExecutionAttribution::Single {
            selected_branch_id,
            executed_branch_id,
            ..
        } = &a.target.attribution
        {
            if *selected_branch_id != outcome.decision.branch_id
                || *executed_branch_id != outcome.decision.branch_id
            {
                E::DifferentCategory
            } else if a
                .target
                .branch_execution
                .as_ref()
                .is_none_or(|e| Some(&e.policy) != outcome.decision.policy.as_ref())
            {
                E::DifferentPolicy
            } else {
                E::Eligible
            }
        } else {
            E::AmbiguousAttribution
        }
    } else {
        E::Unrated
    };
    if o.protection_action == A::NotConsidered
        && o.eligibility == E::Eligible
        && matches!(
            outcome.decision.selection_reason,
            ModelGroupReasonV1::SimpleTask | ModelGroupReasonV1::ComplexTask
        )
    {
        o.protection_action = A::NotNeeded;
    }
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
                execution_policy(input, SMART_SAVING_SCOPE_ID, judgment, "Smart saving"),
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
            Some((
                branch.id.as_str(),
                extension_identity(input).map_or_else(
                    || {
                        execution_policy(
                            input,
                            &branch.id,
                            branch
                                .judgment
                                .as_ref()
                                .expect("validated built-in judgment"),
                            &branch.name,
                        )
                    },
                    |extension| extension.execution_policy(&branch.name),
                ),
                group,
            ))
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
