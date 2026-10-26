use std::collections::BTreeSet;

use crate::{AgentPlanDisplayName, AgentPlanId, CanonicalDigest, WorkspaceId};

use super::*;

impl ExecutionFactEnvelopeV1 {
    pub fn validate(&self) -> Result<(), ExecutionFactError> {
        self.validate_contract(false)
    }

    /// Authenticates immutable observation rows written under supported older
    /// contracts. This is a storage recovery seam, not an ingestion path.
    pub fn validate_persisted_contract(&self) -> Result<(), ExecutionFactError> {
        self.validate_contract(true)
    }

    fn validate_contract(&self, allow_persisted_v1: bool) -> Result<(), ExecutionFactError> {
        let current_wire = self.schema_version == EXECUTION_FACT_SCHEMA_V2
            && self.schema_digest.as_str() == EXECUTION_FACT_PORT_DIGEST_V2;
        let persisted_pre_stage = allow_persisted_v1
            && self.schema_version == EXECUTION_FACT_SCHEMA_V2
            && self.schema_digest.as_str()
                == "sha256:2babb083d6f0005ded11c5712f7b8183117b6ce06f87aa1892c6ad1aa891a481";
        let current = current_wire || persisted_pre_stage;
        let persisted_history = allow_persisted_v1
            && self.schema_version == EXECUTION_FACT_SCHEMA_V2
            && self.schema_digest.as_str()
                == "sha256:857e4f2b42278ac3877e34f2188eeecc7c1d188832316b23f577f3bc0b82b1ac";
        let persisted_extension = allow_persisted_v1
            && self.schema_version == EXECUTION_FACT_SCHEMA_V2
            && self.schema_digest.as_str()
                == "sha256:5f42929cdb812799e4e4a10885324debb21e6489bde5e935599de57b54b8ab23";
        let persisted_pre_admission = allow_persisted_v1
            && self.schema_version == EXECUTION_FACT_SCHEMA_V2
            && self.schema_digest.as_str()
                == "sha256:57592f8645e74ed2c13a968d8b782bde1819f5f1aa8729b2b5fefa9f5f8d20e6";
        let persisted_pre_auxiliary = allow_persisted_v1
            && self.schema_version == EXECUTION_FACT_SCHEMA_V2
            && self.schema_digest.as_str()
                == "sha256:3cb9b100385c1f05a92097c2f28eb5c47d61ca495d32d41a8528b7227c81a465";
        let persisted_pre_membership = allow_persisted_v1
            && self.schema_version == EXECUTION_FACT_SCHEMA_V2
            && self.schema_digest.as_str()
                == "sha256:806871b6b7a3c1a563e2e214e9af5f0a0b18d1f704e093494c2e1b2316b08069";
        let persisted_pre_opening = allow_persisted_v1
            && self.schema_version == EXECUTION_FACT_SCHEMA_V2
            && self.schema_digest.as_str()
                == "sha256:7ff01a649de96b1f62e069b872b23609d64a3734b5ad42c337324d3eb7cfc9f2";
        let persisted_pre_c = allow_persisted_v1
            && self.schema_version == EXECUTION_FACT_SCHEMA_V2
            && self.schema_digest.as_str()
                == "sha256:4f0dbcc9f7862133e73e451f64d7a6d6e5de146d958ac8bf3a4d1acc885b6b1d";
        let persisted_v1 = allow_persisted_v1
            && self.schema_version == "hiroute.observation.execution-fact-envelope/v1"
            && self.schema_digest.as_str()
                == "sha256:5aad4c450a2a296ec557b7a934bfbf70ad69a8e8fe9bc7539db46830a2940069";
        let persisted_pre_branch = allow_persisted_v1
            && self.schema_version == EXECUTION_FACT_SCHEMA_V2
            && self.schema_digest.as_str()
                == "sha256:de91d4f2333db66f0ec3f8b63ce192267dee0a56b65f34180b06996b232fb2c6";
        let persisted_function_only = allow_persisted_v1
            && self.schema_version == EXECUTION_FACT_SCHEMA_V2
            && self.schema_digest.as_str()
                == "sha256:80b4ff13af8abc79d7721fea42141300beade2acf1d2659983ceaafcdc6e4799";
        if !(current
            || persisted_history
            || persisted_extension
            || persisted_pre_admission
            || persisted_pre_auxiliary
            || persisted_pre_membership
            || persisted_pre_opening
            || persisted_v1
            || persisted_pre_branch
            || persisted_function_only
            || persisted_pre_c)
            || self.channel != ExecutionFactChannelV1::ExecutionFact
        {
            return Err(ExecutionFactError::UnsupportedSchema);
        }
        if !current_wire
            && matches!(
                self.fact,
                ExecutionFactV1::RequestInputSelection(_)
                    | ExecutionFactV1::StageAssessmentRecorded(_)
            )
        {
            return Err(ExecutionFactError::InvalidFact);
        }
        if self.sequence == 0
            || self.occurred_at_unix_nanos == 0
            || self.producer.revision.trim().is_empty()
            || WorkspaceId::parse(self.correlation.workspace_id.as_str()).is_err()
            || SessionId::parse(self.correlation.conversation_id.as_str()).is_err()
            || TurnId::parse(self.correlation.turn_id.as_str()).is_err()
            || LogicalRequestId::parse(self.correlation.request_id.as_str()).is_err()
            || EventId::parse(self.event_id.as_str()).is_err()
            || ProducerId::parse(self.producer.stream.producer_id.as_str()).is_err()
            || ProducerEpoch::parse(self.producer.stream.producer_epoch.as_str()).is_err()
            || StreamId::parse(self.producer.stream.stream_id.as_str()).is_err()
            || self
                .attempt_id
                .as_ref()
                .is_some_and(|value| AttemptId::parse(value.as_str()).is_err())
        {
            return Err(ExecutionFactError::InvalidEnvelope);
        }
        if !(current || persisted_history)
            && matches!(&self.fact, ExecutionFactV1::AgentTurnAdmitted(_))
        {
            return Err(ExecutionFactError::InvalidFact);
        }
        if !(current
            || persisted_history
            || persisted_extension
            || persisted_pre_admission
            || persisted_pre_auxiliary
            || persisted_pre_membership)
            && matches!(
                &self.fact,
                ExecutionFactV1::AgentTurnFinished {
                    opening_cause: Some(_),
                    ..
                }
            )
        {
            return Err(ExecutionFactError::InvalidFact);
        }
        if !(current || persisted_history || persisted_pre_admission || persisted_pre_auxiliary)
            && matches!(
                &self.fact,
                ExecutionFactV1::AgentTurnFinished {
                    history_membership: Some(_),
                    ..
                } | ExecutionFactV1::BranchAssessmentRecorded {
                    history_membership: Some(_),
                    ..
                }
            )
        {
            return Err(ExecutionFactError::InvalidFact);
        }
        if !(current || persisted_history || persisted_pre_admission)
            && matches!(&self.fact, ExecutionFactV1::RouteDecision(route) if route.native_auxiliary)
        {
            return Err(ExecutionFactError::InvalidFact);
        }
        if !(current || persisted_extension) {
            let extension_data = match &self.fact {
                ExecutionFactV1::RouteDecision(route) => {
                    route.complexity.as_ref().is_some_and(|decision| {
                        decision.extension_reason.is_some()
                            || matches!(
                                decision.selection_reason,
                                crate::ModelGroupReasonV1::ExtensionDecision
                                    | crate::ModelGroupReasonV1::DecisionUnavailable
                            )
                            || decision.policy.as_ref().is_some_and(|policy| {
                                policy.extension.is_some() || policy.floor_millis.is_none()
                            })
                    })
                }
                ExecutionFactV1::AgentTurnFinished {
                    branch_execution, ..
                } => branch_execution.as_ref().is_some_and(|execution| {
                    execution.policy.extension.is_some() || execution.policy.floor_millis.is_none()
                }),
                _ => false,
            };
            if extension_data {
                return Err(ExecutionFactError::InvalidFact);
            }
        }
        self.trust.validate()?;
        self.fact.validate()?;
        if (current
            || persisted_history
            || persisted_extension
            || persisted_pre_admission
            || persisted_pre_auxiliary
            || persisted_pre_membership
            || persisted_pre_opening)
            && matches!(&self.fact, ExecutionFactV1::RouteDecision(route)
            if route.complexity.as_ref().is_some_and(|d| d.decision_protocol.is_none()
                || (d.decision_protocol != Some(DecisionProtocolV1::SystemOneC) && d.routing_observation.is_some())))
        {
            return Err(ExecutionFactError::InvalidFact);
        }
        if !(current
            || persisted_history
            || persisted_extension
            || persisted_pre_admission
            || persisted_pre_auxiliary
            || persisted_pre_membership
            || persisted_pre_opening)
            && matches!(&self.fact, ExecutionFactV1::RouteDecision(route)
                if route.complexity.as_ref().is_some_and(|decision| decision.routing_observation.is_some()))
        {
            return Err(ExecutionFactError::InvalidFact);
        }
        if !(current
            || persisted_history
            || persisted_extension
            || persisted_pre_admission
            || persisted_pre_auxiliary
            || persisted_pre_membership
            || persisted_pre_opening
            || persisted_pre_c)
            && matches!(
                &self.fact,
                ExecutionFactV1::RouteDecision(route)
                    if matches!(route.requirements.tool_choice,
                        ToolChoiceRequirementV1::RequiredNamed {
                            tool_kind: ToolKindRequirementV1::Custom, ..
                        })
            )
        {
            return Err(ExecutionFactError::InvalidFact);
        }
        if let Some(assessment) = self.fact.assessment_record()
            && assessment.trigger_request_id != &self.correlation.request_id
        {
            return Err(ExecutionFactError::InvalidFact);
        }
        if let ExecutionFactV1::StageAssessmentRecorded(assessment) = &self.fact
            && (self.trust.agent_plan_id.as_ref() != Some(&assessment.plan_id)
                || !matches!(&self.trust.route, crate::ModelRequestRouteV2::Plan { revision, .. }
                    if *revision == assessment.plan_revision))
        {
            return Err(ExecutionFactError::InvalidTrustIdentity);
        }
        if let Some(pricing) = &self.pricing {
            if !(current
                || persisted_history
                || persisted_extension
                || persisted_pre_admission
                || persisted_pre_auxiliary
                || persisted_pre_membership
                || persisted_pre_opening
                || persisted_function_only
                || persisted_pre_c)
                || !matches!(self.fact, ExecutionFactV1::AttemptStarted { .. })
            {
                return Err(ExecutionFactError::InvalidFact);
            }
            pricing.validate()?;
            for quote in [&pricing.quote, &pricing.reference_quote]
                .into_iter()
                .flatten()
            {
                if quote.exact_target.workspace_id != self.correlation.workspace_id {
                    return Err(ExecutionFactError::InvalidFact);
                }
            }
        }
        let attempt_scoped = matches!(
            self.fact,
            ExecutionFactV1::AttemptStarted { .. }
                | ExecutionFactV1::AttemptFinished(_)
                | ExecutionFactV1::SemanticCommit { .. }
                | ExecutionFactV1::UsageAndCache { .. }
        );
        if attempt_scoped != self.attempt_id.is_some() {
            return Err(ExecutionFactError::InvalidAttemptScope);
        }
        if let Some(loss) = &self.loss_watermark {
            if loss.first_sequence == 0
                || loss.first_sequence > loss.last_sequence
                || loss.last_sequence >= self.sequence
                || self.completeness_delta != Some(CompletenessDeltaV1::Partial)
            {
                return Err(ExecutionFactError::InvalidLossWatermark);
            }
        } else if self.completeness_delta == Some(CompletenessDeltaV1::Partial) {
            return Err(ExecutionFactError::InvalidLossWatermark);
        }
        Ok(())
    }

