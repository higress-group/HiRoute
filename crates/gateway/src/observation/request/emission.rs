use super::super::contracts::{EXECUTION_FACT_PORT_DIGEST, LIFECYCLE_FACT_PORT_DIGEST};
use super::super::crypto::stable_id;
use super::super::schema::{
    CorrelationV1, EXECUTION_FACT_SCHEMA, ExecutionFactEnvelopeV1, ExecutionFactV1,
    LIFECYCLE_FACT_SCHEMA, LifecycleFactEnvelopeV1, LifecycleFactV1,
};
use super::super::unix_nanos;
use super::RequestObservation;

impl RequestObservation {
    pub(in super::super) fn emit_execution(&self, fact: ExecutionFactV1, attempt_id: Option<&str>) {
        if !self.inner.enabled {
            return;
        }
        let metadata = self.inner.metadata.clone();
        let agent_plan_id = match &metadata.route {
            hiroute_domain::ModelRequestRouteV2::Plan { .. } => metadata.agent_plan_id.clone(),
            hiroute_domain::ModelRequestRouteV2::Fixed { .. } => None,
        };
        let correlation = self.correlation();
        let key = self.inner.key;
        let fact_kind = execution_kind(&fact);
        let attempt_id = attempt_id.map(ToOwned::to_owned);
        let pricing = if let ExecutionFactV1::AttemptStarted {
            stable_binding_id,
            model_configuration_id,
            profile_digest,
            ..
        } = &fact
        {
            let at_ms = (unix_nanos() / 1_000_000) as i64;
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.inner.prices.freeze_attempt(
                    stable_binding_id,
                    model_configuration_id,
                    profile_digest,
                    at_ms,
                )
            }))
            .ok()
            .filter(|value| value.validate().is_ok())
        } else {
            None
        };
        self.inner
            .channels
            .execution
            .publish(move |producer, sequence, loss| {
                let sequence_bytes = sequence.to_be_bytes();
                ExecutionFactEnvelopeV1 {
                    schema_version: EXECUTION_FACT_SCHEMA.into(),
                    schema_digest: EXECUTION_FACT_PORT_DIGEST.into(),
                    pricing,
                    channel: "execution_fact".into(),
                    producer,
                    sequence,
                    event_id: stable_id(
                        "fact-event",
                        &key,
                        b"execution-event",
                        &[
                            metadata.request_id.as_bytes(),
                            &sequence_bytes,
                            fact_kind.as_bytes(),
                        ],
                    ),
                    correlation,
                    attempt_id,
                    authority_id: metadata.authority_id,
                    authority_epoch: metadata.authority_epoch,
                    served_model_id: metadata.served_model_id,
                    selector_source: "trusted_model_alias".into(),
                    agent_plan_id,
                    route: metadata.route,
                    plan_display_name: metadata.plan_display_name,
                    gateway_publication_revision: metadata.publication_revision.to_string(),
                    gateway_publication_digest: metadata.publication_digest,
                    grant_id: metadata.grant_id,
                    grant_generation: metadata.grant_generation,
                    ingress_protocol: metadata.ingress_protocol,
                    occurred_at_unix_nanos: unix_nanos(),
                    fact,
                    completeness_delta: loss.as_ref().map(|_| "partial".into()),
                    loss_watermark: loss,
                }
            });
    }

    pub(in super::super) fn emit_lifecycle(&self, fact: LifecycleFactV1) {
        if !self.inner.enabled {
            return;
        }
        let metadata = self.inner.metadata.clone();
        let correlation = self.correlation();
        let key = self.inner.key;
        let fact_kind = lifecycle_kind(&fact);
        self.inner
            .channels
            .lifecycle
            .publish(move |producer, sequence, loss| {
                let sequence_bytes = sequence.to_be_bytes();
                LifecycleFactEnvelopeV1 {
                    schema_version: LIFECYCLE_FACT_SCHEMA.into(),
                    schema_digest: LIFECYCLE_FACT_PORT_DIGEST.into(),
                    channel: "lifecycle".into(),
                    producer,
                    sequence,
                    event_id: stable_id(
                        "lifecycle-event",
                        &key,
                        b"lifecycle-event",
                        &[
                            metadata.request_id.as_bytes(),
                            &sequence_bytes,
                            fact_kind.as_bytes(),
                        ],
                    ),
                    correlation,
                    occurred_at_unix_nanos: unix_nanos(),
                    fact,
                    completeness_delta: loss.as_ref().map(|_| "partial".into()),
                    loss_watermark: loss,
                }
            });
    }

    pub(in super::super) fn correlation(&self) -> CorrelationV1 {
        let metadata = &self.inner.metadata;
        CorrelationV1 {
            workspace_id: metadata.workspace_id.clone(),
            conversation_id: metadata.conversation_id.clone(),
            session_scope: metadata.session_scope.clone(),
            correlation_provenance: metadata.correlation_provenance.clone(),
            turn_id: metadata.turn_id.clone(),
            request_id: metadata.request_id.clone(),
        }
    }
}

fn execution_kind(fact: &ExecutionFactV1) -> &'static str {
    match fact {
        ExecutionFactV1::RequestInputSelection(_) => "request_input_selection",
        ExecutionFactV1::StageAssessmentRecorded(_) => "stage_assessment_recorded",
        ExecutionFactV1::AgentTurnAdmitted(..) => "agent_turn_admitted",
        ExecutionFactV1::RouteDecision { .. } => "route_decision",
        ExecutionFactV1::CandidateDecision(..) => "candidate_decision",
        ExecutionFactV1::CredentialLease { .. } => "credential_lease",
        ExecutionFactV1::RuntimeState { .. } => "runtime_state",
        ExecutionFactV1::AttemptStarted { .. } => "attempt_started",
        ExecutionFactV1::AttemptFinished(..) => "attempt_finished",
        ExecutionFactV1::SemanticCommit { .. } => "semantic_commit",
        ExecutionFactV1::UsageAndCache { .. } => "usage_and_cache",
        ExecutionFactV1::RequestFinished { .. } => "request_finished",
        ExecutionFactV1::AgentTurnFinished { .. } => "agent_turn_finished",
        ExecutionFactV1::BranchAssessmentRecorded { .. } => "branch_assessment_recorded",
    }
}

fn lifecycle_kind(fact: &LifecycleFactV1) -> &'static str {
    match fact {
        LifecycleFactV1::RequestAccepted { .. } => "request_accepted",
        LifecycleFactV1::CanonicalRequestAccepted { .. } => "canonical_request_accepted",
        LifecycleFactV1::AttemptStarted { .. } => "attempt_started",
        LifecycleFactV1::AttemptFinished { .. } => "attempt_finished",
        LifecycleFactV1::ResponseFrameAccepted { .. } => "response_frame_accepted",
        LifecycleFactV1::RequestFinished { .. } => "request_finished",
    }
}
