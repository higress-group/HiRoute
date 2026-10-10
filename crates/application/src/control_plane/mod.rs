mod agent_connection;
pub(crate) mod agent_settings;
mod compute;
mod plan_content;
mod plan_draft;
mod plan_lifecycle;
mod publication_checkpoint;
pub(crate) mod subscription_login;

use hiroute_application_api::{
    APPLY_COMPUTE_SAVE_OPERATION_V2, APPLY_SUBSCRIPTION_CHECK_OPERATION_V2, ApplyRequestV1,
    CHANGE_SPEC_SCHEMA_V1, COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2, ComputeConnectionApplyRequestV1,
    ComputeConnectionAuthorizationRequestV1, ComputeConnectionOptionsRequestV1,
    ComputeConnectionPreviewRequestV1, ComputeConnectionTestRequestV1, ComputeSavePreviewRequestV2,
    ComputeScanRequestV1, ComputeSubscriptionCandidatesV2,
    ComputeSubscriptionCheckPreviewRequestV2, ComputeSubscriptionDiscoveryStateV2, ErrorCode,
    LocalControlRequestV2, MachineEnvelopeV2, PreviewRequestV1, PreviewResultV1, PrincipalKind,
};
use serde_json::Value;

use super::{ApplicationService, accepted_apply, failed, map_control_error, succeeded};

pub(crate) use agent_connection::{
    dispatch_apply_agent_connection, dispatch_launch_descriptor, dispatch_preview_agent_connection,
    dispatch_status_agent_connection,
};

pub(crate) fn dispatch_scan_compute(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.protected_grant.is_some()
        || serde_json::from_value::<ComputeScanRequestV1>(request.payload.clone()).is_err()
    {
        return failed(ErrorCode::InvalidArguments, request.request_id);
    }
    let Some(port) = compute_port(service) else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    match compute::scan_compute(port) {
        Ok(result) => succeeded(result, request.request_id),
        Err(error) => failed(map_compute_error(error), request.request_id),
    }
}

pub(crate) fn dispatch_connection_options(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.protected_grant.is_some()
        || serde_json::from_value::<ComputeConnectionOptionsRequestV1>(request.payload.clone())
            .is_err()
    {
        return failed(ErrorCode::InvalidArguments, request.request_id);
    }
    let Some(port) = compute_port(service) else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    match compute::connection_options(port) {
        Ok(mut result) => {
            result.subscriptions = service
                .ports
                .as_ref()
                .and_then(|ports| ports.compute_management.as_deref())
                .map(|management| {
                    management.compute_subscriptions().unwrap_or_else(|_| {
                        ComputeSubscriptionCandidatesV2 {
                            candidates: Vec::new(),
                            discovery_state:
                                ComputeSubscriptionDiscoveryStateV2::RuntimeUnavailable,
                            reason_code: Some("subscription_discovery_failed".into()),
                        }
                    })
                });
            succeeded(result, request.request_id)
        }
        Err(error) => failed(map_compute_error(error), request.request_id),
    }
}

