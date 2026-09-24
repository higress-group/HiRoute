//! Subscription flow decisions over the shared compute-management contracts.
//!
//! Transport registration and durable writes stay in the centralized control-plane assembly.

use std::collections::BTreeSet;

use hiroute_application_api::{
    COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2, COMPUTE_SUBSCRIPTION_CHECK_SCHEMA_V2, CanonicalDigest,
    ComputeCandidateFactStateV2, ComputeCandidateProducerV2, ComputeCandidateRefV2,
    ComputeCandidateViewV2, ComputeConnectionApplyRequestV1, ComputeManagementChangeV2,
    ComputeManagementIntentV2, ComputeManagementSubjectV2, ComputeSavedSourceExpectationV2,
    ComputeSubscriptionCheckChangeV2, ComputeSubscriptionCheckPreviewV2,
    ComputeSubscriptionCheckResultV2, ComputeSubscriptionCheckStatusV2, ComputeValidationRefV2,
    OperationCancelRequestV1, OperationReferenceV1, RevisionSetV1,
};
use hiroute_domain::{
    CHANGE_SPEC_SCHEMA_V1, ChangeSpecV1, ComputeManagementRepositoryPort, ControlRepositoryPort,
    GatewayAuthenticationSemanticsV1, SubscriptionCheckIntentV2, TransactionPlanV1, WorkspaceId,
};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum SubscriptionPreparationError {
    #[error("candidate needs protected credential input")]
    NeedsCredential,
    #[error("CPA candidate needs an approved subscription check")]
    NeedsApproval,
    #[error("selected model references are invalid")]
    InvalidModelSelection,
    #[error("subscription request is invalid")]
    InvalidRequest,
}

pub struct ComputeSubscriptionPreparedPreviewV2 {
    pub result: ComputeSubscriptionCheckPreviewV2,
    pub plan: TransactionPlanV1,
}

pub struct ComputeSubscriptionPlanner<'a, C, R> {
    candidates: &'a C,
    control: &'a R,
}

