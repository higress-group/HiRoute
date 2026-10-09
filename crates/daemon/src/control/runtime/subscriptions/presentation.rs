//! Fail-closed recovery of presentation-safe facts for a saved native subscription.

use hiroute_application::compute_management::{
    ComputeCandidateFactBasisV2, ComputeCandidateFactValueV2, ComputeCandidateFactsV2,
};
use hiroute_application::control::ComputeManagementControlError;
use hiroute_application_api::{
    COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2, ComputeManagementChangeV2, ComputeManagementIntentV2,
    ComputeManagementSubjectV2, ComputeModelMembershipV2,
};
use hiroute_domain::{
    ComputeManagementFactBasisV2, ComputeManagementFactValueV2, ComputeManagementMembershipV2,
    ComputeManagementProvenanceV2, ComputeManagementSourceV2, ControlRepositoryPort, OperationId,
    OperationState,
};
use hiroute_local_storage::ComputeSubscriptionValidationStateV1;

use super::LocalControlAdapter;
use super::error::map_port;
use super::record::decode_stored;

impl LocalControlAdapter {
    /// Returns the immutable checked candidate only when the retained validation, successful save
    /// operation, and current saved aggregate still describe the same connector-owned source.
    /// Mismatches are ordinary missing presentation facts: the public row remains visible but is
    /// projected partial/unknown by the application layer.
    pub(in crate::control::runtime) fn retained_subscription_candidate_for_source(
        &self,
        source: &ComputeManagementSourceV2,
    ) -> Result<Option<ComputeCandidateFactsV2>, ComputeManagementControlError> {
        let Some(validation) = source.validation.as_ref() else {
            return Ok(None);
        };
        let Ok(approval_id) = OperationId::parse(&validation.approval_operation_id) else {
            return Ok(None);
        };
        let (record, save_operation) = {
            let stores = self
                .stores_lock()
                .map_err(|_| ComputeManagementControlError::Unavailable)?;
            let Some(record) = stores
                .control()
                .compute_subscription_validation(&approval_id)
                .map_err(map_port)?
            else {
                return Ok(None);
            };
            let Some(save_id) = record.save_operation_id.as_deref() else {
                return Ok(None);
            };
            let Ok(save_id) = OperationId::parse(save_id) else {
                return Ok(None);
            };
            let Some(save_operation) = stores
                .control()
                .load_operation(&save_id)
                .map_err(map_port)?
            else {
                return Ok(None);
            };
            (record, save_operation)
        };
        if record.state != ComputeSubscriptionValidationStateV1::Retained
            || save_operation.state != OperationState::Succeeded
            || save_operation.plan.spec().command_id != "compute.connection.apply"
            || save_operation.plan.spec().resource_id.as_deref() != Some(source.source_id.as_str())
            || save_operation
                .plan
                .spec()
                .desired_state
                .get("schema")
                .and_then(serde_json::Value::as_str)
                != Some(COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2)
        {
            return Ok(None);
        }
        let Ok(change) = serde_json::from_value::<ComputeManagementChangeV2>(
            save_operation.plan.spec().desired_state.clone(),
        ) else {
            return Ok(None);
        };
        if change.validate_shape().is_err()
            || !change.key_edits.is_empty()
            || !save_intent_matches_source(change.intent, source)
            || change.selected_model_refs
                != source
                    .models
                    .iter()
                    .map(|model| model.model_ref.clone())
                    .collect::<Vec<_>>()
        {
            return Ok(None);
        }
        let ComputeManagementSubjectV2::Candidate { candidate } = &change.subject else {
            return Ok(None);
        };
        let Ok(stored) = decode_stored(&record.record_json) else {
            return Ok(None);
        };
        let Ok(checked) = stored.checked_facts() else {
            return Ok(None);
        };
        if record.operation_id != approval_id.as_str()
            || record.candidate_ref != checked.candidate.candidate_ref
            || record.candidate_revision != checked.candidate.candidate_revision
            || candidate != &checked.candidate
            || change.validation.as_ref() != checked.validation.as_ref()
            || !saved_source_matches_checked(source, &checked)
        {
            return Ok(None);
        }
        Ok(Some(checked))
    }
}

fn save_intent_matches_source(
    intent: ComputeManagementIntentV2,
    source: &ComputeManagementSourceV2,
) -> bool {
    matches!(
        (intent, source.state),
        (
            ComputeManagementIntentV2::SaveReady,
            hiroute_domain::MaterializationState::Ready
        ) | (
            ComputeManagementIntentV2::SaveDisabled,
            hiroute_domain::MaterializationState::Disabled
        )
    )
}

