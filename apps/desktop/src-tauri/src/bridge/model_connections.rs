use super::{DesktopState, NativeConfirmation, NativeOutcome, confirm, main_window};
use crate::failure::DesktopFailure;
use crate::session::ModelSaveAccepted;
use hiroute_application_api::*;
use hiroute_diagnostics::event::ActionOperation;
use serde::Serialize;
use std::sync::atomic::Ordering;
use tauri::{State, WebviewWindow};

use super::model_connection_web::{
    WebComputeCandidateViewV2, WebComputeConnectionApplyRequestV1, WebComputeDiscoveryRefV1,
    WebComputeManagementChangeV2, WebComputeSavePreviewV2, WebComputeSaveResultV2,
    WebComputeScanResultV1, WebComputeSubscriptionCandidatesV2,
    WebComputeSubscriptionCheckResultV2, WebValidationRefV2,
};

#[derive(Serialize)]
pub struct ProtectedModelInputRegistration {
    input_candidate: ComputeCandidateRefV2,
}

fn erase_envelope<T: Serialize>(
    envelope: MachineEnvelopeV2<T>,
) -> Result<MachineEnvelopeV2<serde_json::Value>, DesktopFailure> {
    Ok(MachineEnvelopeV2 {
        schema_version: envelope.schema_version,
        request_id: envelope.request_id,
        status: envelope.status,
        data: envelope
            .data
            .map(serde_json::to_value)
            .transpose()
            .map_err(|_| "RESPONSE_DATA_INVALID")?,
        operation: envelope.operation,
        warnings: envelope.warnings,
        next_actions: envelope.next_actions,
        error: envelope.error,
    })
}

fn envelope_data<T: Serialize>(envelope: MachineEnvelopeV2<T>) -> Result<T, DesktopFailure> {
    if envelope.error.is_some() {
        return Err(DesktopFailure::backend(erase_envelope(envelope)?));
    }
    envelope.data.ok_or_else(|| "RESPONSE_DATA_MISSING".into())
}

