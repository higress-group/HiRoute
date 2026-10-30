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
        == Some(hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V4)
    {
        return saved_compute_change(
            value
                .get("change")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        );
    }
    if value.get("schema").and_then(serde_json::Value::as_str)
        == Some(hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3)
    {
        serde_json::from_value::<ComputeManagementChangeV3>(value).map(|value| value.change)
    } else {
        serde_json::from_value(value)
    }
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeDefaultsQueryV1 {
    pub source_id: String,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeDefaultFieldViewV1 {
    pub saved_value: serde_json::Value,
    pub effective_value: serde_json::Value,
    pub intent: String,
    pub origin: String,
    pub locator: Option<hiroute_domain::OfficialModelLocatorV1>,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeModelDefaultsV1 {
    pub binding_id: String,
    pub model_configuration_id: Option<String>,
    pub fields:
        std::collections::BTreeMap<hiroute_domain::ModelDefaultFieldV1, ComputeDefaultFieldViewV1>,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeDefaultsResultV1 {
    pub source_id: String,
    pub source_revision: u64,
    pub revisions: hiroute_domain::RevisionSetV1,
    pub reviewed_on: String,
    pub models: Vec<ComputeModelDefaultsV1>,
}
