//! The Linux GUI owns window integration. Product state stays in the standalone console.
mod presentation;
mod runtime;

use hiroute_host_runtime::StorageRecovery;
use runtime::{LiveConsole, NativeSession};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::io::AsyncWriteExt;
use url::Url;
use zeroize::Zeroizing;

const WINDOW: &str = "main";
const STATE_UNAVAILABLE: &str = "SHELL_STATE_UNAVAILABLE";

#[derive(Clone, Default, Serialize)]
struct Status {
    phase: &'static str,
    code: Option<&'static str>,
    logs_path: String,
    platform: &'static str,
    operation_error: Option<&'static str>,
    storage_recovery: Option<StorageRecovery>,
}

#[derive(Default)]
struct Host {
    session: Option<NativeSession>,
    status: Status,
    busy: bool,
}
type HostState<'a> = State<'a, Arc<Mutex<Host>>>;

fn startup_url(url: &Url) -> bool {
    url.scheme() == "tauri"
        && url.host_str() == Some("localhost")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && matches!(url.path(), "" | "/" | "/index.html")
}

fn window_url(window: &WebviewWindow) -> Result<Url, &'static str> {
    if window.label() != WINDOW {
        return Err("SHELL_ORIGIN_DENIED");
    }
    window.url().map_err(|_| "SHELL_ORIGIN_DENIED")
}

fn local(window: &WebviewWindow) -> Result<(), &'static str> {
    if startup_url(&window_url(window)?) {
        Ok(())
    } else {
        Err("SHELL_ORIGIN_DENIED")
    }
}

fn session(
    window: &WebviewWindow,
    host: &Mutex<Host>,
) -> Result<(Url, NativeSession), &'static str> {
    let url = window_url(window)?;
    let host = host.lock().map_err(|_| STATE_UNAVAILABLE)?;
    let session = host
        .session
        .as_ref()
        .filter(|session| session.origin.allows(&url))
        .ok_or("SHELL_ORIGIN_DENIED")?;
    Ok((url, session.clone()))
}

async fn authorize(
    window: &WebviewWindow,
    host: &Mutex<Host>,
    bearer: &str,
) -> Result<NativeSession, &'static str> {
    let (url, session) = session(window, host)?;
    session
        .authorize(&url, Zeroizing::new(bearer.to_owned()))
        .await?;
    if !session.origin.allows(&window_url(window)?) {
        return Err("SHELL_ORIGIN_DENIED");
    }
    Ok(session)
}

enum Action {
    Connect,
    Install { login_start: bool },
    Update { owner: String },
    Recover,
    Restart(NativeSession),
}

fn start(
    window: WebviewWindow,
    host: Arc<Mutex<Host>>,
    action: Action,
) -> Result<(), &'static str> {
    let previous_code = {
        let mut state = host.lock().map_err(|_| STATE_UNAVAILABLE)?;
        if state.busy {
            return Err("SHELL_ACTION_IN_PROGRESS");
        }
        state.busy = true;
        state.session = None;
        state.status.phase = "starting";
        state.status.operation_error = None;
        state.status.storage_recovery = None;
        state.status.code.take()
    };
    if !startup_url(&window_url(&window)?) {
        if window
            .navigate(Url::parse("tauri://localhost/").expect("bundled status URL"))
            .is_err()
        {
            if let Ok(mut state) = host.lock() {
                state.busy = false;
            }
            return Err("SHELL_NAVIGATION_FAILED");
        }
    }
    tauri::async_runtime::spawn(async move {
        let result = async {
            match action {
                Action::Connect => (),
                Action::Install { login_start } => {
                    runtime::install(false).await?;
                    if login_start {
                        runtime::enable_login_start().await?;
                    }
                }
                Action::Recover => {
                    runtime::install(true).await?;
                    runtime::resume_recovered().await?;
                }
                Action::Update { owner } => runtime::update(&owner).await?,
                Action::Restart(session) => session.restart().await?,
            }
            let live = LiveConsole::open().await?;
            host.lock().map_err(|_| STATE_UNAVAILABLE)?.session = Some(live.session()?);
            window
                .navigate(live.url)
                .map_err(|_| "SHELL_NAVIGATION_FAILED")
        }
        .await;
        if let Ok(mut state) = host.lock() {
            state.status.phase = if result.is_ok() { "ready" } else { "failed" };
            state.status.code = match result.err() {
                Some(error @ ("RUNTIME_UPDATE_BLOCKED" | "PACKAGE_PAYLOAD_UNAVAILABLE"))
                    if previous_code == Some("CONSOLE_NATIVE_RELEASE_UNAVAILABLE") =>
                {
                    state.status.operation_error = Some(error);
                    previous_code
                }
                code => code,
            };
            if state.status.code.is_some() {
                state.session = None;
                state.status.storage_recovery = runtime::layout().ok().and_then(|layout| {
                    StorageRecovery::read(&layout.storage_root()).ok().flatten()
                });
            }
            state.busy = false;
        }
    });
    Ok(())
}

