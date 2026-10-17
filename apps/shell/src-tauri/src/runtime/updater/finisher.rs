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
fn err(_: impl std::fmt::Debug) -> &'static str {
    "UPGRADE_STAGE_UNAVAILABLE"
}
fn identifier(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn directory(id: &str) -> Result<PathBuf, &'static str> {
    if !identifier(id) {
        return Err("UPGRADE_ARTIFACT_INVALID");
    }
    Ok(transaction::data().map_err(err)?.join("updates").join(id))
}
pub(super) fn stage(release: &WindowsDesktopRelease) -> Result<PathBuf, &'static str> {
    release.revision_prefix()?;
    let path = directory(&release.sha256)?;
    security::ensure_private_directory(&path).map_err(err)?;
    Ok(path)
}
fn write(path: &Path, value: &impl Serialize) -> Result<(), &'static str> {
    hiroute_platform::durable::write_private_atomic(path, &serde_json::to_vec(value).map_err(err)?)
        .map_err(err)
}
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, &'static str> {
    let mut file = security::open_owner_only(path, false).map_err(err)?;
    let mut bytes = vec![];
    Read::by_ref(&mut file)
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("UPGRADE_STAGE_INVALID");
    }
    serde_json::from_slice(&bytes).map_err(|_| "UPGRADE_STAGE_INVALID")
}
fn lock(path: &Path) -> Result<File, &'static str> {
    let parent = security::ensure_private_directory(path.parent().ok_or("UPGRADE_STAGE_INVALID")?)
        .map_err(err)?;
    match security::open_read_write(path) {
        Ok(f) => {
            security::verify_owner_only_handle(&f, false).map_err(err)?;
            Ok(f)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => security::create_owner_only_file_at(
            &parent,
            path.file_name().ok_or("UPGRADE_STAGE_INVALID")?,
        )
        .map_err(err),
        Err(e) => Err(err(e)),
    }
}
fn helper(directory: &Path) -> Result<PathBuf, &'static str> {
    let path = directory.join("finisher.exe");
    if path.try_exists().map_err(err)? {
        security::open_owner_only(&path, false).map_err(err)?;
        fs::remove_file(&path).map_err(err)?;
    }
    let mut input =
        security::open_identity(&std::env::current_exe().map_err(err)?, false).map_err(err)?;
    let parent = security::ensure_private_directory(directory).map_err(err)?;
    let mut output =
        security::create_owner_only_file_at(&parent, std::ffi::OsStr::new("finisher.exe"))
            .map_err(err)?;
    io::copy(&mut input, &mut output).map_err(err)?;
    output.sync_all().map_err(err)?;
    Ok(path)
}
pub(super) fn verify(release: &WindowsDesktopRelease) -> Result<(), &'static str> {
    release.revision_prefix()?;
    let path = stage(release)?.join("installer.exe");
    let file = security::open_owner_only(&path, false).map_err(err)?;
    if file.metadata().map_err(err)?.len() != release.size {
        return Err("UPGRADE_PACKAGE_SIZE_MISMATCH");
    }
    drop(file);
    if transaction::hash(&path).map_err(err)? != release.sha256 {
        return Err("UPGRADE_DIGEST_MISMATCH");
    }
    Ok(())
}
fn spawn(path: &Path, args: &[&str]) -> Result<(), &'static str> {
    use std::os::windows::process::CommandExt;
    Command::new(path)
        .args(args)
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|_| "UPGRADE_INSTALLER_LAUNCH_FAILED")?;
    Ok(())
}
pub(super) fn launch(
    updater: &Updater,
    release: &WindowsDesktopRelease,
) -> Result<(), &'static str> {
    verify(release)?;
    let directory = stage(release)?;
    let parent = lock(&directory.join("parent.lock"))?;
    parent.try_lock_exclusive().map_err(|_| "UPGRADE_BUSY")?;
    let helper = helper(&directory)?;
    let job = Job {
        schema: "hiroute.windows-shell-update-job/v1".into(),
        release: release.clone(),
        helper_sha256: transaction::hash(&helper).map_err(err)?,
        generation: format!(
            "{:x}-{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(err)?
                .as_nanos(),
            std::process::id()
        ),
    };
    write(&directory.join("job.json"), &job)?;
    let ready = directory.join("ready.json");
    if ready.try_exists().map_err(err)? {
        security::open_owner_only(&ready, false).map_err(err)?;
        fs::remove_file(&ready).map_err(err)?;
    }
    let cancelled = directory.join("cancel.json");
    if cancelled.try_exists().map_err(err)? {
        security::open_owner_only(&cancelled, false).map_err(err)?;
        fs::remove_file(&cancelled).map_err(err)?;
    }
    let committed = directory.join("commit.json");
    if committed.try_exists().map_err(err)? {
        security::open_owner_only(&committed, false).map_err(err)?;
        fs::remove_file(committed).map_err(err)?;
    }
    *updater
        .parent_lock
        .lock()
        .map_err(|_| "UPGRADE_STATE_UNAVAILABLE")? = Some(parent);
    let result = (|| {
        spawn(&helper, &["--update-finish", &release.sha256])?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if ready.try_exists().map_err(err)? {
                let value: serde_json::Value = read(&ready)?;
                if value["ready"] == true && value["generation"] == job.generation {
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Err("UPGRADE_INSTALLER_LAUNCH_FAILED")
    })();
    if result.is_err() {
        let _ = write(&cancelled, &serde_json::json!({"cancelled":true}));
        updater
            .parent_lock
            .lock()
            .map_err(|_| "UPGRADE_STATE_UNAVAILABLE")?
            .take();
    }
    result
}
pub(super) fn commit(release: &WindowsDesktopRelease) -> Result<(), &'static str> {
    let directory = stage(release)?;
    let job: Job = read(&directory.join("job.json"))?;
    if job.schema != "hiroute.windows-shell-update-job/v1" || job.release.sha256 != release.sha256 {
        return Err("UPGRADE_STAGE_INVALID");
    }
    write(
        &directory.join("commit.json"),
        &crate::handoff::WindowsUpdateCommitV1::new(job.generation, release.sha256.clone()),
    )
}
fn wait_parent(directory: &Path) -> Result<File, &'static str> {
    let parent = lock(&directory.join("parent.lock"))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while parent.try_lock_exclusive().is_err() {
        if Instant::now() >= deadline {
            return Err("UPGRADE_PARENT_EXIT_TIMEOUT");
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    Ok(parent)
}
fn finish(id: &str) -> Result<(), &'static str> {
    let directory = directory(id)?;
    let job: Job = read(&directory.join("job.json"))?;
    if job.schema != "hiroute.windows-shell-update-job/v1"
        || job.release.sha256 != id
        || transaction::hash(&std::env::current_exe().map_err(err)?).map_err(err)?
            != job.helper_sha256
    {
        return Err("UPGRADE_STAGE_INVALID");
    }
    let update = lock(&transaction::data().map_err(err)?.join("update.lock"))?;
    update.try_lock_exclusive().map_err(|_| "UPGRADE_BUSY")?;
    verify(&job.release)?;
    write(
        &directory.join("ready.json"),
        &serde_json::json!({"ready":true,"generation":job.generation}),
    )?;
    let _parent = wait_parent(&directory)?;
    if directory.join("cancel.json").try_exists().map_err(err)? {
        return Err("UPGRADE_CANCELLED");
    }
    let committed: crate::handoff::WindowsUpdateCommitV1 =
        read(&directory.join("commit.json")).map_err(|_| "UPGRADE_COMMIT_MISSING")?;
    if !committed.matches(&job.generation, id) {
        return Err("UPGRADE_COMMIT_MISSING");
    }
    verify(&job.release)?;
    use std::os::windows::process::CommandExt;
    let installed = Command::new(directory.join("installer.exe"))
        .args(["/P", "/UPDATE"])
        .creation_flags(0x08000000)
        .status()
        .is_ok_and(|status| status.success());
    if !installed || transaction::has_pending().map_err(err)? {
        transaction::rollback(false).map_err(|_| "SHELL_INSTALL_RECOVERY_REQUIRED")?;
        let resumed = resume_admission().is_ok() && service::available_blocking();
        write(
            &transaction::data()
                .map_err(err)?
                .join("update-outcome.json"),
            &serde_json::json!({"schema":"hiroute.windows-shell-update-outcome/v1","status":"failed","code":"UPGRADE_INSTALL_FAILED","service_available":resumed}),
        )?;
    } else {
        write(
            &transaction::data()
                .map_err(err)?
                .join("update-outcome.json"),
            &serde_json::json!({"schema":"hiroute.windows-shell-update-outcome/v1","status":"succeeded"}),
        )?;
    }
    spawn(
        &transaction::root().map_err(err)?.join("hiroute-shell.exe"),
        &["--recovery-report"],
    )
}
fn recover() -> Result<(), &'static str> {
    let directory = transaction::data().map_err(err)?.join("recovery");
    let _parent = wait_parent(&directory)?;
    let update = lock(&transaction::data().map_err(err)?.join("update.lock"))?;
    update.try_lock_exclusive().map_err(|_| "UPGRADE_BUSY")?;
    let _install =
        hiroute_platform::windows::install::desktop_install_guard().map_err(|_| "UPGRADE_BUSY")?;
    let result = transaction::rollback(false).map_err(|_| "SHELL_INSTALL_RECOVERY_REQUIRED");
    let admission = if daemon().is_ok() {
        resume_admission()
    } else {
        Ok(())
    };
    // The marker prevents a failed recovery from recursively spawning another helper.
    let _ = spawn(
        &transaction::root().map_err(err)?.join("hiroute-shell.exe"),
        &["--recovery-report"],
    );
    result.and(admission)
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
        if code != "UPGRADE_CANCELLED" {
            if let Ok(data) = transaction::data() {
                let _ = write(
                    &data.join("update-outcome.json"),
                    &serde_json::json!({"schema":"hiroute.windows-shell-update-outcome/v1","status":"failed","code":code,"service_available":available}),
                );
            }
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
pub(crate) fn recover_startup(
    app: &tauri::AppHandle,
    state: &Arc<ShellState>,
    manual_retry: bool,
) -> Result<bool, &'static str> {
    let update = lock(&transaction::data().map_err(err)?.join("update.lock"))?;
    update.try_lock_exclusive().map_err(|_| "UPGRADE_BUSY")?;
    if transaction::has_pending().map_err(err)? {
        let _install = hiroute_platform::windows::install::desktop_install_guard()
            .map_err(|_| "UPGRADE_BUSY")?;
        if !manual_retry && std::env::args().any(|a| a == "--recovery-report") {
            return Err("SHELL_INSTALL_RECOVERY_REQUIRED");
        }
        let directory = transaction::data().map_err(err)?.join("recovery");
        security::ensure_private_directory(&directory).map_err(err)?;
        let parent = lock(&directory.join("parent.lock"))?;
        parent.try_lock_exclusive().map_err(|_| "UPGRADE_BUSY")?;
        let helper = helper(&directory)?;
        *state
            .updater
            .parent_lock
            .lock()
            .map_err(|_| "UPGRADE_STATE_UNAVAILABLE")? = Some(parent);
        spawn(&helper, &["--update-recover"])?;
        drop(update);
        app.exit(0);
        return Ok(true);
    }
    if daemon().is_ok() {
        resume_admission()?;
    }
    let outcome = transaction::data()
        .map_err(err)?
        .join("update-outcome.json");
    if outcome.try_exists().map_err(err)? {
        let value: serde_json::Value = read(&outcome)?;
        if value["schema"] != "hiroute.windows-shell-update-outcome/v1" {
            return Err("UPGRADE_STAGE_INVALID");
        }
        if value["status"] == "failed" {
            state.updater.change(|v| {
                v.phase = "failed";
                v.error = Some("UPGRADE_INSTALL_FAILED");
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
