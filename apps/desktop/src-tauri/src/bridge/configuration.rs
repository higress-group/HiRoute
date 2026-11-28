use super::*;
use tauri_plugin_dialog::DialogExt;
#[tauri::command]
pub async fn configuration_export(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<ConfigurationBundleV1, DesktopFailure> {
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
    hiroute_product_session::commands::configuration_export(&context, &product_state).await
}
#[tauri::command]
pub async fn configuration_import(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: ConfigurationBundleV1,
) -> Result<ConfigurationImportResultV1, DesktopFailure> {
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
    hiroute_product_session::commands::configuration_import(&context, &product_state, input).await
}

const CONFIGURATION_BUNDLE_FILENAME: &str = "hiroute-configuration.json";

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ConfigurationBundleSaveOutcome {
    Saved,
    Cancelled,
}

fn write_configuration_bundle(
    path: &std::path::Path,
    bundle: &ConfigurationBundleV1,
) -> std::io::Result<()> {
    let body = serde_json::to_vec(bundle).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::Other, "serialize configuration bundle")
    })?;
    std::fs::write(path, body)
}

#[tauri::command]
pub(super) async fn save_configuration_bundle(
    window: WebviewWindow,
    bundle: ConfigurationBundleV1,
) -> Result<ConfigurationBundleSaveOutcome, DesktopFailure> {
    main_window(&window)?;
    let selected = window
        .dialog()
        .file()
        .set_parent(&window)
        .set_file_name(CONFIGURATION_BUNDLE_FILENAME)
        .add_filter("Configuration JSON", &["json"])
        .blocking_save_file();
    let Some(selected) = selected else {
        return Ok(ConfigurationBundleSaveOutcome::Cancelled);
    };
    let path = selected
        .into_path()
        .map_err(|_| DesktopFailure::from("CONFIGURATION_BUNDLE_SAVE_UNAVAILABLE"))?;
    write_configuration_bundle(&path, &bundle)
        .map_err(|_| DesktopFailure::from("CONFIGURATION_BUNDLE_SAVE_UNAVAILABLE"))?;
    Ok(ConfigurationBundleSaveOutcome::Saved)
}
