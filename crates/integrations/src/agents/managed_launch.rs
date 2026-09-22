use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

use hiroute_domain::{
    ConfigLayerV1, MANAGED_CLAUDE_ENVIRONMENT_REMOVALS_V1, MANAGED_CLAUDE_SETTINGS_ARGUMENT_V1,
    ManagedClaudeLaunchDescriptorV2, ManagedLaunchProfileV1,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub const MANAGED_LAUNCH_ENV_SANITIZED_V1: &str = "MANAGED_LAUNCH_ENV_SANITIZED";
pub const AGENT_AUTH_PRECEDENCE_CONFLICT_V1: &str = "AGENT_AUTH_PRECEDENCE_CONFLICT";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentAuthRoutingFieldV1 {
    AnthropicApiKey,
    AnthropicAuthToken,
    AnthropicBaseUrl,
    AnthropicCustomHeaders,
    AnthropicModel,
    ApiKeyHelper,
    Bedrock,
    Foundry,
    Vertex,
}

impl AgentAuthRoutingFieldV1 {
    pub(crate) fn from_environment_name(name: &str) -> Option<Self> {
        match name {
            "ANTHROPIC_API_KEY" => Some(Self::AnthropicApiKey),
            "ANTHROPIC_AUTH_TOKEN" => Some(Self::AnthropicAuthToken),
            "ANTHROPIC_BASE_URL" => Some(Self::AnthropicBaseUrl),
            "ANTHROPIC_CUSTOM_HEADERS" => Some(Self::AnthropicCustomHeaders),
            "ANTHROPIC_MODEL" => Some(Self::AnthropicModel),
            "CLAUDE_CODE_USE_BEDROCK" => Some(Self::Bedrock),
            "CLAUDE_CODE_USE_FOUNDRY" => Some(Self::Foundry),
            "CLAUDE_CODE_USE_VERTEX" => Some(Self::Vertex),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedLaunchWarningV1 {
    pub code: String,
    pub environment_removals: BTreeSet<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentAuthPrecedenceConflictV1 {
    pub code: String,
    pub layer: ConfigLayerV1,
    pub source_ref: String,
    pub fields: BTreeSet<AgentAuthRoutingFieldV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedClaudeLaunchPreflightV1 {
    pub executable: String,
    pub inherited_environment_removals: BTreeSet<String>,
    pub warnings: Vec<ManagedLaunchWarningV1>,
    pub conflicts: Vec<AgentAuthPrecedenceConflictV1>,
}

impl ManagedClaudeLaunchPreflightV1 {
    pub(super) fn from_observations(
        executable: String,
        profile: &ManagedLaunchProfileV1,
        observations: &[super::filesystem_config::ObservedClaudeSettings],
    ) -> Self {
        let inherited_environment_removals = observations
            .iter()
            .filter(|observation| observation.layer == ConfigLayerV1::Process)
            .flat_map(|observation| observation.settings.env.present_environment_fields.iter())
            .filter(|name| profile.environment_removals.contains(*name))
            .cloned()
            .collect::<BTreeSet<_>>();
        let warnings = (!inherited_environment_removals.is_empty())
            .then(|| ManagedLaunchWarningV1 {
                code: MANAGED_LAUNCH_ENV_SANITIZED_V1.to_owned(),
                environment_removals: inherited_environment_removals.clone(),
            })
            .into_iter()
            .collect();
        let conflicts = observations
            .iter()
            // The managed launcher removes the complete process auth set and merges any explicit
            // caller --settings document into its private overlay. Normal setting-source, project,
            // MCP, permission and Skill semantics stay intact; only the routing/auth fields are
            // replaced. Launch facts (if an embedding supplies one) and machine-managed policy
            // cannot be excluded by that boundary and stay fatal.
            .filter(|observation| {
                matches!(
                    observation.layer,
                    ConfigLayerV1::Launch | ConfigLayerV1::Managed
                )
            })
            .filter_map(|observation| {
                let mut fields = observation
                    .settings
                    .env
                    .present_environment_fields
                    .iter()
                    .filter_map(|name| AgentAuthRoutingFieldV1::from_environment_name(name))
                    .collect::<BTreeSet<_>>();
                if observation.settings.api_key_helper_present {
                    fields.insert(AgentAuthRoutingFieldV1::ApiKeyHelper);
                }
                (!fields.is_empty()).then(|| AgentAuthPrecedenceConflictV1 {
                    code: AGENT_AUTH_PRECEDENCE_CONFLICT_V1.to_owned(),
                    layer: observation.layer,
                    source_ref: observation.source_ref.clone(),
                    fields,
                })
            })
            .collect();
        Self {
            executable,
            inherited_environment_removals,
            warnings,
            conflicts,
        }
    }

    pub fn is_launchable(&self) -> bool {
        self.conflicts.is_empty()
    }

    pub fn complete_environment_removals() -> BTreeSet<String> {
        MANAGED_CLAUDE_ENVIRONMENT_REMOVALS_V1
            .into_iter()
            .map(str::to_owned)
            .collect()
    }
}

/// One prepared managed Claude process. The private overlay remains owned until this value is
/// dropped, so both the interactive CLI and the bounded Live executor use the same routing,
/// credential-destination and cleanup boundary.
pub struct ManagedClaudeProcessV1 {
    command: Command,
    _overlay: TemporaryLaunchOverlay,
}

impl ManagedClaudeProcessV1 {
    pub fn prepare(
        descriptor: &ManagedClaudeLaunchDescriptorV2,
        user_settings: &Value,
        child_arguments: &[OsString],
        trusted_hiroute_executable: &str,
    ) -> Result<Self, ManagedClaudeProcessError> {
        descriptor
            .validate_trusted_helper(trusted_hiroute_executable)
            .map_err(|_| ManagedClaudeProcessError::Unavailable)?;
        verify_claude_executable(descriptor)?;
        let overlay = TemporaryLaunchOverlay::create(descriptor, user_settings)?;
        let mut command = Command::new(&descriptor.executable);
        configure_command(&mut command, descriptor, overlay.path(), child_arguments);
        Ok(Self {
            command,
            _overlay: overlay,
        })
    }

    pub fn command_mut(&mut self) -> &mut Command {
        &mut self.command
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedClaudeProcessError {
    Unavailable,
}

fn configure_command(
    command: &mut Command,
    descriptor: &ManagedClaudeLaunchDescriptorV2,
    overlay: &Path,
    child_arguments: &[OsString],
) {
    for name in &descriptor.environment_removals {
        command.env_remove(name);
        if !matches!(name.as_str(), "ANTHROPIC_BASE_URL" | "ANTHROPIC_MODEL") {
            command.env(name, "");
        }
    }
    for name in [
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    ] {
        command.env_remove(name);
    }
    command.env("ANTHROPIC_BASE_URL", &descriptor.gateway_base_url);
    if let Some(opus) = &descriptor.presets.opus {
        command.env("ANTHROPIC_DEFAULT_OPUS_MODEL", opus);
    }
    if let Some(sonnet) = &descriptor.presets.sonnet {
        command.env("ANTHROPIC_DEFAULT_SONNET_MODEL", sonnet);
    }
    if let Some(haiku) = &descriptor.presets.haiku {
        command.env("ANTHROPIC_DEFAULT_HAIKU_MODEL", haiku);
    }
    if let Some(window) = descriptor.context_window_tokens {
        for key in hiroute_domain::CLAUDE_CONTEXT_CONFLICT_ENVIRONMENT {
            command.env_remove(key);
        }
        command.envs(hiroute_domain::claude_context_environment(window).expect("validated window"));
    }
    command
        .arg(MANAGED_CLAUDE_SETTINGS_ARGUMENT_V1)
        .arg(overlay)
        .args(child_arguments);
}

fn verify_claude_executable(
    descriptor: &ManagedClaudeLaunchDescriptorV2,
) -> Result<(), ManagedClaudeProcessError> {
    let canonical = std::fs::canonicalize(&descriptor.executable)
        .map_err(|_| ManagedClaudeProcessError::Unavailable)?;
    if canonical.as_os_str() != OsStr::new(&descriptor.executable) {
        return Err(ManagedClaudeProcessError::Unavailable);
    }
    let metadata = canonical
        .metadata()
        .map_err(|_| ManagedClaudeProcessError::Unavailable)?;
    if !metadata.is_file() {
        return Err(ManagedClaudeProcessError::Unavailable);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(ManagedClaudeProcessError::Unavailable);
        }
    }
    Ok(())
}

struct TemporaryLaunchOverlay {
    directory: PathBuf,
    path: PathBuf,
}

impl TemporaryLaunchOverlay {
    fn create(
        descriptor: &ManagedClaudeLaunchDescriptorV2,
        user_settings: &Value,
    ) -> Result<Self, ManagedClaudeProcessError> {
        #[cfg(not(unix))]
        {
            let _ = (descriptor, user_settings);
            return Err(ManagedClaudeProcessError::Unavailable);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

            let root = std::env::temp_dir();
            let mut directory = None;
            for _ in 0..32 {
                let mut entropy = [0_u8; 16];
                getrandom::fill(&mut entropy)
                    .map_err(|_| ManagedClaudeProcessError::Unavailable)?;
                let candidate = root.join(format!(
                    "hiroute-claude-launch-{:032x}",
                    u128::from_ne_bytes(entropy)
                ));
                let mut builder = std::fs::DirBuilder::new();
                builder.mode(0o700);
                match builder.create(&candidate) {
                    Ok(()) => {
                        directory = Some(candidate);
                        break;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(_) => return Err(ManagedClaudeProcessError::Unavailable),
                }
            }
            let directory = directory.ok_or(ManagedClaudeProcessError::Unavailable)?;
            let path = directory.join("settings.json");
            let overlay = Self { directory, path };
            std::fs::set_permissions(&overlay.directory, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| ManagedClaudeProcessError::Unavailable)?;
            let mut options = OpenOptions::new();
            options.write(true).create_new(true).mode(0o600);
            let mut file = options
                .open(&overlay.path)
                .map_err(|_| ManagedClaudeProcessError::Unavailable)?;
            write_overlay(&mut file, descriptor, user_settings)?;
            file.sync_all()
                .map_err(|_| ManagedClaudeProcessError::Unavailable)?;
            std::fs::set_permissions(&overlay.path, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| ManagedClaudeProcessError::Unavailable)?;
            Ok(overlay)
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryLaunchOverlay {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.directory);
    }
}

fn write_overlay(
    file: &mut File,
    descriptor: &ManagedClaudeLaunchDescriptorV2,
    user_settings: &Value,
) -> Result<(), ManagedClaudeProcessError> {
    let mut components = Vec::with_capacity(1 + descriptor.helper_argv.len());
    components.push(descriptor.helper_executable.as_str());
    components.extend(descriptor.helper_argv.iter().map(String::as_str));
    let helper_command = components
        .into_iter()
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ");
    let mut overlay = match user_settings {
        Value::Object(fields) => fields.clone(),
        _ => Map::new(),
    };
    overlay.remove("apiKeyHelper");
    let mut env = overlay
        .remove("env")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    for name in MANAGED_CLAUDE_ENVIRONMENT {
        env.remove(name);
    }
    let mut managed_env = Map::new();
    for name in &descriptor.environment_removals {
        if !matches!(name.as_str(), "ANTHROPIC_BASE_URL" | "ANTHROPIC_MODEL") {
            managed_env.insert(name.clone(), json!(""));
        }
    }
    managed_env.insert(
        "ANTHROPIC_BASE_URL".to_owned(),
        json!(descriptor.gateway_base_url),
    );
    for (name, value) in [
        ("ANTHROPIC_DEFAULT_OPUS_MODEL", &descriptor.presets.opus),
        ("ANTHROPIC_DEFAULT_SONNET_MODEL", &descriptor.presets.sonnet),
        ("ANTHROPIC_DEFAULT_HAIKU_MODEL", &descriptor.presets.haiku),
    ] {
        managed_env.insert(name.to_owned(), json!(value.as_deref().unwrap_or("")));
    }
    for (name, value) in managed_env {
        env.insert(name, value);
    }
    if let Some(window) = descriptor.context_window_tokens {
        for key in hiroute_domain::CLAUDE_CONTEXT_CONFLICT_ENVIRONMENT {
            env.remove(key);
        }
        for (key, value) in
            hiroute_domain::claude_context_environment(window).expect("validated window")
        {
            env.insert(key, json!(value));
        }
    }
    overlay.insert("env".to_owned(), Value::Object(env));
    overlay.insert("apiKeyHelper".to_owned(), json!(helper_command));
    serde_json::to_writer(file, &Value::Object(overlay))
        .map_err(|_| ManagedClaudeProcessError::Unavailable)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

const MANAGED_CLAUDE_ENVIRONMENT: [&str; 11] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_CUSTOM_HEADERS",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_VERTEX",
];

#[cfg(test)]
mod context_window_tests {
    use super::*;

    #[test]
    fn managed_claude_context_window_overrides_caller_settings_and_process_environment() {
        let mut descriptor = ManagedClaudeLaunchDescriptorV2::trusted(
            "agent-connection/context-test",
            "claude-messages-v1",
            "/trusted/claude",
            hiroute_domain::CanonicalDigest::of_bytes(b"snapshot"),
            1,
            hiroute_domain::CanonicalDigest::of_bytes(b"publication"),
            "http://127.0.0.1:4321",
            hiroute_domain::AgentClaudePresetValuesV2 {
                opus: Some("hiroute-plan".into()),
                sonnet: None,
                haiku: None,
            },
            "/trusted/hiroute",
        )
        .unwrap();
        descriptor.context_window_tokens = Some(272_000);
        descriptor.validate().unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        write_overlay(&mut file.reopen().unwrap(), &descriptor, &json!({"theme":"dark", "env":{"DISABLE_COMPACT":"1", "CLAUDE_CODE_AUTO_COMPACT_WINDOW":"900000", "UNRELATED":"keep"}})).unwrap();
        let overlay: Value = serde_json::from_reader(file.reopen().unwrap()).unwrap();
        assert_eq!(overlay["theme"], "dark");
        assert_eq!(overlay["env"]["UNRELATED"], "keep");
        assert_eq!(overlay["env"]["CLAUDE_CODE_AUTO_COMPACT_WINDOW"], "272000");
        assert_eq!(overlay["env"]["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], "272000");
        assert!(overlay["env"].get("DISABLE_COMPACT").is_none());
        let mut command = Command::new("/trusted/claude");
        configure_command(&mut command, &descriptor, file.path(), &[]);
        let environment: std::collections::BTreeMap<_, _> = command.get_envs().collect();
        assert_eq!(
            environment[OsStr::new("CLAUDE_CODE_AUTO_COMPACT_WINDOW")],
            Some(OsStr::new("272000"))
        );
        assert_eq!(environment[OsStr::new("DISABLE_COMPACT")], None);
    }
}
