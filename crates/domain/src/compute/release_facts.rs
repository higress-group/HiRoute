use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{AgentProfileV1, CanonicalDigest, NativeReasoningCapabilityV1};

use super::{AGENT_PROFILES_ARTIFACT_SCHEMA_V1, ComputeContractError, RELEASE_FACTS_SCHEMA_V2};

pub const RELEASE_FACTS_TOOL_VERSION_V2: &str = "hiroute-release-facts/2";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelNativeReasoningV1 {
    pub model_configuration_id: String,
    pub capability: NativeReasoningCapabilityV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_render_convention: Option<NativeReasoningRenderConventionV1>,
}

/// Fixed model adaptation metadata, never a user-selectable reasoning axis.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeReasoningRenderConventionV1 {
    ClaudeAdaptiveEffortMessages,
    ZhipuThinkingChat,
}

impl ModelNativeReasoningV1 {
    pub fn validate(&self) -> Result<(), ComputeContractError> {
        self.capability
            .validate()
            .map_err(|_| ComputeContractError::InvalidModelData)?;
        match self.native_render_convention {
            Some(NativeReasoningRenderConventionV1::ClaudeAdaptiveEffortMessages) if !matches!(&self.capability, NativeReasoningCapabilityV1::Discrete { parameter, .. } if parameter == "output_config.effort") =>
            {
                return Err(ComputeContractError::InvalidModelData);
            }
            Some(NativeReasoningRenderConventionV1::ZhipuThinkingChat) if !matches!(&self.capability, NativeReasoningCapabilityV1::Discrete { parameter, profiles, .. } if parameter == "reasoning_effort" && profiles.iter().all(|profile| matches!(profile.as_str(), "low" | "high" | "max"))) =>
            {
                return Err(ComputeContractError::InvalidModelData);
            }
            _ => {}
        }
        Ok(())
    }

    /// Source-local capability choices keep companions only for the convention's own control.
    pub fn with_source_capability(mut self, capability: NativeReasoningCapabilityV1) -> Self {
        self.capability = capability;
        self.native_render_convention = self.native_render_convention.filter(|convention| {
            matches!((&self.capability, convention),
                (NativeReasoningCapabilityV1::Discrete { parameter, .. }, NativeReasoningRenderConventionV1::ClaudeAdaptiveEffortMessages) if parameter == "output_config.effort")
                || matches!((&self.capability, convention),
                    (NativeReasoningCapabilityV1::Discrete { parameter, .. }, NativeReasoningRenderConventionV1::ZhipuThinkingChat) if parameter == "reasoning_effort")
        });
        self
    }

    /// Consumers must check the selected source protocol before rendering this convention.
    pub fn validate_for_protocol(
        &self,
        protocol: super::UpstreamProtocol,
    ) -> Result<(), ComputeContractError> {
        self.validate()?;
        let required = match self.native_render_convention {
            Some(NativeReasoningRenderConventionV1::ClaudeAdaptiveEffortMessages) => {
                Some(super::UpstreamProtocol::Messages)
            }
            Some(NativeReasoningRenderConventionV1::ZhipuThinkingChat) => {
                Some(super::UpstreamProtocol::ChatCompletions)
            }
            None => None,
        };
        if required.is_some_and(|required| required != protocol) {
            return Err(ComputeContractError::InvalidModelData);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseFactsManifestV2 {
    pub schema: String,
    pub tool_version: String,
    pub catalog_id: String,
    pub product_release: String,
    pub sequence: u64,
    pub connector_registry_digest: CanonicalDigest,
    pub model_data_digest: CanonicalDigest,
    pub cross_reference_digest: CanonicalDigest,
}

impl ReleaseFactsManifestV2 {
    pub fn validate_shape(&self) -> Result<(), ComputeContractError> {
        if self.schema != RELEASE_FACTS_SCHEMA_V2
            || self.tool_version != RELEASE_FACTS_TOOL_VERSION_V2
        {
            return Err(ComputeContractError::UnsupportedSchema);
        }
        if self.sequence == 0
            || !valid_release_id(&self.product_release)
            || !valid_release_id(&self.catalog_id)
        {
            return Err(ComputeContractError::InvalidModelData);
        }
        for digest in [
            &self.connector_registry_digest,
            &self.model_data_digest,
            &self.cross_reference_digest,
        ] {
            CanonicalDigest::parse(digest.as_str().to_owned())
                .map_err(|_| ComputeContractError::InvalidModelData)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfilesArtifactV1 {
    pub schema: String,
    pub tool_version: String,
    pub profiles: Vec<AgentProfileV1>,
}

impl AgentProfilesArtifactV1 {
    pub fn validate(&self) -> Result<(), ComputeContractError> {
        if self.schema != AGENT_PROFILES_ARTIFACT_SCHEMA_V1
            || self.tool_version != RELEASE_FACTS_TOOL_VERSION_V2
            || self.profiles.is_empty()
        {
            return Err(ComputeContractError::InvalidModelData);
        }
        let mut profile_ids = BTreeSet::new();
        for profile in &self.profiles {
            if !profile_ids.insert(profile.profile_id.as_str()) || profile.validate().is_err() {
                return Err(ComputeContractError::InvalidModelData);
            }
        }
        Ok(())
    }
}

fn valid_release_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("//")
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
}
