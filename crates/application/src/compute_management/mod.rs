//! Trusted compute-management handoff contracts.
//!
//! Public, serializable views live in `hiroute-application-api`. The facts and protected input
//! bindings in this module cross only trusted in-process adapter boundaries and deliberately have
//! no wire representation.

use std::fmt;

use hiroute_application_api::{
    ComputeCandidateFactStateV2, ComputeCandidateInputStateV2, ComputeCandidateIssueV2,
    ComputeCandidateModelFactBasisV2, ComputeCandidateModelViewV2, ComputeCandidateProducerV2,
    ComputeCandidateProvenanceKindV2, ComputeCandidateRefV2, ComputeCandidateTargetV2,
    ComputeCandidateViewV2, ComputeCheckCorrelationV2, ComputeSavedSourceExpectationV2,
    ComputeValidationRefV2, OperationReferenceV1,
};
use hiroute_domain::{
    CanonicalDigest, ComputeNativeEndpointV3, GatewayAuthenticationSemanticsV1,
    NativeReasoningCapabilityV1, PortError, PortErrorCode, PortResult,
};
use serde::{Deserialize, Serialize};

mod candidates;
mod compilation;
mod dispatch;
mod mutation_support;
mod mutations;
mod query;

pub use candidates::TrustedComputeCandidateRegistry;
pub use compilation::*;
pub use mutations::*;
pub use query::*;

pub(crate) use dispatch::{dispatch_compute_management, is_compute_management_operation};

#[cfg(test)]
mod tests;

/// A protected-input source resolved only by Application's trusted adapter.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProtectedInputSourceDescriptorV1 {
    ManualInput,
    DiscoveredConfig {
        scanner_id: String,
        scanner_version: String,
        source_ref: String,
        field_selector: String,
        observed_revision: u64,
    },
}

/// Complete discovery/config/catalog evidence retained only with trusted in-process candidate
/// facts. It intentionally has no wire or durable representation.
#[derive(Clone, Eq, PartialEq)]
pub struct ComputeDiscoveryEvidenceGuardV1 {
    pub evidence_digest: CanonicalDigest,
}

