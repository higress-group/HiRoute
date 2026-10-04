//! Completed backups belong to one migration batch, including after manual old-data recovery.
use super::*;

pub(super) fn archive_if_needed(
    previous: &BackupSet,
    paths: &[PathBuf; 3],
    backup_root: &Path,
    secret_binding: Option<&MigrationSecretBinding>,
) -> Result<(), LocalStorageError> {
    if previous.phase() != BackupSetPhase::Completed
        || previous.target_schema_version() > LATEST_SCHEMA_VERSION
    {
        return Ok(());
    }
    let versions = [
        database_version(&paths[0])?,
        database_version(&paths[1])?,
        database_version(&paths[2])?,
    ];
    if previous.target_schema_version() == LATEST_SCHEMA_VERSION {
        return Ok(());
    }
    if versions
        .iter()
        .any(|v| *v != Some(previous.target_schema_version()))
    {
        return Err(LocalStorageError::InvalidData);
    }
    let binding = secret_binding.ok_or(LocalStorageError::Locked)?;
    if previous.secrets.manifest().key_id.as_deref() != Some(binding.key_id.as_str()) {
        return Err(LocalStorageError::InvalidData);
    }
    validate_live_set(
        previous,
        paths,
        &target_store_uuid(previous, DatabaseKind::Control),
        &target_store_uuid(previous, DatabaseKind::Runtime),
        binding,
    )?;
    let parent = backup_root.parent().ok_or(LocalStorageError::InvalidData)?;
    let archived = parent.join(format!("migration-set.{}", previous.set_id()));
    match fs::symlink_metadata(&archived) {
        Ok(_) => return Err(LocalStorageError::InvalidData),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    crate::backup::source_snapshot::archive_guide(backup_root, &archived, previous.set_id())?;
    fs::rename(backup_root, &archived)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}
