//! Optional source network settings use the same host authority and model-save confirmation.
use super::*;
use crate::HostAuthority;
use serde::Serialize;

#[derive(Serialize)]
pub struct WebNativeNetworkSavePreviewV1 {
    pub preview: WebComputeSavePreviewV2,
    pub before: NativeSourceNetworkViewV1,
    pub after: hiroute_domain::NativeNetworkPolicyV1,
}

pub async fn native_source_network(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: NativeSourceNetworkQueryV1,
) -> Result<NativeSourceNetworkViewV1, DesktopFailure> {
    super::model_feed::query(window, state, GET_NATIVE_SOURCE_NETWORK_V1, &input).await
}
pub async fn preview_native_network_route(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: NativeNetworkRoutePreviewRequestV1,
) -> Result<NativeNetworkRouteViewV1, DesktopFailure> {
    super::model_feed::query(window, state, PREVIEW_NATIVE_NETWORK_ROUTE_V1, &input).await
}
pub async fn preview_native_source_network(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: NativeSourceNetworkPreviewV1,
) -> Result<WebNativeNetworkSavePreviewV1, DesktopFailure> {
    let view: NativeNetworkSavePreviewV1 =
        super::model_feed::query(window, state, PREVIEW_NATIVE_SOURCE_NETWORK_V1, &input).await?;
    Ok(WebNativeNetworkSavePreviewV1 {
        preview: view.preview.try_into().map_err(DesktopFailure::from)?,
        before: view.before,
        after: view.after,
    })
}
pub async fn apply_native_source_network(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    request: WebComputeConnectionApplyRequestV1,
    language: String,
) -> Result<ModelSaveAccepted, DesktopFailure> {
    if !request.is_native_network_save() {
        return Err("REQUEST_INVALID".into());
    }
    super::model_connections::apply_compute_save(window, state, request, language).await
}
pub async fn preview_compute_save_v4(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    change: WebComputeManagementChangeV2,
    default_preferences: hiroute_domain::SourceDefaultPreferencesV1,
    network_edit: NativeNetworkEditV1,
) -> Result<WebComputeSavePreviewV2, DesktopFailure> {
    let inner = ComputeManagementChangeV3 {
        schema: hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3.into(),
        change: change.try_into().map_err(DesktopFailure::from)?,
        default_preferences,
    };
    let change = ComputeManagementChangeV4 {
        schema: hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V4.into(),
        change: serde_json::to_value(inner).map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?,
        network_edit,
    };
    let preview: ComputeSavePreviewV2 =
        super::model_feed::query(window, state, PREVIEW_COMPUTE_SAVE_V4, &change).await?;
    preview.try_into().map_err(DesktopFailure::from)
}
pub async fn check_model_connection_v2(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    request: NativeModelConnectionCheckRequestV2,
) -> Result<ModelConnectionCheckViewV2, DesktopFailure> {
    main_window(window)?;
    let (client, authority) = {
        let guard = state.0.lock().await;
        let session = guard.as_ref().ok_or("RESIDENT_UNAVAILABLE")?;
        (session.client.clone(), session.authority.clone())
    };
    let status: ClientServiceStatusV1 =
        crate::session::query(&client, "GetClientServiceStatus", &ClientEmptyRequestV1 {}).await?;
    let digest =
        CanonicalDigest::of(&request).map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
    let capability = authority.register_operation(
        &digest,
        &status.revisions,
        CHECK_NATIVE_MODEL_CONNECTION_V2,
    )?;
    let response = client
        .check_native_model_connection_v2(
            &crate::random_id()?,
            request,
            ProtectedClientGrantV2 {
                principal_kind: PrincipalKind::Desktop,
                capability: capability.to_string(),
            },
        )
        .await?;
    let guard = state.0.lock().await;
    if !guard
        .as_ref()
        .is_some_and(|s| std::sync::Arc::ptr_eq(&s.authority, &authority))
    {
        return Err("RESIDENT_UNAVAILABLE".into());
    }
    if response.error.is_some() {
        return Err(DesktopFailure::backend(MachineEnvelopeV2 {
            schema_version: response.schema_version,
            request_id: response.request_id,
            status: response.status,
            data: None,
            operation: response.operation,
            warnings: response.warnings,
            next_actions: response.next_actions,
            error: response.error,
        }));
    }
    response.data.ok_or_else(|| "RESPONSE_DATA_MISSING".into())
}
pub async fn check_registered_model_connection_v2(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    request: RegisteredModelConnectionCheckRequestV2,
) -> Result<ModelConnectionCheckViewV2, DesktopFailure> {
    main_window(window)?;
    let (client, authority) = {
        let guard = state.0.lock().await;
        let session = guard.as_ref().ok_or("RESIDENT_UNAVAILABLE")?;
        (session.client.clone(), session.authority.clone())
    };
    let status: ClientServiceStatusV1 =
        crate::session::query(&client, "GetClientServiceStatus", &ClientEmptyRequestV1 {}).await?;
    let digest =
        CanonicalDigest::of(&request).map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
    let capability = authority.register_operation(
        &digest,
        &status.revisions,
        CHECK_REGISTERED_MODEL_CONNECTION_V2,
    )?;
    let response = client
        .check_registered_model_connection_v2(
            &crate::random_id()?,
            request,
            ProtectedClientGrantV2 {
                principal_kind: PrincipalKind::Desktop,
                capability: capability.to_string(),
            },
        )
        .await?;
    let guard = state.0.lock().await;
    if !guard
        .as_ref()
        .is_some_and(|s| std::sync::Arc::ptr_eq(&s.authority, &authority))
    {
        return Err("RESIDENT_UNAVAILABLE".into());
    }
    if response.error.is_some() {
        return Err(DesktopFailure::backend(MachineEnvelopeV2 {
            schema_version: response.schema_version,
            request_id: response.request_id,
            status: response.status,
            data: None,
            operation: response.operation,
            warnings: response.warnings,
            next_actions: response.next_actions,
            error: response.error,
        }));
    }
    response.data.ok_or_else(|| "RESPONSE_DATA_MISSING".into())
}
pub async fn check_saved_model_connection_v2(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    request: SavedModelConnectionCheckRequestV2,
) -> Result<ModelConnectionCheckViewV2, DesktopFailure> {
    main_window(window)?;
    let (client, authority) = {
        let guard = state.0.lock().await;
        let session = guard.as_ref().ok_or("RESIDENT_UNAVAILABLE")?;
        (session.client.clone(), session.authority.clone())
    };
    let status: ClientServiceStatusV1 =
        crate::session::query(&client, "GetClientServiceStatus", &ClientEmptyRequestV1 {}).await?;
    let digest =
        CanonicalDigest::of(&request).map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
    let capability = authority.register_operation(
        &digest,
        &status.revisions,
        CHECK_SAVED_MODEL_CONNECTION_V2,
    )?;
    let response = client
        .check_saved_model_connection_v2(
            &crate::random_id()?,
            request,
            ProtectedClientGrantV2 {
                principal_kind: PrincipalKind::Desktop,
                capability: capability.to_string(),
            },
        )
        .await?;
    let guard = state.0.lock().await;
    if !guard
        .as_ref()
        .is_some_and(|s| std::sync::Arc::ptr_eq(&s.authority, &authority))
    {
        return Err("RESIDENT_UNAVAILABLE".into());
    }
    if response.error.is_some() {
        return Err(DesktopFailure::backend(MachineEnvelopeV2 {
            schema_version: response.schema_version,
            request_id: response.request_id,
            status: response.status,
            data: None,
            operation: response.operation,
            warnings: response.warnings,
            next_actions: response.next_actions,
            error: response.error,
        }));
    }
    response.data.ok_or_else(|| "RESPONSE_DATA_MISSING".into())
}