#[tauri::command]
fn shell_status(window: WebviewWindow, host: HostState<'_>) -> Result<Status, &'static str> {
    local(&window)?;
    Ok(host.lock().map_err(|_| STATE_UNAVAILABLE)?.status.clone())
}

#[tauri::command]
fn shell_retry_startup(window: WebviewWindow, host: HostState<'_>) -> Result<(), &'static str> {
    local(&window)?;
    start(window, host.inner().clone(), Action::Connect)
}

#[tauri::command]
fn shell_install_runtime(
    window: WebviewWindow,
    host: HostState<'_>,
    login_start: bool,
) -> Result<(), &'static str> {
    local(&window)?;
    if runtime::Installation::discover().err() != Some("INSTALLATION_REQUIRED") {
        return Err("SERVICE_OWNER_UNVERIFIED");
    }
    start(
        window,
        host.inner().clone(),
        Action::Install { login_start },
    )
}

#[tauri::command]
fn shell_recover_runtime(window: WebviewWindow, host: HostState<'_>) -> Result<(), &'static str> {
    local(&window)?;
    if runtime::Installation::discover().err() != Some("SHELL_INSTALL_RECOVERY_REQUIRED") {
        return Err("INSTALLATION_UNAVAILABLE");
    }
    start(window, host.inner().clone(), Action::Recover)
}

fn require_update_state(window: &WebviewWindow, host: &HostState<'_>) -> Result<(), &'static str> {
    local(window)?;
    let state = host.lock().map_err(|_| STATE_UNAVAILABLE)?;
    if state.busy || state.status.code != Some("CONSOLE_NATIVE_RELEASE_UNAVAILABLE") {
        return Err("RUNTIME_UPDATE_BLOCKED");
    }
    Ok(())
}

#[tauri::command]
async fn shell_preview_runtime_update(
    window: WebviewWindow,
    host: HostState<'_>,
) -> Result<runtime::UpdatePreview, &'static str> {
    require_update_state(&window, &host)?;
    runtime::preview_update().await
}

#[tauri::command]
fn shell_update_runtime(
    window: WebviewWindow,
    host: HostState<'_>,
    owner: String,
) -> Result<(), &'static str> {
    require_update_state(&window, &host)?;
    if owner.len() != 64 || !owner.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("RUNTIME_UPDATE_BLOCKED");
    }
    start(window, host.inner().clone(), Action::Update { owner })
}

#[tauri::command]
async fn shell_open_legacy_browser(
    window: WebviewWindow,
    host: HostState<'_>,
) -> Result<(), &'static str> {
    local(&window)?;
    if !matches!(
        host.lock().map_err(|_| STATE_UNAVAILABLE)?.status.code,
        Some("CONSOLE_NATIVE_RELEASE_UNAVAILABLE" | "CONSOLE_NATIVE_ASSETS_UNAVAILABLE")
    ) {
        return Err("CONSOLE_LAUNCH_UNAVAILABLE");
    }
    open(runtime::legacy_browser_url().await?.as_str()).await
}

#[tauri::command]
async fn shell_capabilities(
    window: WebviewWindow,
    host: HostState<'_>,
    bearer: String,
) -> Result<Value, &'static str> {
    let bearer = Zeroizing::new(bearer);
    authorize(&window, &host, &bearer).await?;
    let layout = runtime::layout()?;
    Ok(json!({
        "host": "shell-console", "platform": "linux", "updates": false,
        "cli_entry": false, "quit": false, "titlebar": false, "login_start": false,
        "directory_reveal": "native", "diagnostic_directory": layout.diagnostics_root(),
        "codex_desktop_launch": false, "compositor_frame": true, "open_in_browser": true,
        "restart_service": true, "remove_runtime": true, "reconnect": true,
        "native_clipboard": true, "presentation": true
    }))
}

