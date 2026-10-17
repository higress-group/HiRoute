use crate::ConsoleOrigin;
use serde::Serialize;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tauri::{Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};
mod commands;
mod install_transaction;
pub mod installer;
mod service;
mod tray;
mod updater;
use commands::*;

#[derive(Clone, Serialize)]
pub struct Status {
    phase: &'static str,
    code: Option<&'static str>,
    logs_path: String,
}
pub struct ShellState {
    origin: Mutex<Option<ConsoleOrigin>>,
    status: Mutex<Status>,
    starting: AtomicBool,
    updater: updater::Updater,
}
impl ShellState {
    fn new() -> Self {
        let logs = hiroute_host_runtime::StandaloneLayout::from_environment()
            .ok()
            .map(|l| l.diagnostics_root().display().to_string())
            .unwrap_or_default();
        Self {
            origin: Mutex::new(None),
            status: Mutex::new(Status {
                phase: "starting",
                code: None,
                logs_path: logs,
            }),
            starting: AtomicBool::new(false),
            updater: updater::Updater::new(),
        }
    }
    fn phase(&self, phase: &'static str, code: Option<&'static str>) {
        if let Ok(mut status) = self.status.lock() {
            status.phase = phase;
            status.code = code;
        }
    }
}
fn focus(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
fn start(app: tauri::AppHandle, state: Arc<ShellState>) {
    if state.starting.swap(true, Ordering::AcqRel) {
        return;
    }
    state.phase("starting", None);
    tauri::async_runtime::spawn(async move {
        let outcome = async {
            let (origin, url) = service::ready_launch(&state).await?;
            *state.origin.lock().map_err(|_| "SHELL_STATE_UNAVAILABLE")? = Some(origin);
            let window = app
                .get_webview_window("main")
                .ok_or("SHELL_WINDOW_UNAVAILABLE")?;
            window
                .navigate(url)
                .map_err(|_| "SHELL_NAVIGATION_FAILED")?;
            state.phase("ready", None);
            Ok::<_, &'static str>(())
        }
        .await;
        if let Err(code) = outcome {
            state.phase("failed", Some(code));
            focus(&app);
        }
        state.starting.store(false, Ordering::Release);
    });
}
fn local_status(url: &url::Url) -> bool {
    ((url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (url.scheme() == "http" && url.host_str() == Some("tauri.localhost")))
        && matches!(url.path(), "/" | "/index.html")
}
fn authorize(window: &WebviewWindow, state: &ShellState) -> Result<(), &'static str> {
    let url = window.url().map_err(|_| "SHELL_ORIGIN_DENIED")?;
    if window.label() != "main"
        || !state
            .origin
            .lock()
            .map_err(|_| "SHELL_STATE_UNAVAILABLE")?
            .as_ref()
            .is_some_and(|o| o.allows(&url))
    {
        return Err("SHELL_ORIGIN_DENIED");
    }
    Ok(())
}
fn authorize_status(window: &WebviewWindow) -> Result<(), &'static str> {
    if window.label() != "main" || !local_status(&window.url().map_err(|_| "SHELL_ORIGIN_DENIED")?)
    {
        return Err("SHELL_ORIGIN_DENIED");
    }
    Ok(())
}
pub fn run() {
    if hiroute_platform::windows::install::require_x64().is_err() {
        return;
    }
    let state = Arc::new(ShellState::new());
    let tray_enabled = installer::tray_enabled();
    let background = tray_enabled && std::env::args_os().any(|arg| arg == "--background");
    let outcome = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| focus(app)))
        .plugin(tauri_plugin_dialog::init())
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            shell_status,
            shell_retry_startup,
            shell_capabilities,
            shell_open_in_browser,
            shell_open_external_url,
            shell_save_classifier_openapi,
            shell_login_start_get,
            shell_login_start_set,
            shell_restart_service,
            shell_reconnect,
            shell_update_status,
            shell_update_check,
            shell_update_download,
            shell_update_install,
            shell_update_cancel
        ])
        .on_window_event(move |window, event| {
            if window.label() == "main"
                && let WindowEvent::CloseRequested { api, .. } = event
            {
                if tray_enabled {
                    api.prevent_close();
                    let _ = window.hide();
                } else {
                    window.app_handle().exit(0);
                }
            }
        })
        .setup(move |app| {
            let navigation = state.clone();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("HiRoute")
                .inner_size(1200., 813.)
                .min_inner_size(755., 600.)
                .visible(!background)
                .incognito(true)
                .initialization_script(
                    "Object.defineProperty(window,'__HIROUTE_SHELL__',{value:true});",
                )
                .on_navigation(move |url| {
                    if local_status(url)
                        || navigation
                            .origin
                            .lock()
                            .ok()
                            .and_then(|o| o.clone())
                            .is_some_and(|o| o.allows(url))
                    {
                        return true;
                    }
                    if let Ok(url) = crate::external_url(url.as_str()) {
                        let _ = hiroute_platform::windows::shell_open::url(url.as_str());
                    }
                    false
                })
                .on_new_window(|url, _| {
                    if let Ok(url) = crate::external_url(url.as_str()) {
                        let _ = hiroute_platform::windows::shell_open::url(url.as_str());
                    }
                    tauri::webview::NewWindowResponse::Deny
                })
                .build()?;
            if tray_enabled {
                tray::create(app)?;
            }
            match updater::recover_startup(app.handle(), &state, false) {
                Ok(true) => {}
                Ok(false) => start(app.handle().clone(), state.clone()),
                Err(code) => {
                    state.phase("failed", Some(code));
                    focus(app.handle());
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!());
    if outcome.is_err() {
        hiroute_platform::windows::shell_open::startup_failure();
    }
}
pub fn run_helper() -> Option<i32> {
    updater::run_if_requested().or_else(installer::run_if_requested)
}
