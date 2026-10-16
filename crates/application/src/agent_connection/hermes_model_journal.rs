//! Sealed, non-secret Hermes default effects. Native before-images stay in the restore port.
use hiroute_domain::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum HermesModelFileAction {
    Configure {
        previous_operation: Option<OperationId>,
        provider_id: String,
        endpoint: String,
        model: AdditionalAgentModelV1,
    },
    Restore {
        original_operation: OperationId,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HermesModelFilePayload {
    pub schema: String,
    pub context_id: String,
    pub expected_content: CanonicalDigest,
    pub change: HermesModelFileAction,
}

pub fn settings_hermes_model_file_intent(
    control: &AgentConnectionControlIntentV1,
    context_id: &str,
    expected_content: CanonicalDigest,
    before_fingerprint: Option<CanonicalDigest>,
    change: HermesModelFileAction,
) -> Result<ExternalEffectIntentV1, OperationValidationError> {
    let payload = HermesModelFilePayload {
        schema: "hiroute.settings-hermes-model-file/v1".into(),
        context_id: context_id.into(),
        expected_content,
        change,
    };
    let intent = ExternalEffectIntentV1::from_agent_connection_planner(
        control,
        AgentConnectionEffectRoleV1::ManagedConfiguration,
        before_fingerprint,
        &payload,
        0o600,
    )?;
    decode_settings_hermes_model_file(&intent)
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    Ok(intent)
}

pub fn is_settings_hermes_model(intent: &ExternalEffectIntentV1) -> bool {
    intent.effect_id() == "agent-connection-managed-configuration"
        && intent.desired()["transaction"] == "settings"
        && intent.desired()["subject"]["agent_id"] == "agent_hermes_default"
}

pub fn decode_settings_hermes_model_file(
    intent: &ExternalEffectIntentV1,
) -> PortResult<HermesModelFilePayload> {
    let invalid = || PortError::new(PortErrorCode::InvalidData, "hermes.model.intent");
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
    if !is_settings_hermes_model(intent)
        || intent.desired_mode() != 0o600
        || intent.desired()["subject"]["integration_profile_ref"] != "builtin/hermes-main/v1"
    {
        return Err(invalid());
    }
    let payload: HermesModelFilePayload =
        serde_json::from_value(intent.desired()["payload"].clone()).map_err(|_| invalid())?;
    let digest = payload
        .context_id
        .strip_prefix("agent-context/hermes/sha256:")
        .ok_or_else(invalid)?;
    if payload.schema != "hiroute.settings-hermes-model-file/v1"
        || digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        || intent.desired()["subject"]["profile_id"] != format!("hermes-main-{digest}")
    {
        return Err(invalid());
    }
    if let HermesModelFileAction::Configure {
        provider_id, model, ..
    } = &payload.change
    {
        if provider_id != &super::additional_model_provider_id(&payload.context_id) {
            return Err(invalid());
        }
        model.validate_pi().map_err(|_| invalid())?;
    }
    Ok(payload)
}

pub fn settings_hermes_model_file_for_operation(
    op: &(impl AgentOperationRead + ?Sized),
    intent: &ExternalEffectIntentV1,
) -> PortResult<HermesModelFilePayload> {
    let invalid = || PortError::new(PortErrorCode::InvalidData, "hermes.model.operation");
    let payload = decode_settings_hermes_model_file(intent)?;
    let input = op.agent_input();
    if input.spec.command_id != "agents.settings.apply"
        || input.spec.resource_id.as_deref() != Some(&payload.context_id)
        || !input.external.iter().any(|stored| stored == intent)
    {
        return Err(invalid());
    }
    let spec: AgentSettingsSpecV2 =
        serde_json::from_value(input.spec.desired_state.clone()).map_err(|_| invalid())?;
    let target = spec.native_target.as_ref().ok_or_else(invalid)?;
    if payload.context_id
        != format!(
            "agent-context/hermes/{}",
            CanonicalDigest::of_bytes(target.config_root().as_bytes()).as_str()
        )
    {
        return Err(invalid());
    }
    match (&spec.model, &payload.change) {
        (
            AgentFacetIntent::Configure {
                settings: AgentModelSelectionV2::HermesDefault { plan_id, protocol },
            },
            HermesModelFileAction::Configure { model, .. },
        ) => {
            let [mutation] = input.grants else {
                return Err(invalid());
            };
            let scope = mutation.desired_scope().ok_or_else(invalid)?;
            if model.protocol != *protocol
                || scope.model_grant().routes.len() != 1
                || !matches!(scope.model_grant().routes.get(&model.alias),Some(AgentModelRouteV2::Plan {plan_id:actual,..}) if actual==plan_id)
            {
                return Err(invalid());
            }
        }
        (AgentFacetIntent::Restore { .. }, HermesModelFileAction::Restore { .. }) => {}
        _ => return Err(invalid()),
    }
    Ok(payload)
}
