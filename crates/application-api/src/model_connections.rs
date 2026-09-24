use hiroute_domain::{
    CanonicalDigest, GatewayAuthenticationSemanticsV1, NativeReasoningCapabilityV1,
    UpstreamProtocol,
};
use serde::{Deserialize, Serialize};

use crate::{
    ComputeCandidateRefV2, ComputeCandidateTargetV2, ComputeCandidateViewV2,
    ComputeManagementChangeV2, ComputeModelMembershipV2, OperationReferenceV1,
};

pub const GET_COMPUTE_CANDIDATE_OPERATION_V2: &str = "GetComputeCandidate";
pub const PREVIEW_COMPUTE_SAVE_OPERATION_V2: &str = "PreviewComputeSave";
pub const APPLY_COMPUTE_SAVE_OPERATION_V2: &str = "ApplyComputeSave";
pub const GET_COMPUTE_SAVE_RESULT_OPERATION_V2: &str = "GetComputeSaveResult";
pub const CHECK_NATIVE_MODEL_CONNECTION_OPERATION_V1: &str = "CheckNativeModelConnection";
pub const CHECK_REGISTERED_MODEL_CONNECTION_OPERATION_V1: &str = "CheckRegisteredModelConnection";
pub const PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1: &str =
    "PrepareDiscoveredModelConnection";
pub const CHECK_SAVED_MODEL_CONNECTION_OPERATION_V1: &str = "CheckSavedModelConnection";
pub const CANCEL_NATIVE_MODEL_CONNECTION_CHECK_OPERATION_V1: &str =
    "CancelNativeModelConnectionCheck";
pub const LIST_COMPUTE_SUBSCRIPTIONS_OPERATION_V2: &str = "ListComputeSubscriptions";
pub const PREVIEW_SUBSCRIPTION_CHECK_OPERATION_V2: &str = "PreviewSubscriptionCheck";
pub const APPLY_SUBSCRIPTION_CHECK_OPERATION_V2: &str = "ApplySubscriptionCheck";
pub const GET_SUBSCRIPTION_CHECK_RESULT_OPERATION_V2: &str = "GetSubscriptionCheckResult";
pub const RELEASE_SUBSCRIPTION_CHECK_OPERATION_V2: &str = "ReleaseSubscriptionCheck";

/// Web-safe base interpretation for a user-configured Native API endpoint.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeModelConnectionBaseKindV1 {
    ApiRoot,
    NativeMessagesBase,
    NativeResponsesBase,
}

