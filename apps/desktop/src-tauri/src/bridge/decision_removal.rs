use super::*;

#[tauri::command]
pub(super) async fn preview_decision_removal(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: hiroute_application_api::DecisionRemovalPreviewRequestV1,
) -> Result<hiroute_application_api::DecisionRemovalPreviewV1, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: None,
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::preview_decision_removal(&context, &product_state, input)
        .await
}
