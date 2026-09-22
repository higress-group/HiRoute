use hiroute_domain::{
    AgentPlanDisplayName, CanonicalDigest, ConnectorRuntimeKind, GatewayCandidatePricingIdentityV1,
    GatewayCandidateProtocolProfileV1, GatewayOperationalTargetV1,
};
use hiroute_gateway_core::core::publication::SUPPORTED_COMPILER_VERSION;
use http::Uri;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use thiserror::Error;

use crate::server::core_runtime::model_ir::{RequestCapabilityRequirementsV1, ToolChoice};
use crate::server::core_runtime::profiles::CandidateProtocolProfile;
use crate::server::request_plan::IngressProtocol;

/// V1 is the frozen PROCESS-22002 Oracle envelope. Production request
/// authority adds executable budgets and grants, so it has a distinct shape.
pub const GATEWAY_PUBLICATION_SCHEMA: &str = "hiroute.gateway.publication-snapshot/v3";
/// Product-contract P0 cap. The logical request deadline is always finite and
/// starts before the bounded model selector reads its first body byte.
pub const MAX_LOGICAL_REQUEST_DURATION_MS: u64 = 60 * 60 * 1_000;
/// Product-contract P0 default and hard ceiling for one logical request.
pub const MAX_REQUEST_ATTEMPTS: u32 = 6;
const MAX_ALIAS_BYTES: usize = 128;
const MAX_PURPOSE_BYTES: usize = 1_024;

/// One compiler-sealed attempt binding. It keeps client-bundled logical identity and
/// exact operational transport facts together; Provider credential material
/// is forbidden.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateBindingV1 {
    pub local_id: u32,
    pub stable_target_key: String,
    pub adapter_id: String,
    pub credential_refs: Vec<String>,
    pub credential_destination_ref: String,
    pub upstream_model_id: String,
    pub native_transport_model: String,
    /// Current Registry identity. It is never used as a CPA transport target,
    /// and its Provider path may differ from the bridge-local request path.
    pub endpoint: String,
    pub connector_runtime: ConnectorRuntimeKind,
    pub operational_target: GatewayOperationalTargetV1,
    pub operational_target_digest: CanonicalDigest,
    pub protocol_profiles: Vec<GatewayCandidateProtocolProfileV1>,
    pub protocol_profile_digest: CanonicalDigest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_identity: Option<GatewayCandidatePricingIdentityV1>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AliasGroupIdV1 {
    Economy,
    Primary,
    Free,
    Custom,
}

