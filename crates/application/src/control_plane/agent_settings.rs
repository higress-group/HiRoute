//! V2 independent settings on the existing connect descriptors and protected confirmation flow.
use crate::agent_connection::{
    SettingsConfirmationError, SettingsModelTargetFacts, confirm_agent_settings,
    preview_agent_settings,
};
use crate::{ApplicationService, failed, map_control_error, succeeded};
use hiroute_application_api::{
    AgentCollaborationEffectV2, AgentSettingsApplyV2, AgentSettingsPreviewRequestV2,
    ApplyRequestV1, CHANGE_SPEC_SCHEMA_V1, ErrorCode, LocalControlRequestV2, MachineEnvelopeV2,
};
use serde_json::{Value, json};

pub(crate) fn preview(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    let payload: AgentSettingsPreviewRequestV2 = match serde_json::from_value(request.payload) {
        Ok(payload) => payload,
        Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
    };
    // Restore owns a distinct confirmation and idempotency scope. Keep the descriptor
    // consistent with the journal kind the strict settings reader derives from this spec.
    let restore = request.operation_id == "PreviewAgentConnectionRestore";
    if restore != payload.spec.is_restore_only() {
        return failed(ErrorCode::InvalidArguments, request.request_id);
    }
    let Some(port) = service
        .ports
        .as_ref()
        .and_then(|ports| ports.agent_connection.as_deref())
    else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    let input = match port.settings_facts(&payload.spec) {
        Ok(input) => input,
        Err(error) => return settings_read_failure(error, request.request_id),
    };
    let preview = match preview_agent_settings(payload.spec, &input.facts) {
        Ok(preview) => preview,
        Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
    };
    use hiroute_application_api::{
        AgentModelEffectPreviewV2, AgentResidentServicePreviewV2, AgentSettingsPreviewV2,
    };
    let collaboration_effect = match &preview.spec.collaboration {
        hiroute_application_api::AgentFacetIntent::Configure { settings } => {
            Some(AgentCollaborationEffectV2 {
                action: "configure".into(),
                trigger_mode: settings.trigger_mode,
            })
        }
        _ => None,
    };
    let configured =
        |agent_class: &str,
         provider_id: Option<&String>,
         endpoint: &String,
         selection: &hiroute_application_api::AgentModelSelectionV2| {
            AgentModelEffectPreviewV2::Configure {
                agent_class: agent_class.into(),
                provider_id: provider_id.cloned(),
                endpoint: endpoint.clone(),
                selection: Box::new(selection.clone()),
                authentication: "connection_scoped_local_grant".into(),
                native_adapter: (agent_class == "hermes").then(|| hiroute_application_api::AgentNativeAdapterPreviewV1 {
                    resources: "planned".into(), runtime_loading: hiroute_application_api::AgentNativeRuntimeLoadingV1::Unobserved,
                    next_action: hiroute_application_api::AgentNativeAdapterNextActionV1::ReloadPluginsIfRunning,
                }),
                context_windows: (!preview.context_windows.is_empty())
                    .then(|| preview.context_windows.clone()),
                claude_context_window: (!preview.context_windows.is_empty())
                    .then_some(preview.claude_context_window),
            }
        };
    let model_effect = match (
        &preview.spec.model,
        input.model_file.as_ref().map(|file| &file.target),
    ) {
        (
            hiroute_application_api::AgentFacetIntent::Configure { settings },
            Some(SettingsModelTargetFacts::Hermes {
                provider_id,
                endpoint,
                ..
            }),
        ) => configured("hermes", Some(provider_id), endpoint, settings),
        (
            hiroute_application_api::AgentFacetIntent::Configure { settings },
            Some(SettingsModelTargetFacts::Codex {
                provider_id,
                endpoint,
                ..
            }),
        ) => configured("codex", Some(provider_id), endpoint, settings),
        (
            hiroute_application_api::AgentFacetIntent::Configure { settings },
            Some(SettingsModelTargetFacts::Claude(claude)),
        ) => configured("claude", None, &claude.gateway_base_url, settings),
        (
            hiroute_application_api::AgentFacetIntent::Configure { settings },
            Some(SettingsModelTargetFacts::Additional {
                kind,
                provider_id,
                endpoint,
                ..
            }),
        ) => configured(kind.as_str(), Some(provider_id), endpoint, settings),
        (hiroute_application_api::AgentFacetIntent::Restore { restore_point_ref }, Some(_)) => {
            AgentModelEffectPreviewV2::Restore {
                restore_point_ref: restore_point_ref.clone(),
            }
        }
        (hiroute_application_api::AgentFacetIntent::Keep, _) => AgentModelEffectPreviewV2::Keep,
        (_, None) => return failed(ErrorCode::InvalidArguments, request.request_id),
    };
    succeeded(
        AgentSettingsPreviewV2 {
            schema: "hiroute.agent-settings-preview/v2".into(),
            expected_revisions: input.expected_revisions,
            applicable: preview.blockers.is_empty(),
            resident_service: AgentResidentServicePreviewV2 {
                login_item_required: input
                    .facts
                    .model
                    .as_ref()
                    .is_some_and(|model| model.common.login_item_required),
                login_item_removal_required: input
                    .facts
                    .model
                    .as_ref()
                    .is_some_and(|model| model.common.login_item_removal_required),
            },
            collaboration_effect,
            model_effect,
            preview,
        },
        request.request_id,
    )
}

