//! Source-owned default intent. Historical source and V2 operation bytes stay unchanged.
use super::*;
use std::{collections::BTreeMap, ops::Deref};

pub const COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3: &str = "hiroute.compute-management-change/v3";
pub const COMPUTE_MANAGEMENT_MUTATION_SCHEMA_V3: &str = "hiroute.compute-management-mutation/v3";

#[derive(
    schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ModelDefaultFieldV1 {
    Tool,
    Vision,
    Streaming,
    ContextTokens,
    MaxOutputTokens,
    NativeReasoning,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialModelLocatorV1 {
    pub model_configuration_id: String,
    pub upstream_model_id: String,
    pub connector_id: String,
    pub endpoint_profile_id: String,
    pub protocol_endpoint_id: String,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "intent", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelDefaultPreferenceV1 {
    PreserveSaved,
    FollowOfficial { locator: OfficialModelLocatorV1 },
}
pub type SourceDefaultPreferencesV1 =
    BTreeMap<String, BTreeMap<ModelDefaultFieldV1, ModelDefaultPreferenceV1>>;

pub fn validate_source_default_preferences(
    source: Option<&ComputeManagementSourceV2>,
    preferences: &SourceDefaultPreferencesV1,
) -> Result<(), ComputeManagementErrorV2> {
    if preferences.len() > 4096 {
        return Err(ComputeManagementErrorV2::InvalidMutation);
    }
    for (binding, fields) in preferences {
        let model = source
            .and_then(|s| s.models.iter().find(|m| &m.binding_id == binding))
            .ok_or(ComputeManagementErrorV2::InvalidModel)?;
        if fields.is_empty() {
            return Err(ComputeManagementErrorV2::InvalidMutation);
        }
        for (field, policy) in fields {
            if let ModelDefaultPreferenceV1::FollowOfficial { locator } = policy {
                let basis = match field {
                    ModelDefaultFieldV1::Tool => model.capabilities.tool.basis,
                    ModelDefaultFieldV1::Vision => model.capabilities.vision.basis,
                    ModelDefaultFieldV1::Streaming => model.capabilities.streaming.basis,
                    ModelDefaultFieldV1::ContextTokens => model.capabilities.context_tokens.basis,
                    ModelDefaultFieldV1::MaxOutputTokens => {
                        model.capabilities.max_output_tokens.basis
                    }
                    ModelDefaultFieldV1::NativeReasoning => {
                        model.capabilities.native_reasoning.basis
                    }
                };
                if basis == ComputeManagementFactBasisV2::ConnectorVerified {
                    return Err(ComputeManagementErrorV2::InvalidCapability);
                }
                for id in [
                    &locator.model_configuration_id,
                    &locator.upstream_model_id,
                    &locator.connector_id,
                    &locator.endpoint_profile_id,
                    &locator.protocol_endpoint_id,
                ] {
                    validate_identifier(id)?;
                }
                if model.catalog_configuration_id.as_ref() != Some(&locator.model_configuration_id)
                    || model.upstream_model_id != locator.upstream_model_id
                {
                    return Err(ComputeManagementErrorV2::InvalidModel);
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagementMutationV3 {
    pub schema: String,
    pub source_mutation: ComputeManagementMutationV2,
    pub preferences_before: SourceDefaultPreferencesV1,
    pub preferences_after: SourceDefaultPreferencesV1,
    pub change_spec_digest: CanonicalDigest,
}
impl ComputeManagementMutationV3 {
    pub fn validate(&self, spec: &ChangeSpecV1) -> Result<(), ComputeManagementErrorV2> {
        if self.schema != COMPUTE_MANAGEMENT_MUTATION_SCHEMA_V3
            || spec
                .desired_state
                .get("schema")
                .and_then(serde_json::Value::as_str)
                != Some(COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3)
            || CanonicalDigest::of(spec).map_err(|_| ComputeManagementErrorV2::InvalidDigest)?
                != self.change_spec_digest
            || spec.desired_state.get("default_preferences")
                != Some(
                    &serde_json::to_value(&self.preferences_after)
                        .map_err(|_| ComputeManagementErrorV2::InvalidMutation)?,
                )
        {
            return Err(ComputeManagementErrorV2::InvalidMutation);
        }
        // Verify the original V2 proof against its original desired state, never a rewritten source.
        let mut original = spec.clone();
        original.desired_state = spec
            .desired_state
            .get("change")
            .cloned()
            .ok_or(ComputeManagementErrorV2::InvalidMutation)?;
        self.source_mutation.validate_shape(&original)?;
        validate_source_default_preferences(
            self.source_mutation.expected(),
            &self.preferences_before,
        )?;
        validate_source_default_preferences(self.source_mutation.desired(), &self.preferences_after)
    }
}

/// Version selection is explicit. V2 retains its original serialization and digest validation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ComputeManagementMutation {
    V4(crate::ComputeManagementMutationV4),
    V3(ComputeManagementMutationV3),
    V2(ComputeManagementMutationV2),
}
impl Deref for ComputeManagementMutation {
    type Target = ComputeManagementMutationV2;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::V4(value) => &value.source_mutation,
            Self::V2(value) => value,
            Self::V3(value) => &value.source_mutation,
        }
    }
}
impl ComputeManagementMutation {
    pub fn preferences(
        &self,
    ) -> Option<(&SourceDefaultPreferencesV1, &SourceDefaultPreferencesV1)> {
        match self {
            Self::V4(v) => v.source_mutation.preferences(),
            Self::V2(_) => None,
            Self::V3(v) => Some((&v.preferences_before, &v.preferences_after)),
        }
    }

    pub fn network(&self) -> Option<&crate::NativeNetworkMutationV1> {
        match self {
            Self::V4(v) => Some(&v.network),
            Self::V2(_) | Self::V3(_) => None,
        }
    }
}
