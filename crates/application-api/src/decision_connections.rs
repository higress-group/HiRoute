//! Safe, versioned decision connection DTOs shared by the product hosts and CLI.
use hiroute_domain::{CanonicalDigest, DecisionServiceRecord, DecisionWireProtocolV2};
pub use hiroute_domain::{
    DecisionExpectedSourceRevisionV1, DecisionServiceChangeV2, DecisionServiceOperationInputV2,
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
pub struct DecisionServiceTestRequestV2 {
    pub id: String,
    pub revision: u64,
    pub expected_configuration_digest: CanonicalDigest,
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

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_domain::ComplexityClassifierModeV1;
    use serde_json::Value;

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