pub(crate) fn dispatch_preview_compute(
    service: &ApplicationService,
    mut request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    if request
        .payload
        .get("change")
        .and_then(|change| change.get("schema"))
        .and_then(Value::as_str)
        == Some(COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2)
    {
        let Ok(preview) = serde_json::from_value::<ComputeSavePreviewRequestV2>(request.payload)
        else {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        };
        let Some(management) = management_port(service) else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        return match management.preview_compute_save(preview.change) {
            Ok(result) => succeeded(result, request.request_id),
            Err(error) => failed(map_management_error(error), request.request_id),
        };
    }
    if request.payload.get("candidate").is_some() {
        let Ok(preview) =
            serde_json::from_value::<ComputeSubscriptionCheckPreviewRequestV2>(request.payload)
        else {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        };
        let Some(management) = management_port(service) else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        return match management.preview_subscription_check(preview.candidate) {
            Ok(result) => succeeded(result, request.request_id),
            Err(error) => failed(map_management_error(error), request.request_id),
        };
    }
    let Some(port) = compute_port(service) else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    let payload = match serde_json::from_value::<ComputeConnectionPreviewRequestV1>(
        request.payload.clone(),
    ) {
        Ok(payload) => payload,
        Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
    };
    let prepared = match compute::prepare_compute(port, payload.change) {
        Ok(prepared) => prepared,
        Err(error) => return failed(map_compute_error(error), request.request_id),
    };
    request.payload = serde_json::to_value(PreviewRequestV1::new(prepared.spec.clone()))
        .expect("internal Compute Preview DTO is serializable");
    let response = service.preview_change(request, "compute.connection.apply");
    if response.error.is_some() {
        return response;
    }
    let request_id = response.request_id.unwrap_or_default();
    let result = response
        .data
        .and_then(|value| serde_json::from_value::<PreviewResultV1>(value).ok());
    match result {
        Some(result) => succeeded(compute::public_preview(prepared, result), request_id),
        None => failed(ErrorCode::Internal, request_id),
    }
}

pub(crate) fn dispatch_apply_compute(
    service: &ApplicationService,
    mut request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    let payload =
        match serde_json::from_value::<ComputeConnectionApplyRequestV1>(request.payload.clone()) {
            Ok(payload) if !payload.idempotency_key.is_empty() => payload,
            _ => return failed(ErrorCode::InvalidArguments, request.request_id),
        };
    let Some(ports) = service.ports.as_ref() else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    let management_kind =
        if payload.spec.command_id == hiroute_domain::COMPUTE_SUBSCRIPTION_CHECK_COMMAND_ID_V2 {
            Some(APPLY_SUBSCRIPTION_CHECK_OPERATION_V2)
        } else if payload.spec.command_id == "compute.connection.apply"
            && payload
                .spec
                .desired_state
                .get("schema")
                .and_then(Value::as_str)
                == Some(COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2)
        {
            Some(APPLY_COMPUTE_SAVE_OPERATION_V2)
        } else {
            None
        };
    if let Some(operation_kind) = management_kind {
        if let Some(replay) = replay_apply(
            ports.control.as_ref(),
            PrincipalKind::InteractiveUser,
            operation_kind,
            &payload.idempotency_key,
            &payload.accept_digest,
            &request.request_id,
        ) {
            return replay;
        }
        let Some(management) = ports.compute_management.as_deref() else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        let prepared = if operation_kind == APPLY_SUBSCRIPTION_CHECK_OPERATION_V2 {
            management.prepare_subscription_check(payload, None)
        } else {
            management.prepare_compute_save(payload)
        };
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(error) => return failed(map_management_error(error), request.request_id),
        };
        let Some(mutation) = ports.mutation.as_ref() else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        return match mutation.apply_local_prepared_change(prepared) {
            Ok(operation) => accepted_apply(operation, request.request_id),
            Err(error) => failed(error.error_code(), request.request_id),
        };
    }
    if payload.spec.command_id != "compute.connection.apply" {
        return failed(ErrorCode::InvalidArguments, request.request_id);
    }
    if let Some(replay) = replay_apply(
        ports.control.as_ref(),
        PrincipalKind::InteractiveUser,
        "ApplyComputeConnectionChange",
        &payload.idempotency_key,
        &payload.accept_digest,
        &request.request_id,
    ) {
        return replay;
    }
    let Some(port) = ports.compute.as_deref() else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    if let Err(error) = compute::validate_apply_spec(port, &payload.spec) {
        return failed(map_compute_error(error), request.request_id);
    }
    request.payload = serde_json::to_value(ApplyRequestV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        spec: payload.spec,
        accept_digest: payload.accept_digest,
        expected_revisions: payload.expected_revisions,
        idempotency_key: payload.idempotency_key,
        apply_capability: None,
    })
    .expect("internal Compute Apply DTO is serializable");
    service.apply_change(request, "compute.connection.apply")
}

