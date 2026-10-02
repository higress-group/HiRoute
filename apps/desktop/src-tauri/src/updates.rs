//! Native updater lifecycle. WebView chooses actions; source, paths and trust stay native.
use hiroute_host_runtime::{
    DesktopPackageIdentity, DesktopRelease, UpgradeAction, verify_desktop_app,
};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::{Manager, State, WebviewWindow};
mod download;
mod installer;
pub use installer::finish_update_if_requested;

#[derive(Clone)]
struct PreparedUpdate {
    release: DesktopRelease,
    current: PathBuf,
    staged: PathBuf,
    stage_root: PathBuf,
    download_root: PathBuf,
    old_identity: DesktopPackageIdentity,
    new_identity: DesktopPackageIdentity,
}
impl PreparedUpdate {
    fn revalidate(&self) -> Result<(), String> {
        if verify_desktop_app(&self.current, "ai.hiroute.desktop", None, None)? != self.old_identity
            || verify_desktop_app(
                &self.staged,
                "ai.hiroute.desktop",
                Some(&self.release),
                None,
            )? != self.new_identity
        {
            return Err("UPGRADE_PACKAGE_CHANGED".into());
        }
        Ok(())
    }
}
#[derive(Clone, Serialize)]
pub struct UpdateView {
    current_version: String,
    phase: String,
    available: Option<DesktopRelease>,
    downloaded_bytes: u64,
    active_calls: u64,
    active_tasks: u64,
    error: Option<String>,
    can_install: bool,
    package_ready: bool,
}
struct UpdateInner {
    view: UpdateView,
    prepared: Option<PreparedUpdate>,
}
pub struct Updates {
    inner: Mutex<UpdateInner>,
    cancelled: AtomicBool,
    current_app: Option<PathBuf>,
    data_root: Option<PathBuf>,
}
pub struct UpdateState(pub Arc<Updates>);
impl UpdateState {
    pub fn new(current_version: String, data_root: Option<PathBuf>) -> Self {
        let current_app = std::env::current_exe().ok().and_then(|p| {
            (p.file_name()? == "hiroute-desktop"
                && p.parent()?.file_name()? == "MacOS"
                && p.parent()?.parent()?.file_name()? == "Contents")
                .then(|| {
                    p.parent()
                        .unwrap()
                        .parent()
                        .unwrap()
                        .parent()
                        .unwrap()
                        .to_path_buf()
                })
                .and_then(|app| std::fs::canonicalize(app).ok())
        });
        let current_version = current_app
            .as_ref()
            .and_then(|app| {
                let output = std::process::Command::new("/usr/bin/plutil")
                    .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
                    .arg(app.join("Contents/Info.plist"))
                    .output()
                    .ok()?;
                output
                    .status
                    .success()
                    .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            })
            .unwrap_or(current_version);
        Self(Arc::new(Updates {
            inner: Mutex::new(UpdateInner {
                view: UpdateView {
                    current_version,
                    phase: "idle".into(),
                    available: None,
                    downloaded_bytes: 0,
                    active_calls: 0,
                    active_tasks: 0,
                    error: None,
                    can_install: current_app.is_some(),
                    package_ready: false,
                },
                prepared: None,
            }),
            cancelled: AtomicBool::new(false),
            current_app,
            data_root,
        }))
    }
}
impl Updates {
    fn edit(&self, update: impl FnOnce(&mut UpdateView)) -> Result<(), String> {
        update(
            &mut self
                .inner
                .lock()
                .map_err(|_| "UPGRADE_STATE_UNAVAILABLE")?
                .view,
        );
        Ok(())
    }
    fn view(&self) -> Result<UpdateView, String> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| "UPGRADE_STATE_UNAVAILABLE")?
            .view
            .clone())
    }
    fn begin(&self, phase: &str) -> Result<UpdateView, String> {
        let mut inner = self.inner.lock().map_err(|_| "UPGRADE_STATE_UNAVAILABLE")?;
        if matches!(
            inner.view.phase.as_str(),
            "checking" | "downloading" | "verifying" | "waiting" | "installing"
        ) {
            return Err("UPGRADE_BUSY".into());
        }
        self.cancelled.store(false, Ordering::SeqCst);
        inner.view.phase = phase.into();
        inner.view.error = None;
        Ok(inner.view.clone())
    }
    fn fail(&self, error: String) {
        let _ = self.edit(|v| {
            v.phase = "failed".into();
            v.error = Some(error);
        });
    }
}
fn main_window(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("WINDOW_DENIED".into())
    }
}
#[tauri::command]
pub fn update_status(
    window: WebviewWindow,
    state: State<'_, UpdateState>,
) -> Result<UpdateView, String> {
    main_window(&window)?;
    state.0.view()
}
#[tauri::command]
pub async fn update_check(
    window: WebviewWindow,
    state: State<'_, UpdateState>,
) -> Result<UpdateView, String> {
    main_window(&window)?;
    let view = state.0.begin("checking")?;
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    };
    match download::check(&view.current_version, arch).await {
        Ok(release) => {
            let mut inner = state
                .0
                .inner
                .lock()
                .map_err(|_| "UPGRADE_STATE_UNAVAILABLE")?;
            let reusable = inner.prepared.as_ref().is_some_and(|p| {
                release
                    .as_ref()
                    .is_some_and(|r| p.release.version == r.version && p.release.sha256 == r.sha256)
            });
            if !reusable {
                inner.prepared = None;
            }
            inner.view.available = release;
            inner.view.package_ready = reusable;
            inner.view.phase = if reusable {
                "ready"
            } else if inner.view.available.is_some() {
                "available"
            } else {
                "current"
            }
            .into();
        }
        Err(error) => state.0.fail(error),
    }
    state.0.view()
}
#[tauri::command]
pub async fn update_download(
    window: WebviewWindow,
    state: State<'_, UpdateState>,
) -> Result<UpdateView, String> {
    main_window(&window)?;
    let release = {
        let mut inner = state
            .0
            .inner
            .lock()
            .map_err(|_| "UPGRADE_STATE_UNAVAILABLE")?;
        if !matches!(inner.view.phase.as_str(), "available" | "failed") || inner.prepared.is_some()
        {
            return Err("UPGRADE_DOWNLOAD_NOT_READY".into());
        }
        let release = inner
            .view
            .available
            .clone()
            .ok_or("UPGRADE_NOT_AVAILABLE")?;
        inner.view.phase = "downloading".into();
        inner.view.error = None;
        inner.view.downloaded_bytes = 0;
        state.0.cancelled.store(false, Ordering::SeqCst);
        release
    };
    let result = download::prepare(&state.0, release).await;
    match result {
        Ok(prepared) => {
            let mut inner = state
                .0
                .inner
                .lock()
                .map_err(|_| "UPGRADE_STATE_UNAVAILABLE")?;
            inner.prepared = Some(prepared);
            inner.view.phase = "ready".into();
            inner.view.package_ready = true;
        }
        Err(error) if error == "UPGRADE_CANCELLED" => {
            state.0.edit(|v| v.phase = "available".into())?
        }
        Err(error) => state.0.fail(error),
    }
    state.0.view()
}
#[tauri::command]
pub fn update_cancel(
    window: WebviewWindow,
    state: State<'_, UpdateState>,
) -> Result<UpdateView, String> {
    main_window(&window)?;
    let inner = state
        .0
        .inner
        .lock()
        .map_err(|_| "UPGRADE_STATE_UNAVAILABLE")?;
    if !matches!(
        inner.view.phase.as_str(),
        "downloading" | "verifying" | "waiting"
    ) {
        return Err("UPGRADE_CANCEL_UNAVAILABLE".into());
    }
    state.0.cancelled.store(true, Ordering::SeqCst);
    Ok(inner.view.clone())
}
#[tauri::command]
pub async fn update_install(
    window: WebviewWindow,
    state: State<'_, UpdateState>,
    desktop: State<'_, crate::bridge::DesktopState>,
) -> Result<UpdateView, String> {
    main_window(&window)?;
    let prepared = {
        let mut inner = state
            .0
            .inner
            .lock()
            .map_err(|_| "UPGRADE_STATE_UNAVAILABLE")?;
        if !matches!(inner.view.phase.as_str(), "ready" | "failed") {
            return Err("UPGRADE_INSTALL_NOT_READY".into());
        }
        let prepared = inner.prepared.clone().ok_or("UPGRADE_INSTALL_NOT_READY")?;
        inner.view.phase = "waiting".into();
        inner.view.error = None;
        state.0.cancelled.store(false, Ordering::SeqCst);
        prepared
    };
    let updates = state.0.clone();
    let sessions = desktop.0.clone();
    let handle = window.app_handle().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = install(&updates, &sessions, &handle, prepared).await {
            // Resume only while the owned daemon is still serving. An already requested
            // orderly shutdown must finish; it cannot be converted into a new live session.
            let _ = resident_call(&sessions, |r| {
                if !r.upgrade_stop_requested() {
                    r.upgrade_status(UpgradeAction::Cancel)?;
                }
                Ok(())
            })
            .await;
            updates.fail(error);
        }
    });
    state.0.view()
}
type Sessions = Arc<tokio::sync::Mutex<Option<crate::session::Session>>>;
async fn resident_call<T: Send + 'static>(
    sessions: &Sessions,
    call: impl FnOnce(&mut crate::bootstrap::Resident) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let mut session = sessions.clone().lock_owned().await;
    tokio::task::spawn_blocking(move || {
        call(&mut session.as_mut().ok_or("RESIDENT_UNAVAILABLE")?.resident)
    })
    .await
    .map_err(|_| "UPGRADE_RESIDENT_UNAVAILABLE")?
}
async fn install(
    updates: &Updates,
    sessions: &Sessions,
    app: &tauri::AppHandle,
    prepared: PreparedUpdate,
) -> Result<(), String> {
    resident_call(sessions, |r| {
        if r.upgrade_stop_requested() {
            Err("UPGRADE_RESIDENT_STOPPING_RESTART_REQUIRED".into())
        } else {
            Ok(())
        }
    })
    .await?;
    let root = updates
        .data_root
        .clone()
        .ok_or("PRIVATE_PATH_UNAVAILABLE")?;
    let copy = prepared.clone();
    let handoff = tokio::task::spawn_blocking(move || {
        copy.revalidate()?;
        installer::prepare(&root, &copy)
    })
    .await
    .map_err(|_| "UPGRADE_VERIFY_FAILED")??;
    resident_call(sessions, |r| r.upgrade_status(UpgradeAction::Prepare)).await?;
    loop {
        if updates.cancelled.load(Ordering::SeqCst) {
            resident_call(sessions, |r| r.upgrade_status(UpgradeAction::Cancel)).await?;
            updates.edit(|v| {
                v.phase = "ready".into();
                v.active_calls = 0;
                v.active_tasks = 0;
            })?;
            return Ok(());
        }
        let status = resident_call(sessions, |r| r.upgrade_status(UpgradeAction::Status)).await?;
        updates.edit(|v| {
            v.active_calls = status.active_calls;
            v.active_tasks = status.active_tasks;
        })?;
        if status.drained() {
            let mut inner = updates
                .inner
                .lock()
                .map_err(|_| "UPGRADE_STATE_UNAVAILABLE")?;
            if !updates.cancelled.load(Ordering::SeqCst) {
                inner.view.phase = "installing".into();
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    resident_call(sessions, |r| r.stop_for_upgrade()).await?;
    tokio::task::spawn_blocking(move || installer::launch(&handoff))
        .await
        .map_err(|_| "UPGRADE_INSTALLER_FAILED")??;
    app.exit(0);
    Ok(())
}
