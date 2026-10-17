use super::*;
use serde_json::{Value, json};
use tauri::State;
type StateRef<'a> = State<'a, Arc<ShellState>>;
#[tauri::command]
pub(super) fn shell_status(
    window: WebviewWindow,
    state: StateRef<'_>,
) -> Result<Status, &'static str> {
    authorize_status(&window)?;
    state
        .status
        .lock()
        .map(|s| s.clone())
        .map_err(|_| "SHELL_STATE_UNAVAILABLE")
}
#[tauri::command]
pub(super) fn shell_retry_startup(
    window: WebviewWindow,
    state: StateRef<'_>,
) -> Result<(), &'static str> {
    authorize_status(&window)?;
    recover_and_start(&window, state.inner())
}
fn recover_and_start(window: &WebviewWindow, state: &Arc<ShellState>) -> Result<(), &'static str> {
    match updater::recover_startup(window.app_handle(), state, true) {
        Ok(true) => {}
        Ok(false) => start(window.app_handle().clone(), state.clone()),
        Err(code) => {
            state.phase("failed", Some(code));
            return Err(code);
        }
    }
    Ok(())
}
#[tauri::command]
pub(super) fn shell_capabilities(
    window: WebviewWindow,
    state: StateRef<'_>,
) -> Result<Value, &'static str> {
    if authorize_status(&window).is_err() {
        authorize(&window, &state)?;
    }
    Ok(
        json!({"host":"shell-console","platform":"windows","updates":true,"cli_entry":false,"quit":true,"titlebar":false,"login_start":true,"directory_reveal":"path"}),
    )
}
#[tauri::command]
pub(super) async fn shell_open_in_browser(
    window: WebviewWindow,
    state: StateRef<'_>,
) -> Result<(), &'static str> {
    authorize(&window, &state)?;
    let (_, url) = service::launch().await?;
    hiroute_platform::windows::shell_open::url(url.as_str()).map_err(|_| "BROWSER_UNAVAILABLE")
}
#[tauri::command]
pub(super) fn shell_open_external_url(
    window: WebviewWindow,
    state: StateRef<'_>,
    url: String,
) -> Result<(), &'static str> {
    authorize(&window, &state)?;
    let url = crate::external_url(&url)?;
    hiroute_platform::windows::shell_open::url(url.as_str()).map_err(|_| "BROWSER_UNAVAILABLE")
}
#[tauri::command]
pub(super) async fn shell_save_classifier_openapi(
    window: WebviewWindow,
    state: StateRef<'_>,
) -> Result<&'static str, &'static str> {
    use tauri_plugin_dialog::DialogExt;
    authorize(&window, &state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = window
            .dialog()
            .file()
            .set_file_name("hiroute-decision.openapi.json")
            .add_filter("OpenAPI JSON", &["json"])
            .blocking_save_file()
        else {
            return Ok("cancelled");
        };
        let path = path.into_path().map_err(|_| "FILE_SAVE_UNAVAILABLE")?;
        std::fs::write(
            path,
            include_bytes!("../../../../../decision-extensions/api/decision.openapi.json"),
        )
        .map_err(|_| "FILE_SAVE_UNAVAILABLE")?;
        Ok("saved")
    })
    .await
    .map_err(|_| "FILE_SAVE_UNAVAILABLE")?
}
pub(super) fn login_get() -> Result<Value, &'static str> {
    let native = std::env::current_exe().map_err(|_| "SHELL_LOGIN_UNAVAILABLE")?;
    let run = hiroute_platform::windows::shell_login::enabled(&native)
        .map_err(|_| "SHELL_LOGIN_UNAVAILABLE")?;
    let manager = service::manager(&["service", "autostart", "status", "--output", "json"])?;
    let task = manager["enabled"]
        .as_bool()
        .or_else(|| manager["autostart_enabled"].as_bool())
        .ok_or("SHELL_LOGIN_UNAVAILABLE")?;
    Ok(json!({"enabled":run && task,"shell_enabled":run,"service_enabled":task}))
}
pub(super) fn login_set(enabled: bool) -> Result<Value, &'static str> {
    let before = login_get()?;
    let native = std::env::current_exe().map_err(|_| "SHELL_LOGIN_UNAVAILABLE")?;
    service::manager(&[
        "service",
        "autostart",
        if enabled { "enable" } else { "disable" },
        "--output",
        "json",
    ])?;
    if hiroute_platform::windows::shell_login::set(&native, enabled).is_err() {
        let old = before["service_enabled"] == true;
        let _ = service::manager(&[
            "service",
            "autostart",
            if old { "enable" } else { "disable" },
            "--output",
            "json",
        ]);
        return Err("SHELL_LOGIN_UNAVAILABLE");
    }
    login_get()
}
#[tauri::command]
pub(super) async fn shell_login_start_get(
    window: WebviewWindow,
    state: StateRef<'_>,
) -> Result<Value, &'static str> {
    authorize(&window, &state)?;
    tauri::async_runtime::spawn_blocking(login_get)
        .await
        .map_err(|_| "SHELL_LOGIN_UNAVAILABLE")?
}
#[tauri::command]
pub(super) async fn shell_login_start_set(
    window: WebviewWindow,
    state: StateRef<'_>,
    enabled: bool,
) -> Result<Value, &'static str> {
    authorize(&window, &state)?;
    tauri::async_runtime::spawn_blocking(move || login_set(enabled))
        .await
        .map_err(|_| "SHELL_LOGIN_UNAVAILABLE")?
}
#[tauri::command]
pub(super) async fn shell_restart_service(
    window: WebviewWindow,
    state: StateRef<'_>,
) -> Result<(), &'static str> {
    authorize(&window, &state)?;
    tauri::async_runtime::spawn_blocking(|| {
        service::manager(&["service", "restart", "--output", "json"])
    })
    .await
    .map_err(|_| "SERVICE_RESTART_FAILED")??;
    Ok(())
}
#[tauri::command]
pub(super) fn shell_reconnect(
    window: WebviewWindow,
    state: StateRef<'_>,
) -> Result<(), &'static str> {
    authorize(&window, &state)?;
    window
        .navigate(
            url::Url::parse("http://tauri.localhost/").map_err(|_| "SHELL_NAVIGATION_FAILED")?,
        )
        .map_err(|_| "SHELL_NAVIGATION_FAILED")?;
    *state.origin.lock().map_err(|_| "SHELL_STATE_UNAVAILABLE")? = None;
    recover_and_start(&window, state.inner())
}
#[tauri::command]
pub(super) fn shell_update_status(
    window: WebviewWindow,
    state: StateRef<'_>,
) -> Result<updater::UpdateView, &'static str> {
    authorize(&window, &state)?;
    state.updater.status()
}
#[tauri::command]
pub(super) fn shell_update_check(
    window: WebviewWindow,
    state: StateRef<'_>,
) -> Result<updater::UpdateView, &'static str> {
    authorize(&window, &state)?;
    updater::Updater::check(state.inner().clone())
}
#[tauri::command]
pub(super) fn shell_update_download(
    window: WebviewWindow,
    state: StateRef<'_>,
) -> Result<updater::UpdateView, &'static str> {
    authorize(&window, &state)?;
    updater::Updater::download(state.inner().clone())
}
#[tauri::command]
pub(super) fn shell_update_install(
    window: WebviewWindow,
    state: StateRef<'_>,
) -> Result<updater::UpdateView, &'static str> {
    authorize(&window, &state)?;
    updater::Updater::install(window.app_handle().clone(), state.inner().clone())
}
#[tauri::command]
pub(super) fn shell_update_cancel(
    window: WebviewWindow,
    state: StateRef<'_>,
) -> Result<updater::UpdateView, &'static str> {
    authorize(&window, &state)?;
    state.updater.cancel()
}
