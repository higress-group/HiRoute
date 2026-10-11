use hiroute_application_api::{
    APPLY_COMPUTE_SAVE_OPERATION_V2, APPLY_SUBSCRIPTION_CHECK_OPERATION_V2, ApplyResultV1,
    CANCEL_NATIVE_MODEL_CONNECTION_CHECK_OPERATION_V1, CHECK_NATIVE_MODEL_CONNECTION_OPERATION_V1,
    CHECK_REGISTERED_MODEL_CONNECTION_OPERATION_V1, CHECK_SAVED_MODEL_CONNECTION_OPERATION_V1,
    CanonicalDigest, ClientEmptyRequestV1, ComputeCandidateQueryV2,
    ComputeConnectionApplyRequestV1, ComputeManagementQueryV2, ComputeSavePreviewRequestV2,
    ComputeSaveResultQueryV2, ComputeSubscriptionCheckPreviewRequestV2,
    ComputeSubscriptionCheckResultQueryV2, ComputeValidationRefV2, ErrorCode, ErrorV1,
    GET_COMPUTE_CANDIDATE_OPERATION_V2, GET_COMPUTE_OPERATION_V3,
    GET_COMPUTE_SAVE_RESULT_OPERATION_V2, GET_SUBSCRIPTION_CHECK_RESULT_OPERATION_V2,
    LIST_COMPUTE_OPERATION_V3, LIST_COMPUTE_SUBSCRIPTIONS_OPERATION_V2, LocalControlRequestV2,
    MachineEnvelopeV2, NativeModelConnectionCancelRequestV1, NativeModelConnectionCheckRequestV1,
    PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1, PREVIEW_COMPUTE_SAVE_OPERATION_V2,
    PREVIEW_SUBSCRIPTION_CHECK_OPERATION_V2, PrepareDiscoveredModelConnectionRequestV1,
    PrincipalKind, RELEASE_SUBSCRIPTION_CHECK_OPERATION_V2,
    RegisteredModelConnectionCheckRequestV1, SavedModelConnectionCheckRequestV1,
};
use hiroute_domain::WorkspaceId;
use serde_json::Value;

use crate::ApplicationService;
use crate::control::ComputeManagementControlError;

pub(crate) fn is_compute_management_operation(operation: &str) -> bool {
    matches!(
        operation,
        "ListCompute"
            | "GetCompute"
            | LIST_COMPUTE_OPERATION_V3
            | GET_COMPUTE_OPERATION_V3
            | GET_COMPUTE_CANDIDATE_OPERATION_V2
            | PREVIEW_COMPUTE_SAVE_OPERATION_V2
            | APPLY_COMPUTE_SAVE_OPERATION_V2
            | GET_COMPUTE_SAVE_RESULT_OPERATION_V2
            | CHECK_NATIVE_MODEL_CONNECTION_OPERATION_V1
            | CHECK_REGISTERED_MODEL_CONNECTION_OPERATION_V1
            | PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1
            | CHECK_SAVED_MODEL_CONNECTION_OPERATION_V1
            | CANCEL_NATIVE_MODEL_CONNECTION_CHECK_OPERATION_V1
            | LIST_COMPUTE_SUBSCRIPTIONS_OPERATION_V2
            | PREVIEW_SUBSCRIPTION_CHECK_OPERATION_V2
            | APPLY_SUBSCRIPTION_CHECK_OPERATION_V2
            | GET_SUBSCRIPTION_CHECK_RESULT_OPERATION_V2
            | RELEASE_SUBSCRIPTION_CHECK_OPERATION_V2
    )
}

