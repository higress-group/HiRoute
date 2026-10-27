//! Composite assessment has no fabricated model/profile identity.
use super::*;

impl RequestObservation {
    pub(super) fn record_composite_assessment(&self, assessment: &BoundAssessment) {
        let (Some(scope), Some(membership)) = (
            assessment.target.evaluation_scope.as_ref(),
            assessment.target.history_membership.as_ref(),
        ) else {
            return;
        };
        let (Ok(plan_id), Ok(trigger), Ok(first), Ok(last)) = (
            hiroute_domain::AgentPlanId::parse(&assessment.target.plan.plan_id),
            hiroute_domain::LogicalRequestId::parse(&self.inner.metadata.request_id),
            hiroute_domain::TurnId::parse(&assessment.target.first_turn_id),
            hiroute_domain::TurnId::parse(&assessment.target.through_turn_id),
        ) else {
            return;
        };
        let record = hiroute_domain::StageAssessmentRecordedV1 {
            scope: scope.clone(),
            history_membership: membership.clone(),
            segment_id: assessment.target.segment_id.clone(),
            plan_id,
            plan_revision: assessment.target.plan.plan_revision,
            trigger_request_id: trigger,
            target_from_turn_id: first,
            target_through_turn_id: last,
            target_from_ordinal: assessment.target.first_ordinal,
            target_through_ordinal: assessment.target.through_ordinal,
            assessed_at_ms: super::super::unix_nanos() / 1_000_000,
            score: assessment.score,
            partial: assessment.partial,
            reason: assessment.reason.clone(),
        };
        if record.validate().is_ok() {
            self.emit_execution(ExecutionFactV1::StageAssessmentRecorded(record), None);
        }
    }
}
