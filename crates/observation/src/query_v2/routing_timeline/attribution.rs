//! Display attribution from accepted execution receipts, independent of scoring history.
use super::*;

#[derive(PartialEq, Eq)]
pub(super) struct AcceptedCandidate {
    group: ExecutionGroupV1,
    group_id: String,
    declared_order: u32,
    model_configuration_id: String,
    profile_digest: ObservedProfileDigestV1,
    stable_binding_id: String,
}

impl AcceptedCandidate {
    pub(super) fn from_facts(
        candidate: &CandidateDecisionFactV1,
        decision: &RouteDecisionFactV1,
    ) -> Option<Self> {
        if decision.native_auxiliary
            || decision.complexity.is_none()
            || !candidate.eligible
            || candidate.profile_digest.as_str() == "unknown"
            || !decision.groups.iter().any(|group| {
                group.group_id == candidate.group_id
                    && group.ranked_candidate_ids.contains(&candidate.candidate_id)
            })
        {
            return None;
        }
        let group = match (decision.branch, candidate.group_id.as_str()) {
            (
                PlannedBranchV1::SmartSavingSimple | PlannedBranchV1::SmartSavingComplex,
                "economy",
            ) => ExecutionGroupV1::Regular,
            (
                PlannedBranchV1::SmartSavingSimple | PlannedBranchV1::SmartSavingComplex,
                "primary",
            ) => ExecutionGroupV1::Primary,
            (PlannedBranchV1::CustomExactOrder, id) => {
                if branch_group(id, "branch_primary_") {
                    ExecutionGroupV1::Primary
                } else if branch_group(id, "branch_") {
                    ExecutionGroupV1::Regular
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        Some(Self {
            group,
            group_id: candidate.group_id.clone(),
            declared_order: candidate.declared_order,
            model_configuration_id: candidate.model_configuration_id.clone(),
            profile_digest: candidate.profile_digest.clone(),
            stable_binding_id: candidate.stable_binding_id.clone(),
        })
    }
}

fn branch_group(id: &str, prefix: &str) -> bool {
    id.strip_prefix(prefix).is_some_and(|index| {
        index
            .parse::<u16>()
            .is_ok_and(|number| number.to_string() == index)
    })
}

pub(super) fn uniform_group(
    members: &[&Request],
    plan_id: &AgentPlanId,
    plan_revision: u64,
) -> Option<ExecutionGroupV1> {
    let accepted = members.first()?.accepted_candidate.as_ref()?;
    members
        .iter()
        .all(|request| {
            !request.partial
                && request.accepted_candidate.as_ref() == Some(accepted)
                && request.decision.as_ref().is_some_and(|decision| {
                    decision.plan_id.as_ref() == Some(plan_id)
                        && decision.route.plan_revision() == Some(plan_revision)
                })
        })
        .then_some(accepted.group)
}
