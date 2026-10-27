use super::*;
#[tauri::command]
pub async fn manage_subscription_login(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: ComputeSubscriptionLoginRequestV1,
) -> Result<ComputeSubscriptionLoginResultV1, DesktopFailure> {
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
    hiroute_product_session::commands::manage_subscription_login(&context, &product_state, request)
        .await
}
#[tauri::command]
pub async fn submit_subscription_login_callback(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    login_ref: String,
    callback: String,
) -> Result<ComputeSubscriptionLoginResultV1, DesktopFailure> {
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
    hiroute_product_session::commands::submit_subscription_login_callback(
        &context,
        &product_state,
        login_ref,
        callback,
    )
    .await
}
