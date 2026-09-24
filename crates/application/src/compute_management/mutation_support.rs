//! Pure validation and mapping helpers for the management mutation planner.

use std::collections::{BTreeMap, BTreeSet};

use hiroute_application_api::{
    ComputeCandidateProducerV2, ComputeCandidateRefV2, ComputeManagementIntentV2,
    ComputeModelMembershipV2, ComputeSaveChangeViewV2, ComputeValidationRefV2,
};
use hiroute_domain::{
    CanonicalDigest, ComputeManagedCapabilitiesV2, ComputeManagedCredentialV2,
    ComputeManagedModelV2, ComputeManagementFactBasisV2, ComputeManagementFactValueV2,
    ComputeManagementMembershipV2, ComputeManagementProvenanceV2, ComputeManagementSourceV2,
    ComputeManagementValidationV2, GatewayAuthenticationSemanticsV1, MaterializationState,
};

use super::mutations::ComputeManagementPlanningErrorV2;
use super::{
    ComputeCandidateFactBasisV2, ComputeCandidateFactValueV2, ComputeCandidateFactsV2,
    ComputeCredentialBindingV2,
};

pub(super) fn candidate_lineage_digest(
    facts: &ComputeCandidateFactsV2,
) -> Result<CanonicalDigest, ComputeManagementPlanningErrorV2> {
    if let Some(digest) = &facts.trusted_lineage_digest {
        if facts.existing_source_id.is_none() {
            return Err(ComputeManagementPlanningErrorV2::InvalidCandidate);
        }
        return Ok(digest.clone());
    }
    let stable_provenance = match &facts.provenance {
        super::ComputeCandidateProvenanceV2::Registered {
            connection_option_id,
            ..
        } => serde_json::json!({"kind":"registered","connection_option_id":connection_option_id}),
        super::ComputeCandidateProvenanceV2::UserConfigured { .. } => {
            serde_json::json!({"kind":"user_configured"})
        }
        super::ComputeCandidateProvenanceV2::ConnectorOwnedPendingApproval { connector_id } => {
            serde_json::json!({"kind":"connector_owned_pending","connector_id":connector_id})
        }
        super::ComputeCandidateProvenanceV2::ConnectorOwned {
            connector_id,
            account_ref,
            ..
        } => serde_json::json!({
            "kind":"connector_owned",
            "connector_id":connector_id,
            "account_ref":account_ref,
        }),
    };
    CanonicalDigest::of(&(
        "hiroute.compute-management-lineage/v3",
        facts.producer,
        &facts.lineage_ref,
        stable_provenance,
    ))
    .map_err(|_| ComputeManagementPlanningErrorV2::InvalidCandidate)
}

pub(super) fn map_provenance(
    provenance: &super::ComputeCandidateProvenanceV2,
) -> Result<ComputeManagementProvenanceV2, ComputeManagementPlanningErrorV2> {
    Ok(match provenance {
        super::ComputeCandidateProvenanceV2::Registered {
            connection_option_id,
            registry_version,
            catalog_digest,
        } => ComputeManagementProvenanceV2::Registered {
            connection_option_id: connection_option_id.clone(),
            registry_version: registry_version.clone(),
            catalog_digest: catalog_digest.clone(),
        },
        super::ComputeCandidateProvenanceV2::UserConfigured {
            configuration_revision,
            evidence_digest,
        } => ComputeManagementProvenanceV2::UserConfigured {
            configuration_revision: *configuration_revision,
            evidence_digest: evidence_digest.clone(),
        },
        super::ComputeCandidateProvenanceV2::ConnectorOwned {
            connector_id,
            account_ref,
        } => ComputeManagementProvenanceV2::ConnectorOwned {
            connector_id: connector_id.clone(),
            account_ref: account_ref.clone(),
        },
        super::ComputeCandidateProvenanceV2::ConnectorOwnedPendingApproval { .. } => {
            return Err(ComputeManagementPlanningErrorV2::ApprovalRequired);
        }
    })
}

