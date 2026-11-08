use serde::{Deserialize, Serialize};

use crate::DecisionFailureCodeV2;
pub use hiroute_domain::{PlanReferenceBlockerV1, PlanReferenceProtectionV1};

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionRemovalPreviewRequestV1 {
    pub id: String,
    pub expected_revision: u64,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionRemovalPreviewV1 {
    pub id: String,
    pub revision: u64,
    pub failure: Option<DecisionFailureCodeV2>,
    pub blockers: Vec<PlanReferenceBlockerV1>,
}