async fn open(value: impl AsRef<std::ffi::OsStr>) -> Result<(), &'static str> {
    let mut command = runtime::command("/usr/bin/xdg-open");
    command.arg(value).stdout(Stdio::null());
    let mut child = command.spawn().map_err(|_| "EXTERNAL_OPEN_UNAVAILABLE")?;
    match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(_) => Err("EXTERNAL_OPEN_UNAVAILABLE"),
        Err(_) => {
            tauri::async_runtime::spawn(async move {
                let _ = child.wait().await;
            });
            Ok(())
        }
    }
}

#[tauri::command]
async fn shell_open_in_browser(
    window: WebviewWindow,
    host: HostState<'_>,
    bearer: String,
) -> Result<(), &'static str> {
    let bearer = Zeroizing::new(bearer);
    let session = authorize(&window, &host, &bearer).await?;
    open(session.browser_url().await?.as_str()).await
}

#[tauri::command]
async fn shell_open_external_url(
    window: WebviewWindow,
    host: HostState<'_>,
    bearer: String,
    url: String,
) -> Result<(), &'static str> {
    let bearer = Zeroizing::new(bearer);
    authorize(&window, &host, &bearer).await?;
    open(hiroute_shell::external_url(&url)?.as_str()).await
}

#[tauri::command]
async fn shell_open_diagnostic_directory(
    window: WebviewWindow,
    host: HostState<'_>,
    bearer: String,
) -> Result<(), &'static str> {
    let bearer = Zeroizing::new(bearer);
    authorize(&window, &host, &bearer).await?;
    let directory = runtime::layout()?.diagnostics_root();
    if !directory.is_dir() || directory.canonicalize().ok().as_ref() != Some(&directory) {
        return Err("APP_DATA_UNAVAILABLE");
    }
    open(directory).await
}

#[tauri::command]
async fn shell_save_classifier_openapi(
    window: WebviewWindow,
    host: HostState<'_>,
    bearer: String,
) -> Result<&'static str, &'static str> {
    use tauri_plugin_dialog::DialogExt;
    let bearer = Zeroizing::new(bearer);
    authorize(&window, &host, &bearer).await?;
    let dialog = window
        .dialog()
        .file()
        .set_file_name("hiroute-decision.openapi.json")
        .add_filter("OpenAPI JSON", &["json"])
        .set_parent(&window);
    let selected = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file())
        .await
        .map_err(|_| "FILE_SAVE_UNAVAILABLE")?;
    let Some(path) = selected else {
        return Ok("cancelled");
    };
    authorize(&window, &host, &bearer).await?;
    tokio::fs::write(
        path.into_path().map_err(|_| "FILE_SAVE_UNAVAILABLE")?,
        include_bytes!("../../../../../decision-extensions/api/decision.openapi.json"),
    )
    .await
    .map_err(|_| "FILE_SAVE_UNAVAILABLE")?;
    Ok("saved")
}

#[tauri::command]
async fn shell_write_clipboard(
    window: WebviewWindow,
    host: HostState<'_>,
    bearer: String,
    value: String,
) -> Result<bool, &'static str> {
    let bearer = Zeroizing::new(bearer);
    authorize(&window, &host, &bearer).await?;
    if value.len() > 1024 * 1024 {
        return Err("CLIPBOARD_UNAVAILABLE");
    }
    let mut child = runtime::command("/usr/bin/wl-copy")
        .args(["--type", "text/plain;charset=utf-8"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|_| "CLIPBOARD_UNAVAILABLE")?;
    let mut input = child.stdin.take().ok_or("CLIPBOARD_UNAVAILABLE")?;
    tokio::time::timeout(Duration::from_secs(3), async {
        input
            .write_all(value.as_bytes())
            .await
            .map_err(|_| "CLIPBOARD_UNAVAILABLE")?;
        drop(input);
        child
            .wait()
            .await
            .map(|status| status.success())
            .map_err(|_| "CLIPBOARD_UNAVAILABLE")
    })
    .await
    .map_err(|_| "CLIPBOARD_UNAVAILABLE")?
}

#[tauri::command]
async fn shell_restart_service(
    window: WebviewWindow,
    host: HostState<'_>,
    bearer: String,
) -> Result<(), &'static str> {
    let bearer = Zeroizing::new(bearer);
    let session = authorize(&window, &host, &bearer).await?;
    start(window, host.inner().clone(), Action::Restart(session))
}

#[tauri::command]
async fn shell_remove_runtime(
    window: WebviewWindow,
    host: HostState<'_>,
    bearer: String,
) -> Result<(), &'static str> {
    let bearer = Zeroizing::new(bearer);
    let session = authorize(&window, &host, &bearer).await?;
    {
        let mut state = host.lock().map_err(|_| STATE_UNAVAILABLE)?;
        if state.busy {
            return Err("SHELL_ACTION_IN_PROGRESS");
        }
        state.busy = true;
    }
    let result = session.remove().await;
    host.lock().map_err(|_| STATE_UNAVAILABLE)?.busy = false;
    result?;
    start(window, host.inner().clone(), Action::Connect)
}