pub(super) fn map_validation(
    value: Option<&ComputeValidationRefV2>,
) -> Option<ComputeManagementValidationV2> {
    value.map(|validation| ComputeManagementValidationV2 {
        approval_operation_id: validation.approval_operation.operation_id.clone(),
        validation_ref: validation.validation_ref.clone(),
        validation_revision: validation.validation_revision,
    })
}

pub(super) fn validate_saved_validation(
    source: &ComputeManagementSourceV2,
    requested: Option<&ComputeValidationRefV2>,
) -> Result<(), ComputeManagementPlanningErrorV2> {
    let requested = map_validation(requested);
    if source.validation != requested {
        return Err(ComputeManagementPlanningErrorV2::ValidationConflict);
    }
    Ok(())
}

pub(super) fn select_candidate_models(
    source_id: &str,
    current: Option<&ComputeManagementSourceV2>,
    facts: &ComputeCandidateFactsV2,
    selected: &[String],
    intent: ComputeManagementIntentV2,
) -> Result<Vec<ComputeManagedModelV2>, ComputeManagementPlanningErrorV2> {
    ensure_unique_nonempty(selected)?;
    selected
        .iter()
        .map(|model_ref| {
            let candidate = facts
                .models
                .iter()
                .find(|candidate| &candidate.model_ref == model_ref)
                .filter(|candidate| candidate_model_can_be_saved(facts, candidate, intent))
                .ok_or(ComputeManagementPlanningErrorV2::ModelNotSelectable)?;
            let before = current.and_then(|source| {
                source.models.iter().find(|model| {
                    model.model_ref == candidate.model_ref
                        || (facts.producer == ComputeCandidateProducerV2::Native
                            && model.upstream_model_id == candidate.upstream_model_id)
                })
            });
            let binding_id = match before {
                Some(model) => model.binding_id.clone(),
                None => managed_binding_id(source_id, &candidate.model_ref)?,
            };
            let mut desired = ComputeManagedModelV2 {
                model_ref: before.map_or_else(
                    || candidate.model_ref.clone(),
                    |model| model.model_ref.clone(),
                ),
                binding_id,
                revision: before.map_or(1, |model| model.revision),
                upstream_model_id: candidate.upstream_model_id.clone(),
                display_name: candidate.display_name.clone(),
                catalog_configuration_id: candidate.catalog_configuration_id.clone(),
                membership: map_membership(candidate.membership),
                execution_eligible: true,
                capabilities: map_capabilities(&candidate.capabilities),
                capability_evidence_digest: candidate.capability_evidence_digest.clone(),
            };
            if before.is_some_and(|before| !same_model_facts(before, &desired)) {
                desired.revision = desired
                    .revision
                    .checked_add(1)
                    .ok_or(ComputeManagementPlanningErrorV2::InvalidCandidate)?;
            }
            Ok(desired)
        })
        .collect()
}

pub(super) fn retain_subscription_models(
    source_id: &str,
    current: &ComputeManagementSourceV2,
    facts: &ComputeCandidateFactsV2,
    selected: &[String],
) -> Result<Vec<ComputeManagedModelV2>, ComputeManagementPlanningErrorV2> {
    ensure_unique_nonempty(selected)?;
    if current.source_id != source_id
        || selected
            != current
                .models
                .iter()
                .map(|model| model.model_ref.clone())
                .collect::<Vec<_>>()
    {
        return Err(ComputeManagementPlanningErrorV2::ModelNotSelectable);
    }
    selected
        .iter()
        .map(|model_ref| {
            let before = current
                .models
                .iter()
                .find(|model| &model.model_ref == model_ref)
                .ok_or(ComputeManagementPlanningErrorV2::ModelNotSelectable)?;
            let candidate = facts
                .models
                .iter()
                .find(|model| &model.model_ref == model_ref);
            let mut desired =
                match candidate.filter(|model| model.selectable && model.reason.is_none()) {
                    Some(candidate) => ComputeManagedModelV2 {
                        model_ref: candidate.model_ref.clone(),
                        binding_id: before.binding_id.clone(),
                        revision: before.revision,
                        upstream_model_id: candidate.upstream_model_id.clone(),
                        display_name: candidate.display_name.clone(),
                        catalog_configuration_id: candidate.catalog_configuration_id.clone(),
                        membership: map_membership(candidate.membership),
                        execution_eligible: true,
                        capabilities: map_capabilities(&candidate.capabilities),
                        capability_evidence_digest: candidate.capability_evidence_digest.clone(),
                    },
                    None => {
                        let mut retained = before.clone();
                        retained.execution_eligible = false;
                        retained
                    }
                };
            if !same_model_facts(before, &desired) {
                desired.revision = before
                    .revision
                    .checked_add(1)
                    .ok_or(ComputeManagementPlanningErrorV2::InvalidCandidate)?;
            }
            Ok(desired)
        })
        .collect()
}