#[tauri::command]
pub async fn compute_subscriptions(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<WebComputeSubscriptionCandidatesV2, DesktopFailure> {
    main_window(&window)?;
    let client = state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .client
        .clone();
    envelope_data(client.compute_subscriptions(&crate::random_id()?).await?).map(Into::into)
}

#[tauri::command]
pub async fn check_subscription(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    candidate: ComputeCandidateRefV2,
    language: String,
) -> Result<WebComputeSubscriptionCheckResultV2, DesktopFailure> {
    main_window(&window)?;
    let epoch = state.2.load(Ordering::SeqCst);
    let (client, close_requested) = {
        let mut guard = state.0.lock().await;
        let session = guard.as_mut().ok_or("RESIDENT_UNAVAILABLE")?;
        let close_requested = session.resume_subscription_interaction();
        (session.client.clone(), close_requested)
    };
    let preview = envelope_data(
        client
            .preview_subscription_check(&crate::random_id()?, candidate)
            .await?,
    )?;
    let context = state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .prepare_subscription_check_confirmation(
            preview,
            language,
            crate::random_id()?,
            close_requested,
        )?;
    match confirm(
        &window,
        &state,
        ActionOperation::SubscriptionCheck,
        NativeConfirmation::SubscriptionCheck(Box::new(context)),
        epoch,
    )
    .await?
    {
        NativeOutcome::SubscriptionCheck(result) => Ok((*result).into()),
        _ => unreachable!(),
    }
}

#[tauri::command]
pub async fn get_subscription_check_result(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    operation: OperationReferenceV1,
) -> Result<WebComputeSubscriptionCheckResultV2, DesktopFailure> {
    main_window(&window)?;
    let client = state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .client
        .clone();
    envelope_data(
        client
            .subscription_check_result(&crate::random_id()?, operation.operation_id)
            .await?,
    )
    .map(Into::into)
}

#[tauri::command]
pub async fn recover_subscription_check(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Option<WebComputeSubscriptionCheckResultV2>, DesktopFailure> {
    main_window(&window)?;
    let result = state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .recover_subscription_check()
        .await?;
    Ok(result.map(Into::into))
}

#[tauri::command]
pub async fn close_subscription_check(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<(), DesktopFailure> {
    main_window(&window)?;
    state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .close_subscription_check()
        .await
}

#[tauri::command]
pub async fn release_subscription_check(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    validation: WebValidationRefV2,
) -> Result<WebComputeSubscriptionCheckResultV2, DesktopFailure> {
    main_window(&window)?;
    state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .release_subscription_validation(validation.try_into().map_err(DesktopFailure::from)?)
        .await
        .map(Into::into)
}

#[tauri::command]
pub async fn compute_management_snapshot(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<ComputeManagementSnapshotV3, DesktopFailure> {
    main_window(&window)?;
    let client = state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .client
        .clone();
    envelope_data(
        client
            .compute_management_snapshot_v3(
                &crate::random_id()?,
                ComputeManagementQueryV2::default(),
            )
            .await?,
    )
}

#[tauri::command]
pub async fn compute_scan(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<WebComputeScanResultV1, DesktopFailure> {
    main_window(&window)?;
    let client = state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .client
        .clone();
    envelope_data(client.scan_compute(&crate::random_id()?).await?).map(Into::into)
}

#[tauri::command]
pub async fn prepare_discovered_model_connection(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    discovery: WebComputeDiscoveryRefV1,
) -> Result<WebComputeCandidateViewV2, DesktopFailure> {
    main_window(&window)?;
    let discovery: ComputeDiscoveryRefV1 = discovery.try_into().map_err(DesktopFailure::from)?;
    let digest_suffix = discovery
        .discovery_ref
        .strip_prefix("discovery/")
        .ok_or("REQUEST_INVALID")?;
    let request = PrepareDiscoveredModelConnectionRequestV1 {
        prepare_id: format!("desktop/{digest_suffix}"),
        discovery,
    };
    let client = state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .client
        .clone();
    let status: ClientServiceStatusV1 =
        crate::session::query(&client, "GetClientServiceStatus", &ClientEmptyRequestV1 {}).await?;
    let digest = CanonicalDigest::of(&request).map_err(|_| "REQUEST_INVALID")?;
    #[cfg(unix)]
    let capability = state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .resident
        .register_discovered_model_prepare(&digest, &status.revisions)?;
    #[cfg(not(unix))]
    let capability: zeroize::Zeroizing<String> = return Err("TRUSTED_AUTHORITY_UNAVAILABLE".into());
    envelope_data(
        client
            .prepare_discovered_model_connection(
                &crate::random_id()?,
                request,
                ProtectedClientGrantV2 {
                    principal_kind: PrincipalKind::Desktop,
                    capability: capability.to_string(),
                },
            )
            .await?,
    )
    .map(Into::into)
}

#[tauri::command]
pub async fn compute_connection_options(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<ComputeConnectionOptionsResultV1, DesktopFailure> {
    main_window(&window)?;
    let client = state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .client
        .clone();
    crate::session::query(
        &client,
        "ListConnectionOptions",
        &ComputeConnectionOptionsRequestV1 {},
    )
    .await
}

#[tauri::command]
pub async fn register_protected_model_input(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    secret: String,
) -> Result<ProtectedModelInputRegistration, DesktopFailure> {
    main_window(&window)?;
    #[cfg(unix)]
    {
        let candidate = state
            .0
            .lock()
            .await
            .as_mut()
            .ok_or("RESIDENT_UNAVAILABLE")?
            .resident
            .register_model_input(zeroize::Zeroizing::new(secret))?;
        Ok(ProtectedModelInputRegistration {
            input_candidate: candidate,
        })
    }
    #[cfg(not(unix))]
    {
        let _ = (state, secret);
        Err("TRUSTED_AUTHORITY_UNAVAILABLE".into())
    }
}

#[tauri::command]
pub async fn release_protected_model_input(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: ComputeCandidateRefV2,
) -> Result<(), DesktopFailure> {
    main_window(&window)?;
    #[cfg(unix)]
    {
        state
            .0
            .lock()
            .await
            .as_mut()
            .ok_or("RESIDENT_UNAVAILABLE")?
            .resident
            .release_model_input(&input)
            .map_err(Into::into)
    }
    #[cfg(not(unix))]
    {
        let _ = (state, input);
        Err("TRUSTED_AUTHORITY_UNAVAILABLE".into())
    }
}

#[tauri::command]
pub async fn check_model_connection(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: NativeModelConnectionCheckRequestV1,
) -> Result<ModelConnectionCheckViewV1, DesktopFailure> {
    main_window(&window)?;
    let client = state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .client
        .clone();
    let status: ClientServiceStatusV1 = crate::session::query(
        &client,
        "GetClientServiceStatus",
        &hiroute_application_api::ClientEmptyRequestV1 {},
    )
    .await?;
    let digest = CanonicalDigest::of(&request).map_err(|_| "REQUEST_INVALID")?;
    #[cfg(unix)]
    let capability = state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .resident
        .register_model_check(&digest, &status.revisions)?;
    #[cfg(not(unix))]
    let capability: zeroize::Zeroizing<String> = return Err("TRUSTED_AUTHORITY_UNAVAILABLE".into());
    envelope_data(
        client
            .check_native_model_connection(
                &crate::random_id()?,
                request,
                ProtectedClientGrantV2 {
                    principal_kind: PrincipalKind::Desktop,
                    capability: capability.to_string(),
                },
            )
            .await?,
    )
}

#[tauri::command]
pub async fn check_saved_model_connection(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: SavedModelConnectionCheckRequestV1,
) -> Result<ModelConnectionCheckViewV1, DesktopFailure> {
    main_window(&window)?;
    let client = state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .client
        .clone();
    let status: ClientServiceStatusV1 = crate::session::query(
        &client,
        "GetClientServiceStatus",
        &hiroute_application_api::ClientEmptyRequestV1 {},
    )
    .await?;
    let digest = CanonicalDigest::of(&request).map_err(|_| "REQUEST_INVALID")?;
    #[cfg(unix)]
    let capability = state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .resident
        .register_saved_model_check(&digest, &status.revisions)?;
    #[cfg(not(unix))]
    let capability: zeroize::Zeroizing<String> = return Err("TRUSTED_AUTHORITY_UNAVAILABLE".into());
    envelope_data(
        client
            .check_saved_model_connection(
                &crate::random_id()?,
                request,
                ProtectedClientGrantV2 {
                    principal_kind: PrincipalKind::Desktop,
                    capability: capability.to_string(),
                },
            )
            .await?,
    )
}

#[tauri::command]
pub async fn check_registered_model_connection(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: RegisteredModelConnectionCheckRequestV1,
) -> Result<ModelConnectionCheckViewV1, DesktopFailure> {
    main_window(&window)?;
    let client = state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .client
        .clone();
    let status: ClientServiceStatusV1 = crate::session::query(
        &client,
        "GetClientServiceStatus",
        &hiroute_application_api::ClientEmptyRequestV1 {},
    )
    .await?;
    let digest = CanonicalDigest::of(&request).map_err(|_| "REQUEST_INVALID")?;
    #[cfg(unix)]
    let capability = state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .resident
        .register_registered_model_check(&digest, &status.revisions)?;
    #[cfg(not(unix))]
    let capability: zeroize::Zeroizing<String> = return Err("TRUSTED_AUTHORITY_UNAVAILABLE".into());
    envelope_data(
        client
            .check_registered_model_connection(
                &crate::random_id()?,
                request,
                ProtectedClientGrantV2 {
                    principal_kind: PrincipalKind::Desktop,
                    capability: capability.to_string(),
                },
            )
            .await?,
    )
}

#[tauri::command]
pub async fn cancel_model_connection_check(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    check_id: String,
) -> Result<(), DesktopFailure> {
    main_window(&window)?;
    let client = state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .client
        .clone();
    envelope_data(
        client
            .cancel_native_model_connection_check(&crate::random_id()?, check_id)
            .await?,
    )
}

#[tauri::command]
pub async fn preview_compute_save(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    change: WebComputeManagementChangeV2,
) -> Result<WebComputeSavePreviewV2, DesktopFailure> {
    main_window(&window)?;
    let client = state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .client
        .clone();
    let change = change.try_into().map_err(DesktopFailure::from)?;
    let preview = envelope_data(
        client
            .preview_compute_save(&crate::random_id()?, change)
            .await?,
    )?;
    preview.try_into().map_err(DesktopFailure::from)
}

#[tauri::command]
pub async fn apply_compute_save(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: WebComputeConnectionApplyRequestV1,
    language: String,
) -> Result<ModelSaveAccepted, DesktopFailure> {
    main_window(&window)?;
    let request: ComputeConnectionApplyRequestV1 =
        request.try_into().map_err(DesktopFailure::from)?;
    let epoch = state.2.load(Ordering::SeqCst);
    let context = state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .prepare_model_save_confirmation(request, language)?;
    match confirm(
        &window,
        &state,
        ActionOperation::ModelSave,
        NativeConfirmation::ModelSave(Box::new(context)),
        epoch,
    )
    .await?
    {
        NativeOutcome::ModelSave(value) => Ok(value),
        _ => unreachable!(),
    }
}

#[tauri::command]
pub async fn get_compute_save_result(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    operation: OperationReferenceV1,
) -> Result<WebComputeSaveResultV2, DesktopFailure> {
    main_window(&window)?;
    let client = state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .client
        .clone();
    envelope_data(
        client
            .get_compute_save_result(&crate::random_id()?, operation)
            .await?,
    )
    .map(Into::into)
}