fn saved_source_matches_checked(
    source: &ComputeManagementSourceV2,
    checked: &ComputeCandidateFactsV2,
) -> bool {
    let (
        ComputeManagementProvenanceV2::ConnectorOwned {
            connector_id,
            account_ref,
        },
        hiroute_application::compute_management::ComputeCandidateProvenanceV2::ConnectorOwned {
            connector_id: checked_connector,
            account_ref: checked_account,
        },
    ) = (&source.provenance, &checked.provenance)
    else {
        return false;
    };
    let Some(target) = checked.target.as_ref() else {
        return false;
    };
    let Some(authentication) = checked.authentication.as_ref() else {
        return false;
    };
    let Some(checked_validation) = checked.validation.as_ref() else {
        return false;
    };
    let validation_matches = source.validation.as_ref().is_some_and(|saved| {
        saved.approval_operation_id == checked_validation.approval_operation.operation_id
            && saved.validation_ref == checked_validation.validation_ref
            && saved.validation_revision == checked_validation.validation_revision
    });
    hiroute_cpa_bridge::CpaAccountKind::from_connector(connector_id).is_some()
        && connector_id == checked_connector
        && account_ref == checked_account
        && source.display_name == checked.display_name
        && source.last_candidate_ref == checked.candidate.candidate_ref
        && source.last_candidate_revision == checked.candidate.candidate_revision
        && source.credentials.is_empty()
        && validation_matches
        && source.target.scheme == target.scheme
        && source.target.authority == target.authority
        && source.target.port == target.port
        && source.target.request_path == target.request_path
        && source.target.upstream_protocol == target.upstream_protocol
        && source.target.protocol_profile_id == target.protocol_profile_id
        && source.target.protocol_profile_revision == target.protocol_profile_revision
        && &source.authentication == authentication
        && source.models.iter().all(|model| {
            let candidate = checked
                .models
                .iter()
                .find(|candidate| model.model_ref == candidate.model_ref);
            if !model.execution_eligible {
                return candidate
                    .is_none_or(|candidate| !candidate.selectable || candidate.reason.is_some());
            }
            candidate.is_some_and(|candidate| {
                candidate.selectable
                    && candidate.reason.is_none()
                    && model.model_ref == candidate.model_ref
                    && model.upstream_model_id == candidate.upstream_model_id
                    && model.display_name == candidate.display_name
                    && model.catalog_configuration_id == candidate.catalog_configuration_id
                    && membership_matches(model.membership, candidate.membership)
                    && model.capability_evidence_digest == candidate.capability_evidence_digest
                    && fact_matches(&model.capabilities.tool, &candidate.capabilities.tool)
                    && fact_matches(&model.capabilities.vision, &candidate.capabilities.vision)
                    && fact_matches(
                        &model.capabilities.streaming,
                        &candidate.capabilities.streaming,
                    )
                    && fact_matches(
                        &model.capabilities.context_tokens,
                        &candidate.capabilities.context_tokens,
                    )
                    && fact_matches(
                        &model.capabilities.max_output_tokens,
                        &candidate.capabilities.max_output_tokens,
                    )
                    && fact_matches(
                        &model.capabilities.native_reasoning,
                        &candidate.capabilities.native_reasoning,
                    )
            })
        })
}

fn membership_matches(
    saved: ComputeManagementMembershipV2,
    checked: ComputeModelMembershipV2,
) -> bool {
    matches!(
        (saved, checked),
        (
            ComputeManagementMembershipV2::Catalog,
            ComputeModelMembershipV2::Catalog
        ) | (
            ComputeManagementMembershipV2::Observed,
            ComputeModelMembershipV2::Observed
        ) | (
            ComputeManagementMembershipV2::UserDeclared,
            ComputeModelMembershipV2::UserDeclared
        )
    )
}

fn fact_matches<T: Eq>(
    saved: &ComputeManagementFactValueV2<T>,
    checked: &ComputeCandidateFactValueV2<T>,
) -> bool {
    saved.value == checked.value
        && matches!(
            (saved.basis, checked.basis),
            (
                ComputeManagementFactBasisV2::RegisteredCatalog,
                ComputeCandidateFactBasisV2::RegisteredCatalog
            ) | (
                ComputeManagementFactBasisV2::RuntimeFallback,
                ComputeCandidateFactBasisV2::RuntimeFallback
            ) | (
                ComputeManagementFactBasisV2::Observed,
                ComputeCandidateFactBasisV2::Observed
            ) | (
                ComputeManagementFactBasisV2::UserDeclared,
                ComputeCandidateFactBasisV2::UserDeclared
            ) | (
                ComputeManagementFactBasisV2::ConnectorVerified,
                ComputeCandidateFactBasisV2::ConnectorVerified
            ) | (
                ComputeManagementFactBasisV2::Unknown,
                ComputeCandidateFactBasisV2::Unknown
            )
        )
}