pub(crate) fn apply(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    if request.operation_id == "ApplyAgentConnectionChange"
        && request.payload.get("schema").is_some()
    {
        let payload: hiroute_application_api::AgentSettingsRetryV1 =
            match serde_json::from_value(request.payload) {
                Ok(value) => value,
                Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
            };
        if payload.schema != "hiroute.agent-settings-retry/v1" {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        }
        let Some(port) = service
            .ports
            .as_ref()
            .and_then(|p| p.agent_connection.as_ref())
        else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        return match port.retry_settings_operation(&payload) {
            Ok(operation) => super::accepted_apply(operation, request.request_id),
            Err(error) => failed(map_control_error(error), request.request_id),
        };
    }
    let payload: AgentSettingsApplyV2 = match serde_json::from_value(request.payload) {
        Ok(payload) => payload,
        Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
    };
    let restore = request.operation_id == "ApplyAgentConnectionRestore";
    if restore != payload.spec.is_restore_only() {
        return failed(ErrorCode::InvalidArguments, request.request_id);
    }
    let operation_kind = if restore {
        "ApplyAgentConnectionRestore"
    } else {
        "ApplyAgentConnectionChange"
    };
    let Some(ports) = service.ports.as_ref() else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    if let Some(replay) = super::replay_apply(
        ports.control.as_ref(),
        hiroute_application_api::PrincipalKind::InteractiveUser,
        operation_kind,
        &payload.idempotency_key,
        &payload.accept_digest,
        &request.request_id,
    ) {
        return replay;
    }
    let Some(port) = ports.agent_connection.as_ref() else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    let apply = ApplyRequestV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        spec: hiroute_domain::ChangeSpecV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            command_id: "agents.settings.apply".into(),
            resource_id: Some(payload.spec.context_id.clone()),
            desired_state: match serde_json::to_value(&payload.spec) {
                Ok(value) => value,
                Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
            },
        },
        accept_digest: payload.accept_digest.clone(),
        expected_revisions: payload.expected_revisions.clone(),
        idempotency_key: payload.idempotency_key.clone(),
        apply_capability: None,
    };
    let port = port.clone();
    let prepared =
        match crate::PreparedTransactionV1::for_current_agent_settings(apply, move || {
            prepare_current_settings(port.as_ref(), payload)
        }) {
            Ok(prepared) => prepared,
            Err(error) => return failed(error.error_code(), request.request_id),
        };
    let Some(mutation) = ports.mutation.as_deref() else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    match mutation.apply_local_prepared_change(prepared) {
        Ok(operation) => super::accepted_apply(operation, request.request_id),
        Err(crate::TransactionError::AgentSettingsRead(error)) => {
            settings_read_failure(error, request.request_id)
        }
        Err(error) => failed(error.error_code(), request.request_id),
    }
}

