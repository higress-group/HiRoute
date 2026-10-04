//! Non-secret additive Qoder model effects, bound to one settings Operation.
use hiroute_domain::{
    AgentConnectionControlIntentV1, AgentConnectionEffectRoleV1, AgentConnectionTransactionKindV1,
    AgentFacetIntent, AgentIngressProtocolV1, AgentModelGrantV2, AgentModelRouteV2,
    AgentModelSelectionV2, AgentOperationRead, AgentSettingsSpecV2, CanonicalDigest,
    ExternalEffectIntentV1, OperationId, OperationValidationError, PortError, PortErrorCode,
    PortResult, QoderAdditionalModelV1,
};
use serde::{Deserialize, Serialize};

const SCHEMA: &str = "hiroute.settings-qoder-model-file/v1";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum QoderModelFileAction {
    Configure {
        previous_operation: Option<OperationId>,
        provider_id: String,
        endpoint: String,
        models: Vec<QoderAdditionalModelV1>,
    },
    Restore {
        original_operation: OperationId,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QoderModelFilePayload {
    schema: String,
    pub context_id: String,
    pub expected_content: CanonicalDigest,
    pub change: QoderModelFileAction,
}

/// A stable, context-owned namespace. It never adopts an existing unowned provider.
pub fn qoder_model_provider_id(context_id: &str) -> String {
    let digest = CanonicalDigest::of_bytes(context_id.as_bytes());
    format!("hiroute-main-{}", &digest.as_str()[7..])
}

pub fn settings_qoder_model_file_intent(
    control: &AgentConnectionControlIntentV1,
    context_id: &str,
    expected_content: CanonicalDigest,
    before_fingerprint: Option<CanonicalDigest>,
    change: QoderModelFileAction,
) -> Result<ExternalEffectIntentV1, OperationValidationError> {
    if control.transaction() != AgentConnectionTransactionKindV1::Settings {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let payload = QoderModelFilePayload {
        schema: SCHEMA.into(),
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
    decode_settings_qoder_model_file(&intent)
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    Ok(intent)
}

pub fn decode_settings_qoder_model_file(
    intent: &ExternalEffectIntentV1,
) -> PortResult<QoderModelFilePayload> {
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
        || value["subject"]["agent_id"] != "agent_qoder_default"
        || value["subject"]["profile_id"] != "qoder-collaboration-v1"
        || value["subject"]["integration_profile_ref"] != "builtin/qoder-collaboration/v1"
    {
        return Err(invalid());
    }
    let payload: QoderModelFilePayload =
        serde_json::from_value(value["payload"].clone()).map_err(|_| invalid())?;
    if payload.schema != SCHEMA || !super::settings::identity(&payload.context_id) {
        return Err(invalid());
    }
    match &payload.change {
        QoderModelFileAction::Configure {
            provider_id,
            endpoint,
            models,
            ..
        } => {
            if provider_id != &qoder_model_provider_id(&payload.context_id)
                || !local_endpoint(endpoint)
                || models.is_empty()
                || models.len() > 256
                || models.iter().any(|model| model.validate().is_err())
                || models.windows(2).any(|pair| pair[0].alias >= pair[1].alias)
            {
                return Err(invalid());
            }
        }
        QoderModelFileAction::Restore { .. } => {}
    }
    Ok(payload)
}

pub fn settings_qoder_model_file_for_operation(
    operation: &(impl AgentOperationRead + ?Sized),
    intent: &ExternalEffectIntentV1,
) -> PortResult<QoderModelFilePayload> {
    let payload = decode_settings_qoder_model_file(intent)?;
    let spec: AgentSettingsSpecV2 =
        serde_json::from_value(operation.agent_input().spec.desired_state.clone())
            .map_err(|_| invalid())?;
    if operation.agent_input().spec.command_id != "agents.settings.apply"
        || spec.context_id != payload.context_id
        || !operation.agent_input().external.contains(intent)
        || spec.restore_native_model.is_some()
    {
        return Err(invalid());
    }
    let state = &operation.agent_input().control["payload"]["state"];
    if state["accept_digest"]
        != serde_json::to_value(operation.accepted_digest()).map_err(|_| invalid())?
    {
        return Err(invalid());
    }
    match (&spec.model, &payload.change) {
        (
            AgentFacetIntent::Configure {
                settings: AgentModelSelectionV2::QoderAdditional { allowed_plan_ids },
            },
            QoderModelFileAction::Configure {
                previous_operation,
                models,
                ..
            },
        ) => {
            let grant: AgentModelGrantV2 =
                serde_json::from_value(state["model_grant"].clone()).map_err(|_| invalid())?;
            let [mutation] = operation.agent_input().grants else {
                return Err(invalid());
            };
            let scope = mutation.desired_scope().ok_or_else(invalid)?;
            if scope.model_grant() != &grant || grant.protocol != AgentIngressProtocolV1::Responses
                || previous_operation.as_ref() == Some(operation.operation_id())
                || models.len() != grant.routes.len() || models.len() != allowed_plan_ids.len()
                || !models.iter().all(|model| matches!(grant.routes.get(&model.alias), Some(AgentModelRouteV2::Plan { plan_id, alias, .. }) if allowed_plan_ids.contains(plan_id) && alias.as_str() == model.alias))
            { return Err(invalid()); }
        }
        (
            AgentFacetIntent::Restore { restore_point_ref },
            QoderModelFileAction::Restore { original_operation },
        ) if original_operation != operation.operation_id()
            && *restore_point_ref == super::codex_model_restore_point_ref(original_operation) => {}
        _ => return Err(invalid()),
    }
    Ok(payload)
}

fn local_endpoint(value: &str) -> bool {
    value
        .strip_prefix("http://127.0.0.1:")
        .or_else(|| value.strip_prefix("http://[::1]:"))
        .and_then(|rest| rest.strip_suffix(hiroute_domain::QODER_MODEL_BASE_PATH))
        .is_some_and(|port| {
            !port.is_empty()
                && port.bytes().all(|b| b.is_ascii_digit())
                && port.parse::<u16>().is_ok_and(|p| p != 0)
        })
}
fn invalid() -> PortError {
    PortError::new(PortErrorCode::InvalidData, "qoder.settings.intent")
}