    pub fn occurred_at_ms(&self) -> Result<i64, ExecutionFactError> {
        i64::try_from(self.occurred_at_unix_nanos / 1_000_000)
            .map_err(|_| ExecutionFactError::InvalidEnvelope)
    }

    pub fn facts_completeness(&self) -> FactsCompleteness {
        match self.completeness_delta {
            Some(CompletenessDeltaV1::Partial) => FactsCompleteness::Partial,
            Some(CompletenessDeltaV1::Unknown) => FactsCompleteness::Unknown,
            None => FactsCompleteness::Complete,
        }
    }
}

impl FrozenExecutionTrustV1 {
    pub fn validate(&self) -> Result<(), ExecutionFactError> {
        if self.authority_id.trim().is_empty()
            || self.authority_epoch == 0
            || self.served_model_id.trim().is_empty()
            || self.route.validate().is_err()
            || match &self.route {
                crate::ModelRequestRouteV2::Plan { .. } => self
                    .agent_plan_id
                    .as_ref()
                    .is_none_or(|id| AgentPlanId::parse(id.as_str()).is_err()),
                crate::ModelRequestRouteV2::Fixed { .. } => {
                    self.agent_plan_id.is_some() || self.plan_display_name.is_some()
                }
            }
            || self
                .plan_display_name
                .as_ref()
                .is_some_and(|name| AgentPlanDisplayName::parse(name.clone()).is_err())
            || !valid_revision(&self.gateway_publication_revision)
            || CanonicalDigest::parse(self.gateway_publication_digest.as_str()).is_err()
            || self.grant_id.trim().is_empty()
            || self.grant_generation == 0
            || self.ingress_protocol == IngressProtocolV1::Unknown
        {
            Err(ExecutionFactError::InvalidTrustIdentity)
        } else {
            Ok(())
        }
    }
}

