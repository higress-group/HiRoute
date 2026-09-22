use hiroute_domain::{
    AgentClaudePresetValuesV2, AgentConfigChangeV1, AgentConnectionControlIntentV1,
    AgentConnectionEffectRoleV1, AgentConnectionTransactionKindV1, AgentFacetIntent,
    AgentModelGrantV2, AgentSettingsSpecV2, CanonicalDigest, ExternalEffectIntentV1, OperationId,
    OperationV1, OperationValidationError, PortError, PortErrorCode, PortResult,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

const SCHEMA: &str = "hiroute.settings-claude-model-file/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClaudeLaunchSnapshotIntent {
    pub native_presets: AgentClaudePresetValuesV2,
    pub presets: AgentClaudePresetValuesV2,
    pub executable: String,
    pub installation_digest: CanonicalDigest,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClaudeModelFileAction {
    Configure {
        previous_operation: Option<OperationId>,
        change: AgentConfigChangeV1,
        snapshot: Box<ClaudeLaunchSnapshotIntent>,
        gateway_base_url: String,
        trusted_hiroute_executable: String,
    },
    Restore {
        original_operation: OperationId,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClaudeModelFilePayload {
    schema: String,
    pub context_id: String,
    pub expected_content: CanonicalDigest,
    pub change: ClaudeModelFileAction,
}

pub fn settings_claude_model_file_intent(
    control: &AgentConnectionControlIntentV1,
    context_id: &str,
    expected_content: CanonicalDigest,
    before_fingerprint: Option<CanonicalDigest>,
    change: ClaudeModelFileAction,
) -> Result<ExternalEffectIntentV1, OperationValidationError> {
    if control.transaction() != AgentConnectionTransactionKindV1::Settings {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let payload = ClaudeModelFilePayload {
        schema: SCHEMA.into(),
        context_id: context_id.into(),
        expected_content,
        change,
    };
    validate_payload(&payload)?;
    let intent = ExternalEffectIntentV1::from_agent_connection_planner(
        control,
        AgentConnectionEffectRoleV1::ManagedConfiguration,
        before_fingerprint,
        &payload,
        0o600,
    )?;
    decode_settings_claude_model_file(&intent)
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    Ok(intent)
}

pub fn decode_settings_claude_model_file(
    intent: &ExternalEffectIntentV1,
) -> PortResult<ClaudeModelFilePayload> {
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
    let value = intent.desired();
    if intent.effect_id() != "agent-connection-managed-configuration"
        || intent.desired_mode() != 0o600
        || value["transaction"] != "settings"
        || value["subject"]["agent_id"] != "agent_claude_default"
        || value["subject"]["profile_id"] != "claude-messages-v1"
        || value["subject"]["integration_profile_ref"] != "builtin/claude-messages/v1"
    {
        return Err(invalid());
    }
    let payload: ClaudeModelFilePayload =
        serde_json::from_value(value["payload"].clone()).map_err(|_| invalid())?;
    validate_payload(&payload).map_err(|_| invalid())?;
    Ok(payload)
}

pub fn settings_claude_model_file_for_operation(
    operation: &OperationV1,
    intent: &ExternalEffectIntentV1,
) -> PortResult<ClaudeModelFilePayload> {
    let payload = decode_settings_claude_model_file(intent)?;
    let spec: AgentSettingsSpecV2 =
        serde_json::from_value(operation.plan.spec().desired_state.clone())
            .map_err(|_| invalid())?;
    if operation.plan.spec().command_id != "agents.settings.apply"
        || spec.context_id != payload.context_id
        || !operation.plan.external().contains(intent)
    {
        return Err(invalid());
    }
    let state = &operation.plan.control()["payload"]["state"];
    if state["accept_digest"]
        != serde_json::to_value(&operation.accepted_digest).map_err(|_| invalid())?
    {
        return Err(invalid());
    }
    match (&spec.model, &payload.change) {
        (
            AgentFacetIntent::Configure { settings },
            ClaudeModelFileAction::Configure {
                previous_operation,
                snapshot,
                ..
            },
        ) => {
            let grant: AgentModelGrantV2 =
                serde_json::from_value(state["model_grant"].clone()).map_err(|_| invalid())?;
            let [mutation] = operation.plan.agent_access_grants() else {
                return Err(invalid());
            };
            let scope = mutation.desired_scope().ok_or_else(invalid)?;
            if scope.model_grant() != &grant
                || previous_operation.as_ref() == Some(&operation.operation_id)
                || grant
                    .claude_preset_values(settings, &snapshot.native_presets)
                    .map_err(|_| invalid())?
                    != snapshot.presets
            {
                return Err(invalid());
            }
        }
        (
            AgentFacetIntent::Restore { restore_point_ref },
            ClaudeModelFileAction::Restore { original_operation },
        ) if original_operation != &operation.operation_id
            && *restore_point_ref == super::codex_model_restore_point_ref(original_operation) => {}
        _ => return Err(invalid()),
    }
    Ok(payload)
}

fn validate_payload(payload: &ClaudeModelFilePayload) -> Result<(), OperationValidationError> {
    let invalid = || OperationValidationError::UnregisteredEffectPlan;
    if payload.schema != SCHEMA || !super::settings::identity(&payload.context_id) {
        return Err(invalid());
    }
    match &payload.change {
        ClaudeModelFileAction::Configure {
            previous_operation,
            change,
            snapshot,
            gateway_base_url,
            trusted_hiroute_executable,
        } => {
            if !local_gateway_base_url(gateway_base_url)
                || change.validate().is_err()
                || change.fields.iter().any(|field| {
                    !matches!(
                        field.path.as_str(),
                        "apiKeyHelper"
                            | "hiroute.auth_environment"
                            | "env.ANTHROPIC_BASE_URL"
                            | "env.ANTHROPIC_DEFAULT_OPUS_MODEL"
                            | "env.ANTHROPIC_DEFAULT_SONNET_MODEL"
                            | "env.ANTHROPIC_DEFAULT_HAIKU_MODEL"
                    )
                })
                || !Path::new(trusted_hiroute_executable).is_absolute()
                || trusted_hiroute_executable.contains(['\0', '\r', '\n'])
                || previous_operation.as_ref().is_some_and(|previous| {
                    OperationId::parse(previous.as_str().to_owned()).is_err()
                })
                || !Path::new(&snapshot.executable).is_absolute()
                || snapshot.executable.contains(['\0', '\r', '\n'])
                || CanonicalDigest::parse(snapshot.installation_digest.as_str()).is_err()
                || [&snapshot.native_presets, &snapshot.presets]
                    .into_iter()
                    .any(|presets| {
                        [&presets.opus, &presets.sonnet, &presets.haiku]
                            .into_iter()
                            .filter_map(|value| value.as_deref())
                            .any(|value| !hiroute_domain::valid_client_model_name(value))
                    })
            {
                return Err(invalid());
            }
        }
        ClaudeModelFileAction::Restore { original_operation } => {
            OperationId::parse(original_operation.as_str().to_owned()).map_err(|_| invalid())?;
        }
    }
    Ok(())
}

fn local_gateway_base_url(value: &str) -> bool {
    value
        .strip_suffix("/v1")
        .and_then(|origin| {
            origin
                .strip_prefix("http://127.0.0.1:")
                .or_else(|| origin.strip_prefix("http://[::1]:"))
        })
        .is_some_and(|port| {
            !port.is_empty()
                && port.bytes().all(|byte| byte.is_ascii_digit())
                && port.parse::<u16>().is_ok_and(|port| port != 0)
        })
}

fn invalid() -> PortError {
    PortError::new(PortErrorCode::InvalidData, "claude.settings.intent")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> ClaudeModelFilePayload {
        ClaudeModelFilePayload {
            schema: SCHEMA.into(),
            context_id: "agent-context/claude/test".into(),
            expected_content: CanonicalDigest::of_bytes(b""),
            change: ClaudeModelFileAction::Configure {
                previous_operation: None,
                change: AgentConfigChangeV1::preview(
                    &hiroute_domain::AgentConfigDocumentV1::default(),
                    std::collections::BTreeMap::new(),
                )
                .unwrap(),
                snapshot: Box::new(ClaudeLaunchSnapshotIntent {
                    native_presets: AgentClaudePresetValuesV2 {
                        opus: Some("Vendor/Opus[1m]".into()),
                        sonnet: None,
                        haiku: None,
                    },
                    presets: AgentClaudePresetValuesV2 {
                        opus: Some("hr-plan-opus".into()),
                        sonnet: None,
                        haiku: None,
                    },
                    executable: "/opt/claude/bin/claude".into(),
                    installation_digest: CanonicalDigest::of_bytes(b"installation"),
                }),
                gateway_base_url: "http://127.0.0.1:8080/v1".into(),
                trusted_hiroute_executable: "/opt/hiroute/bin/hiroute".into(),
            },
        }
    }

    #[test]
    fn native_change_and_optional_launcher_facts_are_sealed_without_original_secret_bytes() {
        let mut original = payload();
        if let ClaudeModelFileAction::Configure { change, .. } = &mut original.change {
            *change = AgentConfigChangeV1::preview(
                &hiroute_domain::AgentConfigDocumentV1 {
                    fields: std::collections::BTreeMap::from([
                        (
                            "apiKeyHelper".into(),
                            serde_json::json!({"configured":true}),
                        ),
                        (
                            "hiroute.auth_environment".into(),
                            serde_json::json!({"configured":true}),
                        ),
                    ]),
                },
                std::collections::BTreeMap::from([
                    ("hiroute.auth_environment".into(), None),
                    (
                        "env.ANTHROPIC_BASE_URL".into(),
                        Some(serde_json::json!("http://127.0.0.1:8080")),
                    ),
                ]),
            )
            .unwrap();
        }
        assert!(validate_payload(&original).is_ok());
        let value = serde_json::to_value(&original).unwrap();
        let snapshot = &value["change"]["snapshot"];
        assert_eq!(snapshot["native_presets"]["opus"], "Vendor/Opus[1m]");
        assert_eq!(snapshot["presets"]["opus"], "hr-plan-opus");
        assert_eq!(snapshot["executable"], "/opt/claude/bin/claude");
        assert!(snapshot["native_presets"]["sonnet"].is_null());
        assert_eq!(
            value["change"]["change"]["fields"][0]["path"],
            "env.ANTHROPIC_BASE_URL"
        );
        assert_eq!(
            value["change"]["change"]["fields"][1]["path"],
            "hiroute.auth_environment"
        );
        assert_eq!(
            value["change"]["change"]["fields"][1]["before"],
            serde_json::json!({"configured":true})
        );
        assert!(
            !value.to_string().contains("user-owned-helper")
                && !value.to_string().contains("user-secret")
        );
        let decoded: ClaudeModelFilePayload = serde_json::from_value(value).unwrap();
        assert!(validate_payload(&decoded).is_ok());
    }

    #[test]
    fn snapshot_rejects_remote_gateway_and_unsafe_helper() {
        for endpoint in [
            "https://example.com/v1",
            "http://127.0.0.1:0/v1",
            "http://127.0.0.1:8080/other",
        ] {
            let mut candidate = payload();
            if let ClaudeModelFileAction::Configure {
                gateway_base_url, ..
            } = &mut candidate.change
            {
                *gateway_base_url = endpoint.into();
            }
            assert!(validate_payload(&candidate).is_err());
        }
        for executable in ["hiroute", "/opt/hiroute\nother", "/opt/hiroute\0other"] {
            let mut candidate = payload();
            if let ClaudeModelFileAction::Configure {
                trusted_hiroute_executable,
                ..
            } = &mut candidate.change
            {
                *trusted_hiroute_executable = executable.into();
            }
            assert!(validate_payload(&candidate).is_err());
        }
    }

    #[test]
    fn snapshot_rejects_unsafe_claude_executable() {
        for executable in ["claude", "/opt/claude\nother", "/opt/claude\0other"] {
            let mut candidate = payload();
            if let ClaudeModelFileAction::Configure { snapshot, .. } = &mut candidate.change {
                snapshot.executable = executable.into();
            }
            assert!(validate_payload(&candidate).is_err());
        }
    }

    #[test]
    fn native_change_rejects_invalid_semantic_file_changes() {
        let mut value = serde_json::to_value(payload()).unwrap();
        value["change"]["change"] = serde_json::json!({"fields": []});
        assert!(serde_json::from_value::<ClaudeModelFilePayload>(value).is_err());
    }
}
