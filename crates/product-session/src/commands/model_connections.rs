use super::{ClientContext, CommandState, NativeConfirmation, NativeOutcome, confirm, main_window};
use crate::HostAuthority;
use crate::failure::DesktopFailure;
use crate::session::ModelSaveAccepted;
use hiroute_application_api::*;
use hiroute_diagnostics::event::ActionOperation;
use serde::Serialize;
use std::sync::atomic::Ordering;

use super::model_connection_web::{
    WebComputeCandidateViewV2, WebComputeConnectionApplyRequestV1, WebComputeDiscoveryRefV1,
    WebComputeManagementChangeV2, WebComputeSavePreviewV2, WebComputeSaveResultV2,
    WebComputeScanResultV2, WebComputeSubscriptionCandidatesV2,
    WebComputeSubscriptionCheckResultV2, WebValidationRefV2,
};

#[derive(Serialize)]
pub struct ProtectedModelInputRegistration {
    input_candidate: ComputeCandidateRefV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_id: Option<String>,
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

pub async fn compute_subscriptions(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    source_id: Option<String>,
    expected_source_revision: Option<u64>,
) -> Result<WebComputeSubscriptionCandidatesV2, DesktopFailure> {
    main_window(window)?;
    let client = state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .client
        .clone();
    match (source_id, expected_source_revision) {
        (None, None) => {
            envelope_data(client.compute_subscriptions(&crate::random_id()?).await?).map(Into::into)
        }
        (Some(source_id), Some(expected_source_revision)) => {
            let value: hiroute_application_api::ComputeSubscriptionCandidatesV2 =
                crate::session::query(
                    &client,
                    hiroute_application_api::LIST_COMPUTE_SUBSCRIPTIONS_OPERATION_V2,
                    &hiroute_application_api::ComputeSubscriptionCandidatesQueryV1 {
                        source: Some(hiroute_application_api::ComputeSavedSubscriptionSourceV1 {
                            source_id,
                            expected_source_revision,
                        }),
                    },
                )
                .await?;
            Ok(value.into())
        }
        _ => Err("REQUEST_INVALID".into()),
    }
}

pub async fn check_subscription(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    candidate: ComputeCandidateRefV2,
    language: String,
) -> Result<WebComputeSubscriptionCheckResultV2, DesktopFailure> {
    main_window(window)?;
    let epoch = state.2.load(Ordering::SeqCst);
    let (client, close_requested) = {
        let mut guard = super::confirmation_session(state, epoch).await?;
        let session = guard.as_mut().ok_or("RESIDENT_UNAVAILABLE")?;
        let close_requested = session.resume_subscription_interaction();
        (session.client.clone(), close_requested)
    };
    let preview = envelope_data(
        client
            .preview_subscription_check(&crate::random_id()?, candidate)
            .await?,
    )?;
    let context = super::confirmation_session(state, epoch)
        .await?
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .prepare_subscription_check_confirmation(
            preview,
            language,
            crate::random_id()?,
            close_requested,
        )?;
    match confirm(
        window,
        state,
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

pub async fn get_subscription_check_result(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    operation: OperationReferenceV1,
) -> Result<WebComputeSubscriptionCheckResultV2, DesktopFailure> {
    main_window(window)?;
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

pub async fn recover_subscription_check(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
) -> Result<Option<WebComputeSubscriptionCheckResultV2>, DesktopFailure> {
    main_window(window)?;
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

pub async fn close_subscription_check(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
) -> Result<(), DesktopFailure> {
    main_window(window)?;
    state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .close_subscription_check()
        .await
}

pub async fn release_subscription_check(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    validation: WebValidationRefV2,
) -> Result<WebComputeSubscriptionCheckResultV2, DesktopFailure> {
    main_window(window)?;
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

pub async fn compute_key_masks(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    query: ComputeKeyMasksQueryV1,
) -> Result<ComputeKeyMasksViewV1, DesktopFailure> {
    main_window(window)?;
    let client = state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .client
        .clone();
    crate::session::query(&client, GET_COMPUTE_KEY_MASKS_OPERATION_V1, &query).await
}

pub async fn compute_management_snapshot(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
) -> Result<ComputeManagementSnapshotV3, DesktopFailure> {
    main_window(window)?;
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

pub async fn compute_scan(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    native_target: Option<hiroute_domain::AgentNativeTargetV1>,
) -> Result<WebComputeScanResultV2, DesktopFailure> {
    main_window(window)?;
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
            .scan_compute_points(
                &crate::random_id()?,
                ComputeScanRequestV2 {
                    schema: COMPUTE_SCAN_REQUEST_SCHEMA_V2.into(),
                    native_target,
                },
            )
            .await?,
    )
    .map(Into::into)
}

pub async fn prepare_discovered_model_connection(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    discovery: WebComputeDiscoveryRefV1,
) -> Result<WebComputeCandidateViewV2, DesktopFailure> {
    main_window(window)?;
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
    let capability = state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .authority
        .register_discovered_model_prepare(&digest, &status.revisions)?;
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

pub async fn compute_connection_options(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    include_request_model_ids: Option<bool>,
) -> Result<ComputeConnectionOptionsResultV1, DesktopFailure> {
    main_window(window)?;
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
        &ComputeConnectionOptionsRequestV1 {
            candidate: None,
            include_request_model_ids: include_request_model_ids.unwrap_or(false),
        },
    )
    .await
}

pub async fn register_protected_model_input(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    secret: String,
    source_id: Option<String>,
) -> Result<ProtectedModelInputRegistration, DesktopFailure> {
    main_window(window)?;
    {
        let candidate = state
            .0
            .lock()
            .await
            .as_mut()
            .ok_or("RESIDENT_UNAVAILABLE")?
            .authority
            .register_model_input(zeroize::Zeroizing::new(secret))?;
        let key_id = source_id
            .as_deref()
            .map(|source| hiroute_domain::managed_key_id(source, &candidate.candidate_ref))
            .transpose()
            .map_err(|_| "CREDENTIAL_INPUT_INVALID")?;
        Ok(ProtectedModelInputRegistration {
            input_candidate: candidate,
            key_id,
        })
    }
}

pub async fn release_protected_model_input(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: ComputeCandidateRefV2,
) -> Result<(), DesktopFailure> {
    main_window(window)?;
    {
        state
            .0
            .lock()
            .await
            .as_mut()
            .ok_or("RESIDENT_UNAVAILABLE")?
            .authority
            .release_model_input(&input)
            .map_err(Into::into)
    }
}

pub async fn check_model_connection(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    request: NativeModelConnectionCheckRequestV1,
) -> Result<ModelConnectionCheckViewV1, DesktopFailure> {
    main_window(window)?;
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
    let capability = state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .authority
        .register_model_check(&digest, &status.revisions)?;
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

pub async fn check_saved_model_connection(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    request: SavedModelConnectionCheckRequestV1,
) -> Result<ModelConnectionCheckViewV1, DesktopFailure> {
    main_window(window)?;
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
    let capability = state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .authority
        .register_saved_model_check(&digest, &status.revisions)?;
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

pub async fn check_registered_model_connection(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    request: RegisteredModelConnectionCheckRequestV1,
) -> Result<ModelConnectionCheckViewV1, DesktopFailure> {
    main_window(window)?;
    if request.display_name.is_none() {
        return Err("REQUEST_INVALID".into());
    }
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
    let capability = state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .authority
        .register_registered_model_check(&digest, &status.revisions)?;
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

pub async fn cancel_model_connection_check(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    check_id: String,
) -> Result<(), DesktopFailure> {
    main_window(window)?;
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

pub async fn preview_compute_save(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    change: WebComputeManagementChangeV2,
) -> Result<WebComputeSavePreviewV2, DesktopFailure> {
    main_window(window)?;
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

pub async fn apply_compute_save(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    request: WebComputeConnectionApplyRequestV1,
    language: String,
) -> Result<ModelSaveAccepted, DesktopFailure> {
    main_window(window)?;
    let request: ComputeConnectionApplyRequestV1 =
        request.try_into().map_err(DesktopFailure::from)?;
    let epoch = state.2.load(Ordering::SeqCst);
    let context = super::confirmation_session(state, epoch)
        .await?
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .prepare_model_save_confirmation(request, language)
        .await?;
    match confirm(
        window,
        state,
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

pub async fn get_compute_save_result(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    operation: OperationReferenceV1,
) -> Result<WebComputeSaveResultV2, DesktopFailure> {
    main_window(window)?;
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

pub async fn compute_defaults(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: hiroute_application_api::ComputeDefaultsQueryV1,
) -> Result<hiroute_application_api::ComputeDefaultsResultV1, DesktopFailure> {
    super::model_feed::query(window, state, "GetComputeDefaults", &input).await
}
pub async fn preview_compute_save_v3(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    change: WebComputeManagementChangeV2,
    default_preferences: hiroute_domain::SourceDefaultPreferencesV1,
) -> Result<WebComputeSavePreviewV2, DesktopFailure> {
    let change = hiroute_application_api::ComputeManagementChangeV3 {
        schema: hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3.into(),
        change: change.try_into().map_err(DesktopFailure::from)?,
        default_preferences,
    };
    let result: hiroute_application_api::ComputeSavePreviewV2 = super::model_feed::query(
        window,
        state,
        "PreviewComputeSaveV3",
        &hiroute_application_api::ComputeSavePreviewRequestV3 { change },
    )
    .await?;
    result.try_into().map_err(DesktopFailure::from)
}