impl ExecutionFactV1 {
    pub fn validate(&self) -> Result<(), ExecutionFactError> {
        match self {
            Self::AgentTurnAdmitted(admission) => admission.validate(),
            Self::RequestInputSelection(selection) => selection.validate(),
            Self::StageAssessmentRecorded(assessment) => assessment.validate(),
            Self::RouteDecision(route) => route.validate(),
            Self::CandidateDecision(candidate) => candidate.validate(),
            Self::CredentialLease {
                stable_binding_id,
                credential_ref,
                key_id,
                credential_generation,
                ..
            } => {
                nonempty([stable_binding_id, credential_ref])?;
                if key_id.is_some() != credential_generation.is_some() {
                    return Err(ExecutionFactError::InvalidFact);
                }
                Ok(())
            }
            Self::RuntimeState {
                stable_binding_id, ..
            } => nonempty([stable_binding_id]),
            Self::AttemptStarted {
                ordinal,
                candidate_id,
                stable_binding_id,
                profile_digest,
                credential_ref,
                key_id,
                provider_name,
                request_model,
                model_configuration_id,
                adapter_revision,
                start_reason,
                ..
            } => {
                if *ordinal == 0 || profile_digest.validate().is_err() {
                    return Err(ExecutionFactError::InvalidFact);
                }
                nonempty([
                    candidate_id,
                    stable_binding_id,
                    credential_ref,
                    key_id,
                    provider_name,
                    request_model,
                    model_configuration_id,
                    adapter_revision,
                    start_reason,
                ])
            }
            Self::AttemptFinished(fact) => fact.validate(),
            Self::SemanticCommit {
                ordinal, frame_id, ..
            } => {
                if *ordinal == 0 {
                    return Err(ExecutionFactError::InvalidFact);
                }
                nonempty([frame_id])
            }
            Self::UsageAndCache {
                ordinal,
                input_provenance,
                output_provenance,
                billable_provenance,
                cache_read_provenance,
                cache_write_provenance,
                reasoning_provenance,
                input_tokens,
                output_tokens,
                billable_tokens,
                cache_read_tokens,
                cache_write_tokens,
                reasoning_tokens,
                ..
            } => {
                if *ordinal == 0
                    || !usage_dimension_valid(*input_tokens, *input_provenance)
                    || !usage_dimension_valid(*output_tokens, *output_provenance)
                    || !usage_dimension_valid(*billable_tokens, *billable_provenance)
                    || !usage_dimension_valid(*cache_read_tokens, *cache_read_provenance)
                    || !usage_dimension_valid(*cache_write_tokens, *cache_write_provenance)
                    || !usage_dimension_valid(*reasoning_tokens, *reasoning_provenance)
                {
                    return Err(ExecutionFactError::InvalidFact);
                }
                Ok(())
            }
            Self::RequestFinished {
                attempts_started,
                attempts_finished,
                accepted_attempt_ordinal,
                ..
            } => {
                if attempts_finished > attempts_started
                    || accepted_attempt_ordinal
                        .is_some_and(|value| value == 0 || value > *attempts_started)
                {
                    Err(ExecutionFactError::InvalidFact)
                } else {
                    Ok(())
                }
            }
            Self::AgentTurnFinished {
                history_membership,
                branch_execution,
                agent_turn_id,
                segment_id,
                ordinal,
                plan_revision,
                selected_branch_id,
                executed_branch_id,
                model_configuration_id,
                profile_digest,
                attribution,
                started_at_ms,
                finished_at_ms,
                first_request_id,
                last_request_id,
                ..
            } => {
                if let Some(membership) = history_membership {
                    membership.validate()?;
                }
                nonempty([agent_turn_id, segment_id, selected_branch_id])?;
                if *ordinal == 0
                    || branch_execution.as_ref().is_some_and(|execution| {
                        !execution.policy.validate()
                            || execution.candidate_index >= 128
                            || *attribution != AgentTurnAttributionV1::Single
                    })
                    || *plan_revision == 0
                    || *started_at_ms == 0
                    || finished_at_ms < started_at_ms
                    || first_request_id.is_some() != last_request_id.is_some()
                    || profile_digest
                        .as_ref()
                        .is_some_and(|digest| digest.validate().is_err())
                {
                    return Err(ExecutionFactError::InvalidFact);
                }
                match attribution {
                    AgentTurnAttributionV1::Single => {
                        let Some(executed_branch_id) = executed_branch_id else {
                            return Err(ExecutionFactError::InvalidFact);
                        };
                        let Some(model_configuration_id) = model_configuration_id else {
                            return Err(ExecutionFactError::InvalidFact);
                        };
                        if profile_digest.is_none() {
                            return Err(ExecutionFactError::InvalidFact);
                        }
                        nonempty([executed_branch_id, model_configuration_id])
                    }
                    AgentTurnAttributionV1::Mixed | AgentTurnAttributionV1::Unknown => {
                        if executed_branch_id.is_some()
                            || model_configuration_id.is_some()
                            || profile_digest.is_some()
                        {
                            Err(ExecutionFactError::InvalidFact)
                        } else {
                            Ok(())
                        }
                    }
                }
            }
            Self::BranchAssessmentRecorded {
                history_membership,
                segment_id,
                model_configuration_id,
                profile_digest,
                plan_revision,
                target_from_ordinal,
                target_through_ordinal,
                assessed_at_ms,
                score,
                reason,
                ..
            } => {
                if let Some(membership) = history_membership {
                    membership.validate()?;
                }
                nonempty([segment_id, model_configuration_id])?;
                if *plan_revision == 0
                    || *target_from_ordinal == 0
                    || target_from_ordinal > target_through_ordinal
                    || *assessed_at_ms == 0
                    || !score.is_finite()
                    || !(0.0..=1.0).contains(score)
                    || profile_digest.validate().is_err()
                    || reason.as_ref().is_some_and(|value| {
                        value.trim().is_empty() || value.chars().count() > 1_024
                    })
                {
                    Err(ExecutionFactError::InvalidFact)
                } else {
                    Ok(())
                }
            }
            Self::ValueSnapshot {
                traffic_kind: _,
                usage: _,
                value,
            } => value
                .validate()
                .map_err(|_| ExecutionFactError::InvalidFact),
        }
    }
}

