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

mod finisher;
pub(super) use finisher::{recover_startup, run_if_requested};
#[derive(Clone, Serialize)]
pub(super) struct UpdateView {
    current_version: &'static str,
    phase: &'static str,
    available: Option<WindowsDesktopRelease>,
    downloaded_bytes: u64,
    active_calls: u64,
    active_tasks: u64,
    error: Option<&'static str>,
    can_install: bool,
    package_ready: bool,
    recovery: Option<&'static str>,
}
pub(super) struct Updater {
    view: Mutex<UpdateView>,
    cancel: AtomicBool,
    parent_lock: Mutex<Option<File>>,
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
            }),
            cancel: AtomicBool::new(false),
            parent_lock: Mutex::new(None),
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
        view.recovery = None;
        Ok(())
    }
    fn failure(&self, code: &'static str) {
        self.change(|v| {
            v.phase = "failed";
            v.error = Some(code);
        });
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
                    .map_err(|_| "UPGRADE_CHECK_FAILED")?
                    .error_for_status()
                    .map_err(|_| "UPGRADE_CHECK_FAILED")?;
                if response
                    .content_length()
                    .is_some_and(|n| n > MAX_RELEASE_FEED_BYTES as u64)
                {
                    return Err("UPGRADE_CATALOG_INVALID");
                }
                let mut bytes = vec![];
                while let Some(chunk) =
                    response.chunk().await.map_err(|_| "UPGRADE_CHECK_FAILED")?
                {
                    if bytes.len() + chunk.len() > MAX_RELEASE_FEED_BYTES {
                        return Err("UPGRADE_CATALOG_INVALID");
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
                Ok::<_, &'static str>(())
            }
            .await;
            if let Err(code) = result {
                state.updater.failure(code);
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
        let initial = state.updater.status()?;
        tauri::async_runtime::spawn(async move {
            let result = async {
                let directory = finisher::stage(&release)?;
                let installer = directory.join("installer.exe");
                if installer
                    .try_exists()
                    .map_err(|_| "UPGRADE_STAGE_UNAVAILABLE")?
                {
                    std::fs::remove_file(&installer).map_err(|_| "UPGRADE_STAGE_UNAVAILABLE")?;
                }
                let held = security::ensure_private_directory(&directory)
                    .map_err(|_| "UPGRADE_STAGE_UNAVAILABLE")?;
                let mut file = security::create_owner_only_file_at(
                    &held,
                    std::ffi::OsStr::new("installer.exe"),
                )
                .map_err(|_| "UPGRADE_STAGE_UNAVAILABLE")?;
                let mut response = http()?
                    .get(release.download_url())
                    .send()
                    .await
                    .map_err(|_| "UPGRADE_DOWNLOAD_FAILED")?
                    .error_for_status()
                    .map_err(|_| "UPGRADE_DOWNLOAD_FAILED")?;
                if response.content_length().is_some_and(|n| n != release.size) {
                    return Err("UPGRADE_PACKAGE_SIZE_MISMATCH");
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
                        return Err("UPGRADE_PACKAGE_SIZE_MISMATCH");
                    }
                    file.write_all(&chunk)
                        .map_err(|_| "UPGRADE_STAGE_UNAVAILABLE")?;
                    sha.update(&chunk);
                    state.updater.change(|v| v.downloaded_bytes = count);
                }
                file.sync_all().map_err(|_| "UPGRADE_STAGE_UNAVAILABLE")?;
                drop(file);
                state.updater.change(|v| v.phase = "verifying");
                state.updater.cancelled()?;
                if count != release.size {
                    return Err("UPGRADE_PACKAGE_SIZE_MISMATCH");
                }
                if format!("{:x}", sha.finalize()) != release.sha256 {
                    return Err("UPGRADE_DIGEST_MISMATCH");
                }
                state.updater.change(|v| {
                    v.phase = "ready";
                    v.package_ready = true;
                });
                Ok::<_, &'static str>(())
            }
            .await;
            if let Err(code) = result {
                state.updater.change(|v| {
                    v.package_ready = false;
                    v.phase = if code == "UPGRADE_CANCELLED" {
                        "available"
                    } else {
                        "failed"
                    };
                    v.error = (code != "UPGRADE_CANCELLED").then_some(code);
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
                Ok::<_, &'static str>(())
            })();
            if let Err(code) = result {
                if let Ok(mut parent) = state.updater.parent_lock.lock() {
                    parent.take();
                }
                let resumed = resume_admission().is_ok() && service::available_blocking();
                state.updater.change(|v| {
                    v.phase = if code == "UPGRADE_CANCELLED" {
                        "ready"
                    } else {
                        "failed"
                    };
                    v.error = (code != "UPGRADE_CANCELLED").then_some(code);
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
        let view = self.view.lock().map_err(|_| "UPGRADE_STATE_UNAVAILABLE")?;
        if !matches!(view.phase, "downloading" | "verifying" | "waiting") {
            return Err("UPGRADE_NOT_CANCELLABLE");
        }
        self.cancel.store(true, Ordering::Release);
        Ok(view.clone())
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
