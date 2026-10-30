use super::*;
use super::{DesktopFailure, DesktopState, main_window};
use hiroute_application_api::{ApplyResultV1, ClassifierDecisionTestResultV1};
use tauri::{State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

const CLASSIFIER_OPENAPI_FILENAME: &str = "hiroute-decision.openapi.json";
const CLASSIFIER_OPENAPI: &str =
    include_str!("../../../../../decision-extensions/api/decision.openapi.json");

#[tauri::command]
pub(super) async fn decision_connection_sources(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<DecisionConnectionSourcesV1, DesktopFailure> {
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
    hiroute_product_session::commands::decision_connection_sources(&context, &product_state).await
}

#[tauri::command]
pub(super) async fn decision_services_v2(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<DecisionServicesV2, DesktopFailure> {
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
    hiroute_product_session::commands::decision_services_v2(&context, &product_state).await
}

#[tauri::command]
pub(super) async fn save_decision_service_v2(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: DecisionServiceSaveInputV2,
) -> Result<ApplyResultV1, DesktopFailure> {
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
    hiroute_product_session::commands::save_decision_service_v2(&context, &product_state, input)
        .await
}

#[tauri::command]
pub(super) async fn test_decision_service_v2(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: DecisionServiceTestRequestV2,
) -> Result<DecisionServiceTestResultV2, DesktopFailure> {
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
    hiroute_product_session::commands::test_decision_service_v2(&context, &product_state, input)
        .await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ClassifierOpenApiSaveOutcome {
    Saved,
    Cancelled,
}

fn write_classifier_openapi(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::write(path, CLASSIFIER_OPENAPI.as_bytes())
}

#[tauri::command]
pub(super) async fn save_classifier_openapi(
    window: WebviewWindow,
) -> Result<ClassifierOpenApiSaveOutcome, DesktopFailure> {
    main_window(&window)?;
    let selected = window
        .dialog()
        .file()
        .set_parent(&window)
        .set_file_name(CLASSIFIER_OPENAPI_FILENAME)
        .add_filter("OpenAPI JSON", &["json"])
        .blocking_save_file();
    let Some(selected) = selected else {
        return Ok(ClassifierOpenApiSaveOutcome::Cancelled);
    };
    let path = selected
        .into_path()
        .map_err(|_| DesktopFailure::from("CLASSIFIER_OPENAPI_SAVE_UNAVAILABLE"))?;
    write_classifier_openapi(&path)
        .map_err(|_| DesktopFailure::from("CLASSIFIER_OPENAPI_SAVE_UNAVAILABLE"))?;
    Ok(ClassifierOpenApiSaveOutcome::Saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_writer_emits_the_canonical_contract() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(CLASSIFIER_OPENAPI_FILENAME);
        write_classifier_openapi(&path).unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), CLASSIFIER_OPENAPI);
        let document: serde_json::Value = serde_json::from_str(CLASSIFIER_OPENAPI).unwrap();
        assert_eq!(document["openapi"], "3.1.0");
    }
}
#[tauri::command]
pub(super) async fn decision_services(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<crate::session::DecisionServiceList, DesktopFailure> {
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
    hiroute_product_session::commands::decision_services(&context, &product_state).await
}
#[tauri::command]
pub(super) async fn save_decision_service(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: DecisionServiceSaveInput,
) -> Result<ApplyResultV1, DesktopFailure> {
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
    hiroute_product_session::commands::save_decision_service(&context, &product_state, input).await
}
#[tauri::command]
pub(super) async fn test_classifier_decision(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: ClassifierDecisionTestInput,
) -> Result<ClassifierDecisionTestResultV1, DesktopFailure> {
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
    hiroute_product_session::commands::test_classifier_decision(&context, &product_state, input)
        .await
}
#[tauri::command]
pub(super) async fn save_classifier_header_secret(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: ClassifierHeaderSecretSaveInput,
) -> Result<ApplyResultV1, DesktopFailure> {
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
    hiroute_product_session::commands::save_classifier_header_secret(
        &context,
        &product_state,
        input,
    )
    .await
}
