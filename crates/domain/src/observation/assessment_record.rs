//! One interpretation of model and composite-stage assessment records.
use super::{
    ExecutionFactV1, HistoryMembershipV1, LogicalRequestId, ObservedProfileDigestV1,
    StageEvaluationScopeV1, TurnId,
};
use crate::AgentPlanId;

#[derive(Clone, Copy)]
pub struct AssessmentRecordRef<'a> {
    pub history_membership: Option<&'a HistoryMembershipV1>,
    pub segment_id: &'a str,
    pub plan_id: &'a AgentPlanId,
    pub plan_revision: u64,
    pub model_configuration_id: Option<&'a str>,
    pub profile_digest: Option<&'a ObservedProfileDigestV1>,
    pub stage_scope: Option<&'a StageEvaluationScopeV1>,
    pub trigger_request_id: &'a LogicalRequestId,
    pub target_from_turn_id: &'a TurnId,
    pub target_through_turn_id: &'a TurnId,
    pub target_from_ordinal: u64,
    pub target_through_ordinal: u64,
    pub assessed_at_ms: u64,
    pub score: f64,
    pub partial: bool,
    pub reason: Option<&'a str>,
}

impl ExecutionFactV1 {
    pub fn assessment_record(&self) -> Option<AssessmentRecordRef<'_>> {
        match self {
            Self::BranchAssessmentRecorded {
                history_membership,
                segment_id,
                plan_id,
                plan_revision,
                model_configuration_id,
                profile_digest,
                trigger_request_id,
                target_from_turn_id,
                target_through_turn_id,
                target_from_ordinal,
                target_through_ordinal,
                assessed_at_ms,
                score,
                partial,
                reason,
            } => Some(AssessmentRecordRef {
                history_membership: history_membership.as_ref(),
                segment_id,
                plan_id,
                plan_revision: *plan_revision,
                model_configuration_id: Some(model_configuration_id),
                profile_digest: Some(profile_digest),
                stage_scope: None,
                trigger_request_id,
                target_from_turn_id,
                target_through_turn_id,
                target_from_ordinal: *target_from_ordinal,
                target_through_ordinal: *target_through_ordinal,
                assessed_at_ms: *assessed_at_ms,
                score: *score,
                partial: *partial,
                reason: reason.as_deref(),
            }),
            Self::StageAssessmentRecorded(value) => Some(AssessmentRecordRef {
                history_membership: Some(&value.history_membership),
                segment_id: &value.segment_id,
                plan_id: &value.plan_id,
                plan_revision: value.plan_revision,
                model_configuration_id: None,
                profile_digest: None,
                stage_scope: Some(&value.scope),
                trigger_request_id: &value.trigger_request_id,
                target_from_turn_id: &value.target_from_turn_id,
                target_through_turn_id: &value.target_through_turn_id,
                target_from_ordinal: value.target_from_ordinal,
                target_through_ordinal: value.target_through_ordinal,
                assessed_at_ms: value.assessed_at_ms,
                score: value.score,
                partial: value.partial,
                reason: value.reason.as_deref(),
            }),
            _ => None,
        }
    }
}
