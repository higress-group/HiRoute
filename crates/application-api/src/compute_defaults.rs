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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_link: Option<hiroute_domain::ComputeSubscriptionApiLinkIntentV1>,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSavePreviewRequestV3 {
    pub change: ComputeManagementChangeV3,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagementChangeV5 {
    pub schema: String,
    pub change: ComputeManagementChangeV3,
    pub api_link: hiroute_domain::ComputeSubscriptionApiLinkIntentV1,
}

pub fn saved_compute_change(
    value: serde_json::Value,
) -> Result<ComputeManagementChangeV2, serde_json::Error> {
    if value.get("schema").and_then(serde_json::Value::as_str)
        == Some(hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V5)
        || value.get("schema").and_then(serde_json::Value::as_str)
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
    #[serde(default, skip_serializing_if = "native_reference_disabled")]
    pub include_native_reference: bool,
}
fn native_reference_disabled(value: &bool) -> bool {
    !value
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_reasoning_reference: Option<hiroute_domain::NativeReasoningCapabilityV1>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_reference_is_opt_in_without_changing_existing_wire_shapes() {
        let old = serde_json::json!({"source_id":"source/test"});
        let query: ComputeDefaultsQueryV1 = serde_json::from_value(old.clone()).unwrap();
        assert!(!query.include_native_reference);
        assert_eq!(serde_json::to_value(query).unwrap(), old);
        let old = serde_json::json!({"binding_id":"binding/test","model_configuration_id":null,"fields":{}});
        let model: ComputeModelDefaultsV1 = serde_json::from_value(old.clone()).unwrap();
        assert!(model.native_reasoning_reference.is_none());
        assert_eq!(serde_json::to_value(model).unwrap(), old);
        let schema = command_schemas("compute.defaults.get").unwrap();
        assert!(
            schema
                .request
                .to_string()
                .contains("include_native_reference")
        );
        assert!(
            schema
                .response
                .to_string()
                .contains("native_reasoning_reference")
        );
        let preferences = command_schemas("compute.defaults.preview")
            .unwrap()
            .request
            .to_string();
        for intent in [
            "custom",
            "follow_metadata",
            "preserve_saved",
            "follow_official",
        ] {
            assert!(
                preferences.contains(intent),
                "missing live schema variant {intent}"
            );
        }
    }
}
