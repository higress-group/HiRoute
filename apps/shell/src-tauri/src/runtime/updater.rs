//! Windows update admission stays in the host; the console cannot choose a URL or executable.
use super::{ShellState, service};
use hiroute_host_runtime::{
    MAX_RELEASE_FEED_BYTES, StandaloneLayout, WINDOWS_RELEASE_FEED, WebsiteReleasesV3,
    WindowsDesktopRelease,
};
use hiroute_platform::windows::{background, lifecycle, security};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

mod diagnostic;
mod finisher;
mod staging;
use diagnostic::{UpdateDiagnostic, UpdateFailure};
pub(super) use finisher::{recover_startup, run_if_requested};
#[derive(Clone, Serialize)]
pub(super) struct UpdateView {
    current_version: &'static str,
    phase: &'static str,
    available: Option<WindowsDesktopRelease>,
    downloaded_bytes: u64,
    active_calls: u64,
    active_tasks: u64,
    error: Option<String>,
    can_install: bool,
    package_ready: bool,
    recovery: Option<&'static str>,
    diagnostic: Option<UpdateDiagnostic>,
}
pub(super) struct Updater {
    view: Mutex<UpdateView>,
    cancel: AtomicBool,
    parent_lock: Mutex<Option<File>>,
    ready_stage: Mutex<Option<staging::Stage>>,
}
impl Updater {
    pub(super) fn new() -> Self {
        Self {
            view: Mutex::new(UpdateView {
                current_version: env!("CARGO_PKG_VERSION"),
                phase: "idle",
                available: None,
                downloaded_bytes: 0,
                active_calls: 0,
                active_tasks: 0,
                error: None,
                can_install: super::installer::program_root()
                    .is_ok_and(|root| root.join("shell-package.json").is_file()),
                package_ready: false,
                recovery: None,
                diagnostic: None,
            }),
            cancel: AtomicBool::new(false),
            parent_lock: Mutex::new(None),
            ready_stage: Mutex::new(None),
        }
    }
    pub(super) fn status(&self) -> Result<UpdateView, &'static str> {
        self.view
            .lock()
            .map(|v| v.clone())
            .map_err(|_| "UPGRADE_STATE_UNAVAILABLE")
    }
    fn change(&self, f: impl FnOnce(&mut UpdateView)) {
        if let Ok(mut v) = self.view.lock() {
            f(&mut v);
        }
    }
    fn begin(&self, phase: &'static str) -> Result<(), &'static str> {
        let mut view = self.view.lock().map_err(|_| "UPGRADE_STATE_UNAVAILABLE")?;
        if matches!(
            view.phase,
            "checking" | "downloading" | "verifying" | "waiting" | "installing"
        ) {
            return Err("UPGRADE_BUSY");
        }
        self.cancel.store(false, Ordering::Release);
        view.phase = phase;
        view.error = None;
        view.diagnostic = None;
        view.recovery = None;
        Ok(())
    }
    fn cancelled(&self) -> Result<(), &'static str> {
        if self.cancel.load(Ordering::Acquire) {
            Err("UPGRADE_CANCELLED")
        } else {
            Ok(())
        }
    }
    pub(super) fn check(state: Arc<ShellState>) -> Result<UpdateView, &'static str> {
        state.updater.begin("checking")?;
        let initial = state.updater.status()?;
        tauri::async_runtime::spawn(async move {
            let result = async {
                let client = http()?;
                let mut response = client
                    .get(WINDOWS_RELEASE_FEED)
                    .send()
                    .await
                    .map_err(|_| "UPGRADE_CHECK_FAILED")?;
                catalog_status(response.status())?;
                if response
                    .content_length()
                    .is_some_and(|n| n > MAX_RELEASE_FEED_BYTES as u64)
                {
                    return Err("UPGRADE_CATALOG_INVALID".into());
                }
                let mut bytes = vec![];
                while let Some(chunk) =
                    response.chunk().await.map_err(|_| "UPGRADE_CHECK_FAILED")?
                {
                    if bytes.len() + chunk.len() > MAX_RELEASE_FEED_BYTES {
                        return Err("UPGRADE_CATALOG_INVALID".into());
                    }
                    bytes.extend_from_slice(&chunk);
                }
                let release =
                    WebsiteReleasesV3::parse(&bytes)?.windows_update(env!("CARGO_PKG_VERSION"))?;
                state.updater.change(|v| {
                    v.phase = if release.is_some() {
                        "available"
                    } else {
                        "current"
                    };
                    v.available = release;
                    v.package_ready = false;
                    v.downloaded_bytes = 0;
                });
                Ok::<_, UpdateFailure>(())
            }
            .await;
            if let Err(code) = result {
                state.updater.change(|v| {
                    v.phase = "check_failed";
                    v.error = Some(code.code.into());
                    v.diagnostic = Some(code.detail.clone());
                });
            }
        });
        Ok(initial)
    }
    pub(super) fn download(state: Arc<ShellState>) -> Result<UpdateView, &'static str> {
        if !state.updater.status()?.can_install {
            return Err("UPGRADE_INSTALLATION_UNAVAILABLE");
        }
        let release = state
            .updater
            .status()?
            .available
            .ok_or("UPGRADE_NOT_AVAILABLE")?;
        release.revision_prefix()?;
        state.updater.begin("downloading")?;
        state
            .updater
            .ready_stage
            .lock()
            .map_err(|_| "UPGRADE_STATE_UNAVAILABLE")?
            .take();
        let initial = state.updater.status()?;
        tauri::async_runtime::spawn(async move {
            let result = async {
                staging::background_cleanup(Some(release.sha256.clone()));
                let stage = staging::download(&release)?;
                let directory = &stage.path;
                let installer = directory.join("installer.exe");
                if installer
                    .try_exists()
                    .map_err(UpdateFailure::io("check-download-file"))?
                {
                    std::fs::remove_file(&installer)
                        .map_err(UpdateFailure::io("remove-previous-download"))?;
                }
                let held = security::ensure_private_directory(&directory)
                    .map_err(UpdateFailure::io("open-download-directory"))?;
                let mut file = security::create_owner_only_file_at(
                    &held,
                    std::ffi::OsStr::new("installer.exe"),
                )
                .map_err(UpdateFailure::io("create-update-download"))?;
                let mut response = http()?
                    .get(release.download_url())
                    .send()
                    .await
                    .map_err(|_| "UPGRADE_DOWNLOAD_FAILED")?
                    .error_for_status()
                    .map_err(|_| "UPGRADE_DOWNLOAD_FAILED")?;
                if response.content_length().is_some_and(|n| n != release.size) {
                    return Err("UPGRADE_PACKAGE_SIZE_MISMATCH".into());
                }
                let mut sha = Sha256::new();
                let mut count = 0;
                while let Some(chunk) = response
                    .chunk()
                    .await
                    .map_err(|_| "UPGRADE_DOWNLOAD_FAILED")?
                {
                    state.updater.cancelled()?;
                    count += chunk.len() as u64;
                    if count > release.size {
                        return Err("UPGRADE_PACKAGE_SIZE_MISMATCH".into());
                    }
                    file.write_all(&chunk)
                        .map_err(UpdateFailure::io("write-update-download"))?;
                    sha.update(&chunk);
                    state.updater.change(|v| v.downloaded_bytes = count);
                }
                file.sync_all()
                    .map_err(UpdateFailure::io("flush-update-download"))?;
                drop(file);
                state.updater.change(|v| v.phase = "verifying");
                state.updater.cancelled()?;
                if count != release.size {
                    return Err("UPGRADE_PACKAGE_SIZE_MISMATCH".into());
                }
                if format!("{:x}", sha.finalize()) != release.sha256 {
                    return Err("UPGRADE_DIGEST_MISMATCH".into());
                }
                let stage = stage.into_reader()?;
                *state
                    .updater
                    .ready_stage
                    .lock()
                    .map_err(|_| "UPGRADE_STATE_UNAVAILABLE")? = Some(stage);
                state.updater.change(|v| {
                    v.phase = "ready";
                    v.package_ready = true;
                });
                Ok::<_, UpdateFailure>(())
            }
            .await;
            if let Err(code) = result {
                if code.code != "UPGRADE_CANCELLED" {
                    code.record(service::available_blocking());
                }
                let _ = staging::discard(&release);
                state.updater.change(|v| {
                    v.package_ready = false;
                    v.phase = if code.code == "UPGRADE_CANCELLED" {
                        "available"
                    } else {
                        "failed"
                    };
                    v.error = (code.code != "UPGRADE_CANCELLED").then(|| code.code.into());
                    v.diagnostic = (code.code != "UPGRADE_CANCELLED").then(|| code.detail.clone());
                });
            }
        });
        Ok(initial)
    }
    pub(super) fn install(
        app: tauri::AppHandle,
        state: Arc<ShellState>,
    ) -> Result<UpdateView, &'static str> {
        let before = state.updater.status()?;
        if !before.package_ready {
            return Err("UPGRADE_PACKAGE_NOT_READY");
        }
        let release = before.available.ok_or("UPGRADE_NOT_AVAILABLE")?;
        state.updater.begin("waiting")?;
        let initial = state.updater.status()?;
        tauri::async_runtime::spawn_blocking(move || {
            let result = (|| {
                finisher::verify(&release)?; // recheck staged bytes before pausing admission
                let (layout, definition) = daemon()?;
                let endpoint = layout.lifecycle_endpoint();
                let mut status = lifecycle::upgrade(
                    &endpoint,
                    &definition.daemon,
                    "upgrade-prepare",
                    Duration::from_secs(2),
                )
                .map_err(drain_error)?;
                loop {
                    state.updater.change(|v| {
                        v.active_calls = status.active_calls;
                        v.active_tasks = status.active_tasks;
                    });
                    state.updater.cancelled()?;
                    if status.drained() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                    status = lifecycle::upgrade(
                        &endpoint,
                        &definition.daemon,
                        "upgrade-status",
                        Duration::from_secs(2),
                    )
                    .map_err(drain_error)?;
                }
                state.updater.cancelled()?;
                finisher::launch(&state.updater, &release)?;
                {
                    let mut view = state
                        .updater
                        .view
                        .lock()
                        .map_err(|_| "UPGRADE_STATE_UNAVAILABLE")?;
                    state.updater.cancelled()?;
                    view.phase = "installing";
                    finisher::commit(&release)?;
                }
                app.exit(0);
                Ok::<_, UpdateFailure>(())
            })();
            if let Err(code) = result {
                if code.code == "UPGRADE_CANCELLED" {
                    let _ = finisher::cancel(&release);
                }
                if let Ok(mut parent) = state.updater.parent_lock.lock() {
                    parent.take();
                }
                let resumed = resume_admission().is_ok() && service::available_blocking();
                if code.code != "UPGRADE_CANCELLED" {
                    code.record(resumed);
                }
                state.updater.change(|v| {
                    v.phase = if code.code == "UPGRADE_CANCELLED" {
                        "ready"
                    } else {
                        "failed"
                    };
                    v.error = (code.code != "UPGRADE_CANCELLED").then(|| code.code.into());
                    v.diagnostic = Some(code.detail.clone());
                    v.recovery = Some(if resumed {
                        "service_available"
                    } else {
                        "restart_required"
                    });
                });
            }
        });
        Ok(initial)
    }
    pub(super) fn cancel(&self) -> Result<UpdateView, &'static str> {
        let mut view = self.view.lock().map_err(|_| "UPGRADE_STATE_UNAVAILABLE")?;
        if matches!(view.phase, "ready" | "failed") {
            let release = view.available.as_ref().ok_or("UPGRADE_NOT_AVAILABLE")?;
            let mut ready = self
                .ready_stage
                .lock()
                .map_err(|_| "UPGRADE_STATE_UNAVAILABLE")?;
            if let Err(failure) = staging::discard_ready(release, &mut ready) {
                view.package_ready = ready.is_some();
                view.phase = "failed";
                view.error = Some(failure.code.into());
                view.diagnostic = Some(failure.detail);
                return Err(failure.code);
            }
            view.phase = "available";
            view.package_ready = false;
            view.downloaded_bytes = 0;
            view.error = None;
            view.diagnostic = None;
            return Ok(view.clone());
        }
        if !matches!(view.phase, "downloading" | "verifying" | "waiting") {
            return Err("UPGRADE_NOT_CANCELLABLE");
        }
        self.cancel.store(true, Ordering::Release);
        Ok(view.clone())
    }
}
fn catalog_status(status: reqwest::StatusCode) -> Result<(), &'static str> {
    match status {
        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE => {
            Err("UPGRADE_CATALOG_UNAVAILABLE")
        }
        status if status.is_success() => Ok(()),
        _ => Err("UPGRADE_CHECK_FAILED"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_catalog_cannot_be_mistaken_for_a_completed_update_check() {
        for status in [404, 410] {
            assert_eq!(
                catalog_status(reqwest::StatusCode::from_u16(status).unwrap()),
                Err("UPGRADE_CATALOG_UNAVAILABLE"),
            );
        }
        for status in [302, 403, 500] {
            assert_eq!(
                catalog_status(reqwest::StatusCode::from_u16(status).unwrap()),
                Err("UPGRADE_CHECK_FAILED"),
            );
        }
        assert_eq!(catalog_status(reqwest::StatusCode::OK), Ok(()));
    }
}
fn http() -> Result<reqwest::Client, &'static str> {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|_| "UPGRADE_NETWORK_UNAVAILABLE")
}
fn daemon() -> Result<(StandaloneLayout, background::Definition), &'static str> {
    let layout = StandaloneLayout::from_environment().map_err(|_| "INSTALLATION_UNAVAILABLE")?;
    let definition =
        background::read_definition(&layout.data_root.join("service/windows-background.json"))
            .map_err(|_| "INSTALLATION_UNAVAILABLE")?;
    if definition.manager != background::Manager::Task {
        return Err("INSTALL_SCM_MANAGED");
    }
    Ok((layout, definition))
}
fn drain_error(error: std::io::Error) -> &'static str {
    if error.kind() == std::io::ErrorKind::Unsupported {
        "DRAIN_UNSUPPORTED"
    } else {
        "UPGRADE_DRAIN_FAILED"
    }
}
pub(super) fn resume_admission() -> Result<(), &'static str> {
    let (layout, definition) = daemon()?;
    if !hiroute_platform::windows::task::active(&definition)
        .map_err(|_| "SERVICE_STATUS_UNAVAILABLE")?
    {
        return Ok(());
    }
    match lifecycle::upgrade(
        &layout.lifecycle_endpoint(),
        &definition.daemon,
        "upgrade-cancel",
        Duration::from_secs(2),
    ) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::Unsupported => Ok(()),
        Err(_) => Err("UPGRADE_RESUME_FAILED"),
    }
}
