use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

/// Verify the exact owner-local paths in a discovery fixture before excluding them
/// from its privacy assertion. All other fields remain visible to the caller's scan.
pub fn redact_discovery_connection_paths(
    agents: &mut [Value],
    agent_home: &Path,
    instance_root: &Path,
) -> bool {
    let home = agent_home.join(".codex");
    let Some(access) = agents
        .iter_mut()
        .find(|agent| agent["agent_id"] == "agent_codex_default")
        .and_then(|agent| agent["codex_access"].as_object_mut())
    else {
        return false;
    };
    if access.remove("codex_home") != Some(json!(home))
        || access.remove("target_file") != Some(json!(home.join("hiroute.config.toml")))
    {
        return false;
    }
    let Some(Value::Object(commands)) = access.remove("commands") else {
        return false;
    };
    if commands.len() != 3
        || !["bash/zsh", "fish", "PowerShell"].iter().all(|shell| {
            commands
                .get(*shell)
                .and_then(Value::as_str)
                .is_some_and(|command| {
                    command.contains(home.to_string_lossy().as_ref())
                        && command.contains("codex --profile hiroute")
                })
        })
    {
        return false;
    }
    let Some(desktop) = agents
        .iter_mut()
        .find(|agent| agent["agent_id"] == "agent_codex_desktop")
        .and_then(Value::as_object_mut)
    else {
        return false;
    };
    desktop.remove("codex_desktop")
        == Some(json!({
            "codex_home": instance_root.join("codex"),
            "user_data": instance_root.join("desktop"),
            "source_codex_home": home,
            "shared_skills": agent_home.join(".agents/skills"),
        }))
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioState {
    Green,
    ExpectedRed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlShellScenarioV1 {
    pub schema: String,
    pub process: String,
    pub scenario_id: String,
    pub command_states: Vec<ControlCommandStateV1>,
    pub gateway_bound_state: ScenarioState,
    pub gateway_bound_evidence: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlCommandStateV1 {
    pub command_id: String,
    pub state: ScenarioState,
    pub expected_exit: u8,
    pub expected_status: String,
    pub expected_error_code: Option<String>,
}

impl ControlShellScenarioV1 {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != "hiroute.control-shell-scenario/v1"
            || self.process != "PROCESS-25016"
            || self.command_states.is_empty()
            || self.gateway_bound_state != ScenarioState::Green
            || self.gateway_bound_evidence != "installed-standalone-headless-management-loop"
        {
            return Err("invalid control-shell scenario boundary");
        }
        if self.command_states.iter().any(|state| {
            state.command_id.is_empty()
                || state.expected_status.is_empty()
                || state.state != ScenarioState::Green
                || match state.expected_status.as_str() {
                    "succeeded" => state.expected_exit != 0 || state.expected_error_code.is_some(),
                    "usage_error" => {
                        state.expected_exit != 2
                            || state.expected_error_code.as_deref() != Some("UNKNOWN_COMMAND")
                    }
                    "not_found" => {
                        state.expected_exit != 5
                            || state.expected_error_code.as_deref() != Some("RESOURCE_NOT_FOUND")
                    }
                    _ => true,
                }
        }) {
            return Err("control-shell command state is incomplete");
        }
        Ok(())
    }
}
