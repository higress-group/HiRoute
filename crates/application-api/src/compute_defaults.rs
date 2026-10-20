//! Versioned save intent without extending released V2 request/source readers.
use crate::*;
use hiroute_domain::SourceDefaultPreferencesV1;
use serde::{Deserialize, Serialize};

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagementChangeV3 {
    pub schema: String,
    pub change: ComputeManagementChangeV2,
    pub default_preferences: SourceDefaultPreferencesV1,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSavePreviewRequestV3 {
    pub change: ComputeManagementChangeV3,
}

pub fn saved_compute_change(
    value: serde_json::Value,
) -> Result<ComputeManagementChangeV2, serde_json::Error> {
    if value.get("schema").and_then(serde_json::Value::as_str)
        == Some(hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3)
    {
        serde_json::from_value::<ComputeManagementChangeV3>(value).map(|value| value.change)
    } else {
        serde_json::from_value(value)
    }
}
