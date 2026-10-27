//! Completed backups belong to one migration batch, including after manual old-data recovery.
use super::*;

pub(super) fn admit_current(
    backup_root: &Path,
    live_root: &Path,
    binding: Option<&MigrationSecretBinding>,
) -> Result<bool, LocalStorageError> {
    let Some(sources) = BackupSet::completed_current_sources(backup_root, LATEST_SCHEMA_VERSION)?
    else {
        return Ok(false);
    };
    let binding = binding.ok_or(LocalStorageError::Locked)?;
    if sources[2].store_uuid != binding.store_uuid
        || sources[2].key_id.as_deref() != Some(binding.key_id.as_str())
    {
        return Err(LocalStorageError::InvalidData);
    }
    for (index, path) in live_database_paths(live_root).iter().enumerate() {
        validate_owner_file(path)?;
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let version: u32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let sql = if index == 2 {
            "SELECT store_uuid FROM secret_store_meta WHERE singleton=1"
        } else {
            "SELECT store_uuid FROM storage_meta WHERE singleton=1"
        };
        let identity: String = db.query_row(sql, [], |row| row.get(0))?;
        if version != LATEST_SCHEMA_VERSION || identity != sources[index].store_uuid {
            return Err(LocalStorageError::InvalidData);
        }
    }
    // The caller still opens all three stores and validates the complete current
    // dependency closure. This neither audits old content nor authorizes Restore.
    Ok(true)
}

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
    hiroute_platform::durable::rename(backup_root, &archived)?;
    hiroute_platform::durable::directory_barrier(parent)?;
    Ok(())
}
