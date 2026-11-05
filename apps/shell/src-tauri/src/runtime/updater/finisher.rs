use super::super::install_transaction as transaction;
use super::*;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::Command,
};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    schema: String,
    release: WindowsDesktopRelease,
    helper_sha256: String,
    generation: String,
}
pub(super) fn identifier(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn directory(id: &str) -> Result<PathBuf, UpdateFailure> {
    if !identifier(id) {
        return Err("UPGRADE_ARTIFACT_INVALID".into());
    }
    Ok(transaction::data()
        .map_err(UpdateFailure::io("resolve-update-state"))?
        .join("updates")
        .join(id))
}
pub(super) fn write(path: &Path, value: &impl Serialize) -> Result<(), UpdateFailure> {
    hiroute_platform::durable::write_private_atomic(
        path,
        &serde_json::to_vec(value).map_err(|_| UpdateFailure::from("UPGRADE_STAGE_INVALID"))?,
    )
    .map_err(UpdateFailure::io("write-stage-record"))
}
pub(super) fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, UpdateFailure> {
    let mut file =
        security::open_owner_only(path, false).map_err(UpdateFailure::io("open-stage-record"))?;
    let mut bytes = vec![];
    Read::by_ref(&mut file)
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(UpdateFailure::io("read-stage-record"))?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("UPGRADE_STAGE_INVALID".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| UpdateFailure::from("UPGRADE_STAGE_INVALID"))
}
pub(super) fn lock(path: &Path) -> Result<File, UpdateFailure> {
    let parent = security::ensure_private_directory(path.parent().ok_or("UPGRADE_STAGE_INVALID")?)
        .map_err(UpdateFailure::io("open-update-directory"))?;
    match security::open_read_write(path) {
        Ok(f) => {
            security::verify_owner_only_handle(&f, false)
                .map_err(UpdateFailure::io("open-update-lock"))?;
            Ok(f)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => security::create_owner_only_file_at(
            &parent,
            path.file_name().ok_or("UPGRADE_STAGE_INVALID")?,
        )
        .map_err(UpdateFailure::io("open-update-file")),
        Err(e) => Err(UpdateFailure::io("open-update-lock")(e)),
    }
}
fn helper(directory: &Path) -> Result<PathBuf, UpdateFailure> {
    let path = directory.join("finisher.exe");
    if path
        .try_exists()
        .map_err(UpdateFailure::io("check-update-file"))?
    {
        security::open_owner_only(&path, false).map_err(UpdateFailure::io("open-stage-record"))?;
        fs::remove_file(&path).map_err(UpdateFailure::io("remove-update-helper"))?;
    }
    let mut input = security::open_identity(
        &std::env::current_exe().map_err(UpdateFailure::io("locate-update-helper"))?,
        false,
    )
    .map_err(UpdateFailure::io("open-current-helper"))?;
    let parent = security::ensure_private_directory(directory)
        .map_err(UpdateFailure::io("open-update-directory"))?;
    let mut output =
        security::create_owner_only_file_at(&parent, std::ffi::OsStr::new("finisher.exe"))
            .map_err(UpdateFailure::io("create-update-file"))?;
    io::copy(&mut input, &mut output).map_err(UpdateFailure::io("copy-update-helper"))?;
    output
        .sync_all()
        .map_err(UpdateFailure::io("flush-update-helper"))?;
    Ok(path)
}
pub(super) fn verify(release: &WindowsDesktopRelease) -> Result<(), UpdateFailure> {
    release.revision_prefix()?;
    let stage = staging::open(release)?;
    let path = stage.path.join("installer.exe");
    verify_installer(&path, release)
}
pub(super) fn verify_installer(
    path: &Path,
    release: &WindowsDesktopRelease,
) -> Result<(), UpdateFailure> {
    let file = security::open_owner_only(path, false)
        .map_err(UpdateFailure::io("open-update-installer"))?;
    if file
        .metadata()
        .map_err(UpdateFailure::io("open-update-installer"))?
        .len()
        != release.size
    {
        return Err("UPGRADE_PACKAGE_SIZE_MISMATCH".into());
    }
    drop(file);
    if transaction::hash(path).map_err(UpdateFailure::io("hash-update-file"))? != release.sha256 {
        return Err("UPGRADE_DIGEST_MISMATCH".into());
    }
    Ok(())
}
fn spawn(path: &Path, args: &[&str]) -> Result<(), UpdateFailure> {
    use std::os::windows::process::CommandExt;
    Command::new(path)
        .args(args)
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|error| {
            let mut failure = UpdateFailure::io("launch-update-helper")(error);
            failure.code = "UPGRADE_INSTALLER_LAUNCH_FAILED";
            failure
        })?;
    Ok(())
}
pub(super) fn launch(
    updater: &Updater,
    release: &WindowsDesktopRelease,
) -> Result<(), UpdateFailure> {
    verify(release)?;
    let stage = staging::open(release)?;
    let directory = &stage.path;
    let parent = lock(&directory.join("parent.lock"))?;
    parent
        .try_lock_exclusive()
        .map_err(UpdateFailure::io_code("lock-update-stage", "UPGRADE_BUSY"))?;
    let helper = helper(&directory)?;
    let job = Job {
        schema: "hiroute.windows-shell-update-job/v1".into(),
        release: release.clone(),
        helper_sha256: transaction::hash(&helper).map_err(UpdateFailure::io("hash-update-file"))?,
        generation: format!(
            "{:x}-{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| UpdateFailure::from("UPGRADE_STAGE_UNAVAILABLE"))?
                .as_nanos(),
            std::process::id()
        ),
    };
    write(&directory.join("job.json"), &job)?;
    let failed = directory.join("failed.json");
    if failed
        .try_exists()
        .map_err(UpdateFailure::io("check-finisher-failure"))?
    {
        security::open_owner_only(&failed, false)
            .map_err(UpdateFailure::io("read-finisher-failure"))?;
        fs::remove_file(&failed).map_err(UpdateFailure::io("clear-finisher-failure"))?;
    }
    let ready = directory.join("ready.json");
    if ready
        .try_exists()
        .map_err(UpdateFailure::io("check-update-file"))?
    {
        security::open_owner_only(&ready, false).map_err(UpdateFailure::io("open-stage-record"))?;
        fs::remove_file(&ready).map_err(UpdateFailure::io("open-stage-record"))?;
    }
    let cancelled = directory.join("cancel.json");
    if cancelled
        .try_exists()
        .map_err(UpdateFailure::io("remove-update-file"))?
    {
        security::open_owner_only(&cancelled, false)
            .map_err(UpdateFailure::io("open-stage-record"))?;
        fs::remove_file(&cancelled).map_err(UpdateFailure::io("open-stage-record"))?;
    }
    let committed = directory.join("commit.json");
    if committed
        .try_exists()
        .map_err(UpdateFailure::io("remove-update-file"))?
    {
        security::open_owner_only(&committed, false)
            .map_err(UpdateFailure::io("open-stage-record"))?;
        fs::remove_file(committed).map_err(UpdateFailure::io("open-stage-record"))?;
    }
    *updater
        .parent_lock
        .lock()
        .map_err(|_| UpdateFailure::from("UPGRADE_STATE_UNAVAILABLE"))? = Some(parent);
    let result = (|| {
        spawn(&helper, &["--update-finish", &release.sha256])?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if failed
                .try_exists()
                .map_err(UpdateFailure::io("check-finisher-failure"))?
            {
                let value: serde_json::Value = read(&failed)?;
                if value["generation"] == job.generation {
                    return Err(child_failure(&value)?);
                }
            }
            if ready
                .try_exists()
                .map_err(UpdateFailure::io("check-update-file"))?
            {
                let value: serde_json::Value = read(&ready)?;
                if value["ready"] == true && value["generation"] == job.generation {
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Err("UPGRADE_INSTALLER_LAUNCH_FAILED".into())
    })();
    if result.is_err() {
        let _ = write(&cancelled, &serde_json::json!({"cancelled":true}));
        updater
            .parent_lock
            .lock()
            .map_err(|_| UpdateFailure::from("UPGRADE_STATE_UNAVAILABLE"))?
            .take();
    }
    result
}
pub(super) fn cancel(release: &WindowsDesktopRelease) -> Result<(), UpdateFailure> {
    let stage = staging::open(release)?;
    write(
        &stage.path.join("cancel.json"),
        &serde_json::json!({"cancelled": true}),
    )
}
pub(super) fn child_failure(value: &serde_json::Value) -> Result<UpdateFailure, UpdateFailure> {
    if value["schema"] != "hiroute.windows-shell-update-failure/v1" {
        return Err("UPGRADE_STAGE_INVALID".into());
    }
    let code = match value["code"].as_str() {
        Some("UPGRADE_BUSY") => "UPGRADE_BUSY",
        Some("UPGRADE_STAGE_INVALID") => "UPGRADE_STAGE_INVALID",
        Some("UPGRADE_DIGEST_MISMATCH") => "UPGRADE_DIGEST_MISMATCH",
        Some("UPGRADE_PACKAGE_SIZE_MISMATCH") => "UPGRADE_PACKAGE_SIZE_MISMATCH",
        Some("UPGRADE_INSTALLER_LAUNCH_FAILED") => "UPGRADE_INSTALLER_LAUNCH_FAILED",
        _ => "UPGRADE_STAGE_UNAVAILABLE",
    };
    let detail: UpdateDiagnostic = serde_json::from_value(value["diagnostic"].clone())
        .map_err(|_| UpdateFailure::from("UPGRADE_STAGE_INVALID"))?;
    if !detail.valid() {
        return Err("UPGRADE_STAGE_INVALID".into());
    }
    Ok(UpdateFailure { code, detail })
}
pub(super) fn commit(release: &WindowsDesktopRelease) -> Result<(), UpdateFailure> {
    let stage = staging::open(release)?;
    let directory = &stage.path;
    let job: Job = read(&directory.join("job.json"))?;
    if job.schema != "hiroute.windows-shell-update-job/v1" || job.release.sha256 != release.sha256 {
        return Err("UPGRADE_STAGE_INVALID".into());
    }
    write(
        &directory.join("commit.json"),
        &crate::handoff::WindowsUpdateCommitV1::new(job.generation, release.sha256.clone()),
    )
}
fn wait_parent(directory: &Path) -> Result<File, UpdateFailure> {
    let parent = lock(&directory.join("parent.lock"))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while parent.try_lock_exclusive().is_err() {
        if Instant::now() >= deadline {
            return Err("UPGRADE_PARENT_EXIT_TIMEOUT".into());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    Ok(parent)
}
fn finish(id: &str) -> Result<(), UpdateFailure> {
    let directory = directory(id)?;
    let job: Job = read(&directory.join("job.json"))?;
    if job.schema != "hiroute.windows-shell-update-job/v1"
        || job.release.sha256 != id
        || transaction::hash(
            &std::env::current_exe().map_err(UpdateFailure::io("locate-update-helper"))?,
        )
        .map_err(UpdateFailure::io("hash-update-file"))?
            != job.helper_sha256
    {
        return Err("UPGRADE_STAGE_INVALID".into());
    }
    let _stage = staging::open(&job.release)?;
    let update = lock(
        &transaction::data()
            .map_err(UpdateFailure::io("resolve-update-state"))?
            .join("update.lock"),
    )?;
    update
        .try_lock_exclusive()
        .map_err(UpdateFailure::io_code("lock-update-stage", "UPGRADE_BUSY"))?;
    verify(&job.release)?;
    write(
        &directory.join("ready.json"),
        &serde_json::json!({"ready":true,"generation":job.generation}),
    )?;
    let _parent = wait_parent(&directory)?;
    if directory
        .join("cancel.json")
        .try_exists()
        .map_err(UpdateFailure::io("check-update-file"))?
    {
        return Err("UPGRADE_CANCELLED".into());
    }
    let committed: crate::handoff::WindowsUpdateCommitV1 = read(&directory.join("commit.json"))
        .map_err(|mut failure| {
            failure.code = "UPGRADE_COMMIT_MISSING";
            failure.detail.step = "read-update-commit".into();
            failure
        })?;
    if !committed.matches(&job.generation, id) {
        return Err("UPGRADE_COMMIT_MISSING".into());
    }
    verify(&job.release)?;
    use std::os::windows::process::CommandExt;
    let status = Command::new(directory.join("installer.exe"))
        .args(["/P", "/UPDATE"])
        .creation_flags(0x08000000)
        .status()
        .map_err(|error| {
            let mut failure = UpdateFailure::io("launch-update-installer")(error);
            failure.code = "UPGRADE_INSTALLER_LAUNCH_FAILED";
            failure
        })?;
    if !status.success()
        || transaction::has_pending().map_err(UpdateFailure::io("open-update-file"))?
    {
        transaction::rollback(false).map_err(UpdateFailure::io_code(
            "rollback-update",
            "SHELL_INSTALL_RECOVERY_REQUIRED",
        ))?;
        let resumed = resume_admission().is_ok() && service::available_blocking();
        let mut failure = UpdateFailure::from("UPGRADE_INSTALL_FAILED");
        failure.detail.step = "run-update-installer".into();
        failure.detail.process_exit = status.code();
        failure.record(resumed);
    } else {
        write(
            &transaction::data()
                .map_err(UpdateFailure::io("open-update-file"))?
                .join("update-outcome.json"),
            &serde_json::json!({"schema":"hiroute.windows-shell-update-outcome/v1","status":"succeeded"}),
        )?;
    }
    spawn(
        &transaction::root()
            .map_err(UpdateFailure::io("open-update-file"))?
            .join("hiroute-shell.exe"),
        &["--recovery-report"],
    )
}
fn recover() -> Result<(), UpdateFailure> {
    let directory = transaction::data()
        .map_err(UpdateFailure::io("resolve-update-state"))?
        .join("recovery");
    let _parent = wait_parent(&directory)?;
    let update = lock(
        &transaction::data()
            .map_err(UpdateFailure::io("resolve-update-state"))?
            .join("update.lock"),
    )?;
    update
        .try_lock_exclusive()
        .map_err(UpdateFailure::io_code("lock-update-stage", "UPGRADE_BUSY"))?;
    let _install = hiroute_platform::windows::install::desktop_install_guard()
        .map_err(UpdateFailure::io_code("lock-update-stage", "UPGRADE_BUSY"))?;
    let result = transaction::rollback(false).map_err(UpdateFailure::io_code(
        "rollback-update",
        "SHELL_INSTALL_RECOVERY_REQUIRED",
    ));
    let admission = if daemon().is_ok() {
        resume_admission()
    } else {
        Ok(())
    };
    // The marker prevents a failed recovery from recursively spawning another helper.
    let _ = spawn(
        &transaction::root()
            .map_err(UpdateFailure::io("open-update-file"))?
            .join("hiroute-shell.exe"),
        &["--recovery-report"],
    );
    result.and(admission.map_err(UpdateFailure::from))
}
pub(crate) fn run_if_requested() -> Option<i32> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("--update-finish") if args.len() == 2 => finish(&args[1]),
        Some("--update-recover") if args.len() == 1 => recover(),
        _ => return None,
    };
    if let Err(code) = result {
        let available = resume_admission().is_ok() && service::available_blocking();
        if code.code != "UPGRADE_CANCELLED" {
            if let Some(id) = args.get(1).filter(|id| identifier(id)) {
                if let Ok(directory) = directory(id) {
                    if let Ok(job) = read::<Job>(&directory.join("job.json")) {
                        let _ = write(
                            &directory.join("failed.json"),
                            &serde_json::json!({
                                "schema": "hiroute.windows-shell-update-failure/v1", "generation": job.generation,
                                "code": code.code, "diagnostic": code.detail
                            }),
                        );
                    }
                }
            }
            code.record(available);
        }
        if let Ok(root) = transaction::root() {
            let _ = spawn(&root.join("hiroute-shell.exe"), &["--recovery-report"]);
        }
        Some(1)
    } else {
        Some(0)
    }
}
/// A killed installer may leave admission paused even if it had not reached file replacement.
fn recover_startup_inner(
    app: &tauri::AppHandle,
    state: &Arc<ShellState>,
    manual_retry: bool,
) -> Result<bool, UpdateFailure> {
    let update = lock(
        &transaction::data()
            .map_err(UpdateFailure::io("resolve-update-state"))?
            .join("update.lock"),
    )?;
    update
        .try_lock_exclusive()
        .map_err(UpdateFailure::io_code("lock-update-stage", "UPGRADE_BUSY"))?;
    if transaction::has_pending().map_err(UpdateFailure::io("open-update-file"))? {
        let _install = hiroute_platform::windows::install::desktop_install_guard()
            .map_err(UpdateFailure::io_code("lock-update-stage", "UPGRADE_BUSY"))?;
        if !manual_retry && std::env::args().any(|a| a == "--recovery-report") {
            return Err("SHELL_INSTALL_RECOVERY_REQUIRED".into());
        }
        let directory = transaction::data()
            .map_err(UpdateFailure::io("resolve-update-state"))?
            .join("recovery");
        security::ensure_private_directory(&directory)
            .map_err(UpdateFailure::io("open-update-directory"))?;
        let parent = lock(&directory.join("parent.lock"))?;
        parent
            .try_lock_exclusive()
            .map_err(UpdateFailure::io_code("lock-update-stage", "UPGRADE_BUSY"))?;
        let helper = helper(&directory)?;
        *state
            .updater
            .parent_lock
            .lock()
            .map_err(|_| UpdateFailure::from("UPGRADE_STATE_UNAVAILABLE"))? = Some(parent);
        spawn(&helper, &["--update-recover"])?;
        drop(update);
        app.exit(0);
        return Ok(true);
    }
    if daemon().is_ok() {
        resume_admission()?;
    }
    let outcome = transaction::data()
        .map_err(UpdateFailure::io("open-update-file"))?
        .join("update-outcome.json");
    if outcome
        .try_exists()
        .map_err(UpdateFailure::io("check-update-file"))?
    {
        let value: serde_json::Value = read(&outcome)?;
        if value["schema"] != "hiroute.windows-shell-update-outcome/v1" {
            return Err("UPGRADE_STAGE_INVALID".into());
        }
        if value["status"] == "failed" {
            state.updater.change(|v| {
                v.phase = "failed";
                v.error = Some(
                    crate::installer_diagnostic::safe_code(
                        value["code"].as_str().unwrap_or("UPGRADE_INSTALL_FAILED"),
                    )
                    .unwrap_or("UPGRADE_INSTALL_FAILED")
                    .into(),
                );
                v.diagnostic =
                    serde_json::from_value::<UpdateDiagnostic>(value["diagnostic"].clone())
                        .ok()
                        .filter(UpdateDiagnostic::valid);
                v.recovery = Some(if value["service_available"] == true {
                    "service_available"
                } else {
                    "restart_required"
                });
            });
        }
    }
    Ok(false)
}

pub(crate) fn recover_startup(
    app: &tauri::AppHandle,
    state: &Arc<ShellState>,
    manual_retry: bool,
) -> Result<bool, &'static str> {
    let result = recover_startup_inner(app, state, manual_retry).map_err(|failure| {
        failure.record(service::available_blocking());
        failure.code
    });
    if matches!(result, Ok(false)) {
        staging::background_cleanup(None);
    }
    result
}