#[tauri::command]
fn shell_reconnect(window: WebviewWindow, host: HostState<'_>) -> Result<(), &'static str> {
    // Recovery after expiry only returns this window to bundled startup.
    session(&window, &host)?;
    start(window, host.inner().clone(), Action::Connect)
}

async fn presentation_access(
    window: &WebviewWindow,
    host: &Mutex<Host>,
    bearer: Option<String>,
) -> Result<(), &'static str> {
    if local(window).is_ok() {
        return Ok(());
    }
    let bearer = Zeroizing::new(bearer.ok_or("SHELL_SESSION_DENIED")?);
    authorize(window, host, &bearer).await.map(|_| ())
}

#[tauri::command]
async fn shell_presentation_get(
    window: WebviewWindow,
    host: HostState<'_>,
    bearer: Option<String>,
) -> Result<presentation::Presentation, &'static str> {
    presentation_access(&window, &host, bearer).await?;
    presentation::get()
}

#[tauri::command]
async fn shell_presentation_set(
    window: WebviewWindow,
    host: HostState<'_>,
    bearer: Option<String>,
    value: presentation::Presentation,
) -> Result<(), &'static str> {
    presentation_access(&window, &host, bearer).await?;
    presentation::set(value)
}

fn focus(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn print_version() {
    let identity = hiroute_host_runtime::BuildIdentityV1::compiled(
        "linux-desktop",
        env!("CARGO_PKG_VERSION"),
        option_env!("HIROUTE_BUILD_SOURCE_SHA"),
    );
    let mut result = serde_json::to_value(identity).expect("fixed build identity");
    result["console_assets_sha256"] = json!(option_env!("HIROUTE_LINUX_CONSOLE_ASSETS_SHA256"));
    println!("{result}");
}

pub fn run() {
    // GTK initializes GDK before constructing the GtkApplication. Set the
    // Wayland surface name early so it matches the installed desktop entry.
    glib::set_prgname(Some("ai.hiroute.desktop"));
    let logs_path = runtime::layout()
        .map(|layout| layout.diagnostics_root().display().to_string())
        .unwrap_or_default();
    let host = Arc::new(Mutex::new(Host {
        status: Status {
            phase: "starting",
            logs_path,
            platform: "linux",
            ..Default::default()
        },
        ..Default::default()
    }));
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| focus(app)))
        .plugin(tauri_plugin_dialog::init())
        .manage(host.clone())
        .invoke_handler(tauri::generate_handler![
            shell_status, shell_retry_startup, shell_install_runtime, shell_recover_runtime, shell_open_legacy_browser,
            shell_preview_runtime_update, shell_update_runtime,
            shell_capabilities, shell_open_in_browser, shell_open_external_url, shell_save_classifier_openapi,
            shell_restart_service, shell_remove_runtime, shell_reconnect, shell_open_diagnostic_directory, shell_write_clipboard,
            shell_presentation_get, shell_presentation_set,
        ])
        .setup(move |app| {
            let navigation = host.clone();
            let window = WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("index.html".into()))
                .title("HiRoute")
                .inner_size(1060., 760.)
                .min_inner_size(620., 520.)
                .decorations(std::env::var_os("WAYLAND_DISPLAY").is_none())
                .incognito(true)
                .initialization_script(
                    "Object.defineProperties(window,{__HIROUTE_SHELL__:{value:true},__HIROUTE_LINUX_SHELL__:{value:true}});"
                )
                .on_navigation(move |url| startup_url(url) || navigation.lock().ok()
                    .and_then(|state| state.session.clone()).is_some_and(|session| session.origin.allows(url)))
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .build()?;
            start(window, host.clone(), Action::Connect)?;
            Ok(())
        })
        .run(tauri::generate_context!());
    if result.is_err() {
        eprintln!("SHELL_STARTUP_FAILED");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_status_admits_only_the_actual_webkit_app_url() {
        assert!(startup_url(&Url::parse("tauri://localhost").unwrap()));
        for url in [
            "http://127.0.0.1:5838/",
            "tauri://other/index.html",
            "tauri://localhost/?code=example",
            "tauri://localhost/other",
        ] {
            assert!(!startup_url(&Url::parse(url).unwrap()));
        }
    }
}