pub(crate) fn dispatch_test_compute(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    let Ok(test) = serde_json::from_value::<ComputeConnectionTestRequestV1>(request.payload) else {
        return failed(ErrorCode::InvalidArguments, request.request_id);
    };
    let Some(management) = management_port(service) else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    let result = match test {
        ComputeConnectionTestRequestV1::Native { request } => management
            .check_native_model_connection(request)
            .and_then(|value| {
                serde_json::to_value(value)
                    .map_err(|_| super::control::ComputeManagementControlError::Corrupt)
            }),
        ComputeConnectionTestRequestV1::Registered { request } => management
            .check_registered_model_connection(request)
            .and_then(|value| {
                serde_json::to_value(value)
                    .map_err(|_| super::control::ComputeManagementControlError::Corrupt)
            }),
        ComputeConnectionTestRequestV1::Discovered { request } => management
            .prepare_discovered_model_connection(request)
            .and_then(|value| {
                serde_json::to_value(value)
                    .map_err(|_| super::control::ComputeManagementControlError::Corrupt)
            }),
        ComputeConnectionTestRequestV1::Saved { request } => management
            .check_saved_model_connection(request)
            .and_then(|value| {
                serde_json::to_value(value)
                    .map_err(|_| super::control::ComputeManagementControlError::Corrupt)
            }),
    };
    match result {
        Ok(result) => succeeded(result, request.request_id),
        Err(error) => failed(map_management_error(error), request.request_id),
    }
}

pub(crate) fn dispatch_authorize_compute(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    let Ok(action) =
        serde_json::from_value::<ComputeConnectionAuthorizationRequestV1>(request.payload)
    else {
        return failed(ErrorCode::InvalidArguments, request.request_id);
    };
    let Some(management) = management_port(service) else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    let result = match action {
        ComputeConnectionAuthorizationRequestV1::Result { operation } => {
            management.compute_subscription_check_result(&operation)
        }
        ComputeConnectionAuthorizationRequestV1::Release { validation } => {
            if validation.validate_shape().is_err() {
                return failed(ErrorCode::InvalidArguments, request.request_id);
            }
            management.release_subscription_check(&validation)
        }
    };
    match result {
        Ok(result) => succeeded(result, request.request_id),
        Err(error) => failed(map_management_error(error), request.request_id),
    }
}

pub(crate) fn dispatch_preview_routing(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request
        .payload
        .get("change")
        .and_then(|c| c.get("schema"))
        .and_then(Value::as_str)
        == Some(hiroute_application_api::PUBLICATION_CHECKPOINT_CHANGE_SCHEMA_V1)
    {
        return publication_checkpoint::dispatch_preview(service, request);
    }
    if request
        .payload
        .get("change")
        .and_then(|c| c.get("schema"))
        .and_then(Value::as_str)
        == Some(hiroute_application_api::PLAN_CONTENT_CHANGE_SCHEMA_V2)
    {
        return plan_content::dispatch_preview(service, request);
    }
    if request
        .payload
        .get("change")
        .and_then(|c| c.get("schema"))
        .and_then(Value::as_str)
        == Some(hiroute_application_api::PLAN_LIFECYCLE_CHANGE_SCHEMA_V1)
    {
        return plan_lifecycle::dispatch_preview(service, request);
    }
    if request
        .payload
        .get("change")
        .and_then(|c| c.get("schema"))
        .and_then(Value::as_str)
        == Some(hiroute_domain::PLAN_DRAFT_CHANGE_SCHEMA_V1)
    {
        return plan_draft::dispatch_preview(service, request);
    }

    failed(ErrorCode::SchemaIncompatible, request.request_id)
}