/// Public fact bases accepted from the Desktop custom-API form. Registered and observed facts
/// are intentionally absent: an ordinary client cannot claim either authority.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeUserFactBasisV1 {
    UserDeclared,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeUserFactValueV1<T> {
    pub value: Option<T>,
    pub basis: NativeUserFactBasisV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeUserCapabilityDeclarationV1 {
    pub tool: NativeUserFactValueV1<bool>,
    pub vision: NativeUserFactValueV1<bool>,
    pub streaming: NativeUserFactValueV1<bool>,
    pub context_tokens: NativeUserFactValueV1<u64>,
    pub max_output_tokens: NativeUserFactValueV1<u64>,
    pub native_reasoning: NativeUserFactValueV1<NativeReasoningCapabilityV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeUserModelDeclarationV1 {
    pub upstream_model_id: String,
    pub display_name: String,
    /// Kept only so the closed UI shape can round-trip. A user-configured request must leave it
    /// absent; the daemon never turns a typed string into catalog provenance.
    pub catalog_configuration_id: Option<String>,
    pub membership: ComputeModelMembershipV2,
    pub capabilities: NativeUserCapabilityDeclarationV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeUserModelEndpointV1 {
    pub base_url: String,
    pub base_kind: NativeModelConnectionBaseKindV1,
    pub request_path_override: Option<String>,
    pub inventory_path_override: Option<String>,
    pub protocol: UpstreamProtocol,
    pub protocol_profile_id: String,
    pub protocol_profile_revision: u64,
    pub authentication: GatewayAuthenticationSemanticsV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeUserModelConnectionDraftV1 {
    /// Display only; does not confer provider, capability, billing or credential authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_template_id: Option<String>,
    #[serde(default)]
    pub inference_model_id: Option<String>,
    pub candidate_ref: Option<String>,
    pub lineage_ref: String,
    pub display_name: String,
    pub existing_source_id: Option<String>,
    #[serde(default)]
    pub expected_source_revision: Option<u64>,
    pub edit_revision: u64,
    pub check_id: String,
    pub base_url: String,
    pub base_kind: NativeModelConnectionBaseKindV1,
    pub request_path_override: Option<String>,
    pub inventory_path_override: Option<String>,
    pub protocol: UpstreamProtocol,
    pub protocol_profile_id: String,
    pub protocol_profile_revision: u64,
    pub authentication: GatewayAuthenticationSemanticsV1,
    #[serde(default)]
    pub additional_endpoints: Vec<NativeUserModelEndpointV1>,
    pub configuration_revision: u64,
    pub models: Vec<NativeUserModelDeclarationV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeModelConnectionCheckRequestV1 {
    pub draft: NativeUserModelConnectionDraftV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_candidate: Option<ComputeCandidateRefV2>,
}

/// Closed request for a catalog-registered API option. Endpoint, protocol, authentication,
/// capability, billing, and provider identity are deliberately absent and are resolved by the
/// daemon from its authenticated release catalog.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegisteredModelConnectionCheckRequestV1 {
    #[serde(default)]
    pub inference_model_id: Option<String>,
    #[serde(default)]
    pub models: Vec<NativeUserModelDeclarationV1>,
    pub connection_option_id: String,
    pub expected_catalog: crate::ComputeCatalogProvenanceViewV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_ref: Option<String>,
    pub lineage_ref: String,
    pub edit_revision: u64,
    pub check_id: String,
    pub input_candidate: ComputeCandidateRefV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_source_revision: Option<u64>,
}

/// Correlation-only request for turning one exact local discovery into a save candidate.
///
/// Source, scanner, selector, path, endpoint, authentication, model, capability, credential, and
/// protected input facts are intentionally absent. The daemon reconstructs all of them after an
/// exact rescan under the protected Desktop operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareDiscoveredModelConnectionRequestV1 {
    pub discovery: crate::ComputeDiscoveryRefV1,
    pub prepare_id: String,
}

impl PrepareDiscoveredModelConnectionRequestV1 {
    pub fn valid(&self) -> bool {
        self.discovery.valid()
            && !self.prepare_id.trim().is_empty()
            && self.prepare_id.trim() == self.prepare_id
            && self.prepare_id.len() <= 256
            && !self.prepare_id.contains('\0')
    }
}

impl RegisteredModelConnectionCheckRequestV1 {
    pub fn valid(&self) -> bool {
        let bounded = |value: &str| {
            !value.trim().is_empty()
                && value.trim() == value
                && value.len() <= 256
                && !value.contains('\0')
        };
        bounded(&self.connection_option_id)
            && self.candidate_ref.as_ref().is_none_or(|candidate_ref| {
                bounded(candidate_ref) && candidate_ref == &self.input_candidate.candidate_ref
            })
            && bounded(&self.lineage_ref)
            && self.edit_revision != 0
            && bounded(&self.check_id)
            && self.input_candidate.validate_shape().is_ok()
            && matches!(
                (&self.existing_source_id, self.expected_source_revision),
                (None, None) | (Some(_), Some(1..))
            )
            && self
                .existing_source_id
                .as_ref()
                .is_none_or(|source_id| bounded(source_id))
    }
}

/// Closed correlation-only request for rechecking one exact durable Native source. Every target,
/// provenance, model, lineage and credential fact is reconstructed by the daemon.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SavedModelConnectionCheckRequestV1 {
    pub source_id: String,
    pub expected_source_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_ref: Option<String>,
    pub edit_revision: u64,
    pub check_id: String,
}

impl SavedModelConnectionCheckRequestV1 {
    pub fn valid(&self) -> bool {
        let bounded = |value: &str| {
            !value.trim().is_empty()
                && value.trim() == value
                && value.len() <= 256
                && !value.contains('\0')
        };
        bounded(&self.source_id)
            && self.expected_source_revision != 0
            && self
                .candidate_ref
                .as_ref()
                .is_none_or(|value| bounded(value))
            && self.edit_revision != 0
            && bounded(&self.check_id)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeModelConnectionCancelRequestV1 {
    pub check_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeCandidateQueryV2 {
    pub candidate: ComputeCandidateRefV2,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSavePreviewRequestV2 {
    pub change: ComputeManagementChangeV2,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSaveResultQueryV2 {
    pub operation: OperationReferenceV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSubscriptionCandidatesV2 {
    pub candidates: Vec<ComputeCandidateViewV2>,
    pub discovery_state: ComputeSubscriptionDiscoveryStateV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeSubscriptionDiscoveryStateV2 {
    Complete,
    RuntimeUnavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSubscriptionCheckPreviewRequestV2 {
    pub candidate: ComputeCandidateRefV2,
}

/// Public same-UID connection probe selected explicitly by the caller. The wrapper keeps the
/// released CLI operation stable while preserving the closed request shapes owned by each
/// existing Application path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputeConnectionTestRequestV1 {
    Native {
        request: NativeModelConnectionCheckRequestV1,
    },
    Registered {
        request: RegisteredModelConnectionCheckRequestV1,
    },
    Discovered {
        request: PrepareDiscoveredModelConnectionRequestV1,
    },
    Saved {
        request: SavedModelConnectionCheckRequestV1,
    },
}

/// Read or release one exact subscription authorization result. Starting the check remains the
/// normal connection Preview/Apply flow so authorization cannot bypass digest, revision, or
/// idempotency admission.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputeConnectionAuthorizationRequestV1 {
    Result {
        operation: OperationReferenceV1,
    },
    Release {
        validation: crate::ComputeValidationRefV2,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSubscriptionCheckResultQueryV2 {
    pub operation: OperationReferenceV1,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelConnectionReachabilityV1 {
    NotRun,
    Reachable,
    TransportFailed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelConnectionAuthenticationStatusV1 {
    NotRun,
    NotRequired,
    Verified,
    Rejected,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelConnectionDirectoryStatusV1 {
    NotRun,
    Available,
    Empty,
    Unavailable,
    Invalid,
    Partial,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelConnectionProtocolStatusV1 {
    Selected,
    Inconclusive,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelConnectionInferenceStatusV1 {
    NotRun,
    Verified,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConnectionCheckIssueV1 {
    pub code: String,
    pub message_key: String,
    pub retryable: bool,
}

/// Safe result of an explicitly requested bounded connection and directory check.
///
/// It never contains a credential, protected input slot, source selector, redirect target, or
/// upstream response body. Inference remains `not_run` unless the user explicitly selects a single-model text test.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConnectionCheckViewV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inference_model_id: Option<String>,
    pub candidate: ComputeCandidateViewV2,
    pub target: ComputeCandidateTargetV2,
    pub inventory_path: Option<String>,
    pub reachability: ModelConnectionReachabilityV1,
    pub authentication: ModelConnectionAuthenticationStatusV1,
    pub directory: ModelConnectionDirectoryStatusV1,
    pub protocol: ModelConnectionProtocolStatusV1,
    pub inference: ModelConnectionInferenceStatusV1,
    pub checked_model_count: u32,
    pub invalid_model_count: u32,
    pub pages_read: u8,
    /// Backend completion time for the bounded check, expressed as Unix epoch milliseconds.
    pub checked_at_unix_ms: u64,
    pub input_digest: CanonicalDigest,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<ModelConnectionCheckIssueV1>,
}

impl ModelConnectionCheckViewV1 {
    pub fn matches_check(
        &self,
        candidate_ref: &str,
        edit_revision: u64,
        check_id: &str,
        input_digest: &CanonicalDigest,
    ) -> bool {
        self.candidate.candidate.candidate_ref == candidate_ref
            && self.candidate.correlation.candidate_ref == candidate_ref
            && self.candidate.correlation.edit_revision == edit_revision
            && self.candidate.correlation.check_id == check_id
            && &self.candidate.correlation.input_digest == input_digest
            && &self.input_digest == input_digest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_domain::{GatewayAuthenticationSemanticsV1, UpstreamProtocol};
    use serde_json::json;

    #[test]
    fn check_view_rejects_protected_transport_fields() {
        let value = json!({
            "candidate": {
                "candidate":{"candidate_ref":"candidate/native","candidate_revision":2},
                "correlation":{"candidate_ref":"candidate/native","edit_revision":7,"check_id":"check/7","input_digest":CanonicalDigest::of_bytes(b"input")},
                "producer":"native","provenance":"user_configured","display_name":"Custom",
                "models":[],"input_state":"provided","fact_state":"complete","issues":[]
            },
            "target":{"scheme":"https","authority":"api.example.test","port":443,
                "request_path":"/v1/responses","upstream_protocol":"responses",
                "protocol_profile_id":"profile/custom","protocol_profile_revision":1},
            "inventory_path":"/v1/models",
            "reachability":"reachable","authentication":"verified","directory":"empty",
            "protocol":"selected","inference":"not_run","checked_model_count":0,
            "invalid_model_count":0,"pages_read":1,"checked_at_unix_ms":1,
            "input_digest":CanonicalDigest::of_bytes(b"input"),
            "issues":[],"input_slot":"secret-slot"
        });
        assert!(serde_json::from_value::<ModelConnectionCheckViewV1>(value).is_err());
    }

    #[test]
    fn query_shapes_are_closed_and_keep_backend_revision_separate() {
        let query = ComputeCandidateQueryV2 {
            candidate: ComputeCandidateRefV2 {
                candidate_ref: "candidate/native".into(),
                candidate_revision: 42,
            },
        };
        let mut value = serde_json::to_value(query).unwrap();
        value["edit_revision"] = json!(7);
        assert!(serde_json::from_value::<ComputeCandidateQueryV2>(value).is_err());

        let target = crate::ComputeCandidateTargetV2 {
            scheme: "https".into(),
            authority: "api.example.test".into(),
            port: 443,
            request_path: "/v1/responses".into(),
            upstream_protocol: UpstreamProtocol::Responses,
            protocol_profile_id: "profile/custom".into(),
            protocol_profile_revision: 1,
        };
        let authentication = GatewayAuthenticationSemanticsV1::None;
        assert_eq!(target.port, 443);
        assert_eq!(authentication, GatewayAuthenticationSemanticsV1::None);
    }

    #[test]
    fn registered_check_shape_is_closed_and_binds_the_protected_candidate() {
        let mut request = RegisteredModelConnectionCheckRequestV1 {
            inference_model_id: None,
            models: Vec::new(),
            connection_option_id: "bailian.payg.cn.v1".into(),
            expected_catalog: crate::ComputeCatalogProvenanceViewV1 {
                product_release: "release-v1".into(),
                catalog_binding_id: "client-bundled/current".into(),
                release_sequence: 7,
                connector_registry_digest: CanonicalDigest::of_bytes(b"registry"),
                model_data_digest: CanonicalDigest::of_bytes(b"models"),
                cross_reference_digest: CanonicalDigest::of_bytes(b"cross-reference"),
            },
            candidate_ref: None,
            lineage_ref: "lineage/registered".into(),
            edit_revision: 1,
            check_id: "check/registered/1".into(),
            input_candidate: ComputeCandidateRefV2 {
                candidate_ref: "candidate/protected-input".into(),
                candidate_revision: 1,
            },
            existing_source_id: None,
            expected_source_revision: None,
        };
        assert!(request.valid());
        let encoded = serde_json::to_value(&request).unwrap();
        for forbidden in [
            "base_url",
            "billing_class",
            "authentication",
            "capabilities",
        ] {
            assert!(encoded.get(forbidden).is_none());
        }
        let mut unknown = encoded;
        unknown["api_key"] = json!("must-not-enter-json");
        assert!(
            serde_json::from_value::<RegisteredModelConnectionCheckRequestV1>(unknown).is_err()
        );

        request.existing_source_id = Some("source/registered".into());
        assert!(!request.valid());
        request.expected_source_revision = Some(2);
        assert!(request.valid());
        request.candidate_ref = Some("candidate/different".into());
        assert!(!request.valid());
    }

    #[test]
    fn saved_check_shape_contains_only_server_correlation() {
        let request = SavedModelConnectionCheckRequestV1 {
            source_id: "source/native".into(),
            expected_source_revision: 4,
            candidate_ref: Some("candidate/native".into()),
            edit_revision: 9,
            check_id: "check/saved/9".into(),
        };
        assert!(request.valid());
        let encoded = serde_json::to_value(&request).unwrap();
        assert_eq!(encoded.as_object().unwrap().len(), 5);
        for forbidden in [
            "lineage_ref",
            "base_url",
            "inventory_path",
            "connection_option_id",
            "authentication",
            "credential_id",
            "credential_generation",
            "input_slot",
            "models",
            "capabilities",
        ] {
            assert!(encoded.get(forbidden).is_none(), "{forbidden}");
        }

        let mut injected = encoded;
        injected["credential_id"] = json!("credential/forged");
        assert!(serde_json::from_value::<SavedModelConnectionCheckRequestV1>(injected).is_err());
    }

    #[test]
    fn discovered_prepare_shape_is_closed_and_opaque() {
        let request = PrepareDiscoveredModelConnectionRequestV1 {
            discovery: crate::ComputeDiscoveryRefV1 {
                discovery_ref: format!("discovery/{}", "a".repeat(64)),
                discovery_revision: 7,
            },
            prepare_id: "prepare/device-scan/7".into(),
        };
        assert!(request.valid());
        let value = serde_json::to_value(&request).unwrap();
        let rendered = value.to_string();
        for forbidden in [
            "source_ref",
            "scanner_id",
            "field_selector",
            "input_slot",
            "base_url",
            "authority",
            "api_key",
        ] {
            assert!(!rendered.contains(forbidden));
        }
        let mut unknown = value;
        unknown["connection_option_id"] = json!("zhipu.coding-plan.cn.v1");
        assert!(
            serde_json::from_value::<PrepareDiscoveredModelConnectionRequestV1>(unknown).is_err()
        );
        let mut invalid = request;
        invalid.prepare_id = "x".repeat(256);
        assert!(invalid.valid());
        invalid.prepare_id.push('x');
        assert!(!invalid.valid());
        invalid.prepare_id = " prepare".into();
        assert!(!invalid.valid());
    }
}
