//! Explicit C capability probe under the existing protected diagnostic operation.
use hiroute_domain::{CanonicalDigest, FrozenNativeProtocolV1};
use serde::{Deserialize, Serialize};

pub const HISTORICAL_APPLICABILITY_TEST_SCHEMA_V1: &str =
    "hiroute.historical-applicability-test/v1";
pub const HISTORICAL_APPLICABILITY_TEST_RESULT_SCHEMA_V1: &str =
    "hiroute.historical-applicability-test-result/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CCapabilityShapeV1 {
    Standard,
    MaxBranches,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CCapabilityStateV1 {
    Verified,
    Unsupported,
    NotTested,
    TestFailed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalApplicabilityTestRequestV1 {
    pub schema: String,
    pub service_id: String,
    pub service_revision: u64,
    pub protocol: FrozenNativeProtocolV1,
    pub shape: CCapabilityShapeV1,
}
impl HistoricalApplicabilityTestRequestV1 {
    pub fn validate(&self) -> bool {
        self.schema == HISTORICAL_APPLICABILITY_TEST_SCHEMA_V1
            && !self.service_id.is_empty()
            && self.service_id.len() <= 128
            && self.service_revision > 0
            && self.protocol.validate().is_ok()
            && match (&self.protocol.decision, self.shape) {
                (
                    hiroute_domain::DecisionDefinitionV1::Ordinal { .. },
                    CCapabilityShapeV1::Standard,
                ) => self.protocol.scopes.len() == 1,
                (hiroute_domain::DecisionDefinitionV1::Categorical { options, .. }, shape) => {
                    (2..=16).contains(&options.len())
                        && options
                            .iter()
                            .all(|o| self.protocol.scopes.contains_key(&o.id))
                        && (shape != CCapabilityShapeV1::MaxBranches
                            || options.len() == 16
                                && options.iter().all(|o| o.refinement.is_some()))
                }
                _ => false,
            }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalApplicabilityTestResultV1 {
    pub schema: String,
    pub state: CCapabilityStateV1,
    pub service_id: String,
    pub service_revision: u64,
    pub endpoint_digest: CanonicalDigest,
    pub model: String,
    pub protocol_digest: CanonicalDigest,
    pub shape: CCapabilityShapeV1,
    pub question_count: u16,
    pub tested_at_unix_ms: u64,
    pub duration_millis: u64,
    pub reason: String,
}

#[cfg(test)]
#[path = "classifier_diagnostic_c_tests.rs"]
mod tests;
