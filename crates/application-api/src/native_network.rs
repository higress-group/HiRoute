//! Versioned Native source policy carriers; released connection shapes stay strict.
use hiroute_domain::{
    CanonicalDigest, NativeNetworkModeV1, NativeNetworkPolicyV1, NativeNetworkRouteReasonV1,
    UpstreamProtocol,
};
use serde::{Deserialize, Serialize};

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeNetworkEditV1 {
    pub expected_network_revision: u64,
    pub policy: NativeNetworkPolicyV1,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagementChangeV4 {
    pub schema: String,
    /// A strict V2/V3 change, validated at the application boundary.
    pub change: serde_json::Value,
    pub network_edit: NativeNetworkEditV1,
}

pub const GET_NATIVE_SOURCE_NETWORK_V1: &str = "GetNativeSourceNetworkV1";
pub const PREVIEW_NATIVE_SOURCE_NETWORK_V1: &str = "PreviewNativeSourceNetworkV1";
pub const APPLY_NATIVE_SOURCE_NETWORK_V1: &str = "ApplyNativeSourceNetworkV1";
pub const PREVIEW_NATIVE_NETWORK_ROUTE_V1: &str = "PreviewNativeNetworkRouteV1";
pub const PREVIEW_COMPUTE_SAVE_V4: &str = "PreviewComputeSaveV4";
pub const CHECK_NATIVE_MODEL_CONNECTION_V2: &str = "CheckNativeModelConnectionV2";
pub const CHECK_REGISTERED_MODEL_CONNECTION_V2: &str = "CheckRegisteredModelConnectionV2";
pub const CHECK_SAVED_MODEL_CONNECTION_V2: &str = "CheckSavedModelConnectionV2";

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSourceNetworkQueryV1 {
    pub source_id: String,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSourceNetworkPreviewV1 {
    pub source_id: String,
    pub expected_source_revision: u64,
    pub network_edit: NativeNetworkEditV1,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeNetworkPreviewTargetV1 {
    Draft {
        endpoint_url: String,
    },
    Registered {
        connection_option_id: String,
        protocol: UpstreamProtocol,
    },
    Saved {
        source_id: String,
        expected_source_revision: u64,
        protocol: Option<UpstreamProtocol>,
    },
    Decision {
        id: String,
        revision: u64,
    },
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeNetworkRoutePreviewRequestV1 {
    pub target: NativeNetworkPreviewTargetV1,
    pub policy: Option<NativeNetworkPolicyV1>,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeNetworkApplicationStateV1 {
    Applied,
    Pending,
    Unavailable,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeNetworkRouteKindV1 {
    Direct,
    Proxy,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeNetworkRouteViewV1 {
    pub mode: NativeNetworkModeV1,
    pub route: Option<NativeNetworkRouteKindV1>,
    pub reason: Option<NativeNetworkRouteReasonV1>,
    pub destination: String,
    pub state_revision: CanonicalDigest,
    pub failure: Option<NativeNetworkFailureV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeNetworkAffectedModelV1 {
    pub binding_id: String,
    pub display_name: String,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeNetworkAffectedDecisionV1 {
    pub service_id: String,
    pub revision: u64,
    pub display_name: String,
    pub model: String,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSourceNetworkViewV1 {
    pub source_id: String,
    pub source_revision: u64,
    pub network_revision: u64,
    pub policy: NativeNetworkPolicyV1,
    pub applied_revision: Option<u64>,
    pub application_state: NativeNetworkApplicationStateV1,
    pub operation: Option<crate::OperationReferenceV1>,
    pub routes: Vec<NativeNetworkRouteViewV1>,
    pub models: Vec<NativeNetworkAffectedModelV1>,
    pub decisions: Vec<NativeNetworkAffectedDecisionV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeNetworkSavePreviewV1 {
    pub preview: crate::ComputeSavePreviewV2,
    pub before: NativeSourceNetworkViewV1,
    pub after: NativeNetworkPolicyV1,
}

pub use hiroute_domain::{NativeNetworkFailurePhaseV1, NativeNetworkFailureV1};

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeNetworkCheckEvidenceV1 {
    pub policy: NativeNetworkPolicyV1,
    pub expected_network_revision: u64,
    pub route: NativeNetworkRouteViewV1,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConnectionCheckViewV2 {
    pub check: crate::ModelConnectionCheckViewV1,
    pub network: NativeNetworkCheckEvidenceV1,
    pub failure: Option<NativeNetworkFailureV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeModelConnectionCheckRequestV2 {
    pub request: crate::NativeModelConnectionCheckRequestV1,
    pub network_edit: NativeNetworkEditV1,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegisteredModelConnectionCheckRequestV2 {
    pub request: crate::RegisteredModelConnectionCheckRequestV1,
    pub network_edit: NativeNetworkEditV1,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SavedModelConnectionCheckRequestV2 {
    pub request: crate::SavedModelConnectionCheckRequestV1,
    pub network_edit: Option<NativeNetworkEditV1>,
}

/// Explicit same-user CLI probes; legacy V1 test wrappers remain unchanged.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputeConnectionTestRequestV2 {
    Native {
        request: NativeModelConnectionCheckRequestV2,
    },
    Registered {
        request: RegisteredModelConnectionCheckRequestV2,
    },
    Saved {
        request: SavedModelConnectionCheckRequestV2,
    },
}
