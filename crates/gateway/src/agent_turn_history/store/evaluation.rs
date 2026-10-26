//! Evaluation identity is separate from executor attribution and capture gaps.
use super::*;

pub(super) fn segment_key(
    active: &ActiveTurn,
    attribution: &ExecutionAttribution,
) -> Option<SegmentKey> {
    match attribution {
        ExecutionAttribution::Single {
            executed_branch_id,
            model_configuration_id,
            profile_digest,
            ..
        } => Some(SegmentKey {
            branch_execution: active.executions.first()?.branch_execution.clone(),
            plan: active.plan.clone(),
            selected_branch_id: active.decision.as_ref()?.branch_id.clone(),
            executed_branch_id: executed_branch_id.clone(),
            model_configuration_id: Some(model_configuration_id.clone()),
            profile_digest: Some(profile_digest.clone()),
            evaluation_scope: None,
        }),
        ExecutionAttribution::Mixed => {
            let first = active.executions.first()?;
            let policy = &first.branch_execution.as_ref()?.policy;
            if active.executions.iter().any(|execution| {
                execution.executed_branch_id != first.executed_branch_id
                    || execution.branch_execution.as_ref().map(|e| &e.policy) != Some(policy)
            }) {
                return None;
            }
            let scope = hiroute_domain::StageEvaluationScopeV1 {
                version: 1,
                executed_branch_id: first.executed_branch_id.clone(),
                policy: policy.clone(),
            };
            scope.validate().ok()?;
            Some(SegmentKey {
                branch_execution: None,
                plan: active.plan.clone(),
                selected_branch_id: active.decision.as_ref()?.branch_id.clone(),
                executed_branch_id: first.executed_branch_id.clone(),
                model_configuration_id: None,
                profile_digest: None,
                evaluation_scope: Some(scope),
            })
        }
        ExecutionAttribution::Unknown => None,
    }
}

pub(super) fn scope_bytes(scope: &Option<hiroute_domain::StageEvaluationScopeV1>) -> usize {
    scope.as_ref().map_or(0, |scope| {
        512 + scope.executed_branch_id.capacity() + scope.policy.name.capacity()
    })
}
