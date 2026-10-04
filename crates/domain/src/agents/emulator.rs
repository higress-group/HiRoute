use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{AgentIngressProtocolV1, AgentProfileV1, ModelAlias};

pub const CONNECTIVITY_PROBE_PROMPT_V1: &str =
    "Reply with exactly HIRoute connectivity probe ready.";
pub const CONNECTIVITY_PROBE_RESPONSE_V1: &str = "HIRoute connectivity probe ready.";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTrafficKindV1 {
    ConnectivityProbe,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuiltInAgentProbeV1 {
    pub profile_id: String,
    pub integration_profile_ref: String,
    pub protocol: AgentIngressProtocolV1,
    pub model_alias: ModelAlias,
    pub traffic_kind: AgentTrafficKindV1,
    pub prompt: String,
    pub allow_tools: bool,
}

impl BuiltInAgentProbeV1 {
    /// The closed constructor deliberately exposes no prompt, provider, protocol, or tool input.
    pub fn for_profile(
        profile: &AgentProfileV1,
        model_alias: ModelAlias,
    ) -> Result<Self, AgentEmulatorError> {
        profile
            .validate()
            .map_err(|_| AgentEmulatorError::InvalidProfile)?;
        Ok(Self {
            profile_id: profile.profile_id.clone(),
            integration_profile_ref: profile.integration_profile_ref.clone(),
            protocol: profile
                .model_connection()
                .map_err(|_| AgentEmulatorError::InvalidProfile)?
                .ok_or(AgentEmulatorError::InvalidProfile)?
                .ingress_protocol,
            model_alias,
            traffic_kind: AgentTrafficKindV1::ConnectivityProbe,
            prompt: CONNECTIVITY_PROBE_PROMPT_V1.to_owned(),
            allow_tools: false,
        })
    }

    pub fn validate_for(&self, profile: &AgentProfileV1) -> Result<(), AgentEmulatorError> {
        profile
            .validate()
            .map_err(|_| AgentEmulatorError::InvalidProfile)?;
        if self.profile_id != profile.profile_id
            || self.integration_profile_ref != profile.integration_profile_ref
            || Some(self.protocol) != profile.client_protocol()
            || self.traffic_kind != AgentTrafficKindV1::ConnectivityProbe
            || self.prompt != CONNECTIVITY_PROBE_PROMPT_V1
            || self.allow_tools
        {
            Err(AgentEmulatorError::ContractDrift)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum AgentEmulatorError {
    #[error("Agent emulator profile is invalid")]
    InvalidProfile,
    #[error("Agent emulator request drifted from its closed connectivity-probe contract")]
    ContractDrift,
}
