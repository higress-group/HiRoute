//! A stopped historical directory changes namespace, never representation.
use super::*;

fn identity(path: &Path) -> Result<[u64; 2], LocalStorageError> {
    // Keep ancestor/type admission with the existing held-directory adapter.
    let _directory = hiroute_diagnostics::files::PrivateDir::open_existing(path)
        .map_err(|_| LocalStorageError::Permission)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let file = rustix::fs::open(
            path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(std::io::Error::from)?;
        let file = fs::File::from(file);
        let meta = file.metadata()?;
        Ok([meta.dev(), meta.ino()])
    }
    #[cfg(windows)]
    {
        let file = hiroute_platform::windows::security::open_identity(path, true)?;
        let (volume, index) = hiroute_platform::windows::security::file_identity(&file)?;
        Ok([u64::from(volume), index])
    }
}

pub(super) fn capture(root: &Path, destination: &Path) -> Result<Value, LocalStorageError> {
    let source = identity(&root.join("observation"))?;
    if source[0] != identity(destination)?[0] {
        return Err(LocalStorageError::InvalidData.at_upgrade_stage(
            "Old Observation must be sealed on the same filesystem; no full-copy fallback is allowed",
        ).with_recovery(hiroute_host_runtime::StorageRecoveryCause::NamespaceUnavailable,
            hiroute_host_runtime::StorageRecoveryAction::PreserveAndDiagnose));
    }
    Ok(json!({"identity":source,"contents":"preserved_without_recursive_verification"}))
}

/// Check only held directory identities; never enumerate historical contents.
pub(super) fn preflight(root: &Path, destination_parent: &Path) -> Result<(), LocalStorageError> {
    capture(root, destination_parent).map(|_| ())
}

pub(super) fn verify_directory(path: &Path, value: &Value) -> Result<(), LocalStorageError> {
    let seal = &value["sealed_observation"];
    if seal.as_object().is_none_or(|o| o.len() != 2)
        || seal["contents"] != "preserved_without_recursive_verification"
        || json!(identity(path)?) != seal["identity"]
    {
        return Err(LocalStorageError::InvalidData.at_upgrade_stage(
            "Sealed Observation is missing or its directory identity changed; preserve all generations and use the original recovery guide",
        ));
    }
    Ok(())
}

pub(super) fn finish(
    root: &Path,
    backup: &Path,
    stage: &Path,
    metadata: &Value,
) -> Result<(), LocalStorageError> {
    let _stage = identity(stage)?;
    let value = read_manifest(stage)?;
    if value["schema"] != "hiroute.upgrade-source-backup/v2"
        || value["source_storage"] != root.to_string_lossy().as_ref()
        || value["source_info"]["batch_id"] != metadata["batch_id"]
    {
        return Err(LocalStorageError::InvalidData);
    }
    // This scans/checks only the copied recovery materials. The old history is
    // not yet present here, or is deliberately excluded after an interrupted move.
    verify_files(stage, &value)?;
    let live = root.join("observation");
    let sealed = stage.join("storage/observation");
    if sealed.exists() {
        if live.exists() {
            return Err(LocalStorageError::InvalidData.at_upgrade_stage(
                "Both live and sealed Observation exist during cutover; preserve both and use the original recovery guide",
            ));
        }
        verify_directory(&sealed, &value)?;
    } else {
        verify_directory(&live, &value)?;
        if identity(&stage.join("storage"))?[0] != identity(&live)?[0] {
            return Err(LocalStorageError::InvalidData);
        }
        checkpoint("prepared")?;
        hiroute_platform::durable::rename(&live, &sealed)?;
        hiroute_platform::durable::directory_barrier(root)?;
        checkpoint("renamed")?;
        verify_directory(&sealed, &value)?;
    }
    hiroute_platform::durable::directory_barrier(root)?;
    hiroute_platform::durable::rename(stage, backup.join("source"))?;
    hiroute_platform::durable::directory_barrier(backup)?;
    checkpoint("published")?;
    publish_locator(
        backup,
        value["source_info"]["batch_id"]
            .as_str()
            .ok_or(LocalStorageError::InvalidData)?,
    )
}

fn checkpoint(_phase: &str) -> Result<(), LocalStorageError> {
    #[cfg(test)]
    if FAULT.with(|fault| fault.get() == Some(_phase)) {
        return Err(std::io::Error::other("interrupted sealed-history publication").into());
    }
    Ok(())
}

#[cfg(test)]
thread_local! {
    pub(crate) static FAULT: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
mod tests;