pub(crate) fn dispatch_apply_routing(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request
        .payload
        .get("change")
        .and_then(|c| c.get("schema"))
        .and_then(Value::as_str)
        == Some(hiroute_application_api::PUBLICATION_CHECKPOINT_CHANGE_SCHEMA_V1)
    {
        return publication_checkpoint::dispatch_apply(service, request);
    }
    if request
        .payload
        .get("change")
        .and_then(|c| c.get("schema"))
        .and_then(Value::as_str)
        == Some(hiroute_application_api::PLAN_CONTENT_CHANGE_SCHEMA_V2)
    {
        return plan_content::dispatch_apply(service, request);
    }
    if request
        .payload
        .get("change")
        .and_then(|c| c.get("schema"))
        .and_then(Value::as_str)
        == Some(hiroute_application_api::PLAN_LIFECYCLE_CHANGE_SCHEMA_V1)
    {
        return plan_lifecycle::dispatch_apply(service, request);
    }
    if request
        .payload
        .get("change")
        .and_then(|c| c.get("schema"))
        .and_then(Value::as_str)
        == Some(hiroute_domain::PLAN_DRAFT_CHANGE_SCHEMA_V1)
    {
        return plan_draft::dispatch_apply(service, request);
    }

    failed(ErrorCode::SchemaIncompatible, request.request_id)
}

fn compute_port(service: &ApplicationService) -> Option<&dyn super::control::ComputeFactsPort> {
    service
        .ports
        .as_ref()
        .and_then(|ports| ports.compute.as_deref())
}

fn management_port(
    service: &ApplicationService,
) -> Option<&dyn super::control::ComputeManagementControlPort> {
    service
        .ports
        .as_ref()
        .and_then(|ports| ports.compute_management.as_deref())
}

fn map_management_error(error: super::control::ComputeManagementControlError) -> ErrorCode {
    use super::control::ComputeManagementControlError as E;
    match error {
        E::Invalid => ErrorCode::InvalidArguments,
        E::NotFound | E::RegisteredOptionUnavailable | E::DiscoveryUnavailable => {
            ErrorCode::ResourceNotFound
        }
        E::Conflict
        | E::RegisteredCatalogChanged
        | E::RegisteredSourceMismatch
        | E::SavedSourceMismatch
        | E::DiscoveryChanged => ErrorCode::RevisionConflict,
        E::PreviewStale => ErrorCode::ChangePreviewStale,
        E::ActionRequired | E::RecheckContextUnavailable | E::DiscoveryNotImportable => {
            ErrorCode::ActionRequired
        }
        E::Unavailable => ErrorCode::DaemonUnavailable,
        E::Corrupt => ErrorCode::Internal,
    }
}

fn map_compute_error(error: compute::ComputeControlError) -> ErrorCode {
    match error {
        compute::ComputeControlError::InvalidArguments => ErrorCode::InvalidArguments,
        compute::ComputeControlError::InvalidSelection => ErrorCode::ResourceNotFound,
        compute::ComputeControlError::RevisionConflict => ErrorCode::RevisionConflict,
        compute::ComputeControlError::ActionRequired => ErrorCode::ActionRequired,
        compute::ComputeControlError::Control(error) => map_control_error(error),
    }
}

#[cfg(test)]
mod tests;

/// Early replay is a read optimization; the writer performs the authoritative lookup again.
pub(crate) fn replay_apply(
    control: &dyn crate::control::ControlStatePort,
    principal: PrincipalKind,
    kind: &str,
    key: &str,
    digest: &hiroute_domain::CanonicalDigest,
    request_id: &str,
) -> Option<MachineEnvelopeV2<Value>> {
    match control.operation_for_idempotency(
        &hiroute_domain::WorkspaceId::default(),
        principal,
        kind,
        key,
    ) {
        Ok(None) => None,
        Ok(Some(existing)) if &existing.accepted_digest == digest => {
            Some(accepted_apply(existing, request_id.to_owned()))
        }
        Ok(Some(_)) => Some(failed(
            ErrorCode::IdempotencyKeyReused,
            request_id.to_owned(),
        )),
        Err(error) => Some(failed(map_control_error(error), request_id.to_owned())),
    }
}
