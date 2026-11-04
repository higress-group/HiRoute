//! Safe, versioned decision connection DTOs shared by the product hosts and CLI.
use hiroute_domain::{CanonicalDigest, DecisionServiceRecord, DecisionWireProtocolV2};
pub use hiroute_domain::{
    DECISION_SERVICE_OPERATION_INPUT_SCHEMA_V3, DecisionContextModeV1, DecisionContextPolicyV1,
    DecisionExpectedSourceRevisionV1, DecisionServiceChangeV2, DecisionServiceChangeV3,
    DecisionServiceOperationInputV2, DecisionServiceOperationInputV3, TypeSafeLongContextV1,
};
use serde::{Deserialize, Serialize};

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionSourceStateV1 {
    Ready,
    Disabled,
    MissingCredentials,
    Missing,
    Unavailable,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionSourceAuthKindV1 {
    NativeApiKey,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionSourceSuggestionV1 {
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub wire_protocol: Option<DecisionWireProtocolV2>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionSourceSummaryV1 {
    pub source_id: String,
    pub lineage_digest: Option<CanonicalDigest>,
    pub revision: Option<u64>,
    pub display_name: Option<String>,
    pub provider_label: Option<String>,
    pub endpoint_host: Option<String>,
    pub auth_kind: Option<DecisionSourceAuthKindV1>,
    pub state: DecisionSourceStateV1,
    pub credential_state_revision: Option<CanonicalDigest>,
    pub suggestion: Option<DecisionSourceSuggestionV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionConnectionSourcesV1 {
    pub sources: Vec<DecisionSourceSummaryV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionServiceViewV2 {
    pub service: DecisionServiceRecord,
    pub configuration_digest: CanonicalDigest,
    pub credential_state_revision: Option<CanonicalDigest>,
    pub source: Option<DecisionSourceSummaryV1>,
    /// Current summaries of all sources referenced by retained revisions of this service.
    pub related_sources: Vec<DecisionSourceSummaryV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionServicesV2 {
    pub services: Vec<DecisionServiceViewV2>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionContextPreviewRequestV1 {
    pub endpoint: String,
    pub wire_protocol: DecisionWireProtocolV2,
    pub model: String,
    pub policy: hiroute_domain::DecisionContextPolicyV1,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DecisionServicesSchemaV3 {
    #[serde(rename = "hiroute.decision-services/v3")]
    V3,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionServicesV3 {
    pub schema: DecisionServicesSchemaV3,
    pub services: Vec<DecisionServiceViewV2>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionServiceTestRequestV2 {
    pub id: String,
    pub revision: u64,
    pub expected_configuration_digest: CanonicalDigest,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionServiceNetworkTestRequestV3 {
    pub id: String,
    pub revision: u64,
    pub expected_configuration_digest: CanonicalDigest,
    /// Opt into typed decision failures while keeping the released V3 result by default.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_decision_failure: bool,
}

impl From<DecisionServiceNetworkTestRequestV3> for DecisionServiceTestRequestV2 {
    fn from(request: DecisionServiceNetworkTestRequestV3) -> Self {
        Self {
            id: request.id,
            revision: request.revision,
            expected_configuration_digest: request.expected_configuration_digest,
        }
    }
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionTestOutcomeV2 {
    Passed,
    Failed,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DecisionFailureCodeV2 {
    DecisionInvalidConfiguration,
    DecisionSourceNotFound,
    DecisionSourceLineageMismatch,
    DecisionSourceDisabled,
    DecisionSourceUnavailable,
    DecisionNoUsableCredentials,
    DecisionRevisionConflict,
    DecisionInUse,
    DecisionTimeout,
    DecisionAuthenticationRejected,
    DecisionRateLimited,
    DecisionInputRejected,
    DecisionMalformedOutput,
    DecisionTransportUnavailable,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DecisionServiceTestResultSchemaV2 {
    #[serde(rename = "hiroute.decision-service-test-result/v2")]
    V2,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionServiceTestResultV2 {
    pub schema: DecisionServiceTestResultSchemaV2,
    pub outcome: DecisionTestOutcomeV2,
    pub failure_code: Option<DecisionFailureCodeV2>,
    pub duration_millis: Option<u64>,
    pub tested_configuration_digest: CanonicalDigest,
    pub tested_credential_state_revision: Option<CanonicalDigest>,
}

pub const TEST_DECISION_SERVICE_V3: &str = "TestDecisionServiceV3";

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DecisionServiceTestResultSchemaV3 {
    #[serde(rename = "hiroute.decision-service-test-result/v3")]
    V3,
}

/// Network evidence describes the admitted request, independently of its Key state.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionServiceTestResultV3 {
    pub schema: DecisionServiceTestResultSchemaV3,
    pub check: DecisionServiceTestResultV2,
    pub network: Option<crate::NativeNetworkCheckEvidenceV1>,
    pub failure: Option<crate::NativeNetworkFailureV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_failure: Option<hiroute_domain::DecisionFailureV1>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_domain::ComplexityClassifierModeV1;
    use serde_json::Value;

    #[test]
    fn current_product_contract_retains_exact_context_and_configuration_identity() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../apps/desktop/tests/decision-services-v3-contract.json"
        ))
        .unwrap();
        let services: crate::DecisionServicesV3 =
            serde_json::from_value(fixture["decision_services_v3"]["result"].clone()).unwrap();
        let change: hiroute_domain::DecisionServiceChangeV3 = serde_json::from_value(
            fixture["save_decision_service_v3"]["args"]["input"]["change"].clone(),
        )
        .unwrap();
        let preview: crate::DecisionContextPreviewRequestV1 =
            serde_json::from_value(fixture["preview_decision_context"]["args"]["input"].clone())
                .unwrap();
        let context: hiroute_domain::ResolvedDecisionContextV1 =
            serde_json::from_value(fixture["preview_decision_context"]["result"].clone()).unwrap();
        assert!(context.validate());
        assert!(preview.policy.validate());
        assert!(change.validate());
        let saved = change.service.unwrap();
        assert_eq!(saved.context.as_ref().unwrap().resolved, context);
        assert_eq!(
            CanonicalDigest::of(&saved).unwrap(),
            services.services[0].configuration_digest
        );
        assert_eq!(
            services.services[0].service,
            hiroute_domain::DecisionServiceRecord::V3(saved)
        );
        let _: ComplexityClassifierModeV1 =
            serde_json::from_value(fixture["classifier_v3"].clone()).unwrap();
        // A context-stripped V3 never becomes a legacy record.
        let mut stripped = fixture["classifier_v3"]["service"].clone();
        stripped.as_object_mut().unwrap().remove("context");
        assert!(serde_json::from_value::<DecisionServiceRecord>(stripped).is_err());
    }

    #[test]
    fn product_contract_examples_deserialize_and_keep_exact_identity() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../apps/desktop/tests/decision-services-v2-contract.json"
        ))
        .unwrap();
        let sources: DecisionConnectionSourcesV1 =
            serde_json::from_value(fixture["decision_connection_sources"]["result"].clone())
                .unwrap();
        let services: DecisionServicesV2 =
            serde_json::from_value(fixture["decision_services_v2"]["result"].clone()).unwrap();
        let change: DecisionServiceChangeV2 = serde_json::from_value(
            fixture["save_decision_service_v2"]["args"]["input"]["change"].clone(),
        )
        .unwrap();
        let request: DecisionServiceTestRequestV2 =
            serde_json::from_value(fixture["test_decision_service_v2"]["args"]["input"].clone())
                .unwrap();
        let result: DecisionServiceTestResultV2 =
            serde_json::from_value(fixture["test_decision_service_v2"]["result"].clone()).unwrap();
        let _: ComplexityClassifierModeV1 =
            serde_json::from_value(fixture["classifier_v2"].clone()).unwrap();

        assert_eq!(sources.sources[0].source_id, "source/example");
        assert!(matches!(
            services.services[0].service,
            DecisionServiceRecord::V1(_)
        ));
        assert!(matches!(
            services.services[1].service,
            DecisionServiceRecord::V2(_)
        ));
        let saved = change.service.unwrap();
        assert_eq!(
            CanonicalDigest::of(&saved).unwrap(),
            request.expected_configuration_digest
        );
        assert_eq!(
            result.tested_configuration_digest,
            request.expected_configuration_digest
        );
        assert_eq!(
            result.tested_credential_state_revision,
            sources.sources[0].credential_state_revision
        );
        assert_eq!(
            serde_json::to_value(saved).unwrap(),
            fixture["classifier_v2"]["service"]
        );
    }

    #[test]
    fn network_evidence_wraps_the_strict_v2_check_without_redefining_credential_state() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../apps/desktop/tests/decision-services-v2-contract.json"
        ))
        .unwrap();
        let check: DecisionServiceTestResultV2 =
            serde_json::from_value(fixture["test_decision_service_v2"]["result"].clone()).unwrap();
        let value = DecisionServiceTestResultV3 {
            schema: DecisionServiceTestResultSchemaV3::V3,
            check: check.clone(),
            network: None,
            failure: None,
            decision_failure: None,
        };
        let encoded = serde_json::to_value(&value).unwrap();
        assert!(serde_json::from_value::<DecisionServiceTestResultV2>(encoded.clone()).is_err());
        assert_eq!(
            serde_json::from_value::<DecisionServiceTestResultV3>(encoded.clone())
                .unwrap()
                .check,
            check
        );
        let mut extra = encoded;
        extra["network_state_revision"] = "not-an-admitted-route".into();
        assert!(serde_json::from_value::<DecisionServiceTestResultV3>(extra).is_err());
    }

    #[test]
    fn legacy_network_probe_request_preserves_its_canonical_digest() {
        let legacy = DecisionServiceTestRequestV2 {
            id: "decision/fixture".into(),
            revision: 1,
            expected_configuration_digest: CanonicalDigest::of_bytes(b"fixture"),
        };
        let encoded = serde_json::to_value(&legacy).unwrap();
        let mut network: DecisionServiceNetworkTestRequestV3 =
            serde_json::from_value(encoded.clone()).unwrap();
        assert!(!network.include_decision_failure);
        assert_eq!(serde_json::to_value(&network).unwrap(), encoded);
        assert_eq!(
            CanonicalDigest::of(&network).unwrap(),
            CanonicalDigest::of(&legacy).unwrap()
        );
        network.include_decision_failure = true;
        assert_ne!(
            CanonicalDigest::of(&network).unwrap(),
            CanonicalDigest::of(&legacy).unwrap()
        );
        assert!(
            serde_json::from_value::<DecisionServiceTestRequestV2>(
                serde_json::to_value(network).unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn service_and_test_schemas_reject_unknown_versions() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../apps/desktop/tests/decision-services-v2-contract.json"
        ))
        .unwrap();
        let mut service = fixture["classifier_v2"]["service"].clone();
        service["schema"] = "hiroute.decision-service/v3".into();
        assert!(serde_json::from_value::<DecisionServiceRecord>(service).is_err());
        let mut result = fixture["test_decision_service_v2"]["result"].clone();
        result["schema"] = "hiroute.decision-service-test-result/v3".into();
        assert!(serde_json::from_value::<DecisionServiceTestResultV2>(result).is_err());
    }
}
