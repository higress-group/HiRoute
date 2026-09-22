//! Deterministic bridge from saved management truth to local compiler inputs.

use hiroute_domain::{
    ComputeCredentialSelectionV2, ComputeManagedCapabilitiesV2, ComputeManagedModelV2,
    ComputeManagementFactBasisV2, ComputeManagementMembershipV2, ComputeManagementProvenanceV2,
    ComputeManagementSourceV2, ComputeManagementTargetV2, GatewayAuthenticationSemanticsV1,
    MaterializationState, NativeReasoningCapabilityV1,
};
use thiserror::Error;

use super::mutation_support::{managed_binding_id, map_capabilities, map_membership};
use super::{ComputeCandidateFactsV2, ComputeCandidateModelFactsV2, ComputeCandidateProvenanceV2};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComputeManagementEligibilityV2 {
    CatalogMatched,
    RuntimeQualified,
    UserConfirmed,
    ConnectorVerified,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComputeManagementCredentialCompilationV2 {
    Native {
        ordered: Vec<ComputeCredentialSelectionV2>,
    },
    ConnectorOwned {
        connector_id: String,
        account_ref: String,
        validation_ref: String,
        validation_revision: u64,
    },
}

/// One source-local model fact ready for the central routing snapshot join.
///
/// This value deliberately does not masquerade as a Registry bundle. The central compiler
/// writer can merge it with ratings, prices, plan references, and shared schema changes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComputeManagementCompilationFactV2 {
    pub source_id: String,
    pub source_revision: u64,
    pub source_lineage_digest: hiroute_domain::CanonicalDigest,
    pub binding_id: String,
    pub binding_revision: u64,
    pub model_ref: String,
    pub upstream_model_id: String,
    pub display_name: String,
    pub catalog_configuration_id: Option<String>,
    pub eligibility: ComputeManagementEligibilityV2,
    pub provenance: ComputeManagementProvenanceV2,
    pub target: ComputeManagementTargetV2,
    pub authentication: GatewayAuthenticationSemanticsV1,
    pub capabilities: ComputeManagedCapabilitiesV2,
    pub capability_evidence_digest: hiroute_domain::CanonicalDigest,
    pub native_reasoning: NativeReasoningCapabilityV1,
    pub credential: ComputeManagementCredentialCompilationV2,
}

pub fn compile_compute_management_source(
    source: &ComputeManagementSourceV2,
) -> Result<Vec<ComputeManagementCompilationFactV2>, ComputeManagementCompilationErrorV2> {
    source
        .validate()
        .map_err(|_| ComputeManagementCompilationErrorV2::InvalidSource)?;
    if source.state != MaterializationState::Ready {
        return Err(ComputeManagementCompilationErrorV2::NotReady);
    }
    let credential = compile_credential(source)?;
    source
        .models
        .iter()
        .filter(|model| model.execution_eligible)
        .map(|model| {
            compile_model(
                source,
                model,
                eligibility(source, model)?,
                credential.clone(),
            )
        })
        .collect()
}

/// Compile a checked Codex subscription model for one Agent connection without adding it to the
/// durable source or the general Plan candidate set. The caller must obtain `checked` through the
/// retained-validation join for this exact saved source and re-intersect the result with live CPA.
pub fn compile_connection_only_codex_model(
    source: &ComputeManagementSourceV2,
    checked: &ComputeCandidateFactsV2,
    model: &ComputeCandidateModelFactsV2,
) -> Result<ComputeManagementCompilationFactV2, ComputeManagementCompilationErrorV2> {
    let (
        ComputeManagementProvenanceV2::ConnectorOwned {
            connector_id,
            account_ref,
        },
        ComputeCandidateProvenanceV2::ConnectorOwned {
            connector_id: checked_connector,
            account_ref: checked_account,
        },
    ) = (&source.provenance, &checked.provenance)
    else {
        return Err(ComputeManagementCompilationErrorV2::AuthorizationMissing);
    };
    if connector_id != checked_connector
        || account_ref != checked_account
        || !model.selectable
        || model.reason.is_some()
        || model.membership != hiroute_application_api::ComputeModelMembershipV2::Catalog
        || model.catalog_configuration_id.is_none()
        || source
            .models
            .iter()
            .any(|saved| saved.model_ref == model.model_ref)
        || !checked.models.iter().any(|candidate| candidate == model)
    {
        return Err(ComputeManagementCompilationErrorV2::EligibilityMismatch);
    }
    let binding_id = managed_binding_id(&source.source_id, &model.model_ref)
        .map_err(|_| ComputeManagementCompilationErrorV2::InvalidSource)?;
    let mut connection_source = source.clone();
    connection_source.models.push(ComputeManagedModelV2 {
        model_ref: model.model_ref.clone(),
        binding_id: binding_id.clone(),
        revision: 1,
        upstream_model_id: model.upstream_model_id.clone(),
        display_name: model.display_name.clone(),
        catalog_configuration_id: model.catalog_configuration_id.clone(),
        membership: map_membership(model.membership),
        execution_eligible: true,
        capabilities: map_capabilities(&model.capabilities),
        capability_evidence_digest: model.capability_evidence_digest.clone(),
    });
    compile_compute_management_source(&connection_source)?
        .into_iter()
        .find(|fact| fact.binding_id == binding_id)
        .ok_or(ComputeManagementCompilationErrorV2::InvalidSource)
}

