use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{AgentClaudePresetValuesV2, CanonicalDigest, valid_client_model_name};

pub const MANAGED_CLAUDE_LAUNCH_DESCRIPTOR_SCHEMA_V2: &str =
    "hiroute.managed-claude-launch-descriptor/v2";
pub const CLAUDE_CODE_MANAGED_LAUNCH_VERSION_V1: &str = "2.1.231";
pub const HIDDEN_AGENT_GRANT_HELPER_VERB_V1: &str = "__internal-agent-grant-v1";
pub const MANAGED_CLAUDE_SETTINGS_ARGUMENT_V1: &str = "--settings";
pub const MANAGED_CLAUDE_SETTING_SOURCES_ARGUMENT_V1: &str = "--setting-sources";

pub const MANAGED_CLAUDE_ENVIRONMENT_REMOVALS_V1: [&str; 8] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_CUSTOM_HEADERS",
    "ANTHROPIC_MODEL",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_VERTEX",
];

/// Only arguments that would override the managed gateway, authentication, or cloud provider
/// routing are refused outright. Model selection, `--settings`, `--setting-sources`, and
/// `--fallback-model` reach the launcher and are directed against the published snapshot:
/// the child resolves them under the managed grant exactly like any other request name.
pub const MANAGED_CLAUDE_CALLER_FORBIDDEN_OPTIONS_V1: [&str; 14] = [
    "--anthropic-api-key",
    "--anthropic-auth-token",
    "--api-key",
    "--api-key-helper",
    "--auth-token",
    "--base-url",
    "--bedrock",
    "--custom-headers",
    "--foundry",
    "--provider",
    "--use-bedrock",
    "--use-foundry",
    "--use-vertex",
    "--vertex",
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentActivationModeV1 {
    ManagedConfiguration,
    ManagedLaunch,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedLaunchProfileV1 {
    /// The exact installation version whose managed-launch capability has been verified
    /// empirically; discovery refuses the managed boundary for any other version.
    pub exact_version: String,
    pub settings_argument: String,
    pub environment_removals: BTreeSet<String>,
    pub caller_forbidden_options: BTreeSet<String>,
}

impl ManagedLaunchProfileV1 {
    pub fn claude_code_2_1_231() -> Self {
        Self {
            exact_version: CLAUDE_CODE_MANAGED_LAUNCH_VERSION_V1.to_owned(),
            settings_argument: MANAGED_CLAUDE_SETTINGS_ARGUMENT_V1.to_owned(),
            environment_removals: MANAGED_CLAUDE_ENVIRONMENT_REMOVALS_V1
                .into_iter()
                .map(str::to_owned)
                .collect(),
            caller_forbidden_options: MANAGED_CLAUDE_CALLER_FORBIDDEN_OPTIONS_V1
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }
    }

    pub fn validate(&self) -> Result<(), ManagedLaunchError> {
        if self != &Self::claude_code_2_1_231() {
            return Err(ManagedLaunchError::UntrustedProfile);
        }
        Ok(())
    }

    pub fn forbids_caller_option(&self, argument: &str) -> bool {
        self.caller_forbidden_options.iter().any(|option| {
            argument == option
                || argument
                    .strip_prefix(option)
                    .is_some_and(|suffix| suffix.starts_with('='))
        })
    }
}

/// The launcher's routing facts for one managed Claude connection. Every field comes from the
/// succeeded settings Operation, its grant, and the current trusted installation choice; nothing
/// here re-derives routing from user configuration at launch time.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedClaudeLaunchDescriptorV2 {
    pub schema: String,
    pub connection_id: String,
    pub profile_id: String,
    pub executable: String,
    /// Digest of the settings Operation's immutable three-slot snapshot facts.
    pub snapshot_digest: CanonicalDigest,
    pub grant_generation: u64,
    pub publication_digest: CanonicalDigest,
    pub gateway_base_url: String,
    /// The three native slot values the managed overlay carries: mapped slots hold the plan
    /// alias, preserved slots hold the snapshot's original value or stay absent.
    pub presets: AgentClaudePresetValuesV2,
    pub helper_executable: String,
    pub helper_argv: Vec<String>,
    pub environment_removals: BTreeSet<String>,
}

impl ManagedClaudeLaunchDescriptorV2 {
    #[allow(clippy::too_many_arguments)]
    pub fn trusted(
        connection_id: impl Into<String>,
        profile_id: impl Into<String>,
        executable: impl Into<String>,
        snapshot_digest: CanonicalDigest,
        grant_generation: u64,
        publication_digest: CanonicalDigest,
        gateway_base_url: impl Into<String>,
        presets: AgentClaudePresetValuesV2,
        trusted_hiroute_executable: impl Into<String>,
    ) -> Result<Self, ManagedLaunchError> {
        let connection_id = connection_id.into();
        let helper_executable = trusted_hiroute_executable.into();
        let value = Self {
            schema: MANAGED_CLAUDE_LAUNCH_DESCRIPTOR_SCHEMA_V2.to_owned(),
            connection_id: connection_id.clone(),
            profile_id: profile_id.into(),
            executable: executable.into(),
            snapshot_digest,
            grant_generation,
            publication_digest,
            gateway_base_url: gateway_base_url.into(),
            presets,
            helper_executable,
            helper_argv: vec![HIDDEN_AGENT_GRANT_HELPER_VERB_V1.to_owned(), connection_id],
            environment_removals: MANAGED_CLAUDE_ENVIRONMENT_REMOVALS_V1
                .into_iter()
                .map(str::to_owned)
                .collect(),
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), ManagedLaunchError> {
        if self.schema != MANAGED_CLAUDE_LAUNCH_DESCRIPTOR_SCHEMA_V2
            || self.grant_generation == 0
            || !valid_connection_id(&self.connection_id)
            || !valid_identifier(&self.profile_id)
            || !is_absolute_path(&self.executable)
            || !is_absolute_path(&self.helper_executable)
            || !is_numeric_loopback_origin(&self.gateway_base_url)
            || self.helper_argv
                != [
                    HIDDEN_AGENT_GRANT_HELPER_VERB_V1.to_owned(),
                    self.connection_id.clone(),
                ]
            || self.environment_removals
                != MANAGED_CLAUDE_ENVIRONMENT_REMOVALS_V1
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
        {
            return Err(ManagedLaunchError::InvalidDescriptor);
        }
        CanonicalDigest::parse(self.snapshot_digest.as_str().to_owned())
            .map_err(|_| ManagedLaunchError::InvalidDescriptor)?;
        CanonicalDigest::parse(self.publication_digest.as_str().to_owned())
            .map_err(|_| ManagedLaunchError::InvalidDescriptor)?;
        [
            &self.presets.opus,
            &self.presets.sonnet,
            &self.presets.haiku,
        ]
        .into_iter()
        .flatten()
        .all(|value| valid_client_model_name(value))
        .then_some(())
        .ok_or(ManagedLaunchError::InvalidDescriptor)?;
        Ok(())
    }

    pub fn validate_trusted_helper(
        &self,
        trusted_hiroute_executable: &str,
    ) -> Result<(), ManagedLaunchError> {
        self.validate()?;
        if self.helper_executable != trusted_hiroute_executable {
            return Err(ManagedLaunchError::UntrustedHelper);
        }
        Ok(())
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains("..")
        && !value.contains("//")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
}

fn valid_connection_id(value: &str) -> bool {
    value
        .strip_prefix("agent-connection/")
        .is_some_and(|suffix| !suffix.is_empty())
        && value.len() <= 256
        && !value.contains("..")
        && !value.contains("//")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
        })
}

fn is_absolute_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value.contains('\0')
        && (value.starts_with('/')
            || (value.len() >= 3
                && value.as_bytes()[1] == b':'
                && matches!(value.as_bytes()[2], b'/' | b'\\')))
}

fn is_numeric_loopback_origin(value: &str) -> bool {
    let Some(authority) = value.strip_prefix("http://") else {
        return false;
    };
    let port = authority
        .strip_prefix("127.0.0.1:")
        .or_else(|| authority.strip_prefix("[::1]:"));
    port.is_some_and(|port| {
        !port.is_empty()
            && port.bytes().all(|byte| byte.is_ascii_digit())
            && port.parse::<u16>().is_ok_and(|port| port != 0)
    })
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum ManagedLaunchError {
    #[error("managed-launch profile is not the compiled trusted capability")]
    UntrustedProfile,
    #[error("managed-launch descriptor is invalid")]
    InvalidDescriptor,
    #[error("managed-launch helper executable is not the current trusted hiroute executable")]
    UntrustedHelper,
}
