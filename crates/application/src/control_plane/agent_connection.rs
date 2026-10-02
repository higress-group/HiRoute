//! Typed AgentConnection command surface.
//!
//! Integration adapters provide only exact discovery/publication facts. Application reproduces
//! Preview, binds the protected Apply authority, and seals the dedicated local grant mutation
//! into the normal recoverable transaction journal.

use hiroute_application_api::{
    AGENT_CONNECTION_PREVIEW_SCHEMA_V1, AgentConnectionApplyRequestV1,
    AgentConnectionEffectPreviewV1, AgentConnectionEffectStateV1, AgentConnectionPreviewRequestV1,
    AgentConnectionPreviewResultV1, AgentConnectionStatusRequestV1, AgentLaunchDescriptorRequestV1,
    ApplyRequestV1, CHANGE_SPEC_SCHEMA_V1, ErrorCode, LocalControlRequestV2, MachineEnvelopeV2,
};
use serde_json::Value;

use super::super::{ApplicationService, failed, failed_with_data, map_control_error, succeeded};
use crate::agent_connection::{
    AgentConnectionPlanner, AgentConnectionPlanningError, AgentConnectionPreviewV1,
    PreviewEffectStateV1,
};

const APPLY_OPERATION: &str = "ApplyAgentConnectionChange";

pub(crate) fn dispatch_preview_agent_connection(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.payload["spec"]["schema_version"]["major"] == 2 {
        return super::agent_settings::preview(service, request);
    }
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    let payload = match serde_json::from_value::<AgentConnectionPreviewRequestV1>(request.payload) {
        Ok(payload) => payload,
        Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
    };
    let Some(port) = service
        .ports
        .as_ref()
        .and_then(|ports| ports.agent_connection.as_deref())
    else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    let input = match port.planning_facts(&payload.spec) {
        Ok(input) => input,
        Err(error) => return failed(map_control_error(error), request.request_id),
    };
    let preview = match AgentConnectionPlanner::preview_connect(payload.spec, input.facts) {
        Ok(preview) => preview,
        Err(error) => return failed(map_planning_error(&error), request.request_id),
    };
    let result = public_preview(&preview, input.warnings, input.blockers);
    if result.applicable {
        succeeded(result, request.request_id)
    } else {
        failed_with_data(
            ErrorCode::AgentAuthPrecedenceConflict,
            result,
            request.request_id,
        )
    }
}

