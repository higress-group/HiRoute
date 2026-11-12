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
    let parent = path.parent().ok_or("UPGRADE_INSTALLER_WRITE_FAILED")?;
    if fs::symlink_metadata(path).is_ok() {
        return Err("UPGRADE_INSTALLER_WRITE_FAILED".into());
    }
    let temporary = parent.join(format!(".marker-{}.tmp", crate::random_id()?));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| "UPGRADE_INSTALLER_WRITE_FAILED")?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "UPGRADE_INSTALLER_WRITE_FAILED")?;
        // Readers must see the complete decision/ack, never a newly created empty file.
        fs::rename(&temporary, path).map_err(|_| "UPGRADE_INSTALLER_WRITE_FAILED")?;
        fs::File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|_| "UPGRADE_INSTALLER_WRITE_FAILED".into())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
pub(super) fn prepare(root: &Path, update: &PreparedUpdate) -> Result<PathBuf, String> {
    hiroute_host_runtime::run_upgrade_preflight(
        &update.staged.join("Contents/MacOS/hirouted"),
        &root.join("storage"),
        &update.current,
    )?;
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
mod control;
pub(super) use control::start;

pub fn finish_update_if_requested() -> Option<Result<(), String>> {
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--finish-upgrade")) {
        return None;
    }
    let result = (|| {
        let path = PathBuf::from(arguments.next().ok_or("UPGRADE_INSTALLER_INVALID")?);
        if arguments.next().is_some() {
            return Err("UPGRADE_INSTALLER_INVALID".into());
        }
        // Only a validated handoff authorizes writing a private result. Every failure,
        // including load(), still reaches the parent-owned stderr below.
        let record = load(&path)?;
        let result = finish(&path, record);
        if let Err(error) = &result {
            let _ = write_private(
                &path.with_file_name("failed.json"),
                &serde_json::to_vec(&serde_json::json!({"code":error})).unwrap(),
            );
        }
        result
    })();
    if let Err(error) = &result {
        eprintln!("{error}");
    }
    Some(result)
}

fn load(path: &Path) -> Result<Handoff, String> {
    let executable =
        fs::canonicalize(std::env::current_exe().map_err(|_| "UPGRADE_INSTALLER_INVALID")?)
            .map_err(|_| "UPGRADE_INSTALLER_INVALID")?;
    let current = executable
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or("UPGRADE_INSTALLER_INVALID")?;
    load_for_app(path, current)
}
fn load_for_app(path: &Path, current: &Path) -> Result<Handoff, String> {
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
        || record.current != current
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
    record.release.revision_prefix()?;
    Ok(record)
}
fn finish(path: &Path, record: Handoff) -> Result<(), String> {
    if nix::unistd::getppid().as_raw() as u32 != record.parent_pid {
        return Err("UPGRADE_INSTALLER_PARENT_MISMATCH".into());
    }
    verify_packages(&record)?;
    let directory = PrivateDir::open_existing(path.parent().unwrap())
        .map_err(|_| "UPGRADE_INSTALLER_INVALID")?;
    write_private(&path.with_file_name("started"), control::STARTED)?;
    control::await_commit(&directory, record.parent_pid, Duration::from_secs(60))?;
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
    verify_packages(&record)?;
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

fn verify_packages(record: &Handoff) -> Result<(), String> {
    if super::verify_update_package(&record.current, None)?
        != record.old_identity
        || super::verify_update_package(&record.staged, Some(&record.release))?
            != record.new_identity
    {
        return Err("UPGRADE_PACKAGE_CHANGED".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
