//! Native entries for the shared daemon-owned feed reads; WebView cannot supply a URL or key.
use super::*;
use hiroute_application_api::*;

#[tauri::command]
pub(super) async fn model_feed_status(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<ModelFeedStatusV2, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::model_feed_status(&context, &product_state).await
}

#[tauri::command]
pub(super) async fn model_feed_refresh(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<ModelFeedStatusV2, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::model_feed_refresh(&context, &product_state).await
}

#[tauri::command]
pub(super) async fn model_feed_settings_apply(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: ModelFeedSettingsRequestV1,
) -> Result<ModelFeedStatusV2, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::model_feed_settings_apply(&context, &product_state, input)
        .await
}

#[tauri::command]
pub(super) async fn model_feed_remove_preview(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: Option<ModelFeedRemovalQueryV2>,
) -> Result<ModelFeedRemovalPreviewV2, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::model_feed_remove_preview(&context, &product_state, input)
        .await
}

#[tauri::command]
pub(super) async fn model_feed_remove_apply(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: ModelFeedRemoveRequestV1,
) -> Result<ModelFeedStatusV2, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::model_feed_remove_apply(&context, &product_state, input)
        .await
}

#[tauri::command]
pub(super) async fn model_availability(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<ModelAvailabilityResultV2, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::model_availability(&context, &product_state).await
}

#[tauri::command]
pub(super) async fn list_model_metadata(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: ListModelMetadataV1,
) -> Result<ModelMetadataPageV1, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::list_model_metadata(&context, &product_state, input).await
}

#[tauri::command]
pub(super) async fn get_model_metadata(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: GetModelMetadataV1,
) -> Result<ModelMetadataDetailV1, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::get_model_metadata(&context, &product_state, input).await
}

#[tauri::command]
pub(super) async fn compute_defaults(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: ComputeDefaultsQueryV1,
) -> Result<ComputeDefaultsResultV1, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::compute_defaults(&context, &product_state, input).await
}

#[tauri::command]
pub(super) async fn preview_compute_save_v3(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    change: hiroute_product_session::commands::WebComputeManagementChangeV2,
    default_preferences: hiroute_domain::SourceDefaultPreferencesV1,
) -> Result<hiroute_product_session::commands::WebComputeSavePreviewV2, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::preview_compute_save_v3(
        &context,
        &product_state,
        change,
        default_preferences,
    )
    .await
}
