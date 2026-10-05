//! Non-secret additive Qoder model effects, bound to one settings Operation.
use hiroute_domain::{
    AdditionalAgentModelV1, AgentConnectionControlIntentV1, AgentConnectionEffectRoleV1,
    AgentConnectionTransactionKindV1, AgentFacetIntent, AgentIngressProtocolV1, AgentKindV1,
    AgentModelGrantV2, AgentModelRouteV2, AgentModelSelectionV2, AgentOperationRead,
    AgentSettingsSpecV2, CanonicalDigest, ExternalEffectIntentV1, OperationId,
    OperationValidationError, PortError, PortErrorCode, PortResult,
};
use serde::{Deserialize, Serialize};

const QODER_SCHEMA: &str = "hiroute.settings-qoder-model-file/v1";
const PI_SCHEMA: &str = "hiroute.settings-pi-model-file/v1";

pub fn additional_model_kind(intent: &ExternalEffectIntentV1) -> PortResult<AgentKindV1> {
    match intent.desired()["subject"]["agent_id"].as_str() {
        Some("agent_qoder_default") => Ok(AgentKindV1::Qoder),
        Some("agent_pi_default") => Ok(AgentKindV1::Pi),
        _ => Err(invalid()),
    }
}
fn schema(kind: AgentKindV1) -> &'static str {
    match kind {
        AgentKindV1::Pi => PI_SCHEMA,
        _ => QODER_SCHEMA,
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum AdditionalModelFileAction {
    Configure {
        previous_operation: Option<OperationId>,
        provider_id: String,
        endpoint: String,
        models: Vec<AdditionalAgentModelV1>,
    },
    Restore {
        original_operation: OperationId,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdditionalModelFilePayload {
    schema: String,
    pub context_id: String,
    pub expected_content: CanonicalDigest,
    pub change: AdditionalModelFileAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pi_settings_content: Option<CanonicalDigest>,
}

/// A stable, context-owned namespace. It never adopts an existing unowned provider.
pub fn additional_model_provider_id(context_id: &str) -> String {
    let digest = CanonicalDigest::of_bytes(context_id.as_bytes());
    format!("hiroute-main-{}", &digest.as_str()[7..])
}

pub fn settings_additional_model_file_intent(
    control: &AgentConnectionControlIntentV1,
    context_id: &str,
    expected_content: CanonicalDigest,
    before_fingerprint: Option<CanonicalDigest>,
    change: AdditionalModelFileAction,
    pi_settings_content: Option<CanonicalDigest>,
) -> Result<ExternalEffectIntentV1, OperationValidationError> {
    if control.transaction() != AgentConnectionTransactionKindV1::Settings {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let payload = AdditionalModelFilePayload {
        schema: schema(if control.subject().agent_id() == "agent_pi_default" {
            AgentKindV1::Pi
        } else {
            AgentKindV1::Qoder
        })
        .into(),
        context_id: context_id.into(),
        expected_content,
        change,
        pi_settings_content,
    };
    let intent = ExternalEffectIntentV1::from_agent_connection_planner(
        control,
        AgentConnectionEffectRoleV1::ManagedConfiguration,
        before_fingerprint,
        &payload,
        0o600,
    )?;
    decode_settings_additional_model_file(&intent)
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    Ok(intent)
}

pub fn decode_settings_additional_model_file(
    intent: &ExternalEffectIntentV1,
) -> PortResult<AdditionalModelFilePayload> {
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
    let kind = additional_model_kind(intent)?;
    let (agent, profile, integration) = match kind {
        AgentKindV1::Pi => (
            "agent_pi_default",
            "pi-responses-v1",
            "builtin/pi-responses/v1",
        ),
        _ => (
            "agent_qoder_default",
            "qoder-collaboration-v1",
            "builtin/qoder-collaboration/v1",
        ),
    };
    if intent.effect_id() != "agent-connection-managed-configuration"
        || intent.desired_mode() != 0o600
        || value["transaction"] != "settings"
        || value["subject"]["agent_id"] != agent
        || value["subject"]["profile_id"] != profile
        || value["subject"]["integration_profile_ref"] != integration
    {
        return Err(invalid());
    }
    let payload: AdditionalModelFilePayload =
        serde_json::from_value(value["payload"].clone()).map_err(|_| invalid())?;
    if payload.schema != schema(kind)
        || (kind == AgentKindV1::Pi) != payload.pi_settings_content.is_some()
        || !super::settings::identity(&payload.context_id)
    {
        return Err(invalid());
    }
    match &payload.change {
        AdditionalModelFileAction::Configure {
            provider_id,
            endpoint,
            models,
            ..
        } => {
            if provider_id != &additional_model_provider_id(&payload.context_id)
                || !local_endpoint(endpoint, kind)
                || models.is_empty()
                || models.len() > 256
                || models.iter().any(|model| {
                    (if kind == AgentKindV1::Pi {
                        model.validate_pi()
                    } else {
                        model.validate()
                    })
                    .is_err()
                })
                || models.windows(2).any(|pair| pair[0].alias >= pair[1].alias)
            {
                return Err(invalid());
            }
        }
        AdditionalModelFileAction::Restore { .. } => {}
    }
    Ok(payload)
}

pub fn settings_additional_model_file_for_operation(
    operation: &(impl AgentOperationRead + ?Sized),
    intent: &ExternalEffectIntentV1,
) -> PortResult<AdditionalModelFilePayload> {
    let payload = decode_settings_additional_model_file(intent)?;
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
    let kind = additional_model_kind(intent)?;
    match (&spec.model, &payload.change) {
        (
            AgentFacetIntent::Configure {
                settings:
                    AgentModelSelectionV2::QoderAdditional { allowed_plan_ids }
                    | AgentModelSelectionV2::PiAdditional { allowed_plan_ids },
            },
            AdditionalModelFileAction::Configure {
                previous_operation,
                models,
                ..
            },
        ) => {
            if matches!(
                &spec.model,
                AgentFacetIntent::Configure {
                    settings: AgentModelSelectionV2::PiAdditional { .. }
                }
            ) != (kind == AgentKindV1::Pi)
            {
                return Err(invalid());
            }
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
            AdditionalModelFileAction::Restore { original_operation },
        ) if original_operation != operation.operation_id()
            && *restore_point_ref == super::codex_model_restore_point_ref(original_operation) => {}
        _ => return Err(invalid()),
    }
    Ok(payload)
}

fn local_endpoint(value: &str, kind: AgentKindV1) -> bool {
    value
        .strip_prefix("http://127.0.0.1:")
        .or_else(|| value.strip_prefix("http://[::1]:"))
        .and_then(|rest| {
            rest.strip_suffix(if kind == AgentKindV1::Pi {
                "/v1"
            } else {
                hiroute_domain::QODER_MODEL_BASE_PATH
            })
        })
        .is_some_and(|port| {
            !port.is_empty()
                && port.bytes().all(|b| b.is_ascii_digit())
                && port.parse::<u16>().is_ok_and(|p| p != 0)
        })
}
fn invalid() -> PortError {
    PortError::new(PortErrorCode::InvalidData, "qoder.settings.intent")
}
