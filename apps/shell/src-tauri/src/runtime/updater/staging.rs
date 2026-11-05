//! A shared lease pins each owned stage through download or native handoff.
//! Retirement requires an exclusive lease, an ownership record and no recovery.
use super::{diagnostic::UpdateFailure, finisher, *};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
const SCHEMA: &str = "hiroute.windows-update-stage/v1";
const FILES: &[&str] = &[
    "installer.exe",
    "finisher.exe",
    "job.json",
    "ready.json",
    "cancel.json",
    "commit.json",
    "parent.lock",
    "stage.lock",
    "stage.json",
    "failed.json",
];

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: String,
    release: WindowsDesktopRelease,
}
pub(super) struct Stage {
    pub path: PathBuf,
    _lease: File,
    _update: Option<File>,
}
impl Stage {
    pub fn into_reader(mut self) -> Result<Self, UpdateFailure> {
        FileExt::unlock(&self._lease).map_err(UpdateFailure::io("release-stage-writer"))?;
        FileExt::try_lock_shared(&self._lease)
            .map_err(UpdateFailure::io_code("lock-update-stage", "UPGRADE_BUSY"))?;
        self._update.take();
        Ok(self)
    }
}
fn same_artifact(left: &WindowsDesktopRelease, right: &WindowsDesktopRelease) -> bool {
    left.version == right.version
        && left.filename == right.filename
        && left.architecture == right.architecture
        && left.distribution == right.distribution
        && left.sha256 == right.sha256
        && left.size == right.size
}
fn invalid() -> UpdateFailure {
    "UPGRADE_STAGE_INVALID".into()
}
pub(super) fn root() -> Result<PathBuf, UpdateFailure> {
    Ok(super::super::install_transaction::data()
        .map_err(UpdateFailure::io("resolve-stage-root"))?
        .join("updates"))
}
fn directory(path: &Path) -> Result<File, UpdateFailure> {
    let held = security::ensure_private_directory(path)
        .map_err(UpdateFailure::io("open-stage-directory"))?;
    security::verify_owner_only_handle(&held, true)
        .map_err(UpdateFailure::io("verify-stage-owner"))?;
    Ok(held)
}
fn lease(path: &Path) -> Result<File, UpdateFailure> {
    let parent =
        security::open_owner_only(path, true).map_err(UpdateFailure::io("open-stage-directory"))?;
    let lease_path = path.join("stage.lock");
    let lock = match security::open_shared_read_write(&lease_path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match security::create_owner_only_file_at(&parent, std::ffi::OsStr::new("stage.lock")) {
                Ok(created) => drop(created),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(UpdateFailure::io("create-stage-lease")(error)),
            }
            // The creation factory deliberately denies sharing. A lease must be
            // reopenable by readers, and share deletion for atomic retirement.
            security::open_shared_read_write(&lease_path)
                .map_err(UpdateFailure::io("open-stage-lease"))?
        }
        Err(error) => return Err(UpdateFailure::io("open-stage-lease")(error)),
    };
    security::verify_owner_only_handle(&lock, false)
        .map_err(UpdateFailure::io("verify-stage-lease"))?;
    hiroute_platform::private_fs::verify_single_link(&lock)
        .map_err(UpdateFailure::io("verify-stage-link"))?;
    Ok(lock)
}
fn entries(path: &Path) -> Result<Vec<String>, UpdateFailure> {
    let mut names = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(UpdateFailure::io("list-stage"))?
        .take(FILES.len() + 1)
    {
        let entry = entry.map_err(UpdateFailure::io("list-stage"))?;
        let name = entry.file_name().into_string().map_err(|_| invalid())?;
        if !FILES.contains(&name.as_str()) {
            return Err(invalid());
        }
        let file = security::open_owner_only(&entry.path(), false)
            .map_err(UpdateFailure::io("verify-stage-entry"))?;
        hiroute_platform::private_fs::verify_single_link(&file)
            .map_err(UpdateFailure::io("verify-stage-link"))?;
        names.push(name);
    }
    if names.len() > FILES.len() {
        return Err(invalid());
    }
    Ok(names)
}
pub(super) fn open(release: &WindowsDesktopRelease) -> Result<Stage, UpdateFailure> {
    open_in(&root()?, release, false)
}
pub(super) fn download(release: &WindowsDesktopRelease) -> Result<Stage, UpdateFailure> {
    let data = super::super::install_transaction::data()
        .map_err(UpdateFailure::io("resolve-stage-root"))?;
    let global = finisher::lock(&data.join("update.lock"))?;
    global
        .try_lock_exclusive()
        .map_err(UpdateFailure::io_code("lock-update-stage", "UPGRADE_BUSY"))?;
    let mut stage = open_in(&root()?, release, true)?;
    stage._update = Some(global);
    Ok(stage)
}
fn open_in(
    root: &Path,
    release: &WindowsDesktopRelease,
    exclusive: bool,
) -> Result<Stage, UpdateFailure> {
    release.revision_prefix()?;
    directory(root)?;
    let path = root.join(&release.sha256);
    let held = directory(&path)?;
    let identity =
        security::file_identity(&held).map_err(UpdateFailure::io("identify-update-stage"))?;
    let marked = path
        .join("stage.json")
        .try_exists()
        .map_err(UpdateFailure::io("check-stage-owner"))?;
    let names = if !marked || exclusive {
        entries(&path)?
    } else {
        Vec::new()
    };
    if marked {
        let record: Record = finisher::read(&path.join("stage.json"))?;
        if record.schema != SCHEMA || !same_artifact(&record.release, release) {
            return Err(invalid());
        }
    } else {
        // Released helpers bind the complete installer and helper digest in job.json.
        // Only that verified namespace can acquire the new ownership record.
        if names.iter().any(|name| name != "stage.lock") {
            let job: serde_json::Value = finisher::read(&path.join("job.json"))?;
            let previous: WindowsDesktopRelease =
                serde_json::from_value(job["release"].clone()).map_err(|_| invalid())?;
            if job["schema"] != "hiroute.windows-shell-update-job/v1"
                || !same_artifact(&previous, release)
                || super::super::install_transaction::hash(&path.join("installer.exe"))
                    .map_err(UpdateFailure::io("admit-previous-installer"))?
                    != release.sha256
                || Some(
                    super::super::install_transaction::hash(&path.join("finisher.exe"))
                        .map_err(UpdateFailure::io("admit-previous-helper"))?
                        .as_str(),
                ) != job["helper_sha256"].as_str()
            {
                return Err(invalid());
            }
        }
    }
    let lock = lease(&path)?;
    if exclusive {
        lock.try_lock_exclusive()
            .map_err(UpdateFailure::io_code("lock-update-stage", "UPGRADE_BUSY"))?;
    } else {
        FileExt::try_lock_shared(&lock)
            .map_err(UpdateFailure::io_code("lock-update-stage", "UPGRADE_BUSY"))?;
    }
    let current = security::open_owner_only(&path, true)
        .map_err(UpdateFailure::io("recheck-update-stage"))?;
    if security::file_identity(&current).map_err(UpdateFailure::io("identify-update-stage"))?
        != identity
    {
        return Err(invalid());
    }
    if !marked {
        finisher::write(
            &path.join("stage.json"),
            &Record {
                schema: SCHEMA.into(),
                release: release.clone(),
            },
        )?;
    }
    Ok(Stage {
        path,
        _lease: lock,
        _update: None,
    })
}
pub(super) fn cleanup(keep: Option<&str>) -> Result<(), UpdateFailure> {
    let data = super::super::install_transaction::data()
        .map_err(UpdateFailure::io("resolve-stage-root"))?;
    let global = finisher::lock(&data.join("update.lock"))?;
    if global.try_lock_exclusive().is_err() {
        return Ok(());
    }
    let _installer = match hiroute_platform::windows::install::desktop_install_guard() {
        Ok(guard) => guard,
        Err(_) => return Ok(()),
    };
    if super::super::install_transaction::has_pending()
        .map_err(UpdateFailure::io("check-stage-recovery"))?
    {
        return Ok(());
    }
    let root = root()?;
    if !root
        .try_exists()
        .map_err(UpdateFailure::io("check-stage-root"))?
    {
        return Ok(());
    }
    cleanup_in(&root, keep)
}
pub(super) fn discard(release: &WindowsDesktopRelease) -> Result<(), UpdateFailure> {
    release.revision_prefix()?;
    let data = super::super::install_transaction::data()
        .map_err(UpdateFailure::io("resolve-stage-root"))?;
    let global = finisher::lock(&data.join("update.lock"))?;
    global
        .try_lock_exclusive()
        .map_err(UpdateFailure::io_code("lock-update-stage", "UPGRADE_BUSY"))?;
    let _installer = hiroute_platform::windows::install::desktop_install_guard()
        .map_err(UpdateFailure::io_code("lock-update-stage", "UPGRADE_BUSY"))?;
    if super::super::install_transaction::has_pending()
        .map_err(UpdateFailure::io("check-stage-recovery"))?
    {
        return Err("SHELL_INSTALL_RECOVERY_REQUIRED".into());
    }
    let path = root()?.join(&release.sha256);
    if !path
        .try_exists()
        .map_err(UpdateFailure::io("check-update-stage"))?
    {
        return Ok(());
    }
    retire(&path, &release.sha256, false)
}
pub(super) fn background_cleanup(keep: Option<String>) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        let _ = cleanup(keep.as_deref());
    });
}
fn cleanup_in(root: &Path, keep: Option<&str>) -> Result<(), UpdateFailure> {
    security::open_owner_only(root, true).map_err(UpdateFailure::io("verify-stage-root"))?;
    // Stream this background sweep without hashing payloads or collecting the whole backlog.
    for entry in fs::read_dir(root).map_err(UpdateFailure::io("list-stages"))? {
        let entry = entry.map_err(UpdateFailure::io("list-stages"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let (id, retired) = match name.strip_prefix(".retired-") {
            Some(id) => (id, true),
            None => (name.as_str(), false),
        };
        if !finisher::identifier(id) || (!retired && Some(id) == keep) {
            continue;
        }
        // Unknown, changed and active namespaces remain for support.
        if !retired && !entry.path().join("stage.json").exists() {
            let adopted = (|| -> Result<(), UpdateFailure> {
                let job: serde_json::Value = finisher::read(&entry.path().join("job.json"))?;
                let release: WindowsDesktopRelease =
                    serde_json::from_value(job["release"].clone()).map_err(|_| invalid())?;
                if release.sha256 != id {
                    return Err(invalid());
                }
                let parent = finisher::lock(&entry.path().join("parent.lock"))?;
                parent
                    .try_lock_exclusive()
                    .map_err(UpdateFailure::io_code("lock-update-stage", "UPGRADE_BUSY"))?;
                drop(open_in(root, &release, true)?);
                Ok(())
            })();
            if adopted.is_err() {
                continue;
            }
        }
        let _ = retire(&entry.path(), id, retired);
    }
    Ok(())
}
fn retire(path: &Path, id: &str, retired: bool) -> Result<(), UpdateFailure> {
    let held =
        security::open_owner_only(path, true).map_err(UpdateFailure::io("verify-retired-stage"))?;
    let names = entries(path)?;
    if names.is_empty() && retired {
        drop(held);
        return fs::remove_dir(path).map_err(UpdateFailure::io("remove-empty-stage"));
    }
    let record: Record = finisher::read(&path.join("stage.json"))?;
    if record.schema != SCHEMA || record.release.sha256 != id {
        return Err(invalid());
    }
    record.release.revision_prefix()?;
    let lock = lease(path)?;
    lock.try_lock_exclusive()
        .map_err(UpdateFailure::io_code("lock-update-stage", "UPGRADE_BUSY"))?;
    let parent = if names.iter().any(|n| n == "parent.lock") {
        let file = finisher::lock(&path.join("parent.lock"))?;
        file.try_lock_exclusive()
            .map_err(UpdateFailure::io_code("lock-update-stage", "UPGRADE_BUSY"))?;
        Some(file)
    } else {
        None
    };
    // The global update lock excludes released helpers, and our exclusive
    // stage lease excludes current readers. Do not retain a deny-delete parent
    // file handle across the directory rename.
    drop(parent);
    let target = path
        .parent()
        .ok_or_else(invalid)?
        .join(format!(".retired-{id}"));
    if !retired {
        if target
            .try_exists()
            .map_err(UpdateFailure::io("check-retired-stage"))?
        {
            return Ok(());
        }
        fs::rename(path, &target).map_err(UpdateFailure::io("retire-stage"))?;
    }
    drop(lock);
    drop(held);
    // The marker is last so an interrupted removal remains attributable.
    for name in FILES.iter().filter(|name| **name != "stage.json") {
        if names.iter().any(|entry| entry == name) {
            fs::remove_file(target.join(name)).map_err(UpdateFailure::io("remove-stage-file"))?;
        }
    }
    fs::remove_file(target.join("stage.json")).map_err(UpdateFailure::io("remove-stage-record"))?;
    fs::remove_dir(target).map_err(UpdateFailure::io("remove-stage-directory"))
}
#[cfg(test)]
mod tests;