impl AliasGroupIdV1 {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Economy => "economy",
            Self::Primary => "primary",
            Self::Free => "free",
            Self::Custom => "custom",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AliasCostPolicyV1 {
    ApiEquivalent,
    StrictFree,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AliasComplexityClassifierV1 {
    pub revision: String,
    pub mode: hiroute_domain::ComplexityClassifierModeV1,
    #[serde(default)]
    pub user_keywords: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "strategy", rename_all = "snake_case", deny_unknown_fields)]
pub enum AliasRequestOwnedRouteV1 {
    Classified {
        classifier: AliasComplexityClassifierV1,
        simple_groups: Vec<AliasGroupIdV1>,
        complex_groups: Vec<AliasGroupIdV1>,
    },
    Ordered {
        cost_policy: AliasCostPolicyV1,
        ordered_groups: Vec<AliasGroupIdV1>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AliasModelGroupV1 {
    pub group_id: AliasGroupIdV1,
    pub candidate_local_ids: Vec<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AliasRoutingV1 {
    pub agent_plan_id: String,
    /// Safe Plan display text frozen by the product publication. Absent on legacy snapshots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_display_name: Option<String>,
    pub request_owned: AliasRequestOwnedRouteV1,
    pub groups: Vec<AliasModelGroupV1>,
}

#[cfg(test)]
pub(crate) fn exact_test_candidate(
    local_id: u32,
    ingress: IngressProtocol,
    no_connect_sentinel: Option<std::net::SocketAddr>,
) -> CandidateBindingV1 {
    use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};

    if let Some(address) = no_connect_sentinel {
        assert!(
            address.ip().is_loopback(),
            "dispatch-only test sentinels must remain loopback"
        );
    }
    let upstream = ingress;
    let upstream_model_id = format!("native-model-{local_id}");
    let native_transport_model = upstream_model_id.clone();
    let mut profile = CandidateProtocolProfile::exact_portable_path(
        ingress,
        upstream,
        native_transport_model.clone(),
        fixed_reasoning("fixed"),
    );
    profile.connector.connector_id = "builtin-openai".into();
    profile.connector.connector_revision = "1".into();
    let canonical: GatewayCandidateProtocolProfileV1 =
        serde_json::from_value(serde_json::to_value(profile).expect("test profile serializes"))
            .expect("gateway and product profile DTOs remain exact");
    let logical_endpoint = format!("https://provider-{local_id}.invalid{}", ingress.path());
    let connector_runtime = ConnectorRuntimeKind::BuiltinNative;
    let operational_target = GatewayOperationalTargetV1::RegisteredHttps {
        uri: logical_endpoint.clone(),
    };
    let protocol_profiles = vec![canonical];
    CandidateBindingV1 {
        local_id,
        stable_target_key: format!("target-{local_id}"),
        adapter_id: "builtin".into(),
        credential_refs: vec![format!("credential-{local_id}")],
        credential_destination_ref: "connection-option/fixture.native.v1".into(),
        upstream_model_id,
        native_transport_model,
        endpoint: logical_endpoint,
        connector_runtime,
        operational_target_digest: CanonicalDigest::of(&operational_target).unwrap(),
        operational_target,
        protocol_profile_digest: CanonicalDigest::of(&protocol_profiles).unwrap(),
        protocol_profiles,
        pricing_identity: None,
    }
}

#[cfg(test)]
pub(crate) fn exact_cpa_test_candidate(
    local_id: u32,
    ingress: IngressProtocol,
    logical_endpoint: &str,
    loopback: std::net::SocketAddr,
) -> CandidateBindingV1 {
    use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};

    let upstream_model_id = format!("native-model-{local_id}");
    let native_transport_model = format!("hiroute-fixture/{upstream_model_id}");
    let mut profile = CandidateProtocolProfile::exact_portable_path(
        ingress,
        ingress,
        native_transport_model.clone(),
        fixed_reasoning("fixed"),
    );
    profile.connector.connector_id = "connector.cpa.fixture".into();
    profile.connector.connector_revision = "1".into();
    let canonical: GatewayCandidateProtocolProfileV1 =
        serde_json::from_value(serde_json::to_value(profile).expect("test profile serializes"))
            .expect("gateway and product profile DTOs remain exact");
    let operational_target = GatewayOperationalTargetV1::ManagedCpaLoopback {
        uri: format!("http://{loopback}{}", ingress.path()),
        runtime_epoch: 1,
        target_epoch: 1,
    };
    let protocol_profiles = vec![canonical];
    CandidateBindingV1 {
        local_id,
        stable_target_key: format!("target-{local_id}"),
        adapter_id: "adapter.cpa.responses@1".into(),
        credential_refs: vec![format!("credential-{local_id}")],
        credential_destination_ref: "connection-option/fixture.cpa.v1".into(),
        upstream_model_id,
        native_transport_model,
        endpoint: logical_endpoint.into(),
        connector_runtime: ConnectorRuntimeKind::CpaBridge,
        operational_target_digest: CanonicalDigest::of(&operational_target).unwrap(),
        operational_target,
        protocol_profile_digest: CanonicalDigest::of(&protocol_profiles).unwrap(),
        protocol_profiles,
        pricing_identity: None,
    }
}

/// One active alias and the complete immutable request-plan closure it owns.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AliasPlanV1 {
    pub served_model_id: String,
    pub purpose: String,
    pub agent_plan_revision: u64,
    pub protocols: Vec<IngressProtocol>,
    pub overall_timeout_ms: u64,
    pub max_attempts: u32,
    /// Absent only on legacy G0 fixtures and durable snapshots, where exact
    /// candidate order remains the closed fallback behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing: Option<AliasRoutingV1>,
    pub candidates: Vec<CandidateBindingV1>,
}

/// Non-secret verifier and authorization projection for one Agent grant.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GrantV1 {
    pub grant_id: String,
    pub generation: u64,
    pub bearer_token_sha256: String,
    pub protocol: IngressProtocol,
    pub routes: std::collections::BTreeMap<String, ModelRouteV2>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelRouteV2 {
    Plan {
        plan_id: String,
        alias: String,
        revision: u64,
        semantic_digest: CanonicalDigest,
    },
    Fixed {
        binding: Box<CandidateBindingV1>,
        binding_digest: CanonicalDigest,
        overall_timeout_ms: u64,
        max_attempts: u32,
    },
}

/// Aggregate, durable gateway authority. Publication revision intentionally
/// differs from every alias's AgentPlan revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayPublicationSnapshotV3 {
    pub schema_version: String,
    pub admission: hiroute_domain::GatewayAdmissionStateV1,
    pub workspace_id: String,
    pub authority_id: String,
    pub authority_epoch: u64,
    pub publication_revision: u64,
    pub payload_digest: String,
    pub catalog_renderer_revision: String,
    pub aliases: Vec<AliasPlanV1>,
    pub grants: Vec<GrantV1>,
}

