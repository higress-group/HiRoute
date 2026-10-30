//! Versioned network operations share the established save writer and protected check authority.
use super::*;
use hiroute_application_api::*;

pub(super) fn handles(operation: &str) -> bool {
    matches!(
        operation,
        GET_NATIVE_SOURCE_NETWORK_V1
            | PREVIEW_NATIVE_SOURCE_NETWORK_V1
            | APPLY_NATIVE_SOURCE_NETWORK_V1
            | PREVIEW_NATIVE_NETWORK_ROUTE_V1
            | PREVIEW_COMPUTE_SAVE_V4
            | CHECK_NATIVE_MODEL_CONNECTION_V2
            | CHECK_REGISTERED_MODEL_CONNECTION_V2
            | CHECK_SAVED_MODEL_CONNECTION_V2
    )
}

pub(super) fn dispatch(
    service: &ApplicationService,
    mut request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    let Some(ports) = service.ports.as_ref() else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    let Some(management) = ports.compute_management.as_ref() else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    if request.operation_id == APPLY_NATIVE_SOURCE_NETWORK_V1 {
        let Ok(apply) =
            serde_json::from_value::<ComputeConnectionApplyRequestV1>(request.payload.clone())
        else {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        };
        if apply
            .spec
            .desired_state
            .get("schema")
            .and_then(Value::as_str)
            != Some(hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V4)
        {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        }
        // One admitted Operation/effect owner; no parallel source-policy writer or receipt.
        request.operation_id = APPLY_COMPUTE_SAVE_OPERATION_V2.into();
        return dispatch_compute_management(service, request);
    }
    let id = request.request_id.clone();
    macro_rules! read {
        ($ty:ty, $method:ident) => {{
            if request.protected_grant.is_some() {
                return failed(ErrorCode::CapabilityDenied, id);
            }
            let Ok(value) = serde_json::from_value::<$ty>(request.payload) else {
                return failed(ErrorCode::InvalidArguments, id);
            };
            result(management.$method(value), id)
        }};
    }
    macro_rules! check {
        ($ty:ty, $method:ident) => {{
            let Some(grant) = request.protected_grant.as_ref() else {
                return failed(ErrorCode::CapabilityDenied, id);
            };
            if grant.principal_kind != PrincipalKind::Desktop {
                return failed(ErrorCode::CapabilityDenied, id);
            }
            let Ok(value) = serde_json::from_value::<$ty>(request.payload) else {
                return failed(ErrorCode::InvalidArguments, id);
            };
            let Ok(digest) = CanonicalDigest::of(&value) else {
                return failed(ErrorCode::InvalidArguments, id);
            };
            let Ok(snapshot) = ports.control.snapshot(&WorkspaceId::default()) else {
                return failed(ErrorCode::DaemonUnavailable, id);
            };
            if ports
                .control
                .validate_protected_capability(
                    &grant.capability,
                    &WorkspaceId::default(),
                    grant.principal_kind,
                    &request.operation_id,
                    &digest,
                    &snapshot.revisions,
                )
                .is_err()
            {
                return failed(ErrorCode::CapabilityDenied, id);
            }
            result(management.$method(value), id)
        }};
    }
    match request.operation_id.as_str() {
        GET_NATIVE_SOURCE_NETWORK_V1 => {
            read!(NativeSourceNetworkQueryV1, get_native_source_network)
        }
        PREVIEW_NATIVE_SOURCE_NETWORK_V1 => {
            read!(NativeSourceNetworkPreviewV1, preview_native_source_network)
        }
        PREVIEW_NATIVE_NETWORK_ROUTE_V1 => read!(
            NativeNetworkRoutePreviewRequestV1,
            preview_native_network_route
        ),
        PREVIEW_COMPUTE_SAVE_V4 => read!(ComputeManagementChangeV4, preview_compute_save_v4),
        CHECK_NATIVE_MODEL_CONNECTION_V2 => check!(
            NativeModelConnectionCheckRequestV2,
            check_native_model_connection_v2
        ),
        CHECK_REGISTERED_MODEL_CONNECTION_V2 => check!(
            RegisteredModelConnectionCheckRequestV2,
            check_registered_model_connection_v2
        ),
        CHECK_SAVED_MODEL_CONNECTION_V2 => check!(
            SavedModelConnectionCheckRequestV2,
            check_saved_model_connection_v2
        ),
        _ => failed(ErrorCode::InvalidArguments, id),
    }
}
