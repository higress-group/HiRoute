use hiroute_domain::ComplexityClassifierModeV1;
use serde::{Deserialize, Serialize};

pub const CLASSIFIER_DECISION_TEST_SCHEMA_V1: &str = "hiroute.classifier-decision-test/v1";
pub const CLASSIFIER_DECISION_TEST_RESULT_SCHEMA_V1: &str =
    "hiroute.classifier-decision-test-result/v1";
pub const CLASSIFIER_DECISION_TEST_LATEST_USER: &str =
    "This is a synthetic connectivity test. Choose the economy branch for this clear, local task.";

/// Typed desired state for the one classifier-specific Secret mutation. The
/// protected value itself is referenced by `input_slot` and never enters this
/// serializable request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClassifierHeaderSecretInputV1 {
    pub secret_id: String,
    pub input_slot: String,
    #[serde(default)]
    pub expected_generation: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClassifierDecisionTestRequestV1 {
    pub schema: String,
    pub classifier: ComplexityClassifierModeV1,
}

impl ClassifierDecisionTestRequestV1 {
    pub fn validate(&self) -> bool {
        self.schema == CLASSIFIER_DECISION_TEST_SCHEMA_V1
            && !matches!(&self.classifier, ComplexityClassifierModeV1::LocalRules)
            && self.classifier.validate().is_ok()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClassifierDecisionTestResultV1 {
    pub schema: String,
    pub outcome: ClassifierDecisionTestOutcomeV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_millis: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassifierDecisionTestOutcomeV1 {
    Passed,
    Failed,
}
