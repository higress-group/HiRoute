use super::*;
use hiroute_application_api::{AgentPreviousRoutingPreviewV1, AgentPreviousRoutingQueryV1};

#[tauri::command]
pub(super) async fn preview_agent_previous_routing(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: AgentPreviousRoutingQueryV1,
) -> Result<AgentPreviousRoutingPreviewV1, DesktopFailure> {
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
    hiroute_product_session::commands::preview_agent_previous_routing(
        &context,
        &product_state,
        input,
    )
    .await
}