impl RouteDecisionFactV1 {
    fn validate(&self) -> Result<(), ExecutionFactError> {
        nonempty([&self.planner_version])?;
        if self.route.validate().is_err()
            || self.max_attempts == 0
            || match &self.route {
                crate::ModelRequestRouteV2::Plan { .. } => self
                    .plan_id
                    .as_ref()
                    .is_none_or(|id| AgentPlanId::parse(id.as_str()).is_err()),
                crate::ModelRequestRouteV2::Fixed { .. } => self.plan_id.is_some(),
            }
            || CanonicalDigest::parse(self.input_digest.as_str()).is_err()
            || CanonicalDigest::parse(self.policy_digest.as_str()).is_err()
            || CanonicalDigest::parse(self.output_digest.as_str()).is_err()
            || self
                .complexity
                .as_ref()
                .is_some_and(|value| value.validate().is_err())
        {
            return Err(ExecutionFactError::InvalidFact);
        }
        if matches!(self.outcome, RouteDecisionOutcomeV1::Ready) != self.outcome_code.is_none() {
            return Err(ExecutionFactError::InvalidFact);
        }
        if let Some(value) = &self.requested_reasoning_value {
            value.validate()?;
        }
        unique_ordinals(self.groups.iter().map(|group| group.ordinal))?;
        unique_ordinals(self.reason_ledger.iter().map(|reason| reason.ordinal))
    }
}

