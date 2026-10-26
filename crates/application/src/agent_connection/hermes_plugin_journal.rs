//! Bundled native resources are sealed in the same admitted model Operation.
use hiroute_domain::{
    AgentConnectionControlIntentV1, AgentConnectionEffectRoleV1, AgentOperationRead,
    CanonicalDigest, ExternalEffectIntentV1, OperationId, OperationValidationError, PortError,
    PortErrorCode, PortResult,
};
use serde::{Deserialize, Serialize};

const MAX_NATIVE_PLUGIN_CONTENT_BYTES: usize = 32 * 1024;

#[derive(Clone)]
pub struct NativePluginFileFacts {
    pub role: AgentConnectionEffectRoleV1,
    pub before_fingerprint: Option<CanonicalDigest>,
    pub expected_content: CanonicalDigest,
    pub content: &'static str,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativePluginFilePayload {
    pub schema: String,
    pub context_id: String,
    pub expected_content: CanonicalDigest,
    pub change: NativePluginFileAction,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativePluginFileAction {
    Configure {
        previous_operation: Option<OperationId>,
        content: String,
    },
    Restore {
        original_operation: OperationId,
    },
}

pub fn settings_native_plugin_file_intent(
    control: &AgentConnectionControlIntentV1,
    context: &str,
    facts: &NativePluginFileFacts,
    change: &NativePluginFileAction,
) -> Result<ExternalEffectIntentV1, OperationValidationError> {
    let payload = NativePluginFilePayload {
        schema: if facts.role == AgentConnectionEffectRoleV1::NativeRoutingAdapter {
            "hiroute.settings-native-plugin-file/v1"
        } else {
            "hiroute.settings-hermes-plugin-file/v1"
        }
        .into(),
        context_id: context.into(),
        expected_content: facts.expected_content.clone(),
        change: change.clone(),
    };
    let intent = ExternalEffectIntentV1::from_agent_connection_planner(
        control,
        facts.role,
        facts.before_fingerprint.clone(),
        &payload,
        0o644,
    )?;
    decode_settings_native_plugin_file(&intent)
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    Ok(intent)
}

pub fn is_settings_native_plugin(intent: &ExternalEffectIntentV1) -> bool {
    matches!(
        intent.effect_id(),
        "agent-connection-hermes-plugin-module"
            | "agent-connection-hermes-plugin-manifest"
            | "agent-connection-native-routing-adapter"
    )
}

pub fn decode_settings_native_plugin_file(
    intent: &ExternalEffectIntentV1,
) -> PortResult<NativePluginFilePayload> {
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
    if !is_settings_native_plugin(intent)
        || intent.desired_mode() != 0o644
        || intent.sensitive()
        || intent.desired()["transaction"] != "settings"
    {
        return Err(invalid());
    }
    let payload: NativePluginFilePayload =
        serde_json::from_value(intent.desired()["payload"].clone()).map_err(|_| invalid())?;
    if intent.effect_id() == "agent-connection-native-routing-adapter" {
        let kind = intent.desired()["subject"]["agent_id"]
            .as_str()
            .and_then(|agent| match agent {
                "agent_pi_default" => Some(("pi", "pi-responses-v1", "builtin/pi-responses/v1")),
                "agent_dsh_default" | "agent_dsh_desktop" => {
                    Some(("dsh", "dsh-responses-v1", "builtin/dsh-responses/v1"))
                }
                "agent_opencode_default" => Some((
                    "opencode",
                    "opencode-responses-v1",
                    "builtin/opencode-responses/v1",
                )),
                _ => None,
            })
            .ok_or_else(invalid)?;
        if payload.schema != "hiroute.settings-native-plugin-file/v1"
            || !payload
                .context_id
                .starts_with(&format!("agent-context/{}/", kind.0))
            || intent.desired()["subject"]["profile_id"] != kind.1
            || intent.desired()["subject"]["integration_profile_ref"] != kind.2
        {
            return Err(invalid());
        }
    } else {
        let suffix = payload
            .context_id
            .strip_prefix("agent-context/hermes/sha256:")
            .ok_or_else(invalid)?;
        if payload.schema != "hiroute.settings-hermes-plugin-file/v1"
            || suffix.len() != 64
            || intent.desired()["subject"]["agent_id"] != "agent_hermes_default"
            || intent.desired()["subject"]["integration_profile_ref"] != "builtin/hermes-main/v1"
            || intent.desired()["subject"]["profile_id"] != format!("hermes-main-{suffix}")
        {
            return Err(invalid());
        }
    }
    if let NativePluginFileAction::Configure { content, .. } = &payload.change
        && (content.is_empty()
            || content.len() > MAX_NATIVE_PLUGIN_CONTENT_BYTES
            || content.contains('\0'))
    {
        return Err(invalid());
    }
    Ok(payload)
}

#[cfg(test)]
#[path = "hermes_plugin_journal_tests.rs"]
mod tests;

pub fn settings_native_plugin_file_for_operation(
    op: &(impl AgentOperationRead + ?Sized),
    intent: &ExternalEffectIntentV1,
) -> PortResult<NativePluginFilePayload> {
    let invalid = || PortError::new(PortErrorCode::InvalidData, "hermes.plugin.operation");
    let payload = decode_settings_native_plugin_file(intent)?;
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
        .find(|candidate| candidate.effect_id() == "agent-connection-managed-configuration")
        .ok_or_else(invalid)?;
    if intent.effect_id() == "agent-connection-native-routing-adapter" {
        let model = super::settings_additional_model_file_for_operation(op, model_intent)?;
        let same = match (&model.change, &payload.change) {
            (
                super::AdditionalModelFileAction::Configure {
                    previous_operation: a,
                    ..
                },
                NativePluginFileAction::Configure {
                    previous_operation: b,
                    ..
                },
            ) => a == b,
            (
                super::AdditionalModelFileAction::Restore {
                    original_operation: a,
                },
                NativePluginFileAction::Restore {
                    original_operation: b,
                },
            ) => a == b,
            _ => false,
        };
        if !same || model.context_id != payload.context_id {
            return Err(invalid());
        }
        return Ok(payload);
    }
    let model = super::settings_hermes_model_file_for_operation(op, model_intent)?;
    match (&model.change, &payload.change) {
        (
            super::HermesModelFileAction::Configure {
                previous_operation: a,
                ..
            },
            NativePluginFileAction::Configure {
                previous_operation: b,
                ..
            },
        ) if a == b => {}
        (
            super::HermesModelFileAction::Restore {
                original_operation: a,
            },
            NativePluginFileAction::Restore {
                original_operation: b,
            },
        ) if a == b => {}
        _ => return Err(invalid()),
    }
    Ok(payload)
}