impl fmt::Debug for ProtectedInputSourceDescriptorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ManualInput => formatter.write_str("ManualInput"),
            Self::DiscoveredConfig {
                scanner_id,
                scanner_version,
                observed_revision,
                ..
            } => formatter
                .debug_struct("DiscoveredConfig")
                .field("scanner_id", scanner_id)
                .field("scanner_version", scanner_version)
                .field("source_ref", &"<redacted>")
                .field("field_selector", &"<redacted>")
                .field("observed_revision", observed_revision)
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComputeCandidateFactBasisV2 {
    RegisteredCatalog,
    RuntimeFallback,
    Observed,
    UserDeclared,
    ConnectorVerified,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComputeCandidateFactValueV2<T> {
    pub value: Option<T>,
    pub basis: ComputeCandidateFactBasisV2,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComputeCandidateCapabilityFactsV2 {
    pub tool: ComputeCandidateFactValueV2<bool>,
    pub vision: ComputeCandidateFactValueV2<bool>,
    pub streaming: ComputeCandidateFactValueV2<bool>,
    pub context_tokens: ComputeCandidateFactValueV2<u64>,
    pub max_output_tokens: ComputeCandidateFactValueV2<u64>,
    pub native_reasoning: ComputeCandidateFactValueV2<NativeReasoningCapabilityV1>,
}

impl ComputeCandidateCapabilityFactsV2 {
    fn validate_shape(&self) -> bool {
        fact_value_has_valid_basis(&self.tool)
            && fact_value_has_valid_basis(&self.vision)
            && fact_value_has_valid_basis(&self.streaming)
            && fact_value_has_valid_basis(&self.context_tokens)
            && fact_value_has_valid_basis(&self.max_output_tokens)
            && fact_value_has_valid_basis(&self.native_reasoning)
            && self
                .native_reasoning
                .value
                .as_ref()
                .is_none_or(|capability| capability.validate().is_ok())
    }
}

fn fact_value_has_valid_basis<T>(fact: &ComputeCandidateFactValueV2<T>) -> bool {
    matches!(
        (&fact.value, fact.basis),
        (None, ComputeCandidateFactBasisV2::Unknown)
            | (
                Some(_),
                ComputeCandidateFactBasisV2::RegisteredCatalog
                    | ComputeCandidateFactBasisV2::RuntimeFallback
                    | ComputeCandidateFactBasisV2::Observed
                    | ComputeCandidateFactBasisV2::UserDeclared
                    | ComputeCandidateFactBasisV2::ConnectorVerified,
            )
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComputeCandidateModelFactsV2 {
    pub model_ref: String,
    pub upstream_model_id: String,
    pub display_name: String,
    pub catalog_configuration_id: Option<String>,
    pub membership: hiroute_application_api::ComputeModelMembershipV2,
    pub capabilities: ComputeCandidateCapabilityFactsV2,
    pub capability_evidence_digest: CanonicalDigest,
    pub selectable: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Eq, PartialEq)]
pub enum ComputeCandidateProvenanceV2 {
    Registered {
        connection_option_id: String,
        registry_version: String,
        catalog_digest: CanonicalDigest,
    },
    UserConfigured {
        configuration_revision: u64,
        evidence_digest: CanonicalDigest,
    },
    ConnectorOwnedPendingApproval {
        connector_id: String,
    },
    ConnectorOwned {
        connector_id: String,
        account_ref: String,
    },
}

impl ComputeCandidateProvenanceV2 {
    pub const fn kind(&self) -> ComputeCandidateProvenanceKindV2 {
        match self {
            Self::Registered { .. } => ComputeCandidateProvenanceKindV2::Registered,
            Self::UserConfigured { .. } => ComputeCandidateProvenanceKindV2::UserConfigured,
            Self::ConnectorOwnedPendingApproval { .. } | Self::ConnectorOwned { .. } => {
                ComputeCandidateProvenanceKindV2::ConnectorOwned
            }
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum ComputeCredentialBindingV2 {
    None,
    NativePendingInput,
    NativeProtected {
        descriptor: ProtectedInputSourceDescriptorV1,
        input_slot: String,
    },
    NativeSaved {
        credential_id: String,
        expected_generation: u64,
    },
    CpaPendingApproval {
        protected_source: ProtectedInputSourceDescriptorV1,
    },
    CpaOwned {
        account_ref: String,
        validation: ComputeValidationRefV2,
    },
}

impl fmt::Debug for ComputeCredentialBindingV2 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("None"),
            Self::NativePendingInput => formatter.write_str("NativePendingInput"),
            Self::NativeProtected { .. } => formatter
                .debug_struct("NativeProtected")
                .field("descriptor", &"<redacted>")
                .field("input_slot", &"<redacted>")
                .finish(),
            Self::NativeSaved {
                expected_generation,
                ..
            } => formatter
                .debug_struct("NativeSaved")
                .field("credential_id", &"<redacted>")
                .field("expected_generation", expected_generation)
                .finish(),
            Self::CpaPendingApproval { .. } => formatter
                .debug_struct("CpaPendingApproval")
                .field("protected_source", &"<redacted>")
                .finish(),
            Self::CpaOwned { validation, .. } => formatter
                .debug_struct("CpaOwned")
                .field("account_ref", &"<redacted>")
                .field("validation", validation)
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct ComputeCandidateFactsV2 {
    pub candidate: ComputeCandidateRefV2,
    pub correlation: ComputeCheckCorrelationV2,
    pub producer: ComputeCandidateProducerV2,
    pub lineage_ref: String,
    /// Exact saved-source lineage accepted only from a trusted producer refreshing an existing
    /// source. It has no public projection and prevents clients from reconstructing or guessing
    /// lineage input.
    pub trusted_lineage_digest: Option<CanonicalDigest>,
    pub display_name: String,
    pub existing_source_id: Option<String>,
    pub evidence_digest: CanonicalDigest,
    pub provenance: ComputeCandidateProvenanceV2,
    /// Absent only while a CPA candidate is awaiting its approved check.
    pub target: Option<ComputeCandidateTargetV2>,
    /// Absent only while a CPA candidate is awaiting its approved check.
    pub authentication: Option<GatewayAuthenticationSemanticsV1>,
    pub models: Vec<ComputeCandidateModelFactsV2>,
    /// Safe server-owned context to persist with a newly saved native source. Never projected in
    /// `ComputeCandidateViewV2` or the management snapshot.
    pub native_recheck: Option<hiroute_domain::ComputeNativeRecheckDescriptorV2>,
    pub additional_native_endpoints: Vec<ComputeNativeEndpointV3>,
    /// Present only for a candidate prepared from an exact discovered configuration. The public
    /// candidate projection and durable save spec cannot observe or reconstruct this guard.
    pub discovery_guard: Option<ComputeDiscoveryEvidenceGuardV1>,
    pub credential_binding: ComputeCredentialBindingV2,
    pub validation: Option<ComputeValidationRefV2>,
}

impl ComputeCandidateFactsV2 {
    pub fn validate_shape(&self) -> PortResult<()> {
        let invalid = invalid_candidate_facts;
        self.candidate.validate_shape().map_err(|_| invalid())?;
        self.correlation
            .validate_for(&self.candidate)
            .map_err(|_| invalid())?;
        if self.lineage_ref.trim().is_empty()
            || self.display_name.trim().is_empty()
            || self.evidence_digest == CanonicalDigest::of_bytes(&[])
            || self
                .validation
                .as_ref()
                .is_some_and(|validation| validation.validate_shape().is_err())
            || self.models.iter().any(|model| {
                model.model_ref.trim().is_empty()
                    || model.upstream_model_id.trim().is_empty()
                    || model.display_name.trim().is_empty()
                    || model
                        .catalog_configuration_id
                        .as_ref()
                        .is_some_and(|value| value.trim().is_empty())
                    || !model.capabilities.validate_shape()
                    || model.capability_evidence_digest == CanonicalDigest::of_bytes(&[])
            })
        {
            return Err(invalid());
        }
        if self
            .trusted_lineage_digest
            .as_ref()
            .is_some_and(|digest| digest == &CanonicalDigest::of_bytes(&[]))
            || self
                .native_recheck
                .as_ref()
                .is_some_and(|descriptor| descriptor.validate().is_err())
            || self.trusted_lineage_digest.is_some() && self.existing_source_id.is_none()
            || self.native_recheck.is_some() && self.producer != ComputeCandidateProducerV2::Native
            || !self.additional_native_endpoints.is_empty()
                && self.producer != ComputeCandidateProducerV2::Native
            || self
                .additional_native_endpoints
                .iter()
                .any(|endpoint| endpoint.validate().is_err())
        {
            return Err(invalid());
        }

        let valid_discovery_guard = match (&self.credential_binding, &self.discovery_guard) {
            (
                ComputeCredentialBindingV2::NativeProtected {
                    descriptor: ProtectedInputSourceDescriptorV1::DiscoveredConfig { .. },
                    ..
                },
                Some(guard),
            ) => guard.evidence_digest != CanonicalDigest::of_bytes(&[]),
            (
                ComputeCredentialBindingV2::NativeProtected {
                    descriptor: ProtectedInputSourceDescriptorV1::ManualInput,
                    ..
                },
                None,
            ) => true,
            (ComputeCredentialBindingV2::NativeProtected { .. }, _) => false,
            (_, None) => true,
            (_, Some(_)) => false,
        };
        if !valid_discovery_guard {
            return Err(invalid());
        }

        match self.producer {
            ComputeCandidateProducerV2::Native => self.validate_native_shape(),
            ComputeCandidateProducerV2::Cpa => self.validate_cpa_shape(),
        }
    }

    /// Projects trusted facts to the only candidate shape exposed to ordinary clients.
    pub fn public_view(
        &self,
        input_state: ComputeCandidateInputStateV2,
        issues: Vec<ComputeCandidateIssueV2>,
    ) -> PortResult<ComputeCandidateViewV2> {
        self.validate_shape()?;
        if !self.input_state_matches_facts(input_state) {
            return Err(invalid_candidate_facts());
        }
        Ok(ComputeCandidateViewV2 {
            candidate: self.candidate.clone(),
            correlation: self.correlation.clone(),
            producer: self.producer,
            provenance: self.provenance.kind(),
            display_name: self.display_name.clone(),
            existing_source_id: self.existing_source_id.clone(),
            models: self
                .models
                .iter()
                .map(|model| ComputeCandidateModelViewV2 {
                    model_ref: model.model_ref.clone(),
                    upstream_model_id: model.upstream_model_id.clone(),
                    display_name: model.display_name.clone(),
                    membership: model.membership,
                    fact_basis: public_model_fact_basis(&model.capabilities),
                    selectable: model.selectable,
                    reason: model.reason.clone(),
                })
                .collect(),
            input_state,
            fact_state: self.fact_state(),
            validation: self.validation.clone(),
            issues,
        })
    }

    fn validate_native_shape(&self) -> PortResult<()> {
        let mut protocols = std::collections::BTreeSet::new();
        if let Some(target) = &self.target {
            protocols.insert(target.upstream_protocol);
        }
        if self.additional_native_endpoints.iter().any(|endpoint| {
            !protocols.insert(endpoint.target.upstream_protocol)
                || self.authentication.as_ref().is_none_or(|authentication| {
                    (*authentication == GatewayAuthenticationSemanticsV1::None)
                        != (endpoint.authentication == GatewayAuthenticationSemanticsV1::None)
                })
        }) {
            return Err(invalid_candidate_facts());
        }
        let valid_provenance = match &self.provenance {
            ComputeCandidateProvenanceV2::Registered {
                connection_option_id,
                registry_version,
                catalog_digest,
            } => {
                !connection_option_id.trim().is_empty()
                    && !registry_version.trim().is_empty()
                    && catalog_digest != &CanonicalDigest::of_bytes(&[])
            }
            ComputeCandidateProvenanceV2::UserConfigured {
                configuration_revision,
                evidence_digest,
            } => *configuration_revision > 0 && evidence_digest != &CanonicalDigest::of_bytes(&[]),
            ComputeCandidateProvenanceV2::ConnectorOwnedPendingApproval { .. }
            | ComputeCandidateProvenanceV2::ConnectorOwned { .. } => false,
        };
        let valid_target = self.target.as_ref().is_some_and(valid_candidate_target);
        let valid_binding = match (&self.authentication, &self.credential_binding) {
            (Some(GatewayAuthenticationSemanticsV1::None), ComputeCredentialBindingV2::None) => {
                true
            }
            (
                Some(
                    GatewayAuthenticationSemanticsV1::Bearer
                    | GatewayAuthenticationSemanticsV1::ApiKeyHeader { .. },
                ),
                ComputeCredentialBindingV2::NativePendingInput,
            ) => true,
            (
                Some(
                    GatewayAuthenticationSemanticsV1::Bearer
                    | GatewayAuthenticationSemanticsV1::ApiKeyHeader { .. },
                ),
                ComputeCredentialBindingV2::NativeProtected { input_slot, .. },
            ) => !input_slot.trim().is_empty(),
            (
                Some(
                    GatewayAuthenticationSemanticsV1::Bearer
                    | GatewayAuthenticationSemanticsV1::ApiKeyHeader { .. },
                ),
                ComputeCredentialBindingV2::NativeSaved {
                    credential_id,
                    expected_generation,
                },
            ) => !credential_id.trim().is_empty() && *expected_generation > 0,
            _ => false,
        };
        if valid_provenance
            && valid_target
            && self
                .authentication
                .as_ref()
                .is_some_and(valid_authentication)
            && self.validation.is_none()
            && valid_binding
        {
            Ok(())
        } else {
            Err(invalid_candidate_facts())
        }
    }

    fn validate_cpa_shape(&self) -> PortResult<()> {
        let valid = match (
            &self.provenance,
            &self.target,
            &self.authentication,
            &self.credential_binding,
            &self.validation,
        ) {
            (
                ComputeCandidateProvenanceV2::ConnectorOwnedPendingApproval { connector_id },
                None,
                None,
                ComputeCredentialBindingV2::CpaPendingApproval { .. },
                None,
            ) => !connector_id.trim().is_empty() && self.models.is_empty(),
            (
                ComputeCandidateProvenanceV2::ConnectorOwned {
                    connector_id,
                    account_ref: provenance_account,
                },
                Some(target),
                Some(authentication),
                ComputeCredentialBindingV2::CpaOwned {
                    account_ref: binding_account,
                    validation: binding_validation,
                },
                Some(validation),
            ) => {
                !connector_id.trim().is_empty()
                    && !provenance_account.trim().is_empty()
                    && provenance_account == binding_account
                    && binding_validation == validation
                    && valid_candidate_target(target)
                    && valid_cpa_authentication(authentication)
            }
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(invalid_candidate_facts())
        }
    }

    fn fact_state(&self) -> ComputeCandidateFactStateV2 {
        match &self.credential_binding {
            ComputeCredentialBindingV2::NativePendingInput => {
                ComputeCandidateFactStateV2::PendingCredential
            }
            ComputeCredentialBindingV2::CpaPendingApproval { .. } => {
                ComputeCandidateFactStateV2::PendingApproval
            }
            _ => ComputeCandidateFactStateV2::Complete,
        }
    }

    pub fn inferred_input_state(&self) -> ComputeCandidateInputStateV2 {
        match &self.credential_binding {
            ComputeCredentialBindingV2::NativePendingInput => ComputeCandidateInputStateV2::Missing,
            ComputeCredentialBindingV2::NativeProtected { .. }
            | ComputeCredentialBindingV2::NativeSaved { .. } => {
                ComputeCandidateInputStateV2::Provided
            }
            ComputeCredentialBindingV2::None
            | ComputeCredentialBindingV2::CpaPendingApproval { .. }
            | ComputeCredentialBindingV2::CpaOwned { .. } => {
                ComputeCandidateInputStateV2::NotRequired
            }
        }
    }

    fn input_state_matches_facts(&self, state: ComputeCandidateInputStateV2) -> bool {
        match &self.credential_binding {
            ComputeCredentialBindingV2::NativePendingInput => matches!(
                state,
                ComputeCandidateInputStateV2::Missing | ComputeCandidateInputStateV2::Unavailable
            ),
            ComputeCredentialBindingV2::NativeProtected { .. }
            | ComputeCredentialBindingV2::NativeSaved { .. } => matches!(
                state,
                ComputeCandidateInputStateV2::Provided | ComputeCandidateInputStateV2::Unavailable
            ),
            ComputeCredentialBindingV2::None
            | ComputeCredentialBindingV2::CpaPendingApproval { .. }
            | ComputeCredentialBindingV2::CpaOwned { .. } => {
                state == ComputeCandidateInputStateV2::NotRequired
            }
        }
    }
}

fn public_model_fact_basis(
    capabilities: &ComputeCandidateCapabilityFactsV2,
) -> ComputeCandidateModelFactBasisV2 {
    let bases = [
        capabilities.tool.basis,
        capabilities.vision.basis,
        capabilities.streaming.basis,
        capabilities.context_tokens.basis,
        capabilities.max_output_tokens.basis,
        capabilities.native_reasoning.basis,
    ];
    if !bases.iter().all(|basis| basis == &bases[0]) {
        return ComputeCandidateModelFactBasisV2::Unknown;
    }
    match bases[0] {
        ComputeCandidateFactBasisV2::RegisteredCatalog => {
            ComputeCandidateModelFactBasisV2::RegisteredCatalog
        }
        ComputeCandidateFactBasisV2::RuntimeFallback => {
            ComputeCandidateModelFactBasisV2::RuntimeFallback
        }
        ComputeCandidateFactBasisV2::Observed => ComputeCandidateModelFactBasisV2::Observed,
        ComputeCandidateFactBasisV2::UserDeclared => ComputeCandidateModelFactBasisV2::UserDeclared,
        ComputeCandidateFactBasisV2::ConnectorVerified => {
            ComputeCandidateModelFactBasisV2::ConnectorVerified
        }
        ComputeCandidateFactBasisV2::Unknown => ComputeCandidateModelFactBasisV2::Unknown,
    }
}

fn invalid_candidate_facts() -> PortError {
    PortError::new(
        PortErrorCode::InvalidData,
        "compute-management-candidate-facts",
    )
}

fn valid_candidate_target(target: &ComputeCandidateTargetV2) -> bool {
    !target.scheme.trim().is_empty()
        && !target.authority.trim().is_empty()
        && target.port != 0
        && target.request_path.starts_with('/')
        && !target.protocol_profile_id.trim().is_empty()
        && target.protocol_profile_revision > 0
}

fn valid_authentication(authentication: &GatewayAuthenticationSemanticsV1) -> bool {
    !matches!(
        authentication,
        GatewayAuthenticationSemanticsV1::ApiKeyHeader { header } if header.trim().is_empty()
    )
}

fn valid_cpa_authentication(authentication: &GatewayAuthenticationSemanticsV1) -> bool {
    matches!(
        authentication,
        GatewayAuthenticationSemanticsV1::Bearer
            | GatewayAuthenticationSemanticsV1::ApiKeyHeader { .. }
    ) && valid_authentication(authentication)
}

/// Application-owned candidate registry entrypoint. Implementations receive trusted in-process
/// facts; a Local Control request cannot construct this value. Implementations must call
/// [`ComputeCandidateFactsV2::validate_shape`] before retaining or projecting a candidate.
pub trait ComputeCandidatePort: Send + Sync {
    fn register_compute_candidate(
        &self,
        facts: ComputeCandidateFactsV2,
    ) -> PortResult<ComputeCandidateViewV2>;

    fn get_compute_candidate(
        &self,
        candidate: &ComputeCandidateRefV2,
    ) -> PortResult<ComputeCandidateViewV2>;

    /// Trusted lookup used only by the Application planner. Implementations resolve only the
    /// current revision for a candidate reference. Unlike the public view, this may contain a
    /// protected logical input slot and therefore has no wire representation.
    fn resolve_compute_candidate(
        &self,
        candidate: &ComputeCandidateRefV2,
    ) -> PortResult<ComputeCandidateFactsV2>;
}

#[derive(Clone, Eq, PartialEq)]
pub struct ComputeApprovedSubscriptionCheckV2 {
    pub approval_operation: OperationReferenceV1,
    pub candidate: ComputeCandidateRefV2,
    pub expected_evidence_digest: CanonicalDigest,
    pub existing_source: Option<ComputeSavedSourceExpectationV2>,
    pub protected_source: ProtectedInputSourceDescriptorV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComputeSubscriptionResourceOwnerV2 {
    ApprovalOperation { operation: OperationReferenceV1 },
    SavedSource(ComputeSavedSourceExpectationV2),
}

/// Opaque reference to the existing effect/recovery record that owns one exact CPA resource.
#[derive(Clone, Eq, PartialEq)]
pub struct ComputeSubscriptionResourceReceiptV2 {
    receipt_ref: String,
    revision: u64,
}

impl fmt::Debug for ComputeSubscriptionResourceReceiptV2 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ComputeSubscriptionResourceReceiptV2")
            .field("receipt_ref", &"<redacted>")
            .field("revision", &self.revision)
            .finish()
    }
}

impl ComputeSubscriptionResourceReceiptV2 {
    pub fn new(receipt_ref: impl Into<String>, revision: u64) -> PortResult<Self> {
        let receipt = Self {
            receipt_ref: receipt_ref.into(),
            revision,
        };
        if receipt.receipt_ref.trim().is_empty() || receipt.revision == 0 {
            return Err(PortError::new(
                PortErrorCode::InvalidData,
                "compute-subscription-resource-receipt",
            ));
        }
        Ok(receipt)
    }

    pub fn receipt_ref(&self) -> &str {
        &self.receipt_ref
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

#[derive(Clone, PartialEq)]
pub struct ComputeSubscriptionValidationFactsV2 {
    pub validation: ComputeValidationRefV2,
    pub original_candidate: ComputeCandidateRefV2,
    pub verified_evidence_digest: CanonicalDigest,
    pub account_ref: String,
    pub inventory_revision: u64,
    pub inventory: Vec<ComputeCandidateModelFactsV2>,
    pub resource_owner: ComputeSubscriptionResourceOwnerV2,
    pub resource_receipt: ComputeSubscriptionResourceReceiptV2,
}

/// Trusted CPA effect boundary. The adapter performs network/process work outside Application's
/// short control transaction and returns only version-bound facts plus an existing effect receipt.
pub trait ComputeSubscriptionMaterializationPort: Send + Sync {
    fn materialize_selected(
        &self,
        input: ComputeApprovedSubscriptionCheckV2,
    ) -> PortResult<ComputeSubscriptionValidationFactsV2>;
}
