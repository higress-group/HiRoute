//! Complete the original confirmed settings Operation before reporting collaboration enabled.
//!
//! Worker CLI selection is a local same-UID channel, so settings persist Plan policy in the
//! successful Operation and install only the routing Skill. No collaboration credential is
//! created. A legacy sealed artifact owned by this feature is removed on the next explicit
//! Configure/Restore so obsolete secret material does not linger.

use super::{LocalControlAdapter, settings_facts::SettingsAgentClass};
use hiroute_application::control::ControlReadError;
use hiroute_domain::{
    AgentFacetIntent, AgentSettingsSpecV2, OperationState, OperationV1, PortError, PortErrorCode,
    PortResult,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Take},
    path::PathBuf,
};
use zeroize::Zeroizing;

fn error() -> PortError {
    PortError::new(PortErrorCode::Conflict, "collaboration.settings")
}

impl LocalControlAdapter {
    pub(super) fn settings_collaboration_state(
        &self,
        spec: &AgentSettingsSpecV2,
    ) -> Result<Value, ControlReadError> {
        match &spec.collaboration {
            AgentFacetIntent::Keep => Ok(Value::Null),
            AgentFacetIntent::Configure { settings } => Ok(json!({
                "worker_channel": "local_trust_v1",
                "trigger_mode": settings.trigger_mode,
            })),
            AgentFacetIntent::Restore { restore_point_ref } => Ok(json!({
                "worker_channel": "local_trust_v1",
                "restore_point_ref": restore_point_ref,
            })),
        }
    }

    pub(super) fn finish_settings_collaboration(&self, operation: &OperationV1) -> PortResult<()> {
        if operation.state != OperationState::Succeeded
            || operation.plan.spec().command_id != "agents.settings.apply"
        {
            return Ok(());
        }
        let spec: AgentSettingsSpecV2 =
            serde_json::from_value(operation.plan.spec().desired_state.clone())
                .map_err(|_| error())?;
        if matches!(spec.collaboration, AgentFacetIntent::Keep) {
            return Ok(());
        }
        let state = operation
            .plan
            .control()
            .pointer("/payload/state/mutations/collaboration")
            .ok_or_else(error)?;
        if state["worker_channel"] != "local_trust_v1" {
            return Err(error());
        }
        if let AgentFacetIntent::Configure { settings } = &spec.collaboration
            && state["trigger_mode"]
                != serde_json::to_value(settings.trigger_mode).map_err(|_| error())?
        {
            return Err(error());
        }
        let class = self
            .settings_agent_for_context(&spec.context_id)
            .ok_or_else(error)?;
        let skill_target = class.skill_target().map_err(|_| error())?;
        {
            let stores = self.stores_lock()?;
            hiroute_application::agent_connection::persist_settings_skill_reference_change(
                stores.control(),
                &self.artifacts,
                operation,
                &skill_target,
            )?;
        }
        // Only historical clients ever owned sealed collaboration credentials.
        if matches!(class, SettingsAgentClass::Qoder | SettingsAgentClass::Pi) {
            return Ok(());
        }
        let legacy = self.legacy_collaboration_artifact_path(&spec.context_id)?;
        if !legacy.exists() {
            return Ok(());
        }
        let encoded = read_private(&legacy)?;
        let (grant, _material) = self
            .stores_lock()?
            .secrets()
            .open_collaboration_bootstrap(&encoded)?;
        if grant.workspace_id != operation.workspace_id
            || grant.context_id != spec.context_id
            || grant.grant_id != format!("collaboration-grant/{}", spec.context_id)
        {
            return Err(error());
        }
        fs::remove_file(legacy).map_err(|_| error())
    }

    fn legacy_collaboration_artifact_path(&self, context: &str) -> PortResult<PathBuf> {
        let class = self.settings_agent_for_context(context).ok_or_else(error)?;
        let native = match class {
            SettingsAgentClass::Codex | SettingsAgentClass::CodexProfile => {
                self.scanner.codex_user_config_target()
            }
            SettingsAgentClass::Claude => self.scanner.claude_user_settings_target(),
            SettingsAgentClass::Qoder | SettingsAgentClass::Pi => return Err(error()),
        };
        let home = native
            .parent()
            .and_then(|path| path.parent())
            .ok_or_else(error)?;
        Ok(home
            .join(".hiroute")
            .join("credential-artifacts")
            .join(match class {
                SettingsAgentClass::Codex | SettingsAgentClass::CodexProfile => "codex.sealed",
                SettingsAgentClass::Claude => "claude-code.sealed",
                SettingsAgentClass::Qoder | SettingsAgentClass::Pi => return Err(error()),
            }))
    }
}

fn read_private(path: &std::path::Path) -> PortResult<Zeroizing<String>> {
    let metadata = fs::symlink_metadata(path).map_err(|_| error())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(error());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != nix::unistd::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
            return Err(error());
        }
    }
    let mut encoded = Zeroizing::new(String::new());
    let mut file: Take<fs::File> = fs::File::open(path).map_err(|_| error())?.take(4097);
    file.read_to_string(&mut encoded).map_err(|_| error())?;
    if encoded.len() > 4096 {
        return Err(error());
    }
    Ok(encoded)
}