impl<'a, C, R> ComputeSubscriptionPlanner<'a, C, R>
where
    C: crate::compute_management::ComputeCandidatePort,
    R: ControlRepositoryPort + ComputeManagementRepositoryPort,
{
    pub fn new(candidates: &'a C, control: &'a R) -> Self {
        Self {
            candidates,
            control,
        }
    }

    pub fn preview(
        &self,
        candidate: ComputeCandidateRefV2,
    ) -> Result<ComputeSubscriptionPreparedPreviewV2, SubscriptionPreparationError> {
        let facts = self
            .candidates
            .resolve_compute_candidate(&candidate)
            .map_err(|_| SubscriptionPreparationError::InvalidRequest)?;
        let protected_source = match &facts.credential_binding {
            crate::compute_management::ComputeCredentialBindingV2::CpaPendingApproval {
                protected_source,
            } => protected_source,
            _ => return Err(SubscriptionPreparationError::InvalidRequest),
        };
        let public = facts
            .public_view(facts.inferred_input_state(), Vec::new())
            .map_err(|_| SubscriptionPreparationError::InvalidRequest)?;
        let snapshot = self
            .control
            .compute_management_snapshot(&WorkspaceId::default())
            .map_err(|_| SubscriptionPreparationError::InvalidRequest)?;
        let existing_source = facts
            .existing_source_id
            .as_ref()
            .map(|source_id| {
                snapshot
                    .sources
                    .iter()
                    .find(|source| &source.source_id == source_id)
                    .map(|source| ComputeSavedSourceExpectationV2 {
                        source_id: source_id.clone(),
                        expected_revision: source.revision,
                    })
                    .ok_or(SubscriptionPreparationError::InvalidRequest)
            })
            .transpose()?;
        let change = prepare_subscription_check(
            &public,
            facts.evidence_digest.clone(),
            existing_source.clone(),
        )?;
        let intent = SubscriptionCheckIntentV2::from_application(
            candidate.candidate_ref.clone(),
            candidate.candidate_revision,
            facts.evidence_digest.clone(),
            existing_source.map(|source| (source.source_id, source.expected_revision)),
        )
        .map_err(|_| SubscriptionPreparationError::InvalidRequest)?;
        // Merely prove that the descriptor exists in trusted facts; it is intentionally absent
        // from both the ChangeSpec and the transaction plan.
        let _ = protected_source;
        let revisions = snapshot.revisions;
        let spec = ChangeSpecV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            command_id: hiroute_domain::COMPUTE_SUBSCRIPTION_CHECK_COMMAND_ID_V2.into(),
            resource_id: Some(candidate.candidate_ref.clone()),
            desired_state: serde_json::to_value(&change)
                .map_err(|_| SubscriptionPreparationError::InvalidRequest)?,
        };
        let plan = TransactionPlanV1::from_compute_subscription_check_planner(spec.clone(), intent)
            .map_err(|_| SubscriptionPreparationError::InvalidRequest)?;
        let accept_digest = CanonicalDigest::of(&(
            "hiroute.compute-subscription-preview/v2",
            &spec,
            &revisions,
            plan.control(),
            plan.external(),
        ))
        .map_err(|_| SubscriptionPreparationError::InvalidRequest)?;
        Ok(ComputeSubscriptionPreparedPreviewV2 {
            result: ComputeSubscriptionCheckPreviewV2 {
                candidate,
                display_scope: public.display_name,
                spec,
                accept_digest,
                expected_revisions: revisions,
            },
            plan,
        })
    }

    pub fn prepare_apply(
        &self,
        request: ComputeConnectionApplyRequestV1,
        apply_capability: Option<String>,
    ) -> Result<crate::PreparedTransactionV1, SubscriptionPreparationError> {
        if request.idempotency_key.trim().is_empty()
            || apply_capability
                .as_ref()
                .is_some_and(|value| value.is_empty())
        {
            return Err(SubscriptionPreparationError::InvalidRequest);
        }
        let change: ComputeSubscriptionCheckChangeV2 =
            serde_json::from_value(request.spec.desired_state.clone())
                .map_err(|_| SubscriptionPreparationError::InvalidRequest)?;
        let reproduced = self.preview(change.candidate)?;
        if reproduced.result.spec != request.spec
            || reproduced.result.accept_digest != request.accept_digest
            || reproduced.result.expected_revisions != request.expected_revisions
        {
            return Err(SubscriptionPreparationError::InvalidRequest);
        }
        crate::PreparedTransactionV1::for_subscription_check(
            hiroute_application_api::ApplyRequestV1 {
                schema_version: CHANGE_SPEC_SCHEMA_V1,
                spec: request.spec,
                accept_digest: request.accept_digest.clone(),
                expected_revisions: request.expected_revisions,
                idempotency_key: request.idempotency_key,
                apply_capability,
            },
            request.accept_digest,
            reproduced.plan,
        )
        .map_err(|_| SubscriptionPreparationError::InvalidRequest)
    }
}

