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
    if request.operation_id == "PreviewAgentConnectionRestore" && !payload.spec.is_restore_only() {
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
        Err(error) => return failed(map_control_error(error), request.request_id),
    };
    let preview = match preview_agent_settings(payload.spec, &input.facts) {
        Ok(preview) => preview,
        Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
    };
    let mut result = serde_json::to_value(&preview).expect("settings preview is serializable");
    result["schema"] = json!("hiroute.agent-settings-preview/v2");
    result["expected_revisions"] = json!(input.expected_revisions);
    result["applicable"] = json!(preview.blockers.is_empty());
    result["resident_service"] = json!({
        "login_item_required": input.facts.login_item_required,
        "login_item_removal_required": input.facts.login_item_removal_required,
    });
    if matches!(
        &preview.spec.collaboration,
        hiroute_application_api::AgentFacetIntent::Configure { .. }
    ) {
        let hiroute_application_api::AgentFacetIntent::Configure { settings } =
            &preview.spec.collaboration
        else {
            unreachable!()
        };
        result["collaboration_effect"] = json!(AgentCollaborationEffectV2 {
            action: "configure".into(),
            trigger_mode: settings.trigger_mode,
        });
    }
    result["model_effect"] = match (&preview.spec.model, &input.model_file.target) {
        (
            hiroute_application_api::AgentFacetIntent::Configure { settings },
            SettingsModelTargetFacts::Codex {
                provider_id,
                endpoint,
            },
        ) => json!({
            "action":"configure", "agent_class":"codex", "provider_id":provider_id, "endpoint":endpoint,
            "selection":settings,
            "authentication":"connection_scoped_local_grant",
        }),
        (
            hiroute_application_api::AgentFacetIntent::Configure { settings },
            SettingsModelTargetFacts::Claude(claude),
        ) => json!({
            "action":"configure", "agent_class":"claude", "endpoint":claude.gateway_base_url,
            "selection":settings,
            "authentication":"connection_scoped_local_grant",
        }),
        (hiroute_application_api::AgentFacetIntent::Restore { restore_point_ref }, _) => {
            json!({"action":"restore","restore_point_ref":restore_point_ref})
        }
        (hiroute_application_api::AgentFacetIntent::Keep, _) => json!({"action":"keep"}),
    };
    if !preview.context_windows.is_empty() {
        result["model_effect"]["context_windows"] = json!(preview.context_windows);
        result["model_effect"]["claude_context_window"] = json!(preview.claude_context_window);
    }
    succeeded(result, request.request_id)
}

pub(crate) fn apply(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    let payload: AgentSettingsApplyV2 = match serde_json::from_value(request.payload) {
        Ok(payload) => payload,
        Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
    };
    let restore = request.operation_id == "ApplyAgentConnectionRestore";
    if restore && !payload.spec.is_restore_only() {
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
    let input = match port.settings_facts(&payload.spec) {
        Ok(input) => input,
        Err(error) => return failed(map_control_error(error), request.request_id),
    };
    if !input.facts.login_item_removal_required && payload.login_item.is_some() {
        // New saves do not own startup state; an unsolicited host declaration must not be
        // silently accepted after an older Desktop has changed the system login item.
        return failed(ErrorCode::InvalidArguments, request.request_id);
    }
    if input.facts.login_item_removal_required
        && !payload
            .login_item
            .as_ref()
            .is_some_and(|declaration| declaration.removes_resident_service())
    {
        // Releasing the last managed connection must also release the login item this
        // feature owns; a restore that leaves it registered never completes.
        return failed(ErrorCode::GatewayUnavailable, request.request_id);
    }
    if input.expected_revisions != payload.expected_revisions {
        return failed(ErrorCode::ChangePreviewStale, request.request_id);
    }
    let confirmed = match confirm_agent_settings(payload.clone(), &input.facts) {
        Ok(confirmed) => confirmed,
        Err(error) => return failed(confirmation_error(error), request.request_id),
    };
    let plan = match input.seal_settings(&confirmed) {
        Ok(plan) => plan,
        Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
    };
    let apply = ApplyRequestV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        spec: plan.spec().clone(),
        accept_digest: payload.accept_digest.clone(),
        expected_revisions: payload.expected_revisions.clone(),
        idempotency_key: payload.idempotency_key.clone(),
        apply_capability: None,
    };
    let expected_plan = plan.clone();
    let port = port.clone();
    let admission = if restore {
        crate::PreparedTransactionV1::for_agent_settings_restore(
            apply,
            payload.accept_digest.clone(),
            plan,
        )
    } else {
        crate::PreparedTransactionV1::for_setup(apply, payload.accept_digest.clone(), plan)
    };
    let prepared = match admission {
        Ok(prepared) => prepared.with_revalidation(move || {
            let stale = || crate::TransactionError::ChangePreviewStale;
            let fresh = port.settings_facts(&payload.spec).map_err(|_| stale())?;
            if fresh.expected_revisions != payload.expected_revisions {
                return Err(stale());
            }
            let confirmed = confirm_agent_settings(payload, &fresh.facts).map_err(|_| stale())?;
            if fresh.seal_settings(&confirmed).map_err(|_| stale())? != expected_plan {
                return Err(stale());
            }
            Ok(())
        }),
        Err(error) => return failed(error.error_code(), request.request_id),
    };
    let Some(mutation) = ports.mutation.as_deref() else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    match mutation.apply_local_prepared_change(prepared) {
        Ok(operation) => super::accepted_apply(operation, request.request_id),
        Err(error) => failed(error.error_code(), request.request_id),
    }
}
fn confirmation_error(error: SettingsConfirmationError) -> ErrorCode {
    match error {
        SettingsConfirmationError::Invalid => ErrorCode::InvalidArguments,
        SettingsConfirmationError::Stale => ErrorCode::ChangePreviewStale,
        SettingsConfirmationError::Blocked => ErrorCode::CapabilityDenied,
    }
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
                if payload.schema_version == hiroute_application_api::AGENT_SETTINGS_SCHEMA_V2 =>
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