impl CandidateDecisionFactV1 {
    fn validate(&self) -> Result<(), ExecutionFactError> {
        nonempty([
            &self.candidate_id,
            &self.stable_binding_id,
            &self.group_id,
            &self.path_id,
            &self.provider_id,
            &self.endpoint_id,
            &self.entitlement_id,
            &self.connector_id,
            &self.connector_revision,
            &self.capability_id,
            &self.capability_revision,
            &self.model_configuration_id,
            &self.native_model,
            &self.adapter_revision,
            &self.serializer_revision,
            &self.decoder_revision,
        ])?;
        if self.eligible == self.exclusion_reason.is_some()
            || self.profile_digest.validate().is_err()
        {
            return Err(ExecutionFactError::InvalidFact);
        }
        Ok(())
    }
}

impl AttemptFinishedFactV1 {
    fn validate(&self) -> Result<(), ExecutionFactError> {
        if self.ordinal == 0 {
            return Err(ExecutionFactError::InvalidFact);
        }
        nonempty([&self.stable_binding_id])
    }
}

fn nonempty<'a>(values: impl IntoIterator<Item = &'a String>) -> Result<(), ExecutionFactError> {
    if values.into_iter().any(|value| value.trim().is_empty()) {
        Err(ExecutionFactError::InvalidFact)
    } else {
        Ok(())
    }
}

fn unique_ordinals(values: impl IntoIterator<Item = u32>) -> Result<(), ExecutionFactError> {
    let mut seen = BTreeSet::new();
    if values.into_iter().any(|value| !seen.insert(value)) {
        Err(ExecutionFactError::InvalidFact)
    } else {
        Ok(())
    }
}

fn usage_dimension_valid(value: Option<u64>, provenance: UsageProvenanceV1) -> bool {
    value.is_some() != matches!(provenance, UsageProvenanceV1::Unknown)
}

fn valid_revision(value: &str) -> bool {
    value
        .parse::<u64>()
        .is_ok_and(|revision| revision > 0 && revision.to_string() == value)
}