pub fn verified_subscription_candidate(
    pending: &crate::compute_management::ComputeCandidateFactsV2,
    validation: crate::compute_management::ComputeSubscriptionValidationFactsV2,
    target: hiroute_application_api::ComputeCandidateTargetV2,
    authentication: GatewayAuthenticationSemanticsV1,
) -> Result<crate::compute_management::ComputeCandidateFactsV2, SubscriptionPreparationError> {
    use crate::compute_management::{ComputeCandidateProvenanceV2, ComputeCredentialBindingV2};
    if validation.original_candidate != pending.candidate
        || validation.verified_evidence_digest == CanonicalDigest::of_bytes(&[])
    {
        return Err(SubscriptionPreparationError::InvalidRequest);
    }
    let connector_id = match &pending.provenance {
        ComputeCandidateProvenanceV2::ConnectorOwnedPendingApproval { connector_id } => {
            connector_id.clone()
        }
        _ => return Err(SubscriptionPreparationError::InvalidRequest),
    };
    let candidate_revision = pending
        .candidate
        .candidate_revision
        .checked_add(1)
        .ok_or(SubscriptionPreparationError::InvalidRequest)?;
    let edit_revision = pending
        .correlation
        .edit_revision
        .checked_add(1)
        .ok_or(SubscriptionPreparationError::InvalidRequest)?;
    let facts = crate::compute_management::ComputeCandidateFactsV2 {
        candidate: ComputeCandidateRefV2 {
            candidate_ref: pending.candidate.candidate_ref.clone(),
            candidate_revision,
        },
        correlation: hiroute_application_api::ComputeCheckCorrelationV2 {
            candidate_ref: pending.candidate.candidate_ref.clone(),
            edit_revision,
            check_id: format!("check/subscription/{edit_revision}"),
            input_digest: CanonicalDigest::of(&(
                "hiroute.compute-subscription-checked-input/v2",
                &validation.validation,
                &validation.verified_evidence_digest,
            ))
            .map_err(|_| SubscriptionPreparationError::InvalidRequest)?,
        },
        producer: ComputeCandidateProducerV2::Cpa,
        lineage_ref: pending.lineage_ref.clone(),
        trusted_lineage_digest: pending.trusted_lineage_digest.clone(),
        display_name: pending.display_name.clone(),
        existing_source_id: pending.existing_source_id.clone(),
        evidence_digest: validation.verified_evidence_digest.clone(),
        provenance: ComputeCandidateProvenanceV2::ConnectorOwned {
            connector_id,
            account_ref: validation.account_ref.clone(),
        },
        target: Some(target),
        authentication: Some(authentication),
        models: validation.inventory,
        native_recheck: None,
        additional_native_endpoints: Vec::new(),
        discovery_guard: None,
        credential_binding: ComputeCredentialBindingV2::CpaOwned {
            account_ref: validation.account_ref,
            validation: validation.validation.clone(),
        },
        validation: Some(validation.validation),
    };
    facts
        .validate_shape()
        .map_err(|_| SubscriptionPreparationError::InvalidRequest)?;
    Ok(facts)
}

/// Prepares one source-level save. Fact completeness controls which transition is legal; it is not
/// a claim that the resulting source is published or runnable.
pub fn prepare_compute_management_change(
    candidate: &ComputeCandidateViewV2,
    selected_model_refs: Vec<String>,
    enable: bool,
    expected_revisions: RevisionSetV1,
) -> Result<ComputeManagementChangeV2, SubscriptionPreparationError> {
    let intent = ComputeManagementIntentV2::from_enable(enable);
    match (candidate.fact_state, intent) {
        (ComputeCandidateFactStateV2::PendingCredential, ComputeManagementIntentV2::SaveReady) => {
            return Err(SubscriptionPreparationError::NeedsCredential);
        }
        (ComputeCandidateFactStateV2::PendingApproval, _) => {
            return Err(SubscriptionPreparationError::NeedsApproval);
        }
        _ => {}
    }

    validate_model_selection(candidate, &selected_model_refs, enable)?;
    let change = ComputeManagementChangeV2 {
        schema: COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.to_owned(),
        subject: ComputeManagementSubjectV2::Candidate {
            candidate: candidate.candidate.clone(),
        },
        expected_revisions,
        selected_model_refs,
        intent,
        key_edits: Vec::new(),
        validation: candidate.validation.clone(),
    };
    change
        .validate_shape()
        .map_err(|_| SubscriptionPreparationError::InvalidRequest)?;
    Ok(change)
}

fn validate_model_selection(
    candidate: &ComputeCandidateViewV2,
    selected: &[String],
    enable: bool,
) -> Result<(), SubscriptionPreparationError> {
    if enable && selected.is_empty() {
        return Err(SubscriptionPreparationError::InvalidModelSelection);
    }
    let unique = selected.iter().collect::<BTreeSet<_>>();
    if unique.len() != selected.len()
        || selected.iter().any(|model_ref| {
            candidate
                .models
                .iter()
                .find(|model| model.model_ref == *model_ref)
                .is_none_or(|model| !model.selectable)
        })
    {
        return Err(SubscriptionPreparationError::InvalidModelSelection);
    }
    Ok(())
}

