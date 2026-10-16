//! Bundled Hermes resources are sealed in the same admitted model Operation.
use hiroute_domain::{
    AgentConnectionControlIntentV1, AgentConnectionEffectRoleV1, AgentOperationRead,
    CanonicalDigest, ExternalEffectIntentV1, OperationId, OperationValidationError, PortError,
    PortErrorCode, PortResult,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct HermesPluginFileFacts {
    pub role: AgentConnectionEffectRoleV1,
    pub before_fingerprint: Option<CanonicalDigest>,
    pub expected_content: CanonicalDigest,
    pub content: &'static str,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HermesPluginFilePayload {
    pub schema: String,
    pub context_id: String,
    pub expected_content: CanonicalDigest,
    pub change: HermesPluginFileAction,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum HermesPluginFileAction {
    Configure {
        previous_operation: Option<OperationId>,
        content: String,
    },
    Restore {
        original_operation: OperationId,
    },
}

pub fn settings_hermes_plugin_file_intent(
    control: &AgentConnectionControlIntentV1,
    context: &str,
    facts: &HermesPluginFileFacts,
    change: &super::HermesModelFileAction,
) -> Result<ExternalEffectIntentV1, OperationValidationError> {
    let change = match change {
        super::HermesModelFileAction::Configure {
            previous_operation, ..
        } => HermesPluginFileAction::Configure {
            previous_operation: previous_operation.clone(),
            content: facts.content.into(),
        },
        super::HermesModelFileAction::Restore { original_operation } => {
            HermesPluginFileAction::Restore {
                original_operation: original_operation.clone(),
            }
        }
    };
    let payload = HermesPluginFilePayload {
        schema: "hiroute.settings-hermes-plugin-file/v1".into(),
        context_id: context.into(),
        expected_content: facts.expected_content.clone(),
        change,
    };
    let intent = ExternalEffectIntentV1::from_agent_connection_planner(
        control,
        facts.role,
        facts.before_fingerprint.clone(),
        &payload,
        0o644,
    )?;
    decode_settings_hermes_plugin_file(&intent)
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    Ok(intent)
}

pub fn is_settings_hermes_plugin(intent: &ExternalEffectIntentV1) -> bool {
    matches!(
        intent.effect_id(),
        "agent-connection-hermes-plugin-module" | "agent-connection-hermes-plugin-manifest"
    )
}

pub fn decode_settings_hermes_plugin_file(
    intent: &ExternalEffectIntentV1,
) -> PortResult<HermesPluginFilePayload> {
    let invalid = || PortError::new(PortErrorCode::InvalidData, "hermes.plugin.intent");
    ExternalEffectIntentV1::from_registered_adapter(
        intent.effect_id(),
        intent.kind(),
        intent.target(),
        intent.before_fingerprint().cloned(),
        intent.desired().clone(),
        intent.desired_mode(),
        intent.sensitive(),
    )
    .map_err(|_| invalid())?;
    if !is_settings_hermes_plugin(intent)
        || intent.desired_mode() != 0o644
        || intent.sensitive()
        || intent.desired()["transaction"] != "settings"
        || intent.desired()["subject"]["agent_id"] != "agent_hermes_default"
        || intent.desired()["subject"]["integration_profile_ref"] != "builtin/hermes-main/v1"
    {
        return Err(invalid());
    }
    let payload: HermesPluginFilePayload =
        serde_json::from_value(intent.desired()["payload"].clone()).map_err(|_| invalid())?;
    let suffix = payload
        .context_id
        .strip_prefix("agent-context/hermes/sha256:")
        .ok_or_else(invalid)?;
    if payload.schema != "hiroute.settings-hermes-plugin-file/v1"
        || suffix.len() != 64
        || intent.desired()["subject"]["profile_id"] != format!("hermes-main-{suffix}")
    {
        return Err(invalid());
    }
    if let HermesPluginFileAction::Configure { content, .. } = &payload.change
        && (content.is_empty() || content.len() > 16 * 1024 || content.contains('\0'))
    {
        return Err(invalid());
    }
    Ok(payload)
}

pub fn settings_hermes_plugin_file_for_operation(
    op: &(impl AgentOperationRead + ?Sized),
    intent: &ExternalEffectIntentV1,
) -> PortResult<HermesPluginFilePayload> {
    let invalid = || PortError::new(PortErrorCode::InvalidData, "hermes.plugin.operation");
    let payload = decode_settings_hermes_plugin_file(intent)?;
    let input = op.agent_input();
    if input.spec.command_id != "agents.settings.apply"
        || input.spec.resource_id.as_deref() != Some(&payload.context_id)
        || !input.external.contains(intent)
    {
        return Err(invalid());
    }
    let model_intent = input
        .external
        .iter()
        .find(|candidate| super::is_settings_hermes_model(candidate))
        .ok_or_else(invalid)?;
    let model = super::settings_hermes_model_file_for_operation(op, model_intent)?;
    match (&model.change, &payload.change) {
        (
            super::HermesModelFileAction::Configure {
                previous_operation: a,
                ..
            },
            HermesPluginFileAction::Configure {
                previous_operation: b,
                ..
            },
        ) if a == b => {}
        (
            super::HermesModelFileAction::Restore {
                original_operation: a,
            },
            HermesPluginFileAction::Restore {
                original_operation: b,
            },
        ) if a == b => {}
        _ => return Err(invalid()),
    }
    Ok(payload)
}
