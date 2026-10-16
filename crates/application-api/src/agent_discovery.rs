//! Public discovery facts shared by CLI and Desktop; discovery remains Application-owned.
use hiroute_domain::{AgentModelSurfaceV2, CanonicalDigest, NativeReasoningCapabilityV1};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredAgentV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_target: Option<hiroute_domain::AgentNativeTargetV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_access: Option<CodexAccessViewV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    pub agent_id: String,
    pub profile_id: String,
    pub version: String,
    pub supported: bool,
    pub configuration_state: String,
    /// Independently located native entry points. Location is not an executable identity,
    /// version, digest, or installation admission gate.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub available_surfaces: BTreeSet<AgentModelSurfaceV2>,
    /// Bounded projection of the native client catalog plus independently proven source/account
    /// coverage. Large native metadata and all credentials remain behind the daemon boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_model_catalog: Option<DiscoveredAgentModelCatalogV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registered_configuration: Option<DiscoveredAgentConfigurationV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovered_credential: Option<DiscoveredCredentialInputV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_hardening: Option<AgentConfigPermissionFindingV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodexAccessViewV1 {
    pub codex_home: String,
    pub slot_id: String,
    pub profile_context_id: String,
    pub root_context_id: String,
    pub selected_mode: String,
    pub slot_occupied: bool,
    pub target_file: String,
    pub profile_name: String,
    pub commands: BTreeMap<String, String>,
    pub pending_operation: Option<String>,
    pub access_revoked: bool,
    pub conflict_fields: Vec<String>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredAgentModelCatalogV1 {
    pub metadata_source: AgentModelCatalogMetadataSourceV1,
    pub native_default_model: String,
    pub models: Vec<DiscoveredAgentModelV1>,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentModelCatalogMetadataSourceV1 {
    UserConfigured,
    TargetCache,
    TargetBundled,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredAgentModelV1 {
    pub client_model_id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_options: Vec<DiscoveredAgentModelSourceV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredAgentModelSourceV1 {
    pub binding_id: String,
    pub source_label: String,
    /// Opaque account/source identity. It is safe for correlation and contains no credential or
    /// human account claim.
    pub account_scope_ref: String,
    pub account_scope_digest: CanonicalDigest,
    pub state: AgentModelSourceCoverageStateV1,
    pub reasoning: NativeReasoningCapabilityV1,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentModelSourceCoverageStateV1 {
    Ready,
    CredentialRequired,
    AuthorizationRequired,
    Disabled,
    ModelUnconfirmed,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredAgentConfigurationV1 {
    pub connection_option_id: String,
    pub endpoint_profile_id: String,
    pub endpoint_profile_revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_configuration_id: Option<String>,
    pub base_url: String,
    pub observed_model_id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_model_alias_hints: BTreeMap<String, String>,
    pub configuration_revision: u64,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredCredentialInputV1 {
    pub source: String,
    pub scanner_id: String,
    pub scanner_version: String,
    pub discovered_source_ref: String,
    pub field_selector: String,
    pub observed_revision: u64,
    /// Opaque descriptor understood only by the protected-input port. It contains no path or
    /// Secret and is bound to the exact scanner revision.
    pub protected_input_slot: String,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfigPermissionFindingV1 {
    pub scanner_id: String,
    pub scanner_version: String,
    pub discovered_source_ref: String,
    pub observed_identity: CanonicalDigest,
    pub observed_revision: u64,
    pub display_path: String,
    pub required_mode: u32,
}

#[derive(schemars::JsonSchema, Serialize)]
pub struct AgentDiscoveryListV1 {
    pub agents: Vec<DiscoveredAgentV1>,
}