fn prepare_current_settings(
    port: &dyn crate::control::AgentConnectionControlPort,
    payload: AgentSettingsApplyV2,
) -> Result<hiroute_domain::TransactionPlanV1, crate::TransactionError> {
    use crate::TransactionError;
    let input = port
        .settings_facts(&payload.spec)
        .map_err(TransactionError::AgentSettingsRead)?;
    let removal_required = input
        .facts
        .model
        .as_ref()
        .is_some_and(|model| model.common.login_item_removal_required);
    if !removal_required && payload.login_item.is_some() {
        return Err(TransactionError::AgentSettings(ErrorCode::InvalidArguments));
    }
    if removal_required
        && !payload
            .login_item
            .as_ref()
            .is_some_and(|declaration| declaration.removes_resident_service())
    {
        return Err(TransactionError::AgentSettings(
            ErrorCode::GatewayUnavailable,
        ));
    }
    if input.expected_revisions != payload.expected_revisions {
        return Err(TransactionError::ChangePreviewStale);
    }
    let confirmed = confirm_agent_settings(payload, &input.facts)
        .map_err(|error| TransactionError::AgentSettings(confirmation_error(error)))?;
    input
        .seal_settings(&confirmed)
        .map_err(|_| TransactionError::AgentSettings(ErrorCode::InvalidArguments))
}
fn confirmation_error(error: SettingsConfirmationError) -> ErrorCode {
    match error {
        SettingsConfirmationError::Invalid => ErrorCode::InvalidArguments,
        SettingsConfirmationError::Stale => ErrorCode::ChangePreviewStale,
        SettingsConfirmationError::Blocked => ErrorCode::CapabilityDenied,
    }
}

fn settings_read_failure(
    error: crate::control::ControlReadError,
    request_id: String,
) -> MachineEnvelopeV2<Value> {
    let mut response = failed(map_control_error(error), request_id);
    if error == crate::control::ControlReadError::CollaborationRootRequired {
        response
            .error
            .as_mut()
            .expect("failed envelope has an error")
            .message_key = "error.agent_collaboration_root_context_required".into();
        response
            .next_actions
            .push(hiroute_application_api::NextActionV1 {
                command_id: "agents.scan".into(),
                input: json!({}),
                reason_code: "COLLABORATION_ROOT_CONTEXT_REQUIRED".into(),
            });
    }
    response
}

pub(crate) fn status(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    let payload: hiroute_application_api::AgentSettingsStatusRequestV2 =
        match serde_json::from_value::<hiroute_application_api::AgentSettingsStatusRequestV2>(
            request.payload,
        ) {
            Ok(payload)
                if payload.schema_version == hiroute_application_api::AGENT_SETTINGS_SCHEMA_V2
                    && (payload.projection
                        != hiroute_application_api::AgentSettingsStatusProjectionV2::Model
                        || (!payload.include_skills && payload.effective.is_none()))
                    && payload
                        .effective
                        .as_ref()
                        .is_none_or(|effective| effective.valid()) =>
            {
                payload
            }
            _ => return failed(ErrorCode::InvalidArguments, request.request_id),
        };
    let Some(port) = service
        .ports
        .as_ref()
        .and_then(|ports| ports.agent_connection.as_deref())
    else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    match port.settings_status(&payload) {
        Ok(result) => succeeded(result, request.request_id),
        Err(error) => failed(map_control_error(error), request.request_id),
    }
}

/// Read-only draft recovery. The ordinary settings preview must still admit every effect.
pub(crate) fn previous_routing(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    let query: hiroute_application_api::AgentPreviousRoutingQueryV1 =
        match serde_json::from_value(request.payload) {
            Ok(query) => query,
            Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
        };
    let Some(ports) = &service.ports else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    let Some(connection) = ports.agent_connection.as_deref() else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    match crate::agent_connection::preview_previous_routing(
        connection,
        ports.control.as_ref(),
        &query,
    ) {
        Ok(preview) => succeeded(preview, request.request_id),
        Err(error) => failed(map_control_error(error), request.request_id),
    }
}