fn compile_model(
    source: &ComputeManagementSourceV2,
    model: &ComputeManagedModelV2,
    eligibility: ComputeManagementEligibilityV2,
    credential: ComputeManagementCredentialCompilationV2,
) -> Result<ComputeManagementCompilationFactV2, ComputeManagementCompilationErrorV2> {
    let capabilities = &model.capabilities;
    if (eligibility == ComputeManagementEligibilityV2::CatalogMatched
        || matches!(
            source.provenance,
            ComputeManagementProvenanceV2::ConnectorOwned { .. }
        ))
        && (capabilities.tool.value.is_none()
            || capabilities.vision.value.is_none()
            || capabilities.streaming.value.is_none()
            || capabilities.context_tokens.value.is_none()
            || capabilities.max_output_tokens.value.is_none())
    {
        return Err(ComputeManagementCompilationErrorV2::UnknownCapability);
    }
    if eligibility == ComputeManagementEligibilityV2::UserConfirmed
        && (model.membership != ComputeManagementMembershipV2::UserDeclared
            || model.catalog_configuration_id.is_some()
            || [
                capabilities.tool.basis,
                capabilities.vision.basis,
                capabilities.streaming.basis,
                capabilities.context_tokens.basis,
                capabilities.max_output_tokens.basis,
                capabilities.native_reasoning.basis,
            ]
            .into_iter()
            .any(|basis| {
                !matches!(
                    basis,
                    ComputeManagementFactBasisV2::UserDeclared
                        | ComputeManagementFactBasisV2::Unknown
                )
            }))
    {
        return Err(ComputeManagementCompilationErrorV2::EligibilityMismatch);
    }
    if eligibility == ComputeManagementEligibilityV2::RuntimeQualified
        && (!matches!(
            model.membership,
            ComputeManagementMembershipV2::Observed | ComputeManagementMembershipV2::UserDeclared
        ) || model.catalog_configuration_id.is_some()
            || capability_bases(capabilities).into_iter().any(|basis| {
                !matches!(
                    basis,
                    ComputeManagementFactBasisV2::RuntimeFallback
                        | ComputeManagementFactBasisV2::Unknown
                        | ComputeManagementFactBasisV2::UserDeclared
                )
            }))
    {
        return Err(ComputeManagementCompilationErrorV2::EligibilityMismatch);
    }
    if matches!(
        source.provenance,
        ComputeManagementProvenanceV2::ConnectorOwned { .. }
    ) && (capabilities.native_reasoning.value.is_none()
        || (eligibility == ComputeManagementEligibilityV2::RuntimeQualified
            && capability_bases(capabilities)
                .into_iter()
                .any(|basis| basis != ComputeManagementFactBasisV2::RuntimeFallback)))
    {
        return Err(ComputeManagementCompilationErrorV2::UnknownCapability);
    }
    let native_reasoning = capabilities
        .native_reasoning
        .value
        .clone()
        .unwrap_or_else(|| NativeReasoningCapabilityV1::Fixed {
            profile: "non-thinking".into(),
        });
    native_reasoning
        .validate()
        .map_err(|_| ComputeManagementCompilationErrorV2::UnknownCapability)?;
    Ok(ComputeManagementCompilationFactV2 {
        source_id: source.source_id.clone(),
        source_revision: source.revision,
        source_lineage_digest: source.lineage_digest.clone(),
        binding_id: model.binding_id.clone(),
        binding_revision: model.revision,
        model_ref: model.model_ref.clone(),
        upstream_model_id: model.upstream_model_id.clone(),
        display_name: model.display_name.clone(),
        catalog_configuration_id: model.catalog_configuration_id.clone(),
        eligibility,
        provenance: source.provenance.clone(),
        target: source.target.clone(),
        authentication: source.authentication.clone(),
        capabilities: capabilities.clone(),
        capability_evidence_digest: model.capability_evidence_digest.clone(),
        native_reasoning,
        credential,
    })
}