impl GatewayPublicationSnapshotV3 {
    #[allow(clippy::too_many_arguments)]
    pub fn seal(
        workspace_id: impl Into<String>,
        authority_id: impl Into<String>,
        authority_epoch: u64,
        publication_revision: u64,
        catalog_renderer_revision: impl Into<String>,
        aliases: Vec<AliasPlanV1>,
        grants: Vec<GrantV1>,
    ) -> Result<Self, PublicationSchemaError> {
        let mut snapshot = Self {
            schema_version: GATEWAY_PUBLICATION_SCHEMA.into(),
            admission: hiroute_domain::GatewayAdmissionStateV1::NewCallsAllowed,
            workspace_id: workspace_id.into(),
            authority_id: authority_id.into(),
            authority_epoch,
            publication_revision,
            payload_digest: String::new(),
            catalog_renderer_revision: catalog_renderer_revision.into(),
            aliases,
            grants,
        };
        snapshot.validate_without_digest()?;
        snapshot.payload_digest = snapshot.canonical_digest()?;
        Ok(snapshot)
    }

    pub fn validate(&self) -> Result<(), PublicationSchemaError> {
        self.validate_without_digest()?;
        if !is_sha256(&self.payload_digest) || self.canonical_digest()? != self.payload_digest {
            return Err(PublicationSchemaError::DigestMismatch);
        }
        Ok(())
    }

    pub fn canonical_digest(&self) -> Result<String, PublicationSchemaError> {
        let mut digestless = self.clone();
        digestless.payload_digest.clear();
        digestless.clear_pricing_identities();
        let bytes = serde_json::to_vec(&digestless).map_err(PublicationSchemaError::Json)?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }

    pub(crate) fn clear_pricing_identities(&mut self) {
        for alias in &mut self.aliases {
            for candidate in &mut alias.candidates {
                candidate.pricing_identity = None;
            }
        }
    }

    pub(crate) fn digest_bytes(&self) -> Result<[u8; 32], PublicationSchemaError> {
        let hex = self
            .payload_digest
            .strip_prefix("sha256:")
            .ok_or(PublicationSchemaError::DigestMismatch)?;
        let mut digest = [0_u8; 32];
        for (index, byte) in digest.iter_mut().enumerate() {
            let offset = index * 2;
            *byte = u8::from_str_radix(&hex[offset..offset + 2], 16)
                .map_err(|_| PublicationSchemaError::DigestMismatch)?;
        }
        Ok(digest)
    }