pub fn prepare_subscription_check(
    candidate: &ComputeCandidateViewV2,
    expected_evidence_digest: CanonicalDigest,
    existing_source: Option<ComputeSavedSourceExpectationV2>,
) -> Result<ComputeSubscriptionCheckChangeV2, SubscriptionPreparationError> {
    if candidate.producer != ComputeCandidateProducerV2::Cpa
        || candidate.fact_state != ComputeCandidateFactStateV2::PendingApproval
    {
        return Err(SubscriptionPreparationError::InvalidRequest);
    }
    let change = ComputeSubscriptionCheckChangeV2 {
        schema: COMPUTE_SUBSCRIPTION_CHECK_SCHEMA_V2.to_owned(),
        candidate: candidate.candidate.clone(),
        expected_evidence_digest,
        existing_source,
    };
    change
        .validate_shape()
        .map_err(|_| SubscriptionPreparationError::InvalidRequest)?;
    Ok(change)
}

/// A client edit counter only filters late responses. Server candidate revisions remain opaque.
pub fn candidate_result_is_current(
    candidate: &ComputeCandidateViewV2,
    current_edit_revision: u64,
    current_check_id: &str,
) -> bool {
    candidate.correlation.edit_revision == current_edit_revision
        && candidate.correlation.check_id == current_check_id
        && candidate.correlation.candidate_ref == candidate.candidate.candidate_ref
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubscriptionCloseAction {
    NoWrite,
    CancelApproval(OperationCancelRequestV1),
    ReleaseValidation(ComputeValidationRefV2),
    ObserveSave(OperationReferenceV1),
}

/// Chooses a safe close action. Once save Operation B exists, closing can only observe B; it must
/// never release Operation A's resource while B may have accepted or completed the handoff.
pub fn subscription_close_action(
    check: Option<&ComputeSubscriptionCheckResultV2>,
    save_operation: Option<&OperationReferenceV1>,
    cancel_idempotency_key: &str,
) -> Result<SubscriptionCloseAction, SubscriptionPreparationError> {
    if let Some(save) = save_operation {
        return Ok(SubscriptionCloseAction::ObserveSave(save.clone()));
    }
    let Some(check) = check else {
        return Ok(SubscriptionCloseAction::NoWrite);
    };
    if let Some(save) = &check.save_operation {
        return Ok(SubscriptionCloseAction::ObserveSave(save.clone()));
    }
    if check.status == ComputeSubscriptionCheckStatusV2::Retained {
        return Err(SubscriptionPreparationError::InvalidRequest);
    }
    match check.status {
        ComputeSubscriptionCheckStatusV2::Checking => {
            if cancel_idempotency_key.trim().is_empty() {
                return Err(SubscriptionPreparationError::InvalidRequest);
            }
            Ok(SubscriptionCloseAction::CancelApproval(
                OperationCancelRequestV1 {
                    operation_id: check.approval_operation.operation_id.clone(),
                    idempotency_key: cancel_idempotency_key.to_owned(),
                },
            ))
        }
        ComputeSubscriptionCheckStatusV2::Verified => check
            .validation
            .clone()
            .map(SubscriptionCloseAction::ReleaseValidation)
            .ok_or(SubscriptionPreparationError::InvalidRequest),
        _ => Ok(SubscriptionCloseAction::NoWrite),
    }
}

pub fn validation_belongs_to_candidate(
    original: &ComputeCandidateRefV2,
    checked: &ComputeCandidateViewV2,
    validation: &ComputeValidationRefV2,
    approval_operation: &OperationReferenceV1,
) -> bool {
    original.candidate_ref == checked.candidate.candidate_ref
        && checked.candidate.candidate_revision > original.candidate_revision
        && checked.validation.as_ref() == Some(validation)
        && validation.approval_operation.operation_id == approval_operation.operation_id
}

#[cfg(test)]
mod tests;
