//! Same-package installer: no WebView paths, privilege escalation, or automatic data rollback.
use super::PreparedUpdate;
use hiroute_diagnostics::files::PrivateDir;
use hiroute_host_runtime::{DesktopPackageIdentity, DesktopRelease};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Handoff {
    schema: String,
    parent_pid: u32,
    data_root: PathBuf,
    current: PathBuf,
    staged: PathBuf,
    previous: PathBuf,
    release: DesktopRelease,
    old_identity: DesktopPackageIdentity,
    new_identity: DesktopPackageIdentity,
}
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| "UPGRADE_INSTALLER_WRITE_FAILED")?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "UPGRADE_INSTALLER_WRITE_FAILED".into())
}
pub(super) fn prepare(root: &Path, update: &PreparedUpdate) -> Result<PathBuf, String> {
    let id = crate::random_id()?;
    let directory = update.download_root.join(format!("install-{id}"));
    crate::bootstrap::private_dir(&directory)?;
    let previous = update
        .current
        .parent()
        .ok_or("UPGRADE_APP_PATH_INVALID")?
        .join(format!(
            ".HiRoute-previous-{}-{id}.app",
            &update.old_identity.revision[..12]
        ));
    let handoff = Handoff {
        schema: "hiroute.desktop-upgrade-handoff/v1".into(),
        parent_pid: std::process::id(),
        data_root: root.to_path_buf(),
        current: update.current.clone(),
        staged: update.staged.clone(),
        previous,
        release: update.release.clone(),
        old_identity: update.old_identity.clone(),
        new_identity: update.new_identity.clone(),
    };
    let path = directory.join("handoff.json");
    write_private(
        &path,
        &serde_json::to_vec(&handoff).map_err(|_| "UPGRADE_INSTALLER_WRITE_FAILED")?,
    )?;
    fs::File::open(&directory)
        .and_then(|f| f.sync_all())
        .map_err(|_| "UPGRADE_INSTALLER_WRITE_FAILED")?;
    Ok(path)
}
pub(super) fn launch(path: &Path) -> Result<(), String> {
    let directory = path.parent().ok_or("UPGRADE_INSTALLER_INVALID")?;
    use std::os::unix::fs::OpenOptionsExt;
    let log = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.join("installer.log"))
        .map_err(|_| "UPGRADE_INSTALLER_WRITE_FAILED")?;
    let mut child =
        Command::new(std::env::current_exe().map_err(|_| "UPGRADE_INSTALLER_UNAVAILABLE")?)
            .arg("--finish-upgrade")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .map_err(|_| "UPGRADE_INSTALLER_UNAVAILABLE")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let private = PrivateDir::open_existing(directory).map_err(|_| "UPGRADE_INSTALLER_INVALID")?;
    loop {
        if let Some(mut ack) = private
            .open_read("started")
            .map_err(|_| "UPGRADE_INSTALLER_INVALID")?
        {
            if ack
                .read_prefix(64)
                .map_err(|_| "UPGRADE_INSTALLER_INVALID")?
                == b"hiroute.upgrade-installer-started/v1\n"
            {
                return Ok(());
            }
            return Err("UPGRADE_INSTALLER_INVALID".into());
        }
        if child
            .try_wait()
            .map_err(|_| "UPGRADE_INSTALLER_FAILED")?
            .is_some()
            || Instant::now() >= deadline
        {
            return Err("UPGRADE_INSTALLER_FAILED".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
pub fn finish_update_if_requested() -> Option<Result<(), String>> {
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--finish-upgrade")) {
        return None;
    }
    Some((|| {
        let path = PathBuf::from(arguments.next().ok_or("UPGRADE_INSTALLER_INVALID")?);
        if arguments.next().is_some() {
            return Err("UPGRADE_INSTALLER_INVALID".into());
        }
        // Invalid arguments never authorize creating files beside an arbitrary supplied path.
        let record = load(&path)?;
        let result = finish(&path, record);
        if let Err(error) = &result {
            // Preserve a small, private result beside the handoff. No credentials or database
            // bytes are involved; the existing manual recovery procedure remains authoritative.
            let _ = write_private(
                &path.with_file_name("failed.json"),
                &serde_json::to_vec(&serde_json::json!({"code":error})).unwrap(),
            );
            eprintln!("{error}");
        }
        result
    })())
}
fn load(path: &Path) -> Result<Handoff, String> {
    if !path.is_absolute() || path.file_name() != Some("handoff.json".as_ref()) {
        return Err("UPGRADE_INSTALLER_INVALID".into());
    }
    let directory = PrivateDir::open_existing(path.parent().ok_or("UPGRADE_INSTALLER_INVALID")?)
        .map_err(|_| "UPGRADE_INSTALLER_INVALID")?;
    let mut file = directory
        .open_read("handoff.json")
        .map_err(|_| "UPGRADE_INSTALLER_INVALID")?
        .ok_or("UPGRADE_INSTALLER_INVALID")?;
    if file.len() > 256 * 1024 {
        return Err("UPGRADE_INSTALLER_INVALID".into());
    }
    let record: Handoff = serde_json::from_slice(
        &file
            .read_prefix(256 * 1024)
            .map_err(|_| "UPGRADE_INSTALLER_INVALID")?,
    )
    .map_err(|_| "UPGRADE_INSTALLER_INVALID")?;
    file.recheck().map_err(|_| "UPGRADE_INSTALLER_INVALID")?;
    if record.schema != "hiroute.desktop-upgrade-handoff/v1"
        || record.parent_pid == 0
        || !record.data_root.is_absolute()
        || !path.starts_with(record.data_root.join("updates"))
        || record.current
            != fs::canonicalize(std::env::current_exe().map_err(|_| "UPGRADE_INSTALLER_INVALID")?)
                .map_err(|_| "UPGRADE_INSTALLER_INVALID")?
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .ok_or("UPGRADE_INSTALLER_INVALID")?
        || record.old_identity.app_path != record.current
        || record.new_identity.app_path != record.staged
        || record.staged.file_name() != Some("HiRoute.app".as_ref())
        || record.staged.parent().and_then(Path::parent) != record.current.parent()
        || !record
            .staged
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|n| n.to_string_lossy().starts_with(".HiRoute-update-"))
        || record.previous.parent() != record.current.parent()
        || !record.previous.file_name().is_some_and(|n| {
            n.to_string_lossy().starts_with(".HiRoute-previous-")
                && n.to_string_lossy().ends_with(".app")
        })
        || fs::symlink_metadata(&record.previous).is_ok()
    {
        return Err("UPGRADE_INSTALLER_INVALID".into());
    }
    PrivateDir::open_existing(&record.data_root).map_err(|_| "PRIVATE_PATH_INVALID")?;
    PrivateDir::open_existing(record.staged.parent().unwrap())
        .map_err(|_| "UPGRADE_INSTALLER_INVALID")?;
    record.release.revision_prefix()?;
    Ok(record)
}
fn finish(path: &Path, record: Handoff) -> Result<(), String> {
    if nix::unistd::getppid().as_raw() as u32 != record.parent_pid {
        return Err("UPGRADE_INSTALLER_PARENT_MISMATCH".into());
    }
    write_private(
        &path.with_file_name("started"),
        b"hiroute.upgrade-installer-started/v1\n",
    )?;
    let pid = nix::unistd::Pid::from_raw(record.parent_pid as i32);
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        match nix::sys::signal::kill(pid, None) {
            Err(nix::errno::Errno::ESRCH) => break,
            Err(_) => return Err("UPGRADE_INSTALLER_PARENT_UNKNOWN".into()),
            Ok(()) if Instant::now() >= deadline => {
                return Err("UPGRADE_DESKTOP_STOP_PENDING".into());
            }
            Ok(()) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    // A newly launched host wins the lock and prevents replacement. Never replace underneath it.
    let _lock = crate::bootstrap::acquire_host_lock(&record.data_root)?;
    if hiroute_host_runtime::verify_desktop_app(&record.current, "ai.hiroute.desktop", None, None)?
        != record.old_identity
        || hiroute_host_runtime::verify_desktop_app(
            &record.staged,
            "ai.hiroute.desktop",
            Some(&record.release),
            None,
        )? != record.new_identity
    {
        return Err("UPGRADE_PACKAGE_CHANGED".into());
    }
    fs::rename(&record.current, &record.previous).map_err(|_| "UPGRADE_REPLACE_FAILED")?;
    if fs::rename(&record.staged, &record.current).is_err() {
        // The data has not been touched. Restore the displaced App name if the second rename
        // itself fails, retaining all staging bytes and the manual recovery package.
        fs::rename(&record.previous, &record.current).map_err(|_| "UPGRADE_APP_RESTORE_FAILED")?;
        return Err("UPGRADE_REPLACE_FAILED".into());
    }
    fs::File::open(record.current.parent().unwrap())
        .and_then(|f| f.sync_all())
        .map_err(|_| "UPGRADE_REPLACE_SYNC_FAILED")?;
    write_private(
        &path.with_file_name("installed.json"),
        &serde_json::to_vec(
            &serde_json::json!({"version":record.release.version,"previous_app":record.previous}),
        )
        .map_err(|_| "UPGRADE_INSTALLER_WRITE_FAILED")?,
    )?;
    drop(_lock);
    #[cfg(all(feature = "desktop-pilot", debug_assertions))]
    let mut restart = Command::new(record.current.join("Contents/MacOS/hiroute-desktop"));
    #[cfg(not(all(feature = "desktop-pilot", debug_assertions)))]
    let mut restart = {
        let mut command = Command::new("/usr/bin/open");
        command.arg("-a").arg(&record.current);
        command
    };
    restart
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "UPGRADE_RESTART_FAILED")?;
    // Keep the installed handoff, DMG and previous App for explicit recovery. The empty owned
    // stage directory is disposable and contains no user data.
    let _ = fs::remove_dir(record.staged.parent().unwrap());
    Ok(())
}
