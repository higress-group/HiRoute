//! Typed Local Control DTOs for the staged pre-Gateway compute slice.

use hiroute_domain::{
    BillingClass, CanonicalDigest, ChangeSpecV1, ComputeControlProjectionV1, ConnectionOrigin,
    ModelMetadataCatalogV1, RevisionSetV1,
};
use serde::{Deserialize, Serialize};

pub const COMPUTE_SCAN_RESULT_SCHEMA_V1: &str = "hiroute.compute-scan-result/v1";
pub const COMPUTE_CONNECTION_OPTIONS_SCHEMA_V1: &str = "hiroute.compute-connection-options/v1";
pub const COMPUTE_CONNECTION_CHANGE_SCHEMA_V1: &str = "hiroute.compute-connection-change/v1";
pub const COMPUTE_CONNECTION_PREVIEW_SCHEMA_V1: &str = "hiroute.compute-connection-preview/v1";

#[derive(schemars::JsonSchema, Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeScanRequestV1 {}

pub const COMPUTE_SCAN_REQUEST_SCHEMA_V2: &str = "hiroute.compute-scan-request/v2";
pub const COMPUTE_SCAN_RESULT_SCHEMA_V2: &str = "hiroute.compute-scan-result/v2";
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeScanRequestV2 {
    pub schema: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_target: Option<hiroute_domain::AgentNativeTargetV1>,
}
impl ComputeScanRequestV2 {
    pub fn valid(&self) -> bool {
        self.schema == COMPUTE_SCAN_REQUEST_SCHEMA_V2
            && self
                .native_target
                .as_ref()
                .is_none_or(|t| t.validate().is_ok())
    }
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeConnectionPointV2 {
    pub connection_ref: String,
    /// A scope label/reference safe for public display; private filesystem selectors remain local.
    pub context_ref: String,
    pub connection_option_id: Option<String>,
    pub endpoint_profile_id: Option<String>,
    pub target: Option<crate::ComputeCandidateTargetV2>,
    pub agent_id: String,
    pub display_name: String,
    pub native_provider_id: Option<String>,
    pub protocol: Option<hiroute_domain::UpstreamProtocol>,
    pub declared_model_ids: Vec<String>,
    pub default_model_id: Option<String>,
    pub configuration_state: String,
    pub discovery: Option<ComputeDiscoveryRefV1>,
    pub actions_required: Vec<String>,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeScanDiagnosticV2 {
    pub agent_id: String,
    pub reason: String,
    pub native_provider_id: Option<String>,
    pub model_id: Option<String>,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeScanResultV2 {
    pub schema: String,
    pub catalog: ComputeCatalogProvenanceViewV1,
    pub connections: Vec<ComputeConnectionPointV2>,
    pub diagnostics: Vec<ComputeScanDiagnosticV2>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputePermissionActionV1 {
    pub discovered_source_ref: String,
    pub observed_identity: CanonicalDigest,
    pub observed_revision: u64,
    pub display_path: String,
    pub required_mode: u32,
}

/// Non-secret facts needed to bind an already discovered local credential to an exact Preview.
/// The actual value remains behind `input_slot` and is read only by the protected Apply path.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeDiscoveredCredentialImportV1 {
    pub input_slot: String,
    pub scanner_id: String,
    pub scanner_version: String,
    pub source_ref: String,
    pub field_selector: String,
    pub observed_revision: u64,
}

/// Opaque, non-secret handle for one exact local compute discovery.
///
/// The daemon derives this from complete trusted discovery and current client-bundled catalog facts.
/// It is safe to project to a WebView, but it is not an authorization and cannot be expanded by
/// an ordinary client into a source, selector, path, authority, or protected input slot.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeDiscoveryRefV1 {
    pub discovery_ref: String,
    pub discovery_revision: u64,
}

impl ComputeDiscoveryRefV1 {
    pub fn valid(&self) -> bool {
        self.discovery_ref
            .strip_prefix("discovery/")
            .is_some_and(|digest| {
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
            && self.discovery_revision != 0
    }
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeScanItemV1 {
    pub agent_id: String,
    pub supported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_provider_id: Option<String>,
    pub configuration_state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovered_source_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_option_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registered_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_configuration_id: Option<String>,
    pub inventory_eligible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery: Option<ComputeDiscoveryRefV1>,
    #[serde(default)]
    pub actions_required: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_action: Option<ComputePermissionActionV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_import: Option<ComputeDiscoveredCredentialImportV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeCatalogProvenanceViewV1 {
    pub product_release: String,
    pub catalog_binding_id: String,
    pub release_sequence: u64,
    pub connector_registry_digest: CanonicalDigest,
    pub model_data_digest: CanonicalDigest,
    pub cross_reference_digest: CanonicalDigest,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeScanResultV1 {
    pub schema: String,
    pub catalog: ComputeCatalogProvenanceViewV1,
    pub items: Vec<ComputeScanItemV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeConnectionOptionsRequestV1 {
    /// Opt in to the existing candidate registry's exact, non-secret public view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<crate::ComputeCandidateRefV2>,
    /// Return exact catalog-to-request IDs only to callers that opt in. Ordinary
    /// responses retain their original shape for strict released readers.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_request_model_ids: bool,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeConnectionOptionV1 {
    pub endpoints: Vec<hiroute_domain::ProtocolEndpointV1>,
    pub known_models: std::collections::BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_model_ids: Option<std::collections::BTreeMap<String, Vec<String>>>,
    pub connection_option_id: String,
    pub display_name: String,
    pub connector_id: String,
    pub connector_revision: u64,
    pub endpoint_profile_id: String,
    pub endpoint_profile_revision: u64,
    pub origin: ConnectionOrigin,
    pub billing_class: BillingClass,
    pub authentication: String,
    pub model_configuration_ids: Vec<String>,
    #[serde(default)]
    pub registered_check_available: bool,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeConnectionOptionsResultV1 {
    pub schema: String,
    pub catalog: ComputeCatalogProvenanceViewV1,
    pub options: Vec<ComputeConnectionOptionV1>,
    /// Safe subscription candidates discovered by the configured CPA runtime. Credential
    /// material and connector-owned account locators never enter this projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscriptions: Option<crate::ComputeSubscriptionCandidatesV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_catalog: Option<ModelMetadataCatalogV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<crate::ComputeCandidateViewV2>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeConnectionChangeV1 {
    pub schema: String,
    pub discovered_source_ref: String,
    pub connection_option_id: String,
    pub model_configuration_id: String,
    pub expected_source_revision: u64,
    pub expected_binding_revision: u64,
    pub expected_inventory_revision: u64,
    pub explicit_materialization: bool,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeConnectionPreviewRequestV1 {
    pub change: ComputeConnectionChangeV1,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeProjectionEffectV1 {
    pub resource_kind: String,
    pub resource_id: String,
    pub expected_revision: u64,
    pub desired_revision: u64,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeConnectionPreviewResultV1 {
    pub schema: String,
    pub normalized_change: ComputeConnectionChangeV1,
    /// Complete sealed planner input. Apply resubmits these exact bytes so the transaction
    /// coordinator can perform idempotency lookup before revision and digest revalidation.
    pub spec: ChangeSpecV1,
    pub projection: ComputeControlProjectionV1,
    pub change_digest: CanonicalDigest,
    pub expected_revisions: RevisionSetV1,
    pub effects: Vec<ComputeProjectionEffectV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeConnectionApplyRequestV1 {
    pub spec: ChangeSpecV1,
    pub accept_digest: CanonicalDigest,
    pub expected_revisions: RevisionSetV1,
    pub idempotency_key: String,
}

#[derive(schemars::JsonSchema, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeConnectionPlannerPayloadV1 {
    pub connection_option_id: String,
    pub source_id: String,
    pub explicit_materialization: bool,
    pub expected_source_revision: u64,
    pub projection: hiroute_domain::PreparedComputeProjectionV1,
}

#[cfg(test)]
mod connection_options_compatibility_tests {
    use super::*;

    #[test]
    fn request_model_defaults_are_opt_in_and_keep_old_request_bytes() {
        let old: ComputeConnectionOptionsRequestV1 =
            serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(!old.include_request_model_ids);
        assert_eq!(serde_json::to_value(old).unwrap(), serde_json::json!({}));
        let opted: ComputeConnectionOptionsRequestV1 =
            serde_json::from_value(serde_json::json!({"include_request_model_ids": true})).unwrap();
        assert!(opted.include_request_model_ids);
    }
}
