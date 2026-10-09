//! Durable A-lifecycle correlation that is independent of CPA-verified account evidence.

use super::*;
use hiroute_local_storage::ComputeSubscriptionValidationRecordV1;

impl LocalControlAdapter {
    pub(super) fn committed_subscription_evidence(
        &self,
        source: &hiroute_domain::ComputeManagementSourceV2,
    ) -> Result<Option<CanonicalDigest>, hiroute_application::control::ComputeManagementControlError>
    {
        let Some(validation) = source.validation.as_ref() else {
            return Ok(None);
        };
        let operation_id = OperationId::parse(&validation.approval_operation_id)
            .map_err(|_| hiroute_application::control::ComputeManagementControlError::Corrupt)?;
        let (record, save_operation) = {
            let stores = self.stores_lock().map_err(|_| {
                hiroute_application::control::ComputeManagementControlError::Unavailable
            })?;
            let record = stores
                .control()
                .compute_subscription_validation(&operation_id)
                .map_err(map_port)?
                .ok_or(hiroute_application::control::ComputeManagementControlError::Corrupt)?;
            let save_operation_id = record
                .save_operation_id
                .as_deref()
                .ok_or(hiroute_application::control::ComputeManagementControlError::Corrupt)
                .and_then(|value| {
                    OperationId::parse(value).map_err(|_| {
                        hiroute_application::control::ComputeManagementControlError::Corrupt
                    })
                })?;
            let save_operation = stores
                .control()
                .load_operation(&save_operation_id)
                .map_err(map_port)?
                .ok_or(hiroute_application::control::ComputeManagementControlError::Corrupt)?;
            (record, save_operation)
        };
        if record.state != hiroute_local_storage::ComputeSubscriptionValidationStateV1::Retained
            || save_operation.state != OperationState::Succeeded
            || save_operation.plan.spec().resource_id.as_deref() != Some(source.source_id.as_str())
        {
            return Err(hiroute_application::control::ComputeManagementControlError::Corrupt);
        }
        let stored = decode_stored(&record.record_json).map_err(map_port)?;
        if stored.validation.validation_ref != validation.validation_ref
            || stored.validation.validation_revision != validation.validation_revision
            || stored.validation.approval_operation.operation_id != validation.approval_operation_id
            || stored
                .existing_source_id
                .as_deref()
                .is_some_and(|source_id| source_id != source.source_id)
        {
            return Err(hiroute_application::control::ComputeManagementControlError::Corrupt);
        }
        self.subscription_source_evidence(&record, &stored)
            .map(Some)
    }

    pub(super) fn subscription_source_evidence(
        &self,
        record: &ComputeSubscriptionValidationRecordV1,
        stored: &StoredSubscriptionValidationV1,
    ) -> Result<CanonicalDigest, hiroute_application::control::ComputeManagementControlError> {
        let operation_id = OperationId::parse(&record.operation_id)
            .map_err(|_| hiroute_application::control::ComputeManagementControlError::Corrupt)?;
        let stores = self.stores_lock().map_err(|_| {
            hiroute_application::control::ComputeManagementControlError::Unavailable
        })?;
        let operation = stores
            .control()
            .load_operation(&operation_id)
            .map_err(map_port)?
            .ok_or(hiroute_application::control::ComputeManagementControlError::Corrupt)?;
        subscription_source_evidence_from_plan(
            &operation.plan,
            &stored.original_candidate,
            &stored.checked_candidate,
        )
    }

    pub(super) fn subscription_source_status(
        &self,
        record: &ComputeSubscriptionValidationRecordV1,
        stored: &StoredSubscriptionValidationV1,
    ) -> Result<
        Option<(ComputeSubscriptionCheckStatusV2, &'static str)>,
        hiroute_application::control::ComputeManagementControlError,
    > {
        let expected = self.subscription_source_evidence(record, stored)?;
        let source = match self.scanner.subscription_source(
            CpaAccountKind::from_candidate(&record.candidate_ref)
                .ok_or(hiroute_application::control::ComputeManagementControlError::Corrupt)?,
        ) {
            Ok(Some(source)) => source,
            Ok(None) => {
                return Ok(Some((
                    ComputeSubscriptionCheckStatusV2::NeedsAuth,
                    "SUBSCRIPTION_NEEDS_AUTH",
                )));
            }
            Err(_) => {
                return Ok(Some((
                    ComputeSubscriptionCheckStatusV2::Unavailable,
                    "SUBSCRIPTION_SOURCE_UNAVAILABLE",
                )));
            }
        };
        if subscription_candidate_ref(&source)? != record.candidate_ref
            || source.evidence_digest() != &expected
        {
            return Ok(Some((
                ComputeSubscriptionCheckStatusV2::SourceChanged,
                "SUBSCRIPTION_SOURCE_CHANGED",
            )));
        }
        Ok(None)
    }
}

pub(super) fn subscription_source_evidence_from_plan(
    plan: &hiroute_domain::TransactionPlanV1,
    original: &ComputeCandidateRefV2,
    checked: &ComputeCandidateRefV2,
) -> Result<CanonicalDigest, hiroute_application::control::ComputeManagementControlError> {
    let intent = plan
        .external()
        .first()
        .and_then(|effect| hiroute_domain::decode_subscription_check_intent(effect.desired()).ok())
        .ok_or(hiroute_application::control::ComputeManagementControlError::Corrupt)?;
    if intent.candidate_ref() != original.candidate_ref
        || intent.candidate_revision() != original.candidate_revision
        || checked.candidate_ref != original.candidate_ref
        || checked.candidate_revision
            != original
                .candidate_revision
                .checked_add(1)
                .ok_or(hiroute_application::control::ComputeManagementControlError::Corrupt)?
    {
        return Err(hiroute_application::control::ComputeManagementControlError::Corrupt);
    }
    Ok(intent.expected_evidence_digest().clone())
}