pub(crate) fn dispatch_compute_management(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    let Some(ports) = service.ports.as_ref() else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    let Some(management) = ports.compute_management.as_ref() else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    let request_id = request.request_id;
    match request.operation_id.as_str() {
        LIST_COMPUTE_SUBSCRIPTIONS_OPERATION_V2 => {
            if request.protected_grant.is_some() {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            if serde_json::from_value::<ClientEmptyRequestV1>(request.payload).is_err() {
                return failed(ErrorCode::InvalidArguments, request_id);
            }
            result(management.compute_subscriptions(), request_id)
        }
        PREVIEW_SUBSCRIPTION_CHECK_OPERATION_V2 => {
            if request.protected_grant.is_some() {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            let Ok(preview) =
                serde_json::from_value::<ComputeSubscriptionCheckPreviewRequestV2>(request.payload)
            else {
                return failed(ErrorCode::InvalidArguments, request_id);
            };
            result(
                management.preview_subscription_check(preview.candidate),
                request_id,
            )
        }
        APPLY_SUBSCRIPTION_CHECK_OPERATION_V2 => {
            let Some(grant) = request.protected_grant else {
                return failed(ErrorCode::CapabilityDenied, request_id);
            };
            if grant.principal_kind != PrincipalKind::Desktop {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            let Ok(apply) =
                serde_json::from_value::<ComputeConnectionApplyRequestV1>(request.payload)
            else {
                return failed(ErrorCode::InvalidArguments, request_id);
            };
            if apply.idempotency_key.trim().is_empty() {
                return failed(ErrorCode::InvalidArguments, request_id);
            }
            if let Ok(Some(operation)) = ports.control.operation_for_idempotency(
                &WorkspaceId::default(),
                grant.principal_kind,
                APPLY_SUBSCRIPTION_CHECK_OPERATION_V2,
                &apply.idempotency_key,
            ) {
                if operation.accepted_digest != apply.accept_digest {
                    return failed(ErrorCode::IdempotencyKeyReused, request_id);
                }
                return accepted(operation, request_id);
            }
            let Some(mutation) = ports.mutation.as_ref() else {
                return failed(ErrorCode::DaemonUnavailable, request_id);
            };
            let prepared =
                match management.prepare_subscription_check(apply, Some(grant.capability)) {
                    Ok(prepared) => prepared,
                    Err(error) => return failed(map_error(error), request_id),
                };
            match mutation.apply_prepared_change(grant.principal_kind, prepared) {
                Ok(operation) => accepted(operation, request_id),
                Err(error) => failed(error.error_code(), request_id),
            }
        }
        GET_SUBSCRIPTION_CHECK_RESULT_OPERATION_V2 => {
            if request.protected_grant.is_some() {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            let Ok(query) =
                serde_json::from_value::<ComputeSubscriptionCheckResultQueryV2>(request.payload)
            else {
                return failed(ErrorCode::InvalidArguments, request_id);
            };
            result(
                management.compute_subscription_check_result(&query.operation),
                request_id,
            )
        }
        RELEASE_SUBSCRIPTION_CHECK_OPERATION_V2 => {
            let Some(grant) = request.protected_grant else {
                return failed(ErrorCode::CapabilityDenied, request_id);
            };
            if grant.principal_kind != PrincipalKind::Desktop {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            let Ok(validation) = serde_json::from_value::<ComputeValidationRefV2>(request.payload)
            else {
                return failed(ErrorCode::InvalidArguments, request_id);
            };
            if validation.validate_shape().is_err() {
                return failed(ErrorCode::InvalidArguments, request_id);
            }
            let digest = match CanonicalDigest::of(&validation) {
                Ok(value) => value,
                Err(_) => return failed(ErrorCode::InvalidArguments, request_id),
            };
            let revisions = match ports.control.snapshot(&WorkspaceId::default()) {
                Ok(value) => value.revisions,
                Err(_) => return failed(ErrorCode::DaemonUnavailable, request_id),
            };
            if ports
                .control
                .validate_protected_capability(
                    &grant.capability,
                    &WorkspaceId::default(),
                    grant.principal_kind,
                    RELEASE_SUBSCRIPTION_CHECK_OPERATION_V2,
                    &digest,
                    &revisions,
                )
                .is_err()
            {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            result(
                management.release_subscription_check(&validation),
                request_id,
            )
        }
        LIST_COMPUTE_OPERATION_V3 | GET_COMPUTE_OPERATION_V3 => {
            if request.protected_grant.is_some() {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            let Ok(query) = serde_json::from_value::<ComputeManagementQueryV2>(request.payload)
            else {
                return failed(ErrorCode::InvalidArguments, request_id);
            };
            result(
                management.compute_management_snapshot_v3(&query),
                request_id,
            )
        }
        "ListCompute" | "GetCompute" => {
            if request.protected_grant.is_some() {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            let Ok(query) = serde_json::from_value::<ComputeManagementQueryV2>(request.payload)
            else {
                return failed(ErrorCode::InvalidArguments, request_id);
            };
            result(management.compute_management_snapshot(&query), request_id)
        }
        GET_COMPUTE_CANDIDATE_OPERATION_V2 => {
            if request.protected_grant.is_some() {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            let Ok(query) = serde_json::from_value::<ComputeCandidateQueryV2>(request.payload)
            else {
                return failed(ErrorCode::InvalidArguments, request_id);
            };
            // Public views contain no protected input locator.
            result(
                management.get_compute_candidate(&query.candidate),
                request_id,
            )
        }
        PREVIEW_COMPUTE_SAVE_OPERATION_V2 => {
            if request.protected_grant.is_some() {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            let Ok(preview) =
                serde_json::from_value::<ComputeSavePreviewRequestV2>(request.payload)
            else {
                return failed(ErrorCode::InvalidArguments, request_id);
            };
            result(management.preview_compute_save(preview.change), request_id)
        }
        APPLY_COMPUTE_SAVE_OPERATION_V2 => {
            if request.protected_grant.is_some() {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            let Ok(apply) =
                serde_json::from_value::<ComputeConnectionApplyRequestV1>(request.payload)
            else {
                return failed(ErrorCode::InvalidArguments, request_id);
            };
            if apply.idempotency_key.trim().is_empty() {
                return failed(ErrorCode::InvalidArguments, request_id);
            }
            if let Ok(Some(operation)) = ports.control.operation_for_idempotency(
                &WorkspaceId::default(),
                PrincipalKind::InteractiveUser,
                APPLY_COMPUTE_SAVE_OPERATION_V2,
                &apply.idempotency_key,
            ) {
                if operation.accepted_digest != apply.accept_digest {
                    return failed(ErrorCode::IdempotencyKeyReused, request_id);
                }
                return accepted(operation, request_id);
            }
            let Some(mutation) = ports.mutation.as_ref() else {
                return failed(ErrorCode::DaemonUnavailable, request_id);
            };
            let prepared = match management.prepare_compute_save(apply) {
                Ok(prepared) => prepared,
                Err(error) => return failed(map_error(error), request_id),
            };
            match mutation.apply_local_prepared_change(prepared) {
                Ok(operation) => accepted(operation, request_id),
                Err(error) => failed(error.error_code(), request_id),
            }
        }
        GET_COMPUTE_SAVE_RESULT_OPERATION_V2 => {
            if request.protected_grant.is_some() {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            let Ok(query) = serde_json::from_value::<ComputeSaveResultQueryV2>(request.payload)
            else {
                return failed(ErrorCode::InvalidArguments, request_id);
            };
            result(management.compute_save_result(&query.operation), request_id)
        }
        CHECK_NATIVE_MODEL_CONNECTION_OPERATION_V1 => {
            let Some(grant) = request.protected_grant.as_ref() else {
                return failed(ErrorCode::CapabilityDenied, request_id);
            };
            if grant.principal_kind != PrincipalKind::Desktop {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            let Ok(check) =
                serde_json::from_value::<NativeModelConnectionCheckRequestV1>(request.payload)
            else {
                return failed(ErrorCode::InvalidArguments, request_id);
            };
            let digest = match CanonicalDigest::of(&check) {
                Ok(value) => value,
                Err(_) => return failed(ErrorCode::InvalidArguments, request_id),
            };
            let revisions = match ports.control.snapshot(&WorkspaceId::default()) {
                Ok(value) => value.revisions,
                Err(_) => return failed(ErrorCode::DaemonUnavailable, request_id),
            };
            if ports
                .control
                .validate_protected_capability(
                    &grant.capability,
                    &WorkspaceId::default(),
                    grant.principal_kind,
                    CHECK_NATIVE_MODEL_CONNECTION_OPERATION_V1,
                    &digest,
                    &revisions,
                )
                .is_err()
            {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            result(management.check_native_model_connection(check), request_id)
        }
        CHECK_REGISTERED_MODEL_CONNECTION_OPERATION_V1 => {
            let Some(grant) = request.protected_grant.as_ref() else {
                return failed(ErrorCode::CapabilityDenied, request_id);
            };
            if grant.principal_kind != PrincipalKind::Desktop {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            let Ok(check) =
                serde_json::from_value::<RegisteredModelConnectionCheckRequestV1>(request.payload)
            else {
                return failed(ErrorCode::InvalidArguments, request_id);
            };
            let digest = match CanonicalDigest::of(&check) {
                Ok(value) => value,
                Err(_) => return failed(ErrorCode::InvalidArguments, request_id),
            };
            let revisions = match ports.control.snapshot(&WorkspaceId::default()) {
                Ok(value) => value.revisions,
                Err(_) => return failed(ErrorCode::DaemonUnavailable, request_id),
            };
            if ports
                .control
                .validate_protected_capability(
                    &grant.capability,
                    &WorkspaceId::default(),
                    grant.principal_kind,
                    CHECK_REGISTERED_MODEL_CONNECTION_OPERATION_V1,
                    &digest,
                    &revisions,
                )
                .is_err()
            {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            result(
                management.check_registered_model_connection(check),
                request_id,
            )
        }
        PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1 => {
            let Some(grant) = request.protected_grant.as_ref() else {
                return failed(ErrorCode::CapabilityDenied, request_id);
            };
            if grant.principal_kind != PrincipalKind::Desktop {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            let Ok(prepare) = serde_json::from_value::<PrepareDiscoveredModelConnectionRequestV1>(
                request.payload,
            ) else {
                return failed(ErrorCode::InvalidArguments, request_id);
            };
            if !prepare.valid() {
                return failed(ErrorCode::InvalidArguments, request_id);
            }
            let digest = match CanonicalDigest::of(&prepare) {
                Ok(value) => value,
                Err(_) => return failed(ErrorCode::InvalidArguments, request_id),
            };
            let revisions = match ports.control.snapshot(&WorkspaceId::default()) {
                Ok(value) => value.revisions,
                Err(_) => return failed(ErrorCode::DaemonUnavailable, request_id),
            };
            if ports
                .control
                .validate_protected_capability(
                    &grant.capability,
                    &WorkspaceId::default(),
                    grant.principal_kind,
                    PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1,
                    &digest,
                    &revisions,
                )
                .is_err()
            {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            result(
                management.prepare_discovered_model_connection(prepare),
                request_id,
            )
        }
        CHECK_SAVED_MODEL_CONNECTION_OPERATION_V1 => {
            let Some(grant) = request.protected_grant.as_ref() else {
                return failed(ErrorCode::CapabilityDenied, request_id);
            };
            if grant.principal_kind != PrincipalKind::Desktop {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            let Ok(check) =
                serde_json::from_value::<SavedModelConnectionCheckRequestV1>(request.payload)
            else {
                return failed(ErrorCode::InvalidArguments, request_id);
            };
            if !check.valid() {
                return failed(ErrorCode::InvalidArguments, request_id);
            }
            let digest = match CanonicalDigest::of(&check) {
                Ok(value) => value,
                Err(_) => return failed(ErrorCode::InvalidArguments, request_id),
            };
            let revisions = match ports.control.snapshot(&WorkspaceId::default()) {
                Ok(value) => value.revisions,
                Err(_) => return failed(ErrorCode::DaemonUnavailable, request_id),
            };
            if ports
                .control
                .validate_protected_capability(
                    &grant.capability,
                    &WorkspaceId::default(),
                    grant.principal_kind,
                    CHECK_SAVED_MODEL_CONNECTION_OPERATION_V1,
                    &digest,
                    &revisions,
                )
                .is_err()
            {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            result(management.check_saved_model_connection(check), request_id)
        }
        CANCEL_NATIVE_MODEL_CONNECTION_CHECK_OPERATION_V1 => {
            if request.protected_grant.is_some() {
                return failed(ErrorCode::CapabilityDenied, request_id);
            }
            let Ok(cancel) =
                serde_json::from_value::<NativeModelConnectionCancelRequestV1>(request.payload)
            else {
                return failed(ErrorCode::InvalidArguments, request_id);
            };
            result(
                management.cancel_native_model_connection_check(&cancel.check_id),
                request_id,
            )
        }
        _ => failed(ErrorCode::UnknownCommand, request_id),
    }
}

fn accepted(
    operation: hiroute_domain::OperationV1,
    request_id: String,
) -> MachineEnvelopeV2<Value> {
    let reference = hiroute_application_api::OperationReferenceV1 {
        operation_id: operation.operation_id.to_string(),
        state: operation.state.as_str().to_owned(),
        sequence: operation.generation,
        cancellable: !operation.state.is_terminal(),
    };
    let data = ApplyResultV1 {
        operation_id: reference.operation_id.clone(),
        accepted_digest: operation.accepted_digest,
        state: reference.state.clone(),
    };
    let mut envelope = MachineEnvelopeV2::accepted(
        serde_json::to_value(data).expect("Apply result is serializable"),
        Some(request_id),
    );
    envelope.operation = Some(reference);
    envelope
}

fn result<T: serde::Serialize>(
    value: Result<T, ComputeManagementControlError>,
    request_id: String,
) -> MachineEnvelopeV2<Value> {
    match value {
        Ok(value) => MachineEnvelopeV2::succeeded(
            serde_json::to_value(value).expect("compute-management DTO is serializable"),
            Some(request_id),
        ),
        Err(error) => failed_control(error, request_id),
    }
}

fn map_error(error: ComputeManagementControlError) -> ErrorCode {
    match error {
        ComputeManagementControlError::Invalid => ErrorCode::InvalidArguments,
        ComputeManagementControlError::NotFound => ErrorCode::ResourceNotFound,
        ComputeManagementControlError::Conflict => ErrorCode::RevisionConflict,
        ComputeManagementControlError::RegisteredOptionUnavailable => ErrorCode::ResourceNotFound,
        ComputeManagementControlError::RegisteredCatalogChanged
        | ComputeManagementControlError::RegisteredSourceMismatch
        | ComputeManagementControlError::SavedSourceMismatch
        | ComputeManagementControlError::DiscoveryChanged => ErrorCode::RevisionConflict,
        ComputeManagementControlError::PreviewStale => ErrorCode::ChangePreviewStale,
        ComputeManagementControlError::ActionRequired
        | ComputeManagementControlError::RecheckContextUnavailable
        | ComputeManagementControlError::DiscoveryNotImportable => ErrorCode::ActionRequired,
        ComputeManagementControlError::DiscoveryUnavailable => ErrorCode::ResourceNotFound,
        ComputeManagementControlError::Unavailable => ErrorCode::DaemonUnavailable,
        ComputeManagementControlError::Corrupt => ErrorCode::Internal,
    }
}

fn failed_control(
    error: ComputeManagementControlError,
    request_id: String,
) -> MachineEnvelopeV2<Value> {
    let mut failure = ErrorV1::new(map_error(error));
    failure.message_key = match error {
        ComputeManagementControlError::RegisteredOptionUnavailable => {
            "compute.registered_option_unavailable"
        }
        ComputeManagementControlError::RegisteredCatalogChanged => {
            "compute.registered_catalog_changed"
        }
        ComputeManagementControlError::RegisteredSourceMismatch => {
            "compute.registered_source_mismatch"
        }
        ComputeManagementControlError::SavedSourceMismatch => "compute.saved_source_mismatch",
        ComputeManagementControlError::RecheckContextUnavailable => {
            "compute.recheck_context_unavailable"
        }
        ComputeManagementControlError::DiscoveryUnavailable => "compute.discovery_unavailable",
        ComputeManagementControlError::DiscoveryChanged => "compute.discovery_changed",
        ComputeManagementControlError::DiscoveryNotImportable => "compute.discovery_not_importable",
        _ => return MachineEnvelopeV2::failed(failure, Some(request_id)),
    }
    .into();
    MachineEnvelopeV2::failed(failure, Some(request_id))
}

fn failed(code: ErrorCode, request_id: String) -> MachineEnvelopeV2<Value> {
    MachineEnvelopeV2::failed(ErrorV1::new(code), Some(request_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_business_failures_keep_closed_message_keys() {
        for (error, code, key) in [
            (
                ComputeManagementControlError::RegisteredOptionUnavailable,
                ErrorCode::ResourceNotFound,
                "compute.registered_option_unavailable",
            ),
            (
                ComputeManagementControlError::RegisteredCatalogChanged,
                ErrorCode::RevisionConflict,
                "compute.registered_catalog_changed",
            ),
            (
                ComputeManagementControlError::RegisteredSourceMismatch,
                ErrorCode::RevisionConflict,
                "compute.registered_source_mismatch",
            ),
        ] {
            let envelope = failed_control(error, "request/registered".into());
            let failure = envelope.error.unwrap();
            assert_eq!(failure.code, code);
            assert_eq!(failure.message_key, key);
        }
    }

    #[test]
    fn discovered_prepare_failures_keep_stable_closed_message_keys() {
        for (error, code, key) in [
            (
                ComputeManagementControlError::DiscoveryUnavailable,
                ErrorCode::ResourceNotFound,
                "compute.discovery_unavailable",
            ),
            (
                ComputeManagementControlError::DiscoveryChanged,
                ErrorCode::RevisionConflict,
                "compute.discovery_changed",
            ),
            (
                ComputeManagementControlError::DiscoveryNotImportable,
                ErrorCode::ActionRequired,
                "compute.discovery_not_importable",
            ),
        ] {
            let envelope = failed_control(error, "request/discovery".into());
            let failure = envelope.error.unwrap();
            assert_eq!(failure.code, code);
            assert_eq!(failure.message_key, key);
        }
    }
}