fn eligibility(
    source: &ComputeManagementSourceV2,
    model: &ComputeManagedModelV2,
) -> Result<ComputeManagementEligibilityV2, ComputeManagementCompilationErrorV2> {
    if (matches!(
        source.provenance,
        ComputeManagementProvenanceV2::Registered { .. }
    ) || model.membership == ComputeManagementMembershipV2::Observed)
        && matches!(
            model.membership,
            ComputeManagementMembershipV2::Observed | ComputeManagementMembershipV2::UserDeclared
        )
        && model.catalog_configuration_id.is_none()
        && capability_bases(&model.capabilities)
            .into_iter()
            .all(|basis| {
                matches!(
                    basis,
                    ComputeManagementFactBasisV2::RuntimeFallback
                        | ComputeManagementFactBasisV2::Unknown
                        | ComputeManagementFactBasisV2::UserDeclared
                )
            })
    {
        return Ok(ComputeManagementEligibilityV2::RuntimeQualified);
    }
    match &source.provenance {
        ComputeManagementProvenanceV2::Registered { .. } => {
            if model.catalog_configuration_id.is_none()
                || model.membership != ComputeManagementMembershipV2::Catalog
            {
                return Err(ComputeManagementCompilationErrorV2::EligibilityMismatch);
            }
            Ok(ComputeManagementEligibilityV2::CatalogMatched)
        }
        ComputeManagementProvenanceV2::UserConfigured { .. } => {
            Ok(ComputeManagementEligibilityV2::UserConfirmed)
        }
        ComputeManagementProvenanceV2::ConnectorOwned { .. } => {
            Ok(ComputeManagementEligibilityV2::ConnectorVerified)
        }
    }
}

fn capability_bases(
    capabilities: &ComputeManagedCapabilitiesV2,
) -> [ComputeManagementFactBasisV2; 6] {
    [
        capabilities.tool.basis,
        capabilities.vision.basis,
        capabilities.streaming.basis,
        capabilities.context_tokens.basis,
        capabilities.max_output_tokens.basis,
        capabilities.native_reasoning.basis,
    ]
}

fn compile_credential(
    source: &ComputeManagementSourceV2,
) -> Result<ComputeManagementCredentialCompilationV2, ComputeManagementCompilationErrorV2> {
    match &source.provenance {
        ComputeManagementProvenanceV2::ConnectorOwned {
            connector_id,
            account_ref,
            ..
        } => {
            let validation = source
                .validation
                .as_ref()
                .ok_or(ComputeManagementCompilationErrorV2::AuthorizationMissing)?;
            Ok(ComputeManagementCredentialCompilationV2::ConnectorOwned {
                connector_id: connector_id.clone(),
                account_ref: account_ref.clone(),
                validation_ref: validation.validation_ref.clone(),
                validation_revision: validation.validation_revision,
            })
        }
        ComputeManagementProvenanceV2::Registered { .. }
        | ComputeManagementProvenanceV2::UserConfigured { .. } => {
            let ordered = match source.authentication {
                GatewayAuthenticationSemanticsV1::None => {
                    vec![ComputeCredentialSelectionV2::NoCredential]
                }
                GatewayAuthenticationSemanticsV1::Bearer
                | GatewayAuthenticationSemanticsV1::ApiKeyHeader { .. } => source
                    .enabled_credentials()
                    .map(|credential| ComputeCredentialSelectionV2::Credential {
                        credential_ref: credential.credential.clone(),
                    })
                    .collect(),
            };
            if ordered.is_empty()
                || ordered
                    .iter()
                    .any(|selection| selection.validate_for(&source.authentication).is_err())
            {
                return Err(ComputeManagementCompilationErrorV2::CredentialMissing);
            }
            Ok(ComputeManagementCredentialCompilationV2::Native { ordered })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ComputeManagementCompilationErrorV2 {
    #[error("saved compute source is invalid")]
    InvalidSource,
    #[error("saved compute source is not ready")]
    NotReady,
    #[error("saved compute source has unknown required capability facts")]
    UnknownCapability,
    #[error("saved compute source eligibility is inconsistent")]
    EligibilityMismatch,
    #[error("saved compute source has no valid credential selection")]
    CredentialMissing,
    #[error("saved connector source has no validated authorization")]
    AuthorizationMissing,
}