fn candidate_model_can_be_saved(
    facts: &ComputeCandidateFactsV2,
    candidate: &super::ComputeCandidateModelFactsV2,
    intent: ComputeManagementIntentV2,
) -> bool {
    candidate.selectable
        // A failed or credential-blocked connection cannot become Ready, but its trusted Native
        // declaration remains valid disabled management state.
        || (intent == ComputeManagementIntentV2::SaveDisabled
            && facts.producer == ComputeCandidateProducerV2::Native
            && candidate.membership == ComputeModelMembershipV2::UserDeclared
            && candidate.reason.as_deref()
                == Some("model_connections.connection_check_required"))
}

pub(super) fn select_saved_models(
    source: &ComputeManagementSourceV2,
    selected: &[String],
) -> Result<Vec<ComputeManagedModelV2>, ComputeManagementPlanningErrorV2> {
    ensure_unique_nonempty(selected)?;
    selected
        .iter()
        .map(|model_ref| {
            source
                .models
                .iter()
                .find(|model| &model.model_ref == model_ref)
                .cloned()
                .ok_or(ComputeManagementPlanningErrorV2::ModelNotSelectable)
        })
        .collect()
}

pub(super) fn map_capabilities(
    value: &super::ComputeCandidateCapabilityFactsV2,
) -> ComputeManagedCapabilitiesV2 {
    ComputeManagedCapabilitiesV2 {
        tool: map_fact(&value.tool),
        vision: map_fact(&value.vision),
        streaming: map_fact(&value.streaming),
        context_tokens: map_fact(&value.context_tokens),
        max_output_tokens: map_fact(&value.max_output_tokens),
        native_reasoning: map_fact(&value.native_reasoning),
    }
}

pub(super) fn map_fact<T: Clone>(
    value: &ComputeCandidateFactValueV2<T>,
) -> ComputeManagementFactValueV2<T> {
    ComputeManagementFactValueV2 {
        value: value.value.clone(),
        basis: match value.basis {
            ComputeCandidateFactBasisV2::RegisteredCatalog => {
                ComputeManagementFactBasisV2::RegisteredCatalog
            }
            ComputeCandidateFactBasisV2::RuntimeFallback => {
                ComputeManagementFactBasisV2::RuntimeFallback
            }
            ComputeCandidateFactBasisV2::Observed => ComputeManagementFactBasisV2::Observed,
            ComputeCandidateFactBasisV2::UserDeclared => ComputeManagementFactBasisV2::UserDeclared,
            ComputeCandidateFactBasisV2::ConnectorVerified => {
                ComputeManagementFactBasisV2::ConnectorVerified
            }
            ComputeCandidateFactBasisV2::Unknown => ComputeManagementFactBasisV2::Unknown,
        },
    }
}

pub(super) fn map_membership(value: ComputeModelMembershipV2) -> ComputeManagementMembershipV2 {
    match value {
        ComputeModelMembershipV2::Catalog => ComputeManagementMembershipV2::Catalog,
        ComputeModelMembershipV2::Observed => ComputeManagementMembershipV2::Observed,
        ComputeModelMembershipV2::UserDeclared => ComputeManagementMembershipV2::UserDeclared,
    }
}

