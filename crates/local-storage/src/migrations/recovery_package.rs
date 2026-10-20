//! Reuse the desktop installer's existing retained previous-App handoff. A
//! headless owner can explicitly supply the old package; missing proof stops an
//! incompatible Observation upgrade before either store family is changed.
use super::*;
use hiroute_host_runtime::{DesktopPackageIdentity, read_desktop_package_identity};
fn identity(
    value: &serde_json::Value,
    path: &Path,
) -> Result<DesktopPackageIdentity, LocalStorageError> {
    let mut value: DesktopPackageIdentity =
        serde_json::from_value(value.clone()).map_err(|_| LocalStorageError::InvalidData)?;
    value.app_path = path.to_path_buf();
    Ok(value)
}
pub(super) fn previous_desktop(storage: &Path) -> Result<Option<PathBuf>, LocalStorageError> {
    let executable = std::env::current_exe()?;
    let Some(current) = executable
        .ancestors()
        .find(|p| p.extension().is_some_and(|ext| ext == "app"))
    else {
        return match hiroute_host_runtime::RetainedRecoveryPackage::read(storage)? {
            Some(package) => {
                package.verify(&executable)?;
                Ok(Some(package.package_root))
            }
            None => Ok(None),
        };
    };
    let data = storage.parent().ok_or(LocalStorageError::InvalidData)?;
    let directory = data.join("updates");
    if !directory.exists() {
        return Ok(None);
    }
    let actual =
        read_desktop_package_identity(current).map_err(|_| LocalStorageError::InvalidData)?;
    for (i, entry) in fs::read_dir(directory)?.enumerate() {
        if i >= 64 {
            return Err(LocalStorageError::InvalidData);
        }
        let entry = entry?;
        if !entry.file_name().to_string_lossy().starts_with("install-") {
            continue;
        }
        let path = entry.path().join("handoff.json");
        if !path.exists() || !entry.path().join("installed.json").exists() {
            continue;
        }
        validate_owner_file(&path)?;
        if fs::metadata(&path)?.len() > 1024 * 1024 {
            return Err(LocalStorageError::InvalidData);
        }
        let record: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)
            .map_err(|_| LocalStorageError::InvalidData)?;
        if record["schema"] != "hiroute.desktop-upgrade-handoff/v1"
            || record["data_root"].as_str() != data.to_str()
            || record["current"].as_str() != current.to_str()
        {
            continue;
        }
        if identity(&record["new_identity"], current)? != actual {
            continue;
        }
        let previous = PathBuf::from(
            record["previous"]
                .as_str()
                .ok_or(LocalStorageError::InvalidData)?,
        );
        if previous.parent() != current.parent()
            || !previous
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(".HiRoute-previous-"))
        {
            return Err(LocalStorageError::InvalidData);
        }
        let old =
            read_desktop_package_identity(&previous).map_err(|_| LocalStorageError::InvalidData)?;
        if identity(&record["old_identity"], &previous)? != old {
            return Err(LocalStorageError::InvalidData);
        }
        return Ok(Some(previous));
    }
    Ok(None)
}
