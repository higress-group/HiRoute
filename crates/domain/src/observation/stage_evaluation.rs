//! A task stage can be fully observed without belonging to one model.
use serde::{Deserialize, Serialize};

use super::{ExecutionFactError, HistoryMembershipV1, LogicalRequestId, TurnId};
use crate::{AgentPlanId, BranchExecutionPolicyV1};

/// The effective common rubric of actual executions, frozen before evaluation.
/// It does not identify an executor or a model group.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageEvaluationScopeV1 {
    pub version: u8,
    pub executed_branch_id: String,
    pub policy: BranchExecutionPolicyV1,
}

impl StageEvaluationScopeV1 {
    pub fn validate(&self) -> Result<(), ExecutionFactError> {
        if self.version != 1
            || self.executed_branch_id.trim().is_empty()
            || self.executed_branch_id.len() > 128
            || self.executed_branch_id.chars().any(char::is_control)
            || !self.policy.validate()
            || self.policy.extension.is_some()
            || self.policy.criteria_digest.is_none()
        {
            return Err(ExecutionFactError::InvalidFact);
        }
        Ok(())
    }
}

/// Composite quality is retained as stage evidence, never as a model score.
/// Existing single-model assessment records keep their original format.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageAssessmentRecordedV1 {
    pub scope: StageEvaluationScopeV1,
    pub history_membership: HistoryMembershipV1,
    pub segment_id: String,
    pub plan_id: AgentPlanId,
    pub plan_revision: u64,
    pub trigger_request_id: LogicalRequestId,
    pub target_from_turn_id: TurnId,
    pub target_through_turn_id: TurnId,
    pub target_from_ordinal: u64,
    pub target_through_ordinal: u64,
    pub assessed_at_ms: u64,
    pub score: f64,
    pub partial: bool,
    pub reason: Option<String>,
}

impl StageAssessmentRecordedV1 {
    pub fn validate(&self) -> Result<(), ExecutionFactError> {
        self.scope.validate()?;
        self.history_membership.validate()?;
        if self.history_membership.request_ids.is_empty()
            || (!self.partial
                && (self.history_membership.clipped || self.history_membership.capture_partial))
            || self.segment_id.trim().is_empty()
            || self.segment_id.len() > 128
            || self.plan_revision == 0
            || self.target_from_ordinal == 0
            || self.target_from_ordinal > self.target_through_ordinal
            || self.assessed_at_ms == 0
            || !self.score.is_finite()
            || !(0.0..=1.0).contains(&self.score)
            || self
                .reason
                .as_ref()
                .is_some_and(|reason| reason.trim().is_empty() || reason.chars().count() > 1024)
        {
            return Err(ExecutionFactError::InvalidFact);
        }
        Ok(())
    }
}
