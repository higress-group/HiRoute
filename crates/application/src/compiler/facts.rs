use std::collections::BTreeSet;

use hiroute_domain::{
    AGENT_PLAN_COMPILER_REVISION_V1, AGENT_PLAN_FACTS_SCHEMA_V1, AgentPlanFactRefsV1, BillingClass,
    CanonicalDigest, ConnectorRuntimeKind, FreeAccess, GatewayCandidateProtocolProfileV1,
    GatewayOperationalTargetV1, MaterializationState, ModelDefinitionV1, ModelEndpointCapabilityV1,
    NativeReasoningCapabilityV1, ProtocolEndpointV1, RatingV1, SourceBindingV1,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OrderingPriceFactV1 {
    pub price_rate_id: String,
    pub price_rate_revision: u64,
    pub offer_ref: String,
    pub model_configuration_id: String,
    pub currency: String,
    pub input_micros_per_million: u64,
    pub output_micros_per_million: u64,
    pub frozen_digest: CanonicalDigest,
}

impl OrderingPriceFactV1 {
    pub fn total_micros_per_million(&self) -> Result<u64, CompilerFactError> {
        self.input_micros_per_million
            .checked_add(self.output_micros_per_million)
            .ok_or(CompilerFactError::InvalidPrice)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FreeCandidateEvidenceV1 {
    pub free_offer_id: String,
    pub free_offer_revision: u64,
    pub offer_ref: String,
    pub access: FreeAccess,
    pub evidence_digest: CanonicalDigest,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateFactAuthorityV1 {
    #[default]
    RegisteredCatalog,
    RuntimeFallback,
    SourceLocalUser,
}

impl CandidateFactAuthorityV1 {
    fn is_registered_catalog(&self) -> bool {
        matches!(self, Self::RegisteredCatalog)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateCompilationFactV1 {
    #[serde(
        default,
        skip_serializing_if = "CandidateFactAuthorityV1::is_registered_catalog"
    )]
    pub authority: CandidateFactAuthorityV1,
    pub connection_option_id: String,
    pub offer_revision: u64,
    pub binding: SourceBindingV1,
    pub model: ModelDefinitionV1,
    pub capability: ModelEndpointCapabilityV1,
    /// Exact trusted Registry record; compiler validates it against the capability before deriving
    /// the executable destination.
    pub protocol_endpoint: ProtocolEndpointV1,
    pub connector_runtime: ConnectorRuntimeKind,
    pub operational_target: GatewayOperationalTargetV1,
    /// Exact model written to the operational connector request. For managed
    /// CPA this is the prepared account-scoped transport alias, not the
    /// Registry `upstream_model_id`.
    pub native_transport_model: String,
    pub protocol_profiles: Vec<GatewayCandidateProtocolProfileV1>,
    /// Frozen pool order of stable, non-secret CredentialResolver selectors. Each Gateway
    /// Attempt selects exactly one entry; this is never credential material.
    pub credential_refs: Vec<String>,
    /// Management credentials retain their exact target binding independently of catalog identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_destination_ref: Option<String>,
    pub source_state: MaterializationState,
    pub inventory_model_matched: bool,
    pub reasoning: NativeReasoningCapabilityV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<RatingV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordering_price: Option<OrderingPriceFactV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free_evidence: Option<FreeCandidateEvidenceV1>,
}

impl CandidateCompilationFactV1 {
    pub fn validate(&self) -> Result<(), CompilerFactError> {
        self.binding
            .validate_shape()
            .map_err(|_| CompilerFactError::CrossReference)?;
        self.reasoning
            .validate()
            .map_err(|_| CompilerFactError::InvalidReasoning)?;
        if self.authority == CandidateFactAuthorityV1::SourceLocalUser {
            return self.validate_source_local_user();
        }
        if !valid_reference(&self.connection_option_id)
            || self.offer_revision == 0
            || self.model.revision == 0
            || !valid_reference(&self.model.publisher_id)
            || self.model.display_name.trim() != self.model.display_name
            || !(1..=128).contains(&self.model.display_name.chars().count())
            || self.model.display_name.chars().any(char::is_control)
            || self.model.capabilities.context_tokens == 0
            || self.model.capabilities.max_output_tokens == 0
            || self.capability.revision == 0
            || self.capability.connector_revision == 0
            || self.capability.endpoint_profile_revision == 0
            || self.capability.required_adapter_revision == 0
            || !valid_reference(&self.capability.connector_id)
            || !valid_reference(&self.capability.endpoint_profile_id)
            || !valid_reference(&self.capability.protocol_endpoint_id)
            || !valid_reference(&self.capability.required_adapter_ref)
            || !valid_credential_refs(&self.credential_refs, self.is_routable())
            || !self.valid_registered_credential_destination()
            || self.protocol_endpoint.protocol_endpoint_id != self.capability.protocol_endpoint_id
            || self.protocol_endpoint.protocol != self.capability.upstream_protocol
            || self.protocol_endpoint.adapter_ref != self.capability.required_adapter_ref
            || self.protocol_endpoint.adapter_revision != self.capability.required_adapter_revision
            || !valid_protocol_endpoint(&self.protocol_endpoint)
            || !self.operational_target.validate_for(
                self.connector_runtime,
                &format!(
                    "{}{}",
                    self.protocol_endpoint.base_url, self.protocol_endpoint.request_path
                ),
            )
            || !hiroute_domain::valid_upstream_model_id(&self.native_transport_model)
            || (self.connector_runtime == ConnectorRuntimeKind::BuiltinNative
                && self.native_transport_model != self.binding.upstream_model_id)
            || self.protocol_profiles.is_empty()
            || (self.connector_runtime == ConnectorRuntimeKind::CpaBridge
                && self.protocol_profiles.first().is_none_or(|profile| {
                    self.operational_target.request_path()
                        != Some(profile.connector.request_path.as_str())
                }))
            || invalid_digest(&self.binding.source_identity_digest)
            || invalid_digest(&self.capability.evidence_digest)
            || self.binding.model_configuration_id != self.model.model_configuration_id
            || self.binding.capability_id != self.capability.capability_id
            || self.binding.model_configuration_id != self.capability.model_configuration_id
            || self.binding.upstream_model_id != self.capability.upstream_model_id
            || self.binding.offer_ref
                != self
                    .ordering_price
                    .as_ref()
                    .map_or(self.binding.offer_ref.as_str(), |price| {
                        price.offer_ref.as_str()
                    })
        {
            return Err(CompilerFactError::CrossReference);
        }
        if let Some(rating) = &self.rating
            && (rating.model_configuration_id != self.model.model_configuration_id
                || !(5..=50).contains(&rating.overall_score_tenths))
        {
            return Err(CompilerFactError::InvalidRating);
        }
        if let Some(price) = &self.ordering_price {
            if !valid_reference(&price.price_rate_id)
                || price.price_rate_revision == 0
                || price.offer_ref != self.binding.offer_ref
                || price.model_configuration_id != self.model.model_configuration_id
                || price.currency.len() != 3
                || !price.currency.bytes().all(|byte| byte.is_ascii_uppercase())
                || invalid_digest(&price.frozen_digest)
            {
                return Err(CompilerFactError::InvalidPrice);
            }
            price.total_micros_per_million()?;
        }
        match (&self.free_evidence, self.binding.billing_class) {
            (Some(free), BillingClass::Free) => {
                if !valid_reference(&free.free_offer_id)
                    || free.free_offer_revision == 0
                    || free.offer_ref != self.binding.offer_ref
                    || invalid_digest(&free.evidence_digest)
                {
                    return Err(CompilerFactError::InvalidFreeEvidence);
                }
            }
            (Some(_), _) => return Err(CompilerFactError::InvalidFreeEvidence),
            (None, _) => {}
        }
        if self.authority == CandidateFactAuthorityV1::RuntimeFallback
            && ((!self.inventory_model_matched
                && self.connector_runtime != ConnectorRuntimeKind::BuiltinNative)
                || self.rating.is_some()
                || self.ordering_price.is_some()
                || self.free_evidence.is_some())
        {
            return Err(CompilerFactError::CrossReference);
        }
        Ok(())
    }

    fn valid_registered_credential_destination(&self) -> bool {
        let Some(destination) = self.credential_destination_ref.as_ref() else {
            return true;
        };
        if self.connector_runtime != ConnectorRuntimeKind::BuiltinNative
            || !valid_protocol_endpoint(&self.protocol_endpoint)
        {
            return false;
        }
        let target = hiroute_domain::ComputeManagementTargetV2 {
            scheme: "https".into(),
            authority: self.protocol_endpoint.base_url["https://".len()..].into(),
            port: 443,
            request_path: self.protocol_endpoint.request_path.clone(),
            upstream_protocol: self.protocol_endpoint.protocol,
            protocol_profile_id: self.protocol_endpoint.adapter_ref.clone(),
            protocol_profile_revision: self.protocol_endpoint.adapter_revision,
        };
        target
            .credential_destination()
            .is_ok_and(|expected| &expected == destination)
    }

    fn validate_source_local_user(&self) -> Result<(), CompilerFactError> {
        if !valid_reference(&self.connection_option_id)
            || self.offer_revision == 0
            || self.model.revision == 0
            || !valid_reference(&self.model.publisher_id)
            || self.model.display_name.trim() != self.model.display_name
            || !(1..=128).contains(&self.model.display_name.chars().count())
            || self.model.display_name.chars().any(char::is_control)
            || self.model.capabilities.context_tokens == 0
            || self.model.capabilities.max_output_tokens == 0
            || self.capability.revision == 0
            || self.capability.connector_revision == 0
            || self.capability.endpoint_profile_revision == 0
            || self.capability.required_adapter_revision == 0
            || !valid_reference(&self.capability.connector_id)
            || !valid_reference(&self.capability.endpoint_profile_id)
            || !valid_reference(&self.capability.protocol_endpoint_id)
            || !valid_reference(&self.capability.required_adapter_ref)
            || !valid_credential_refs(&self.credential_refs, self.is_routable())
            || self
                .credential_destination_ref
                .as_deref()
                .is_none_or(|value| {
                    !value.starts_with("compute-target/") || !valid_reference(value)
                })
            || self.protocol_endpoint.protocol_endpoint_id != self.capability.protocol_endpoint_id
            || self.protocol_endpoint.protocol != self.capability.upstream_protocol
            || self.protocol_endpoint.adapter_ref != self.capability.required_adapter_ref
            || self.protocol_endpoint.adapter_revision != self.capability.required_adapter_revision
            || !matches!(
                self.operational_target,
                GatewayOperationalTargetV1::UserConfiguredNative { .. }
            )
            || !self.operational_target.validate_for(
                self.connector_runtime,
                &format!(
                    "{}{}",
                    self.protocol_endpoint.base_url, self.protocol_endpoint.request_path
                ),
            )
            || !hiroute_domain::valid_upstream_model_id(&self.native_transport_model)
            || self.connector_runtime != ConnectorRuntimeKind::BuiltinNative
            || self.native_transport_model != self.binding.upstream_model_id
            || self.protocol_profiles.is_empty()
            || invalid_digest(&self.binding.source_identity_digest)
            || invalid_digest(&self.capability.evidence_digest)
            || self.binding.source_id != self.connection_option_id
            || self.binding.model_configuration_id != self.model.model_configuration_id
            || self.binding.capability_id != self.capability.capability_id
            || self.binding.model_configuration_id != self.capability.model_configuration_id
            || self.binding.upstream_model_id != self.capability.upstream_model_id
            || self.binding.billing_class != BillingClass::Paid
            || self.inventory_model_matched
            || self.rating.is_some()
            || self.ordering_price.is_some()
            || self.free_evidence.is_some()
        {
            return Err(CompilerFactError::CrossReference);
        }
        Ok(())
    }

    pub fn is_routable(&self) -> bool {
        self.source_state == MaterializationState::Ready
            && match self.authority {
                CandidateFactAuthorityV1::RegisteredCatalog
                | CandidateFactAuthorityV1::RuntimeFallback => {
                    self.connector_runtime == ConnectorRuntimeKind::BuiltinNative
                        || self.inventory_model_matched
                }
                CandidateFactAuthorityV1::SourceLocalUser => !self.inventory_model_matched,
            }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPlanCompilationFactsV1 {
    pub schema: String,
    pub compiler_revision: String,
    pub candidate_scope: CandidateFactScope,
    pub refs: AgentPlanFactRefsV1,
    pub ordering_price_version: String,
    pub ordering_price_digest: CanonicalDigest,
    pub candidates: Vec<CandidateCompilationFactV1>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateFactScope {
    /// Complete local snapshot required for `automatic_all_available`; explicit strategies may
    /// select a subset, but never from a different or partially loaded fact generation.
    AllMaterializedBindings,
}

impl AgentPlanCompilationFactsV1 {
    pub fn validate(&self) -> Result<(), CompilerFactError> {
        if self.candidates.is_empty() {
            return Err(CompilerFactError::InvalidCandidateCount);
        }
        self.validate_snapshot()
    }

    /// Read-only options may describe no candidates; compilation must call `validate`.
    pub fn validate_snapshot(&self) -> Result<(), CompilerFactError> {
        if self.schema != AGENT_PLAN_FACTS_SCHEMA_V1 {
            return Err(CompilerFactError::UnsupportedSchema);
        }
        if self.compiler_revision != AGENT_PLAN_COMPILER_REVISION_V1 {
            return Err(CompilerFactError::UnsupportedCompilerRevision);
        }
        self.refs
            .validate_snapshot()
            .map_err(|_| CompilerFactError::InvalidFactReferences)?;
        if !valid_reference(&self.ordering_price_version)
            || invalid_digest(&self.ordering_price_digest)
        {
            return Err(CompilerFactError::InvalidFactReferences);
        }
        if self.candidates.len() > 512
            || (!self.candidates.is_empty() && self.refs.inventory_revision == 0)
        {
            return Err(CompilerFactError::InvalidCandidateCount);
        }
        let mut binding_ids = BTreeSet::new();
        for candidate in &self.candidates {
            candidate.validate()?;
            if candidate.authority == CandidateFactAuthorityV1::RegisteredCatalog
                && (candidate.binding.model_data_bundle_version
                    != self.refs.model_data_bundle_version
                    || candidate.binding.capability_slice_version
                        != self.refs.capability_slice_version)
            {
                return Err(CompilerFactError::CrossReference);
            }
            if !binding_ids.insert(&candidate.binding.binding_id) {
                return Err(CompilerFactError::DuplicateBinding);
            }
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<CanonicalDigest, CompilerFactError> {
        self.validate()?;
        let mut canonical = self.clone();
        canonical
            .candidates
            .sort_by(|left, right| left.binding.binding_id.cmp(&right.binding.binding_id));
        CanonicalDigest::of(&canonical).map_err(|_| CompilerFactError::Encoding)
    }
}

fn valid_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains("//")
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
        })
}

fn valid_credential_refs(values: &[String], required: bool) -> bool {
    (!required || !values.is_empty())
        && values.len() <= 64
        && values.iter().all(|value| valid_reference(value))
        && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}

fn valid_protocol_endpoint(endpoint: &ProtocolEndpointV1) -> bool {
    let Some(host) = endpoint.base_url.strip_prefix("https://") else {
        return false;
    };
    !host.is_empty()
        && host.len() <= 253
        && !host.ends_with('.')
        && !host.contains([':', '@'])
        && !host.contains("..")
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
        && valid_endpoint_path(&endpoint.request_path)
        && endpoint
            .inventory_path
            .as_deref()
            .is_none_or(valid_endpoint_path)
}

fn valid_endpoint_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains(['?', '#'])
        && !path.contains("..")
        && path.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~')
        })
}

fn invalid_digest(value: &CanonicalDigest) -> bool {
    value == &CanonicalDigest::of_bytes(&[])
        || !matches!(CanonicalDigest::parse(value.as_str()), Ok(parsed) if &parsed == value)
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CompilerFactError {
    #[error("AgentPlan fact schema is unsupported")]
    UnsupportedSchema,
    #[error("AgentPlan compiler revision is unsupported")]
    UnsupportedCompilerRevision,
    #[error("AgentPlan fact references are invalid")]
    InvalidFactReferences,
    #[error("AgentPlan compiler fact candidate count is invalid")]
    InvalidCandidateCount,
    #[error("AgentPlan compiler facts contain a duplicate Binding")]
    DuplicateBinding,
    #[error("AgentPlan compiler fact cross-reference is not closed")]
    CrossReference,
    #[error("AgentPlan compiler fact rating is invalid")]
    InvalidRating,
    #[error("AgentPlan compiler fact price is invalid")]
    InvalidPrice,
    #[error("AgentPlan compiler fact free-offer evidence is invalid")]
    InvalidFreeEvidence,
    #[error("AgentPlan compiler fact reasoning capability is invalid")]
    InvalidReasoning,
    #[error("AgentPlan compiler facts cannot be encoded")]
    Encoding,
}