pub(super) fn same_model_facts(
    before: &ComputeManagedModelV2,
    after: &ComputeManagedModelV2,
) -> bool {
    before.model_ref == after.model_ref
        && before.binding_id == after.binding_id
        && before.upstream_model_id == after.upstream_model_id
        && before.display_name == after.display_name
        && before.catalog_configuration_id == after.catalog_configuration_id
        && before.membership == after.membership
        && before.execution_eligible == after.execution_eligible
        && before.capabilities == after.capabilities
        && before.capability_evidence_digest == after.capability_evidence_digest
}

pub(super) fn management_state(
    intent: ComputeManagementIntentV2,
    provenance: &ComputeManagementProvenanceV2,
    authentication: &GatewayAuthenticationSemanticsV1,
    additional_native_endpoints: &[hiroute_domain::ComputeNativeEndpointV3],
    credentials: &[ComputeManagedCredentialV2],
) -> Result<MaterializationState, ComputeManagementPlanningErrorV2> {
    let needs_native_key = provenance.is_native()
        && (*authentication != GatewayAuthenticationSemanticsV1::None
            || additional_native_endpoints
                .iter()
                .any(|endpoint| endpoint.authentication != GatewayAuthenticationSemanticsV1::None));
    match intent {
        ComputeManagementIntentV2::SaveDisabled if needs_native_key && credentials.is_empty() => {
            Ok(MaterializationState::NeedsCredential)
        }
        ComputeManagementIntentV2::SaveDisabled => Ok(MaterializationState::Disabled),
        ComputeManagementIntentV2::SaveReady
            if needs_native_key && !credentials.iter().any(|credential| credential.enabled) =>
        {
            Err(ComputeManagementPlanningErrorV2::CredentialRequired)
        }
        ComputeManagementIntentV2::SaveReady => Ok(MaterializationState::Ready),
    }
}

pub(super) fn protected_slot(
    input: &ComputeCandidateFactsV2,
) -> Result<&str, ComputeManagementPlanningErrorV2> {
    match &input.credential_binding {
        ComputeCredentialBindingV2::NativeProtected { input_slot, .. } => Ok(input_slot),
        _ => Err(ComputeManagementPlanningErrorV2::CredentialRequired),
    }
}

pub(super) fn key_index(
    credentials: &[ComputeManagedCredentialV2],
    key_id: &str,
    expected_generation: u64,
) -> Result<usize, ComputeManagementPlanningErrorV2> {
    credentials
        .iter()
        .position(|credential| {
            credential.key_id == key_id && credential.credential.generation() == expected_generation
        })
        .ok_or(ComputeManagementPlanningErrorV2::CredentialConflict)
}

pub(super) fn key_index_after_replacement(
    credentials: &[ComputeManagedCredentialV2],
    key_id: &str,
    expected_generation: u64,
    replaced: bool,
) -> Result<usize, ComputeManagementPlanningErrorV2> {
    credentials
        .iter()
        .position(|credential| {
            credential.key_id == key_id
                && (credential.credential.generation() == expected_generation
                    || replaced
                        && expected_generation.checked_add(1)
                            == Some(credential.credential.generation()))
        })
        .ok_or(ComputeManagementPlanningErrorV2::CredentialConflict)
}

pub(super) fn reorder_keys(
    credentials: &mut Vec<ComputeManagedCredentialV2>,
    key_ids: &[String],
) -> Result<(), ComputeManagementPlanningErrorV2> {
    if key_ids.len() != credentials.len() {
        return Err(ComputeManagementPlanningErrorV2::InvalidKeyEdit);
    }
    let mut by_id = std::mem::take(credentials)
        .into_iter()
        .map(|credential| (credential.key_id.clone(), credential))
        .collect::<BTreeMap<_, _>>();
    let mut ordered = Vec::with_capacity(key_ids.len());
    for key_id in key_ids {
        ordered.push(
            by_id
                .remove(key_id)
                .ok_or(ComputeManagementPlanningErrorV2::InvalidKeyEdit)?,
        );
    }
    if !by_id.is_empty() {
        return Err(ComputeManagementPlanningErrorV2::InvalidKeyEdit);
    }
    *credentials = ordered;
    Ok(())
}

