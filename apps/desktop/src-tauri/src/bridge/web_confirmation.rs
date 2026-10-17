use super::{DesktopFailure, main_window};
use std::sync::atomic::Ordering;
use tauri::{Manager, State, WebviewWindow};

pub(crate) fn expire_window_confirmations(app: &tauri::AppHandle, window_label: &str) {
    app.state::<super::DesktopState>()
        .2
        .fetch_add(1, Ordering::SeqCst);
    app.state::<WebConfirmationState>()
        .cancel_context(window_label);
    app.state::<super::WorkerDependencyConfirmationState>()
        .cancel_window(window_label);
}

pub use hiroute_product_session::web_confirmation::{
    ConfirmationBroker as WebConfirmationState, WebConfirmationDecision, WebConfirmationRequest,
};

#[tauri::command]
pub(super) async fn web_confirmation_snapshot(
    window: WebviewWindow,
    state: State<'_, WebConfirmationState>,
) -> Result<Option<WebConfirmationRequest>, DesktopFailure> {
    main_window(&window)?;
    Ok(state.snapshot(window.label()))
}

#[tauri::command]
pub(super) async fn resolve_web_confirmation(
    window: WebviewWindow,
    state: State<'_, WebConfirmationState>,
    input: WebConfirmationDecision,
) -> Result<(), DesktopFailure> {
    main_window(&window)?;
    state.resolve(window.label(), &input.confirmation_id, input.accepted)
}
