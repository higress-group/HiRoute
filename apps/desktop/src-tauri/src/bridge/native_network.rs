use super::*;
use hiroute_product_session::commands::WebNativeNetworkSavePreviewV1;
#[tauri::command]
pub async fn native_source_network(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: NativeSourceNetworkQueryV1,
) -> Result<NativeSourceNetworkViewV1, DesktopFailure> {
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
    hiroute_product_session::commands::native_source_network(&context, &product_state, input).await
}
#[tauri::command]
pub async fn preview_native_network_route(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: NativeNetworkRoutePreviewRequestV1,
) -> Result<NativeNetworkRouteViewV1, DesktopFailure> {
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
    hiroute_product_session::commands::preview_native_network_route(&context, &product_state, input)
        .await
}
#[tauri::command]
pub async fn preview_native_source_network(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: NativeSourceNetworkPreviewV1,
) -> Result<WebNativeNetworkSavePreviewV1, DesktopFailure> {
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
    hiroute_product_session::commands::preview_native_source_network(
        &context,
        &product_state,
        input,
    )
    .await
}
#[tauri::command]
pub async fn apply_native_source_network(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: WebComputeConnectionApplyRequestV1,
    language: String,
) -> Result<ModelSaveAccepted, DesktopFailure> {
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
    hiroute_product_session::commands::apply_native_source_network(
        &context,
        &product_state,
        request,
        language,
    )
    .await
}
#[tauri::command]
pub async fn preview_compute_save_v4(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    change: WebComputeManagementChangeV2,
    default_preferences: hiroute_domain::SourceDefaultPreferencesV1,
    network_edit: NativeNetworkEditV1,
) -> Result<WebComputeSavePreviewV2, DesktopFailure> {
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
    hiroute_product_session::commands::preview_compute_save_v4(
        &context,
        &product_state,
        change,
        default_preferences,
        network_edit,
    )
    .await
}
#[tauri::command]
pub async fn check_model_connection_v2(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: NativeModelConnectionCheckRequestV2,
) -> Result<ModelConnectionCheckViewV2, DesktopFailure> {
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
    hiroute_product_session::commands::check_model_connection_v2(&context, &product_state, request)
        .await
}
#[tauri::command]
pub async fn check_registered_model_connection_v2(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: RegisteredModelConnectionCheckRequestV2,
) -> Result<ModelConnectionCheckViewV2, DesktopFailure> {
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
    hiroute_product_session::commands::check_registered_model_connection_v2(
        &context,
        &product_state,
        request,
    )
    .await
}
#[tauri::command]
pub async fn check_saved_model_connection_v2(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: SavedModelConnectionCheckRequestV2,
) -> Result<ModelConnectionCheckViewV2, DesktopFailure> {
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
    hiroute_product_session::commands::check_saved_model_connection_v2(
        &context,
        &product_state,
        request,
    )
    .await
}