pub(super) fn normalize_key_order(credentials: &mut [ComputeManagedCredentialV2]) {
    for (index, credential) in credentials.iter_mut().enumerate() {
        credential.ordinal = index as u32;
    }
}

pub(super) fn managed_key_id(
    source_id: &str,
    candidate: &ComputeCandidateRefV2,
) -> Result<String, ComputeManagementPlanningErrorV2> {
    let digest = CanonicalDigest::of(&(
        "hiroute.compute-management-key/v2",
        source_id,
        &candidate.candidate_ref,
    ))
    .map_err(|_| ComputeManagementPlanningErrorV2::InvalidKeyEdit)?;
    Ok(format!("credential/managed-{}", digest_suffix(&digest, 24)))
}

pub(super) fn managed_binding_id(
    source_id: &str,
    model_ref: &str,
) -> Result<String, ComputeManagementPlanningErrorV2> {
    let digest = CanonicalDigest::of(&(
        "hiroute.compute-management-binding/v2",
        source_id,
        model_ref,
    ))
    .map_err(|_| ComputeManagementPlanningErrorV2::InvalidCandidate)?;
    Ok(format!("binding/managed-{}", digest_suffix(&digest, 24)))
}

pub(super) fn digest_suffix(digest: &CanonicalDigest, length: usize) -> &str {
    hiroute_domain::digest_suffix(digest, length)
}

pub(super) fn ensure_unique_nonempty(
    values: &[String],
) -> Result<(), ComputeManagementPlanningErrorV2> {
    let unique = values.iter().collect::<BTreeSet<_>>();
    if values.is_empty()
        || values.iter().any(|value| value.trim().is_empty())
        || unique.len() != values.len()
    {
        return Err(ComputeManagementPlanningErrorV2::ModelNotSelectable);
    }
    Ok(())
}

pub(super) fn preview_changes(
    current: Option<&ComputeManagementSourceV2>,
    desired: &ComputeManagementSourceV2,
) -> Vec<ComputeSaveChangeViewV2> {
    let mut changes = vec![ComputeSaveChangeViewV2 {
        resource_kind: "compute_source".into(),
        resource_id: desired.source_id.clone(),
        action: if current.is_some() {
            "update"
        } else {
            "create"
        }
        .into(),
    }];
    let before_models = current
        .map(|source| {
            source
                .models
                .iter()
                .map(|model| (model.model_ref.as_str(), model))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    for model in &desired.models {
        let action = match before_models.get(model.model_ref.as_str()) {
            None => "add",
            Some(before) if *before != model => "update",
            Some(_) => "retain",
        };
        changes.push(ComputeSaveChangeViewV2 {
            resource_kind: "model_binding".into(),
            resource_id: model.binding_id.clone(),
            action: action.into(),
        });
    }
    let after_model_refs = desired
        .models
        .iter()
        .map(|model| model.model_ref.as_str())
        .collect::<BTreeSet<_>>();
    for removed in before_models
        .values()
        .filter(|model| !after_model_refs.contains(model.model_ref.as_str()))
    {
        changes.push(ComputeSaveChangeViewV2 {
            resource_kind: "model_binding".into(),
            resource_id: removed.binding_id.clone(),
            action: "remove".into(),
        });
    }
    let before_keys = current
        .map(|source| {
            source
                .credentials
                .iter()
                .map(|key| (key.key_id.as_str(), key))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let after_keys = desired
        .credentials
        .iter()
        .map(|key| (key.key_id.as_str(), key))
        .collect::<BTreeMap<_, _>>();
    for key_id in before_keys
        .keys()
        .chain(after_keys.keys())
        .copied()
        .collect::<BTreeSet<_>>()
    {
        let action = match (before_keys.get(key_id), after_keys.get(key_id)) {
            (None, Some(_)) => "add",
            (Some(_), None) => "remove",
            (Some(before), Some(after)) if before != after => "update",
            _ => continue,
        };
        changes.push(ComputeSaveChangeViewV2 {
            resource_kind: "credential_key".into(),
            resource_id: key_id.into(),
            action: action.into(),
        });
    }
    changes
}