pub(crate) fn dispatch_apply_agent_connection(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.payload["spec"]["schema_version"]["major"] == 2
        || request.payload["schema"] == "hiroute.agent-settings-retry/v1"
    {
        return super::agent_settings::apply(service, request);
    }
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    let payload = match serde_json::from_value::<AgentConnectionApplyRequestV1>(request.payload) {
        Ok(payload) if !payload.idempotency_key.is_empty() => payload,
        _ => return failed(ErrorCode::InvalidArguments, request.request_id),
    };
    let Some(ports) = service.ports.as_ref() else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    if let Some(replay) = super::replay_apply(
        ports.control.as_ref(),
        hiroute_application_api::PrincipalKind::InteractiveUser,
        APPLY_OPERATION,
        &payload.idempotency_key,
        &payload.accept_digest,
        &request.request_id,
    ) {
        return replay;
    }
    let Some(connection) = ports.agent_connection.as_deref() else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    let input = match connection.planning_facts(&payload.spec) {
        Ok(input) => input,
        Err(error) => return failed(map_control_error(error), request.request_id),
    };
    if !input.blockers.is_empty() {
        return failed(ErrorCode::AgentAuthPrecedenceConflict, request.request_id);
    }
    let recheck_port = ports
        .agent_connection
        .as_ref()
        .expect("Agent port checked")
        .clone();
    let recheck_spec = payload.spec.clone();
    let recheck_digest = payload.accept_digest.clone();
    let recheck_revisions = payload.expected_revisions.clone();
    let preview = match AgentConnectionPlanner::preview_connect(payload.spec, input.facts) {
        Ok(preview) => preview,
        Err(error) => return failed(map_planning_error(&error), request.request_id),
    };
    let plan = match AgentConnectionPlanner::seal_apply(
        &preview,
        &payload.accept_digest,
        &payload.expected_revisions,
        &preview.expected_revisions,
    ) {
        Ok(plan) => plan,
        Err(error) => return failed(map_planning_error(&error), request.request_id),
    };
    let apply = ApplyRequestV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        spec: preview.change_spec,
        accept_digest: payload.accept_digest.clone(),
        expected_revisions: payload.expected_revisions,
        idempotency_key: payload.idempotency_key,
        apply_capability: None,
    };
    let prepared = match crate::PreparedTransactionV1::for_setup(apply, payload.accept_digest, plan)
    {
        Ok(prepared) => prepared.with_revalidation(move || {
            let input = recheck_port
                .planning_facts(&recheck_spec)
                .map_err(|_| crate::TransactionError::ChangePreviewStale)?;
            if !input.blockers.is_empty() {
                return Err(crate::TransactionError::ChangePreviewStale);
            }
            let preview = AgentConnectionPlanner::preview_connect(recheck_spec, input.facts)
                .map_err(|_| crate::TransactionError::ChangePreviewStale)?;
            AgentConnectionPlanner::seal_apply(
                &preview,
                &recheck_digest,
                &recheck_revisions,
                &preview.expected_revisions,
            )
            .map_err(|_| crate::TransactionError::ChangePreviewStale)?;
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

pub(crate) fn dispatch_status_agent_connection(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.payload["schema_version"]["major"] == 2 {
        return super::agent_settings::status(service, request);
    }
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    let payload = match serde_json::from_value::<AgentConnectionStatusRequestV1>(request.payload) {
        Ok(payload) => payload,
        Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
    };
    let Some(port) = service
        .ports
        .as_ref()
        .and_then(|ports| ports.agent_connection.as_deref())
    else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    match port.connection_status(&payload) {
        Ok(status) => succeeded(status, request.request_id),
        Err(error) => failed(map_control_error(error), request.request_id),
    }
}

pub(crate) fn dispatch_launch_descriptor(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    let payload = match serde_json::from_value::<AgentLaunchDescriptorRequestV1>(request.payload) {
        Ok(payload) => payload,
        Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
    };
    let Some(port) = service
        .ports
        .as_ref()
        .and_then(|ports| ports.agent_connection.as_deref())
    else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    match port.managed_launch_descriptor(&payload) {
        Ok(descriptor) => succeeded(descriptor, request.request_id),
        Err(error) => failed(map_control_error(error), request.request_id),
    }
}

fn public_preview(
    preview: &AgentConnectionPreviewV1,
    warnings: Vec<hiroute_application_api::WarningV1>,
    blockers: Vec<String>,
) -> AgentConnectionPreviewResultV1 {
    AgentConnectionPreviewResultV1 {
        schema: AGENT_CONNECTION_PREVIEW_SCHEMA_V1.to_owned(),
        applicable: blockers.is_empty(),
        change_digest: preview.change_digest.clone(),
        expected_revisions: preview.expected_revisions.clone(),
        agent_id: preview.connection.agent_id.clone(),
        profile_id: preview.connection.profile_id.clone(),
        installed_version: preview
            .change_spec
            .desired_state
            .get("installed_version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        activation_mode: preview.connection.activation_mode,
        warnings,
        blockers,
        effects: preview
            .effects
            .iter()
            .map(|effect| AgentConnectionEffectPreviewV1 {
                role: effect.role.clone(),
                state: match effect.state {
                    PreviewEffectStateV1::Planned => AgentConnectionEffectStateV1::Planned,
                    PreviewEffectStateV1::NoFieldChange => {
                        AgentConnectionEffectStateV1::NoFieldChange
                    }
                },
                desired_digest: effect.desired_digest.clone(),
            })
            .collect(),
    }
}

fn map_planning_error(error: &AgentConnectionPlanningError) -> ErrorCode {
    match error {
        AgentConnectionPlanningError::PreviewStale => ErrorCode::ChangePreviewStale,
        AgentConnectionPlanningError::ProfileMismatch => ErrorCode::ResourceNotFound,
        AgentConnectionPlanningError::NativeRoutingUnsupported
        | AgentConnectionPlanningError::CatalogUnavailable => ErrorCode::ActionRequired,
        AgentConnectionPlanningError::SchemaIncompatible => ErrorCode::SchemaIncompatible,
        _ => ErrorCode::InvalidArguments,
    }
}