    fn validate_without_digest(&self) -> Result<(), PublicationSchemaError> {
        if self.schema_version != GATEWAY_PUBLICATION_SCHEMA {
            return Err(PublicationSchemaError::Schema);
        }
        let no_new_calls = self.admission == hiroute_domain::GatewayAdmissionStateV1::NoNewCalls;
        if !valid_workspace_id(&self.workspace_id)
            || self.authority_id.trim().is_empty()
            || self.authority_epoch == 0
            || self.publication_revision == 0
            || self.catalog_renderer_revision.trim().is_empty()
            || (no_new_calls && (!self.aliases.is_empty() || !self.grants.is_empty()))
            || (!no_new_calls && self.grants.is_empty())
        {
            return Err(PublicationSchemaError::Incomplete);
        }
        let mut aliases = HashSet::new();
        let mut binding_ids = HashSet::new();
        for alias in &self.aliases {
            if !valid_alias(&alias.served_model_id)
                || !aliases.insert(alias.served_model_id.as_str())
                || alias.purpose.trim().is_empty()
                || alias.purpose.len() > MAX_PURPOSE_BYTES
                || alias.agent_plan_revision == 0
                || alias.protocols.is_empty()
                || alias.overall_timeout_ms == 0
                || alias.overall_timeout_ms > MAX_LOGICAL_REQUEST_DURATION_MS
                || alias.max_attempts == 0
                || alias.max_attempts > MAX_REQUEST_ATTEMPTS
                || alias.candidates.is_empty()
                || alias.routing.as_ref().is_some_and(|routing| {
                    routing
                        .plan_display_name
                        .as_ref()
                        .is_some_and(|name| AgentPlanDisplayName::parse(name.clone()).is_err())
                })
            {
                return Err(PublicationSchemaError::InvalidAlias(
                    alias.served_model_id.clone(),
                ));
            }
            let mut protocols = HashSet::new();
            if alias
                .protocols
                .iter()
                .any(|value| !protocols.insert(*value))
            {
                return Err(PublicationSchemaError::InvalidAlias(
                    alias.served_model_id.clone(),
                ));
            }
        }
        let fixed = self.grants.iter().flat_map(|grant| {
            grant.routes.values().filter_map(move |route| match route {
                ModelRouteV2::Fixed { binding, .. } => Some((
                    std::slice::from_ref(binding.as_ref()),
                    std::slice::from_ref(&grant.protocol),
                )),
                ModelRouteV2::Plan { .. } => None,
            })
        });
        for (candidates, protocols) in self
            .aliases
            .iter()
            .map(|alias| (alias.candidates.as_slice(), alias.protocols.as_slice()))
            .chain(fixed)
        {
            for candidate in candidates {
                if candidate.local_id == 0
                    || !binding_ids.insert(candidate.local_id)
                    || candidate.stable_target_key.trim().is_empty()
                    || candidate.adapter_id.trim().is_empty()
                    || candidate.credential_refs.is_empty()
                    || candidate.credential_refs.len() > 64
                    || candidate
                        .credential_refs
                        .iter()
                        .any(|value| !valid_reference(value))
                    || candidate
                        .credential_refs
                        .iter()
                        .collect::<HashSet<_>>()
                        .len()
                        != candidate.credential_refs.len()
                    || ["connection-option/", "compute-target/"]
                        .into_iter()
                        .find_map(|prefix| {
                            candidate.credential_destination_ref.strip_prefix(prefix)
                        })
                        .is_none_or(|destination| !valid_reference(destination))
                    || candidate.upstream_model_id.trim().is_empty()
                    || candidate.native_transport_model.trim().is_empty()
                    || (candidate.connector_runtime == ConnectorRuntimeKind::BuiltinNative
                        && candidate.native_transport_model != candidate.upstream_model_id)
                    || !valid_endpoint(&candidate.endpoint)
                    || !candidate
                        .operational_target
                        .validate_for(candidate.connector_runtime, &candidate.endpoint)
                    || !matches!(
                        CanonicalDigest::of(&candidate.operational_target),
                        Ok(digest) if digest == candidate.operational_target_digest
                    )
                    || candidate.protocol_profiles.is_empty()
                    || (candidate.connector_runtime == ConnectorRuntimeKind::CpaBridge
                        && candidate.protocol_profiles.first().is_none_or(|profile| {
                            candidate.operational_target.request_path()
                                != Some(profile.connector.request_path.as_str())
                        }))
                    || candidate.pricing_identity.as_ref().is_some_and(|pricing| {
                        !valid_reference(&pricing.source_id)
                            || CanonicalDigest::parse(
                                pricing.source_identity_digest.as_str().to_owned(),
                            )
                            .is_err()
                            || !valid_reference(&pricing.model_configuration_id)
                            || !valid_reference(&pricing.actual_offer_ref)
                    })
                    || !matches!(
                        CanonicalDigest::of(&candidate.protocol_profiles),
                        Ok(digest) if digest == candidate.protocol_profile_digest
                    )
                {
                    return Err(PublicationSchemaError::InvalidCandidate(candidate.local_id));
                }
                let mut profile_ingress = std::collections::BTreeSet::new();
                let authentication = candidate
                    .protocol_profiles
                    .first()
                    .and_then(|profile| profile.connector.authentication.exact())
                    .ok_or(PublicationSchemaError::InvalidCandidate(candidate.local_id))?;
                for profile in &candidate.protocol_profiles {
                    let gateway_profile: CandidateProtocolProfile = serde_json::to_value(profile)
                        .ok()
                        .and_then(|value| serde_json::from_value(value).ok())
                        .ok_or(PublicationSchemaError::InvalidCandidate(candidate.local_id))?;
                    let minimum_requirements = RequestCapabilityRequirementsV1 {
                        ingress_protocol: gateway_profile.ingress_protocol,
                        text: false,
                        initial_instructions: false,
                        mid_conversation_instructions: false,
                        image_url: false,
                        image_base64: false,
                        image_media_types: Vec::new(),
                        function_tools: false,
                        strict_tools: false,
                        tool_choice: ToolChoice::Auto,
                        parallel_tools: false,
                        tool_roundtrip: false,
                        tool_result_text: false,
                        tool_result_json: false,
                        logical_tool_id_mapping: false,
                        streaming: false,
                        stream_text: false,
                        stream_tool_arguments: false,
                        stream_reasoning: false,
                        stream_usage: false,
                        provider_state: false,
                    };
                    if !profile_ingress.insert(profile.ingress_protocol)
                        || profile.capability.native_model != candidate.native_transport_model
                        || candidate.pricing_identity.as_ref().is_some_and(|pricing| {
                            pricing.model_configuration_id
                                != profile.capability.model_configuration_id
                        })
                        || profile.connector.authentication.exact() != Some(authentication)
                        || (candidate.connector_runtime != ConnectorRuntimeKind::CpaBridge
                            && candidate
                                .operational_target
                                .for_protocol_path(&profile.connector.request_path)
                                .is_none())
                        || gateway_profile.validate(&minimum_requirements).is_err()
                    {
                        return Err(PublicationSchemaError::InvalidCandidate(candidate.local_id));
                    }
                }
                if protocols.iter().any(|protocol| {
                    let expected = match protocol {
                        IngressProtocol::Responses => hiroute_domain::UpstreamProtocol::Responses,
                        IngressProtocol::ChatCompletions => {
                            hiroute_domain::UpstreamProtocol::ChatCompletions
                        }
                        IngressProtocol::Messages => hiroute_domain::UpstreamProtocol::Messages,
                    };
                    !profile_ingress.contains(&expected)
                }) {
                    return Err(PublicationSchemaError::InvalidCandidate(candidate.local_id));
                }
            }
        }
        let mut grant_ids = HashSet::new();
        let mut token_verifiers = HashSet::new();
        for grant in &self.grants {
            if grant.grant_id.trim().is_empty()
                || grant.generation == 0
                || !grant_ids.insert(grant.grant_id.as_str())
                || !is_sha256(&grant.bearer_token_sha256)
                || !token_verifiers.insert(grant.bearer_token_sha256.as_str())
                || grant.routes.is_empty()
            {
                return Err(PublicationSchemaError::InvalidGrant(grant.grant_id.clone()));
            }
            for (name, route) in &grant.routes {
                let valid = hiroute_domain::valid_client_model_name(name)
                    && match route {
                        ModelRouteV2::Plan {
                            plan_id,
                            alias,
                            revision,
                            semantic_digest,
                        } => {
                            name == alias
                                && is_sha256(semantic_digest.as_str())
                                && hiroute_domain::AgentPlanId::parse(plan_id.clone()).is_ok()
                                && self.aliases.iter().any(|target| {
                                    target.served_model_id == *alias
                                        && target.agent_plan_revision == *revision
                                        && target.protocols.contains(&grant.protocol)
                                        && target
                                            .routing
                                            .as_ref()
                                            .is_none_or(|routing| routing.agent_plan_id == *plan_id)
                                })
                        }
                        ModelRouteV2::Fixed {
                            binding,
                            binding_digest,
                            overall_timeout_ms,
                            max_attempts,
                        } => {
                            matches!(CanonicalDigest::of(binding.as_ref()), Ok(digest) if digest == *binding_digest)
                                && *overall_timeout_ms > 0
                                && *overall_timeout_ms <= MAX_LOGICAL_REQUEST_DURATION_MS
                                && *max_attempts > 0
                                && *max_attempts <= MAX_REQUEST_ATTEMPTS
                        }
                    };
                if !valid {
                    return Err(PublicationSchemaError::InvalidGrant(grant.grant_id.clone()));
                }
            }
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct AgentPlanSemanticPayload<'a> {
    semantic_schema: &'static str,
    core_compiler_version: u32,
    served_model_id: &'a str,
    purpose: &'a str,
    protocols: Vec<IngressProtocol>,
    overall_timeout_ms: u64,
    max_attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    routing: &'a Option<AliasRoutingV1>,
    candidates: &'a [CandidateBindingV1],
}

/// Identifies the executable alias in this publication for request provenance.
/// The reachable protocols are a current grant projection, not an immutable
/// AgentPlan revision or a reason to reject a later publication.
pub(crate) fn agent_plan_semantic_digest(
    alias: &AliasPlanV1,
) -> Result<String, PublicationSchemaError> {
    let mut protocols = alias.protocols.clone();
    protocols.sort_unstable();
    let mut candidates = alias.candidates.clone();
    for candidate in &mut candidates {
        candidate.pricing_identity = None;
    }
    let payload = AgentPlanSemanticPayload {
        semantic_schema: "hiroute.gateway.agent-plan-semantics/v1",
        core_compiler_version: SUPPORTED_COMPILER_VERSION,
        served_model_id: &alias.served_model_id,
        purpose: &alias.purpose,
        protocols,
        overall_timeout_ms: alias.overall_timeout_ms,
        max_attempts: alias.max_attempts,
        routing: &alias.routing,
        candidates: &candidates,
    };
    let bytes = serde_json::to_vec(&payload).map_err(PublicationSchemaError::Json)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
pub(crate) fn test_plan_route(name: &str, revision: u64) -> (String, ModelRouteV2) {
    (
        name.into(),
        ModelRouteV2::Plan {
            plan_id: format!("legacy/{name}"),
            alias: name.into(),
            revision,
            semantic_digest: CanonicalDigest::of_bytes(name.as_bytes()),
        },
    )
}

pub fn token_sha256(token: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(token.as_bytes()))
}

pub(crate) fn constant_time_digest_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn valid_alias(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ALIAS_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}

fn valid_workspace_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-'))
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value.contains("//")
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

fn valid_endpoint(value: &str) -> bool {
    let Ok(uri) = value.parse::<Uri>() else {
        return false;
    };
    matches!(uri.scheme_str(), Some("http" | "https"))
        && uri.authority().is_some_and(|authority| {
            !authority.host().is_empty() && !authority.as_str().contains('@')
        })
        && uri.query().is_none()
}

fn is_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Debug, Error)]
pub enum PublicationSchemaError {
    #[error("gateway publication schema is unsupported")]
    Schema,
    #[error("gateway publication is incomplete")]
    Incomplete,
    #[error("gateway publication digest does not match its canonical payload")]
    DigestMismatch,
    #[error("gateway publication alias is invalid: {0}")]
    InvalidAlias(String),
    #[error("gateway publication candidate is invalid: {0}")]
    InvalidCandidate(u32),
    #[error("gateway publication grant is invalid: {0}")]
    InvalidGrant(String),
    #[error("gateway publication JSON failed: {0}")]
    Json(serde_json::Error),
}
