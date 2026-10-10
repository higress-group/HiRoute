use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};

use crate::backup::{
    BackupSet, BackupSetPhase, SingleWriterBackupBarrier, SqliteBackup, database_state_digest,
};
use crate::{DaemonStorageAuthority, LocalStorageError};
use hiroute_domain::CanonicalDigest;

mod agent_access_grant_v9;
mod agent_surface_checks_v22;
mod agent_surface_checks_v25;
#[cfg(test)]
mod agent_surface_checks_v25_tests;
mod current_storage;
mod startup_format;
pub(crate) use startup_format::validate_startup_format;
#[cfg(test)]
mod stopped_read_tests;
pub(crate) use current_storage::{
    acquire_startup_lock, validate_current_storage, validate_recovered_grant_publications,
};
mod collaboration_dependency;
mod completed_batches;
mod compute_v8;
mod convergence_v15;
mod convergence_v17;
mod decision_services_v28;
mod delegation_native_lifecycle_v21;
mod delegation_v10;
#[cfg(test)]
mod integration_v17_tests;
mod plan_authoring_v10;
mod source_prices_v10;
mod subscription_v16;
#[cfg(test)]
mod upgrade_batch_recovery_tests;
mod worker_dependencies_v20;
mod worker_dependencies_v24;
#[cfg(test)]
mod worker_dependencies_v24_tests;
mod worker_dependencies_v26;
mod worker_dependencies_v27;
mod worker_instance_v19;

#[cfg(test)]
mod convergence_tests;

/// Current stable storage format, including native ACP Worker dependency selections.
/// Production source admission is defined in `startup_format`; supported upgrades retain the
/// existing durable three-store backup and recovery coordinator.
pub const LATEST_SCHEMA_VERSION: u32 = 29;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DatabaseKind {
    Control,
    Runtime,
    Secrets,
}

pub(crate) fn open_database(
    authority: &DaemonStorageAuthority,
    path: impl AsRef<Path>,
    kind: DatabaseKind,
    migration_backup_root: impl AsRef<Path>,
) -> Result<Connection, LocalStorageError> {
    open_database_internal(
        authority,
        path.as_ref(),
        kind,
        migration_backup_root.as_ref(),
        None,
        None,
        None,
        None,
    )
}

pub(crate) fn open_database_with_key_id(
    _authority: &DaemonStorageAuthority,
    path: impl AsRef<Path>,
    kind: DatabaseKind,
    migration_backup_root: impl AsRef<Path>,
    key_id: Option<&CanonicalDigest>,
) -> Result<Connection, LocalStorageError> {
    open_database_internal(
        _authority,
        path.as_ref(),
        kind,
        migration_backup_root.as_ref(),
        key_id,
        None,
        None,
        None,
    )
}

#[derive(Clone, Debug)]
pub(crate) struct MigrationSecretBinding {
    pub(crate) store_uuid: String,
    pub(crate) key_id: CanonicalDigest,
    pub(crate) key_verifier: CanonicalDigest,
}

pub(crate) fn open_database_from_set(
    authority: &DaemonStorageAuthority,
    path: &Path,
    kind: DatabaseKind,
    migration_backup_root: &Path,
    expected_store_uuid: Option<&str>,
    secret_binding: Option<&MigrationSecretBinding>,
    target_schema_version: u32,
) -> Result<Connection, LocalStorageError> {
    open_database_internal(
        authority,
        path,
        kind,
        migration_backup_root,
        secret_binding.map(|binding| &binding.key_id),
        secret_binding,
        expected_store_uuid,
        Some(target_schema_version),
    )
}

#[allow(clippy::too_many_arguments)]
fn open_database_internal(
    _authority: &DaemonStorageAuthority,
    path: &Path,
    kind: DatabaseKind,
    migration_backup_root: &Path,
    key_id: Option<&CanonicalDigest>,
    secret_binding: Option<&MigrationSecretBinding>,
    expected_store_uuid: Option<&str>,
    durable_set_target: Option<u32>,
) -> Result<Connection, LocalStorageError> {
    let parent = path.parent().ok_or(LocalStorageError::InvalidData)?;
    prepare_owner_directory(parent)?;
    let existed = path.exists();
    if existed {
        validate_owner_file(path)?;
    }
    let mut connection = Connection::open(path)?;
    if !existed {
        set_owner_file_permissions(path)?;
    }
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = FULL;",
    )?;
    migrate(
        &mut connection,
        path,
        kind,
        migration_backup_root,
        key_id,
        secret_binding,
        durable_set_target,
    )?;
    if kind != DatabaseKind::Secrets {
        initialize_storage_meta(&connection, expected_store_uuid)?;
    }
    Ok(connection)
}

fn initialize_storage_meta(
    connection: &Connection,
    expected_store_uuid: Option<&str>,
) -> Result<(), LocalStorageError> {
    let present = connection
        .query_row(
            "SELECT store_uuid FROM storage_meta WHERE singleton = 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if let Some(present) = present {
        if expected_store_uuid.is_some_and(|expected| expected != present) {
            return Err(LocalStorageError::InvalidData);
        }
        return Ok(());
    }
    let uuid = if let Some(expected) = expected_store_uuid {
        expected.to_owned()
    } else {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|_| LocalStorageError::Crypto)?;
        random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    connection.execute(
        "INSERT INTO storage_meta(singleton, store_uuid, key_id, created_at)
         VALUES (1, ?1, NULL, unixepoch())",
        params![uuid],
    )?;
    Ok(())
}

fn prepare_owner_directory(path: &Path) -> Result<(), LocalStorageError> {
    if path.exists() {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
            return Err(LocalStorageError::Permission);
        }
    } else {
        fs::create_dir_all(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

pub(crate) fn prepare_storage_root(path: &Path) -> Result<(), LocalStorageError> {
    prepare_owner_directory(path)
}

pub(crate) fn validate_owner_file(path: &Path) -> Result<(), LocalStorageError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(LocalStorageError::Permission);
    }
    Ok(())
}

fn set_owner_file_permissions(path: &Path) -> Result<(), LocalStorageError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    validate_owner_file(path)
}

fn migrate(
    connection: &mut Connection,
    path: &Path,
    kind: DatabaseKind,
    backup_root: &Path,
    key_id: Option<&CanonicalDigest>,
    secret_binding: Option<&MigrationSecretBinding>,
    durable_set_target: Option<u32>,
) -> Result<(), LocalStorageError> {
    let target = durable_set_target.unwrap_or(LATEST_SCHEMA_VERSION);
    let current: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if target > LATEST_SCHEMA_VERSION || current > target {
        return Err(LocalStorageError::InvalidData);
    }
    if current > 0 && current < target && durable_set_target.is_none() {
        let backup_path = migration_backup_path(path, backup_root, current)?;
        // `create_bound` reuses only the exact source identity/key/watermark manifest. A stale or
        // half-published final is preserved and replaced by a unique fresh attempt.
        SqliteBackup::create_bound(connection, &backup_path, key_id)?;
    }

    for version in (current + 1)..=target {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(migration_sql(kind, version)?)?;
        if kind == DatabaseKind::Control && version == 8 {
            compute_v8::migrate_semantic_identities(&transaction)?;
        }
        if kind == DatabaseKind::Secrets && version == 15 {
            convergence_v15::ensure_collaboration_preparation_column(&transaction)?;
        }
        if version == 16 {
            convergence_v17::converge_worker_authorization_format(&transaction)?;
        }
        if version == 17 {
            // One experimental v16 branch used this version number for subscriptions and never
            // created the Worker marker. Reconcile both shapes before publishing v17.
            convergence_v17::converge_worker_authorization_format(&transaction)?;
            if kind == DatabaseKind::Control {
                convergence_v17::converge_subscription_validation(&transaction)?;
            }
        }
        if kind == DatabaseKind::Secrets
            && version == 3
            && let Some(binding) = secret_binding
        {
            transaction.execute(
                "UPDATE secret_store_meta
                 SET store_uuid = ?1, key_id = ?2, key_verifier = ?3,
                     updated_at = unixepoch()
                 WHERE singleton = 1 AND store_uuid = '' AND key_id = '' AND key_verifier = ''",
                params![
                    binding.store_uuid,
                    binding.key_id.as_str(),
                    binding.key_verifier.as_str()
                ],
            )?;
        }
        transaction.pragma_update(None, "user_version", version)?;
        transaction.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at, binary_version)
             VALUES (?1, unixepoch(), ?2)",
            params![version, env!("CARGO_PKG_VERSION")],
        )?;
        transaction.commit()?;
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
    convergence_v17::validate_current(&transaction, kind)?;
    transaction.commit()?;
    Ok(())
}

fn migration_backup_path(
    database_path: &Path,
    backup_root: &Path,
    version: u32,
) -> Result<PathBuf, LocalStorageError> {
    let name = database_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(LocalStorageError::InvalidData)?;
    Ok(backup_root.join(format!("{name}.before-v{}.db", version + 1)))
}

pub(crate) struct MigrationSetCoordinator {
    set: Option<BackupSet>,
    control_store_uuid: Option<String>,
    runtime_store_uuid: Option<String>,
}

impl MigrationSetCoordinator {
    pub(crate) fn prepare(
        authority: &DaemonStorageAuthority,
        barrier: &SingleWriterBackupBarrier,
        live_root: &Path,
        backup_root: &Path,
        secret_binding: Option<&MigrationSecretBinding>,
    ) -> Result<Self, LocalStorageError> {
        if BackupSet::durable_manifest_present(backup_root)? {
            let previous = BackupSet::open(authority, backup_root)?;
            completed_batches::archive_if_needed(
                &previous,
                &live_database_paths(live_root),
                backup_root,
                secret_binding,
            )?;
        }
        let durable_present = BackupSet::durable_manifest_present(backup_root)?;
        let mut durable_set = if durable_present {
            Some(BackupSet::open(authority, backup_root)?)
        } else {
            None
        };
        if let Some(set) = durable_set.as_mut()
            && set.phase() != BackupSetPhase::Completed
            && backup_root.join("conversion-in-progress").exists()
        {
            crate::backup::source_snapshot::verify_unchanged(
                live_root.parent().ok_or(LocalStorageError::InvalidData)?,
                backup_root,
            )?;
            set.restore_over_live_root(barrier, live_root)?;
            return Err(LocalStorageError::InvalidData);
        }
        // Reconcile the directory swap before creating or inspecting `live`. In the
        // after-live-rename crash state that path is intentionally absent; recreating an empty
        // directory here would hide the durable restore-ready generation and make recovery fail.
        if let Some(set) = durable_set.as_mut() {
            if set.phase() == BackupSetPhase::Restoring {
                set.restore_over_live_root(barrier, live_root)?;
                return Err(LocalStorageError::InvalidData);
            }
            if set.phase() == BackupSetPhase::Restored {
                return Err(LocalStorageError::InvalidData);
            }
        }
        if !live_root.exists() && durable_present {
            return Err(LocalStorageError::InvalidData);
        }
        prepare_owner_directory(live_root)?;
        let paths = live_database_paths(live_root);
        let versions = [
            database_version(&paths[0])?,
            database_version(&paths[1])?,
            database_version(&paths[2])?,
        ];
        if versions.iter().all(Option::is_none) {
            if durable_present {
                return Err(LocalStorageError::InvalidData);
            }
            return Ok(Self {
                set: None,
                control_store_uuid: None,
                runtime_store_uuid: None,
            });
        }
        if versions.iter().any(Option::is_none) {
            return Err(LocalStorageError::InvalidData);
        }
        let versions = versions.map(|version| version.expect("presence checked"));
        if versions
            .iter()
            .any(|version| *version > LATEST_SCHEMA_VERSION)
        {
            return Err(LocalStorageError::InvalidData);
        }
        if !durable_present
            && versions
                .iter()
                .all(|version| *version == LATEST_SCHEMA_VERSION)
        {
            return Ok(Self {
                set: None,
                control_store_uuid: None,
                runtime_store_uuid: None,
            });
        }
        let secret_binding = secret_binding.ok_or(LocalStorageError::Locked)?;
        let set = if let Some(set) = durable_set {
            set
        } else {
            if versions[0] == 0 || versions.iter().any(|version| *version != versions[0]) {
                // A partially upgraded set without a durable source manifest has no exact group
                // recovery point and must not be adopted.
                return Err(LocalStorageError::InvalidData);
            }
            let control = open_existing_for_snapshot(&paths[0])?;
            let runtime = open_existing_for_snapshot(&paths[1])?;
            let secrets = open_existing_for_snapshot(&paths[2])?;
            BackupSet::create_migration(
                barrier,
                backup_root,
                &control,
                &runtime,
                &secrets,
                &secret_binding.key_id,
                LATEST_SCHEMA_VERSION,
            )?
        };
        if !matches!(
            set.target_schema_version(),
            24 | 25 | 26 | LATEST_SCHEMA_VERSION
        ) || (set.target_schema_version() == 24
            && [
                set.control.manifest(),
                set.runtime.manifest(),
                set.secrets.manifest(),
            ]
            .iter()
            .any(|source| source.schema_version != 23))
            || set.secrets.manifest().key_id.as_deref() != Some(secret_binding.key_id.as_str())
        {
            return Err(LocalStorageError::InvalidData);
        }
        let control_store_uuid = target_store_uuid(&set, DatabaseKind::Control);
        let runtime_store_uuid = target_store_uuid(&set, DatabaseKind::Runtime);
        validate_live_set(
            &set,
            &paths,
            &control_store_uuid,
            &runtime_store_uuid,
            secret_binding,
        )?;
        Ok(Self {
            set: Some(set),
            control_store_uuid: Some(control_store_uuid),
            runtime_store_uuid: Some(runtime_store_uuid),
        })
    }

    pub(crate) fn durable_set_prepared(&self) -> bool {
        self.set.is_some()
    }

    pub(crate) fn target_schema_version(&self) -> u32 {
        self.set
            .as_ref()
            .map_or(LATEST_SCHEMA_VERSION, BackupSet::target_schema_version)
    }

    pub(crate) fn expected_control_store_uuid(&self) -> Option<&str> {
        self.control_store_uuid.as_deref()
    }

    pub(crate) fn expected_runtime_store_uuid(&self) -> Option<&str> {
        self.runtime_store_uuid.as_deref()
    }

    pub(crate) fn mark_control_migrated(
        &mut self,
        barrier: &SingleWriterBackupBarrier,
    ) -> Result<(), LocalStorageError> {
        self.advance_to(barrier, BackupSetPhase::ControlMigrated)
    }

    pub(crate) fn mark_runtime_migrated(
        &mut self,
        barrier: &SingleWriterBackupBarrier,
    ) -> Result<(), LocalStorageError> {
        self.advance_to(barrier, BackupSetPhase::RuntimeMigrated)
    }

    pub(crate) fn mark_secrets_migrated(
        &mut self,
        barrier: &SingleWriterBackupBarrier,
    ) -> Result<(), LocalStorageError> {
        self.advance_to(barrier, BackupSetPhase::SecretsMigrated)
    }

    pub(crate) fn mark_completed(
        &mut self,
        barrier: &SingleWriterBackupBarrier,
    ) -> Result<(), LocalStorageError> {
        self.advance_to(barrier, BackupSetPhase::Completed)
    }

    pub(crate) fn restore_after_failure(
        &mut self,
        barrier: &SingleWriterBackupBarrier,
        live_root: &Path,
    ) -> Result<(), LocalStorageError> {
        let Some(set) = self.set.as_mut() else {
            return Ok(());
        };
        if set.phase() == BackupSetPhase::Completed {
            return Err(LocalStorageError::InvalidData);
        }
        if set.directory().join("source/source-backup.json").exists() {
            crate::backup::source_snapshot::verify_unchanged(
                live_root.parent().ok_or(LocalStorageError::InvalidData)?,
                set.directory(),
            )?;
        }
        set.restore_over_live_root(barrier, live_root)
    }

    fn advance_to(
        &mut self,
        barrier: &SingleWriterBackupBarrier,
        target: BackupSetPhase,
    ) -> Result<(), LocalStorageError> {
        let Some(set) = self.set.as_mut() else {
            return Ok(());
        };
        if phase_rank(set.phase()) >= phase_rank(target)
            && phase_rank(set.phase()) <= phase_rank(BackupSetPhase::Completed)
        {
            return Ok(());
        }
        while phase_rank(set.phase()) < phase_rank(target) {
            let next = match set.phase() {
                BackupSetPhase::SourcePublished => BackupSetPhase::ControlMigrated,
                BackupSetPhase::ControlMigrated => BackupSetPhase::RuntimeMigrated,
                BackupSetPhase::RuntimeMigrated => BackupSetPhase::SecretsMigrated,
                BackupSetPhase::SecretsMigrated => BackupSetPhase::Completed,
                _ => return Err(LocalStorageError::InvalidData),
            };
            set.persist_phase(barrier, next)?;
        }
        (phase_rank(set.phase()) == phase_rank(target))
            .then_some(())
            .ok_or(LocalStorageError::InvalidData)
    }
}

fn live_database_paths(live_root: &Path) -> [PathBuf; 3] {
    [
        live_root.join("control.db"),
        live_root.join("runtime.db"),
        live_root.join("secrets.db"),
    ]
}

fn database_version(path: &Path) -> Result<Option<u32>, LocalStorageError> {
    if !path.exists() {
        return Ok(None);
    }
    validate_owner_file(path)?;
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    Ok(Some(connection.query_row(
        "PRAGMA user_version",
        [],
        |row| row.get(0),
    )?))
}

fn open_existing_for_snapshot(path: &Path) -> Result<Connection, LocalStorageError> {
    validate_owner_file(path)?;
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?)
}

fn target_store_uuid(set: &BackupSet, kind: DatabaseKind) -> String {
    let source = match kind {
        DatabaseKind::Control => set.control.manifest(),
        DatabaseKind::Runtime => set.runtime.manifest(),
        DatabaseKind::Secrets => set.secrets.manifest(),
    };
    if source.store_uuid != "legacy-unbound" && !source.store_uuid.is_empty() {
        return source.store_uuid.clone();
    }
    let digest = CanonicalDigest::of_bytes(
        format!("hiroute.migration-store/v1\0{}\0{kind:?}", set.set_id()).as_bytes(),
    );
    digest.as_str()["sha256:".len()..][..32].to_owned()
}

fn validate_live_set(
    set: &BackupSet,
    paths: &[PathBuf; 3],
    control_target_uuid: &str,
    runtime_target_uuid: &str,
    secret_binding: &MigrationSecretBinding,
) -> Result<(), LocalStorageError> {
    let rank = phase_rank(set.phase());
    let control_target = validate_live_database(
        &paths[0],
        set.control.manifest(),
        DatabaseKind::Control,
        control_target_uuid,
        None,
        set.target_schema_version(),
        rank,
    )?;
    let runtime_target = validate_live_database(
        &paths[1],
        set.runtime.manifest(),
        DatabaseKind::Runtime,
        runtime_target_uuid,
        None,
        set.target_schema_version(),
        rank,
    )?;
    let secrets_target = validate_live_database(
        &paths[2],
        set.secrets.manifest(),
        DatabaseKind::Secrets,
        &secret_binding.store_uuid,
        Some(&secret_binding.key_id),
        set.target_schema_version(),
        rank,
    )?;
    if (rank >= phase_rank(BackupSetPhase::ControlMigrated) && !control_target)
        || (rank >= phase_rank(BackupSetPhase::RuntimeMigrated) && !runtime_target)
        || (rank >= phase_rank(BackupSetPhase::SecretsMigrated) && !secrets_target)
        || (set.phase() == BackupSetPhase::Completed
            && !(control_target && runtime_target && secrets_target))
    {
        return Err(LocalStorageError::InvalidData);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_live_database(
    path: &Path,
    source: &crate::backup::BackupManifest,
    kind: DatabaseKind,
    target_store_uuid: &str,
    target_key_id: Option<&CanonicalDigest>,
    target_schema_version: u32,
    recorded_phase: u8,
) -> Result<bool, LocalStorageError> {
    validate_owner_file(path)?;
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version == source.schema_version {
        if database_state_digest(&connection)? != source.source_state_digest {
            return Err(LocalStorageError::InvalidData);
        }
        return Ok(version == target_schema_version);
    }
    let store_phase = match kind {
        DatabaseKind::Control => 0,
        DatabaseKind::Runtime => 1,
        DatabaseKind::Secrets => 2,
    };
    if recorded_phase < store_phase {
        return Err(LocalStorageError::InvalidData);
    }
    let at_target = version == target_schema_version;
    if !at_target {
        // A store may have committed any contiguous step of the published stable chain.
        // The original source/target, current phase and exact ledger must still agree.
        let first = source.schema_version + 1;
        if recorded_phase != store_phase
            || !(23..=25).contains(&source.schema_version)
            || !(24..=LATEST_SCHEMA_VERSION).contains(&target_schema_version)
            || version <= source.schema_version
            || version >= target_schema_version
            || connection.query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version >= ?1",
                [first],
                |row| row.get::<_, u32>(0),
            )? != version - source.schema_version
        {
            return Err(LocalStorageError::InvalidData);
        }
        for step in first..=version {
            if !connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?1)",
                [step],
                |row| row.get::<_, bool>(0),
            )? {
                return Err(LocalStorageError::InvalidData);
            }
        }
    }

    match kind {
        DatabaseKind::Control | DatabaseKind::Runtime => {
            let binding = connection
                .query_row(
                    "SELECT store_uuid FROM storage_meta WHERE singleton = 1",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            if (source.schema_version >= 23 && binding.as_deref() != Some(target_store_uuid))
                || binding
                    .as_deref()
                    .is_some_and(|binding| !binding.is_empty() && binding != target_store_uuid)
            {
                return Err(LocalStorageError::InvalidData);
            }
        }
        DatabaseKind::Secrets => {
            let binding = connection.query_row(
                "SELECT store_uuid, key_id FROM secret_store_meta WHERE singleton = 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )?;
            if binding.0 != target_store_uuid
                || target_key_id.is_none_or(|key_id| binding.1 != key_id.as_str())
            {
                return Err(LocalStorageError::InvalidData);
            }
        }
    }
    Ok(at_target)
}

fn phase_rank(phase: BackupSetPhase) -> u8 {
    match phase {
        BackupSetPhase::SourcePublished => 0,
        BackupSetPhase::ControlMigrated => 1,
        BackupSetPhase::RuntimeMigrated => 2,
        BackupSetPhase::SecretsMigrated => 3,
        BackupSetPhase::Completed => 4,
        BackupSetPhase::Restoring | BackupSetPhase::Restored => u8::MAX,
    }
}

fn migration_sql(kind: DatabaseKind, version: u32) -> Result<&'static str, LocalStorageError> {
    match (kind, version) {
        (DatabaseKind::Control, 1) => Ok(CONTROL_V1),
        (DatabaseKind::Control, 2) => Ok(CONTROL_V2),
        (DatabaseKind::Control, 3) => Ok(CONTROL_V3),
        (DatabaseKind::Control, 4) => Ok(CONTROL_V4),
        (DatabaseKind::Control, 5) => Ok(CONTROL_V5),
        (DatabaseKind::Control, 6) => Ok(NOOP_V6),
        (DatabaseKind::Control, 7) => Ok(CONTROL_V7),
        (DatabaseKind::Control, 8) => Ok(CONTROL_V8),
        (DatabaseKind::Control, 9) => Ok(NOOP_V9),
        (DatabaseKind::Control, 10) => Ok(source_prices_v10::CONTROL_V10),
        (DatabaseKind::Control, 11) => Ok(plan_authoring_v10::CONTROL),
        (DatabaseKind::Runtime, 1) => Ok(RUNTIME_V1),
        (DatabaseKind::Runtime, 2) => Ok(RUNTIME_V2),
        (DatabaseKind::Runtime, 3) => Ok(RUNTIME_V3),
        (DatabaseKind::Runtime, 4) => Ok(RUNTIME_V4),
        (DatabaseKind::Runtime, 5) => Ok(NOOP_V5),
        (DatabaseKind::Runtime, 6) => Ok(RUNTIME_V6),
        (DatabaseKind::Runtime, 7) => Ok(NOOP_V7),
        (DatabaseKind::Runtime, 8) => Ok(NOOP_V8),
        (DatabaseKind::Runtime, 9) => Ok(NOOP_V9),
        (DatabaseKind::Runtime, 10 | 11) => Ok(NOOP_V9),
        (DatabaseKind::Secrets, 1) => Ok(SECRETS_V1),
        (DatabaseKind::Secrets, 2) => Ok(SECRETS_V2),
        (DatabaseKind::Secrets, 3) => Ok(SECRETS_V3),
        (DatabaseKind::Secrets, 4) => Ok(SECRETS_V4),
        (DatabaseKind::Secrets, 5) => Ok(NOOP_V5),
        (DatabaseKind::Secrets, 6) => Ok(NOOP_V6),
        (DatabaseKind::Secrets, 7) => Ok(NOOP_V7),
        (DatabaseKind::Secrets, 8) => Ok(NOOP_V8),
        (DatabaseKind::Secrets, 9) => Ok(agent_access_grant_v9::SECRETS_V9),
        (DatabaseKind::Secrets, 10 | 11) => Ok(NOOP_V9),
        (DatabaseKind::Control, 12) => Ok(collaboration_dependency::CONTROL),
        (DatabaseKind::Secrets, 12) => Ok(collaboration_dependency::SECRETS),
        (DatabaseKind::Runtime, 12) => Ok(delegation_v10::RUNTIME),
        (DatabaseKind::Control, 13) => Ok(
            "CREATE TABLE agent_skill_installations (workspace_id TEXT NOT NULL, root_ref TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision > 0), record_json TEXT NOT NULL, owner_operation_id TEXT NOT NULL REFERENCES operations(operation_id), PRIMARY KEY(workspace_id, root_ref));",
        ),
        (DatabaseKind::Runtime | DatabaseKind::Secrets, 13) => Ok(NOOP_V9),
        (DatabaseKind::Control, 14 | 15) => Ok(convergence_v15::CONTROL_COMPUTE_MANAGEMENT),
        (DatabaseKind::Secrets, 14) => Ok(convergence_v15::SECRETS_V14_COLLABORATION_PREPARATION),
        (DatabaseKind::Runtime, 14) | (DatabaseKind::Runtime | DatabaseKind::Secrets, 15) => {
            Ok(NOOP_V9)
        }
        (DatabaseKind::Control | DatabaseKind::Runtime | DatabaseKind::Secrets, 16) => {
            Ok(WORKER_AUTHORIZATION_FORMAT_V16)
        }
        (DatabaseKind::Control, 17) => Ok(subscription_v16::CONTROL),
        (DatabaseKind::Runtime | DatabaseKind::Secrets, 17) => Ok(NOOP_V9),
        (DatabaseKind::Runtime, 18) => Ok(RUNTIME_V18),
        (DatabaseKind::Control | DatabaseKind::Secrets, 18) => Ok(NOOP_V9),
        (DatabaseKind::Runtime, 19) => Ok(worker_instance_v19::RUNTIME),
        (DatabaseKind::Control | DatabaseKind::Secrets, 19) => Ok(NOOP_V9),
        (DatabaseKind::Control, 20) => Ok(worker_dependencies_v20::CONTROL),
        (DatabaseKind::Runtime | DatabaseKind::Secrets, 20) => Ok(NOOP_V9),
        (DatabaseKind::Runtime, 21) => Ok(delegation_native_lifecycle_v21::RUNTIME),
        (DatabaseKind::Control | DatabaseKind::Secrets, 21) => Ok(NOOP_V9),
        (DatabaseKind::Control, 22) => Ok(agent_surface_checks_v22::CONTROL),
        (DatabaseKind::Runtime | DatabaseKind::Secrets, 22) => Ok(NOOP_V9),
        (DatabaseKind::Control | DatabaseKind::Runtime | DatabaseKind::Secrets, 23) => {
            Ok(current_storage::MARKER_SQL)
        }
        (DatabaseKind::Control, 24) => Ok(worker_dependencies_v24::CONTROL),
        (DatabaseKind::Runtime | DatabaseKind::Secrets, 24) => Ok(NOOP_V9),
        (DatabaseKind::Control, 25) => Ok(agent_surface_checks_v25::CONTROL),
        (DatabaseKind::Runtime | DatabaseKind::Secrets, 25) => Ok(NOOP_V9),
        (DatabaseKind::Control, 26) => Ok(worker_dependencies_v26::CONTROL),
        (DatabaseKind::Runtime | DatabaseKind::Secrets, 26) => Ok(NOOP_V9),
        (DatabaseKind::Control, 27) => Ok(worker_dependencies_v27::CONTROL),
        (DatabaseKind::Control, 28) => Ok(decision_services_v28::CONTROL),
        (DatabaseKind::Runtime | DatabaseKind::Secrets, 27 | 28) => Ok(NOOP_V9),
        (DatabaseKind::Secrets, 29) => Ok("DROP INDEX secret_owner_fingerprint_idx;
             CREATE UNIQUE INDEX secret_owner_fingerprint_idx
             ON secret_entries(owner_scope, fingerprint) WHERE purpose != 'http-header';"),
        (DatabaseKind::Control | DatabaseKind::Runtime, 29) => Ok(NOOP_V9),
        _ => Err(LocalStorageError::InvalidData),
    }
}

const RUNTIME_V18: &str = r#"
CREATE TABLE worker_settings (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    max_concurrent INTEGER NOT NULL
        CHECK(typeof(max_concurrent) = 'integer' AND max_concurrent BETWEEN 1 AND 1000)
);
"#;

const WORKER_AUTHORIZATION_FORMAT_V16: &str = r#"
CREATE TABLE IF NOT EXISTS worker_authorization_format (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    format_version INTEGER NOT NULL CHECK(format_version = 1)
);
INSERT OR IGNORE INTO worker_authorization_format(singleton, format_version) VALUES(1, 1);
"#;

#[cfg(test)]
pub(crate) fn initialize_control_v7_fixture(
    connection: &Connection,
) -> Result<(), LocalStorageError> {
    for version in 1..=7 {
        connection.execute_batch(migration_sql(DatabaseKind::Control, version)?)?;
    }
    connection.pragma_update(None, "user_version", 7)?;
    Ok(())
}

const CONTROL_V1: &str = r#"
CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at INTEGER NOT NULL,
    binary_version TEXT NOT NULL
);
CREATE TABLE workspace_state (
    workspace_id TEXT PRIMARY KEY,
    desired_json TEXT NOT NULL,
    target_revision INTEGER NOT NULL CHECK(target_revision >= 0),
    desired_digest TEXT NOT NULL,
    owner_operation_id TEXT,
    updated_at INTEGER NOT NULL
);
CREATE TABLE dependency_revisions (
    workspace_id TEXT NOT NULL,
    dependency_key TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision >= 0),
    PRIMARY KEY(workspace_id, dependency_key)
);
CREATE TABLE operations (
    operation_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    principal TEXT NOT NULL,
    operation_kind TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    request_digest TEXT NOT NULL,
    accepted_change_digest TEXT NOT NULL,
    state TEXT NOT NULL,
    generation INTEGER NOT NULL,
    operation_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(workspace_id, principal, operation_kind, idempotency_key)
);
CREATE TABLE operation_steps (
    operation_id TEXT NOT NULL REFERENCES operations(operation_id) ON DELETE CASCADE,
    step_no INTEGER NOT NULL,
    step_kind TEXT NOT NULL,
    state TEXT NOT NULL,
    step_json TEXT NOT NULL,
    PRIMARY KEY(operation_id, step_no)
);
CREATE TABLE control_effects (
    operation_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    before_exists INTEGER NOT NULL,
    before_json TEXT,
    before_revision INTEGER NOT NULL,
    before_digest TEXT,
    before_owner_operation_id TEXT,
    after_revision INTEGER NOT NULL,
    after_digest TEXT NOT NULL,
    compensated INTEGER NOT NULL DEFAULT 0
);
"#;

const CONTROL_V2: &str = r#"
CREATE INDEX operations_recovery_idx ON operations(state, updated_at);
CREATE INDEX operation_steps_state_idx ON operation_steps(operation_id, state);
"#;

const CONTROL_V3: &str = r#"
CREATE TABLE storage_meta (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    store_uuid TEXT NOT NULL,
    key_id TEXT,
    created_at INTEGER NOT NULL
);
CREATE TABLE apply_capabilities (
    capability_digest TEXT PRIMARY KEY,
    principal TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    operation_kind TEXT NOT NULL,
    accepted_digest TEXT NOT NULL,
    expected_revisions_digest TEXT NOT NULL,
    capability_scope TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked INTEGER NOT NULL DEFAULT 0,
    consumed_operation_id TEXT
);
CREATE TABLE writer_claim (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    operation_id TEXT NOT NULL UNIQUE,
    admitted_at INTEGER NOT NULL
);
CREATE TABLE workspace_revision_heads (
    workspace_id TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK(revision >= 0)
);
INSERT INTO workspace_revision_heads(workspace_id, revision)
    SELECT workspace_id, target_revision FROM workspace_state;
ALTER TABLE control_effects ADD COLUMN staged_json TEXT;
ALTER TABLE control_effects ADD COLUMN activated INTEGER NOT NULL DEFAULT 1;
CREATE INDEX apply_capabilities_consumption_idx
    ON apply_capabilities(consumed_operation_id, revoked, expires_at);
"#;

const CONTROL_V4: &str = r#"
ALTER TABLE control_effects ADD COLUMN compute_pool_id TEXT;
ALTER TABLE control_effects ADD COLUMN compute_pool_expected_revision INTEGER;
ALTER TABLE control_effects ADD COLUMN compute_pool_before_json TEXT;
ALTER TABLE control_effects ADD COLUMN compute_pool_after_json TEXT;
CREATE TABLE compute_sources (
    source_id TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK(revision > 0),
    identity_digest TEXT NOT NULL,
    source_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE source_bindings (
    binding_id TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK(revision > 0),
    source_id TEXT NOT NULL,
    binding_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX source_bindings_source_idx ON source_bindings(source_id, binding_id);
CREATE TABLE source_inventory_snapshots (
    source_id TEXT NOT NULL,
    endpoint_profile_id TEXT NOT NULL,
    inventory_revision INTEGER NOT NULL CHECK(inventory_revision > 0),
    inventory_digest TEXT NOT NULL,
    observed_models_json TEXT NOT NULL,
    captured_at INTEGER NOT NULL,
    PRIMARY KEY(source_id, endpoint_profile_id)
);
CREATE TABLE credential_pools (
    pool_id TEXT PRIMARY KEY,
    binding_id TEXT NOT NULL,
    binding_revision INTEGER NOT NULL CHECK(binding_revision > 0),
    binding_digest TEXT NOT NULL,
    source_id TEXT NOT NULL,
    source_revision INTEGER NOT NULL CHECK(source_revision > 0),
    connection_option_id TEXT NOT NULL,
    offer_ref TEXT NOT NULL,
    offer_revision INTEGER NOT NULL CHECK(offer_revision > 0),
    offer_evidence_digest TEXT NOT NULL,
    billing_class TEXT NOT NULL,
    model_configuration_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    homogeneous_identity_digest TEXT NOT NULL,
    authentication_kind TEXT NOT NULL,
    pool_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE price_overrides (
    override_id TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK(revision > 0),
    override_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
"#;

const CONTROL_V5: &str = r#"
CREATE TABLE gateway_publications (
    workspace_id TEXT NOT NULL,
    publication_revision INTEGER NOT NULL CHECK(publication_revision > 0),
    digest TEXT NOT NULL,
    publication_bytes BLOB NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('prepared', 'active', 'lkg', 'historical')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(workspace_id, publication_revision)
);
CREATE UNIQUE INDEX gateway_publications_one_prepared
    ON gateway_publications(workspace_id) WHERE state = 'prepared';
CREATE UNIQUE INDEX gateway_publications_one_active
    ON gateway_publications(workspace_id) WHERE state = 'active';
CREATE UNIQUE INDEX gateway_publications_one_lkg
    ON gateway_publications(workspace_id) WHERE state = 'lkg';
CREATE TABLE publication_heads (
    workspace_id TEXT PRIMARY KEY,
    active_revision INTEGER,
    lkg_revision INTEGER,
    generation INTEGER NOT NULL CHECK(generation >= 0),
    updated_at INTEGER NOT NULL
);
"#;

const CONTROL_V7: &str = r#"
ALTER TABLE control_effects ADD COLUMN compute_source_id TEXT;
ALTER TABLE control_effects ADD COLUMN compute_source_expected_revision INTEGER;
ALTER TABLE control_effects ADD COLUMN compute_source_before_json TEXT;
ALTER TABLE control_effects ADD COLUMN compute_source_after_json TEXT;
"#;

const CONTROL_V8: &str = r#"
ALTER TABLE source_bindings
    ADD COLUMN active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0, 1));
ALTER TABLE credential_pools
    ADD COLUMN active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0, 1));
CREATE TABLE compute_source_identity_aliases (
    canonical_source_id TEXT PRIMARY KEY,
    physical_source_id TEXT NOT NULL UNIQUE,
    canonical_identity_digest TEXT NOT NULL,
    owner_operation_id TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX source_bindings_active_source_idx
    ON source_bindings(active, source_id, binding_id);
CREATE INDEX credential_pools_active_source_idx
    ON credential_pools(active, source_id, pool_id);
"#;

const NOOP_V5: &str = "SELECT 1;";
const NOOP_V6: &str = "SELECT 1;";
const NOOP_V7: &str = "SELECT 1;";
const NOOP_V8: &str = "SELECT 1;";

const RUNTIME_V1: &str = r#"
CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at INTEGER NOT NULL,
    binary_version TEXT NOT NULL
);
CREATE TABLE runtime_state (
    state_key TEXT PRIMARY KEY,
    value_json TEXT NOT NULL,
    value_digest TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK(generation >= 0),
    owner_operation_id TEXT,
    updated_at INTEGER NOT NULL
);
CREATE TABLE runtime_effects (
    operation_id TEXT NOT NULL,
    state_key TEXT NOT NULL,
    before_exists INTEGER NOT NULL,
    before_json TEXT,
    before_digest TEXT,
    before_generation INTEGER NOT NULL,
    before_owner_operation_id TEXT,
    after_digest TEXT NOT NULL,
    after_generation INTEGER NOT NULL,
    compensated INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(operation_id, state_key)
);
"#;

const RUNTIME_V2: &str = r#"
CREATE INDEX runtime_effects_compensation_idx ON runtime_effects(compensated, operation_id);
"#;

const RUNTIME_V3: &str = r#"
CREATE TABLE storage_meta (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    store_uuid TEXT NOT NULL,
    key_id TEXT,
    created_at INTEGER NOT NULL
);
CREATE TABLE runtime_generation_heads (
    state_key TEXT PRIMARY KEY,
    generation INTEGER NOT NULL CHECK(generation >= 0)
);
INSERT INTO runtime_generation_heads(state_key, generation)
    SELECT state_key, generation FROM runtime_state;
ALTER TABLE runtime_effects ADD COLUMN staged_json TEXT;
ALTER TABLE runtime_effects ADD COLUMN activated INTEGER NOT NULL DEFAULT 1;
"#;

const RUNTIME_V4: &str = r#"
CREATE TABLE credential_runtime_state (
    credential_id TEXT PRIMARY KEY,
    state_json TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK(generation >= 0),
    updated_at INTEGER NOT NULL
);
CREATE TABLE binding_runtime_state (
    binding_id TEXT PRIMARY KEY,
    state_json TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK(generation >= 0),
    updated_at INTEGER NOT NULL
);
"#;

const RUNTIME_V6: &str = r#"
CREATE TABLE compute_runtime_state_v1 (
    identity_key TEXT PRIMARY KEY,
    contract_schema TEXT NOT NULL
        CHECK(contract_schema = 'hiroute.compute-runtime-state/v1'),
    identity_json TEXT NOT NULL,
    state_json TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK(generation > 0),
    updated_at_unix_millis INTEGER NOT NULL CHECK(updated_at_unix_millis >= 0)
);
CREATE TABLE legacy_compute_runtime_state_v4 (
    legacy_kind TEXT NOT NULL CHECK(legacy_kind IN ('credential', 'binding')),
    legacy_subject_id TEXT NOT NULL,
    state_json TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK(generation >= 0),
    updated_at_unix_seconds INTEGER NOT NULL,
    PRIMARY KEY(legacy_kind, legacy_subject_id)
);
INSERT INTO legacy_compute_runtime_state_v4(
    legacy_kind, legacy_subject_id, state_json, generation, updated_at_unix_seconds
)
SELECT 'credential', credential_id, state_json, generation, updated_at
FROM credential_runtime_state;
INSERT INTO legacy_compute_runtime_state_v4(
    legacy_kind, legacy_subject_id, state_json, generation, updated_at_unix_seconds
)
SELECT 'binding', binding_id, state_json, generation, updated_at
FROM binding_runtime_state;
DROP TABLE credential_runtime_state;
DROP TABLE binding_runtime_state;
CREATE TRIGGER legacy_compute_runtime_state_v4_no_insert
BEFORE INSERT ON legacy_compute_runtime_state_v4
BEGIN
    SELECT RAISE(ABORT, 'legacy runtime availability is read-only');
END;
CREATE TRIGGER legacy_compute_runtime_state_v4_no_update
BEFORE UPDATE ON legacy_compute_runtime_state_v4
BEGIN
    SELECT RAISE(ABORT, 'legacy runtime availability is read-only');
END;
CREATE TRIGGER legacy_compute_runtime_state_v4_no_delete
BEFORE DELETE ON legacy_compute_runtime_state_v4
BEGIN
    SELECT RAISE(ABORT, 'legacy runtime availability is read-only');
END;
"#;

const SECRETS_V1: &str = r#"
CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at INTEGER NOT NULL,
    binary_version TEXT NOT NULL
);
CREATE TABLE secret_store_meta (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    schema_version INTEGER NOT NULL,
    active_key_version INTEGER NOT NULL,
    rotation_state TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
INSERT INTO secret_store_meta(
    singleton, schema_version, active_key_version, rotation_state, created_at, updated_at
) VALUES (1, 1, 1, 'stable', unixepoch(), unixepoch());
CREATE TABLE secret_entries (
    credential_id TEXT PRIMARY KEY,
    owner_scope TEXT NOT NULL,
    kind TEXT NOT NULL,
    ciphertext BLOB NOT NULL,
    nonce BLOB NOT NULL,
    aad_schema TEXT NOT NULL,
    key_version INTEGER NOT NULL,
    fingerprint TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK(generation > 0),
    owner_operation_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE secret_effects (
    operation_id TEXT NOT NULL,
    credential_id TEXT NOT NULL,
    before_exists INTEGER NOT NULL,
    before_owner_scope TEXT,
    before_kind TEXT,
    before_ciphertext BLOB,
    before_nonce BLOB,
    before_aad_schema TEXT,
    before_key_version INTEGER,
    before_fingerprint TEXT,
    before_generation INTEGER NOT NULL,
    before_owner_operation_id TEXT,
    after_exists INTEGER NOT NULL,
    after_fingerprint TEXT,
    after_generation INTEGER NOT NULL,
    compensated INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(operation_id, credential_id)
);
"#;

const SECRETS_V2: &str = r#"
CREATE TABLE secret_absence_markers (
    credential_id TEXT PRIMARY KEY,
    owner_scope TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK(generation > 0),
    owner_operation_id TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX secret_owner_fingerprint_idx
    ON secret_entries(owner_scope, fingerprint);
CREATE INDEX secret_effects_compensation_idx
    ON secret_effects(compensated, operation_id);
"#;

const SECRETS_V3: &str = r#"
ALTER TABLE secret_store_meta ADD COLUMN store_uuid TEXT NOT NULL DEFAULT '';
ALTER TABLE secret_store_meta ADD COLUMN key_id TEXT NOT NULL DEFAULT '';
ALTER TABLE secret_store_meta ADD COLUMN key_verifier TEXT NOT NULL DEFAULT '';
ALTER TABLE secret_entries ADD COLUMN subject TEXT NOT NULL DEFAULT 'legacy-locked';
ALTER TABLE secret_entries ADD COLUMN purpose TEXT NOT NULL DEFAULT 'legacy-locked';
ALTER TABLE secret_entries ADD COLUMN allowed_destinations_json TEXT NOT NULL DEFAULT '[]';
CREATE TABLE secret_generation_heads (
    credential_id TEXT PRIMARY KEY,
    generation INTEGER NOT NULL CHECK(generation >= 0)
);
INSERT INTO secret_generation_heads(credential_id, generation)
    SELECT credential_id, generation FROM secret_entries;
INSERT OR IGNORE INTO secret_generation_heads(credential_id, generation)
    SELECT credential_id, generation FROM secret_absence_markers;
ALTER TABLE secret_effects ADD COLUMN staged_owner_scope TEXT;
ALTER TABLE secret_effects ADD COLUMN before_subject TEXT;
ALTER TABLE secret_effects ADD COLUMN before_purpose TEXT;
ALTER TABLE secret_effects ADD COLUMN before_allowed_destinations_json TEXT;
ALTER TABLE secret_effects ADD COLUMN staged_subject TEXT;
ALTER TABLE secret_effects ADD COLUMN staged_purpose TEXT;
ALTER TABLE secret_effects ADD COLUMN staged_allowed_destinations_json TEXT;
ALTER TABLE secret_effects ADD COLUMN staged_kind TEXT;
ALTER TABLE secret_effects ADD COLUMN staged_ciphertext BLOB;
ALTER TABLE secret_effects ADD COLUMN staged_nonce BLOB;
ALTER TABLE secret_effects ADD COLUMN staged_aad_schema TEXT;
ALTER TABLE secret_effects ADD COLUMN staged_key_version INTEGER;
ALTER TABLE secret_effects ADD COLUMN activated INTEGER NOT NULL DEFAULT 1;
"#;

const SECRETS_V4: &str = r#"
-- Schema version alignment: compute metadata and runtime facts never enter secrets.db.
"#;

const NOOP_V9: &str = r#"
-- Schema version alignment: AgentAccessGrant records exist only in secrets.db.
"#;

#[cfg(test)]
mod tests {
    use crate::test_tempdir as tempdir;
    use hiroute_domain::CanonicalDigest;

    use super::*;
    use crate::backup::{BackupSet, BackupSetPhase, test_writer_barrier};
    use crate::{ControlStore, LocalSecretStore, LocalStorageSet};

    fn replace_with_legacy_v1_set(storage_root: &Path) {
        drop(
            LocalStorageSet::open_migration_component_fixture(
                &crate::test_storage_authority(),
                &test_writer_barrier(),
                storage_root,
            )
            .unwrap(),
        );
        let live = storage_root.join("live");
        for name in ["control.db", "runtime.db", "secrets.db"] {
            let path = live.join(name);
            for candidate in [
                path.clone(),
                PathBuf::from(format!("{}-wal", path.to_string_lossy())),
                PathBuf::from(format!("{}-shm", path.to_string_lossy())),
            ] {
                if candidate.exists() {
                    fs::remove_file(candidate).unwrap();
                }
            }
        }

        for (name, sql) in [
            ("control.db", CONTROL_V1),
            ("runtime.db", RUNTIME_V1),
            ("secrets.db", SECRETS_V1),
        ] {
            let path = live.join(name);
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch(sql).unwrap();
            connection.pragma_update(None, "user_version", 1).unwrap();
            connection
                .execute(
                    "INSERT INTO schema_migrations(version, applied_at, binary_version)
                     VALUES (1, 1, 'legacy')",
                    [],
                )
                .unwrap();
            if name == "control.db" {
                connection
                    .execute(
                        "INSERT INTO workspace_state(
                            workspace_id, desired_json, target_revision, desired_digest,
                            owner_operation_id, updated_at
                         ) VALUES ('personal/default', '{\"schema\":\"hiroute.control-desired/v1\",\"value\":{}}', 17, ?1, NULL, 1)",
                        [CanonicalDigest::of(&serde_json::json!({})).unwrap().as_str()],
                    )
                    .unwrap();
            }
            if name == "runtime.db" {
                connection
                    .execute(
                        "INSERT INTO runtime_state(
                            state_key, value_json, value_digest, generation,
                            owner_operation_id, updated_at
                         ) VALUES ('active/setup', '{\"schema\":\"hiroute.runtime-state/v1\",\"value\":{\"ready\":false}}', ?1, 4, NULL, 1)",
                        [CanonicalDigest::of_bytes(b"legacy-runtime").as_str()],
                    )
                    .unwrap();
            }
            drop(connection);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
            }
        }
    }

    #[test]
    fn migration_is_versioned_and_idempotent() {
        let directory = tempdir().unwrap();
        let data_root = directory.path().join("data");
        let path = data_root.join("control.db");
        let backup = data_root.join("migration-backups");
        drop(
            open_database(
                &crate::test_storage_authority(),
                &path,
                DatabaseKind::Control,
                &backup,
            )
            .unwrap(),
        );
        let reopened = open_database(
            &crate::test_storage_authority(),
            &path,
            DatabaseKind::Control,
            &backup,
        )
        .unwrap();
        let version: u32 = reopened
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, LATEST_SCHEMA_VERSION);
    }

    #[test]
    fn malformed_unbound_v15_set_is_refused_without_mutating_databases() {
        let directory = tempdir().unwrap();
        let storage_root = directory.path().join("storage");
        let live = storage_root.join("live");
        crate::test_create_dir_all(&live).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&storage_root, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&live, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut before = Vec::new();
        for name in ["control.db", "runtime.db", "secrets.db"] {
            let path = live.join(name);
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE legacy_marker(value TEXT NOT NULL);\n\
                     INSERT INTO legacy_marker(value) VALUES ('permit-era');\n\
                     PRAGMA user_version = 15;",
                )
                .unwrap();
            drop(connection);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            }
            before.push(fs::read(path).unwrap());
        }

        let error = MigrationSetCoordinator::prepare(
            &crate::test_storage_authority(),
            &test_writer_barrier(),
            &live,
            &storage_root.join("migration-set"),
            None,
        )
        .err();
        assert!(matches!(error, Some(LocalStorageError::Locked)));
        for (index, name) in ["control.db", "runtime.db", "secrets.db"]
            .into_iter()
            .enumerate()
        {
            let path = live.join(name);
            assert_eq!(fs::read(&path).unwrap(), before[index]);
            assert_eq!(database_version(&path).unwrap(), Some(15));
        }
        assert!(!storage_root.join("migration-set").exists());
    }

    #[test]
    fn transaction_migration_creates_a_valid_pre_upgrade_backup() {
        let directory = tempdir().unwrap();
        let data_root = directory.path().join("data");
        fs::create_dir(&data_root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&data_root, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let path = data_root.join("control.db");
        let backup_root = data_root.join("migration-backups");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(CONTROL_V1).unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations(version, applied_at, binary_version)
                 VALUES (1, 1, 'old')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO workspace_state(
                    workspace_id, desired_json, target_revision, desired_digest,
                    owner_operation_id, updated_at
                 ) VALUES ('personal/default', '{}', 7, ?1, NULL, 1)",
                params![CanonicalDigest::of_bytes(b"old").as_str()],
            )
            .unwrap();
        drop(connection);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }

        let upgraded = open_database(
            &crate::test_storage_authority(),
            &path,
            DatabaseKind::Control,
            &backup_root,
        )
        .unwrap();
        let version: u32 = upgraded
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, LATEST_SCHEMA_VERSION);
        drop(upgraded);

        let backup_path = backup_root.join("control.db.before-v2.db");
        let backup = SqliteBackup::open(&backup_path).unwrap();
        let restored = directory.path().join("restored/control.db");
        backup.restore_to(&restored).unwrap();
        let restored = Connection::open(restored).unwrap();
        let revision: u64 = restored
            .query_row(
                "SELECT target_revision FROM workspace_state WHERE workspace_id = 'personal/default'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(revision, 7);
    }

    #[test]
    fn transaction_secret_v1_upgrade_adds_absence_ownership_and_keeps_a_backup() {
        let directory = tempdir().unwrap();
        let data_root = directory.path().join("data");
        fs::create_dir(&data_root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&data_root, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let path = data_root.join("secrets.db");
        let backup_root = data_root.join("migration-backups");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(SECRETS_V1).unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations(version, applied_at, binary_version)
                 VALUES (1, 1, 'old')",
                [],
            )
            .unwrap();
        drop(connection);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }

        let key_id = CanonicalDigest::of_bytes(b"migration-test-key");
        let upgraded = open_database_with_key_id(
            &crate::test_storage_authority(),
            &path,
            DatabaseKind::Secrets,
            &backup_root,
            Some(&key_id),
        )
        .unwrap();
        let marker_table: String = upgraded
            .query_row(
                "SELECT name FROM sqlite_master
                 WHERE type = 'table' AND name = 'secret_absence_markers'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marker_table, "secret_absence_markers");
        drop(upgraded);
        let backup = SqliteBackup::open(backup_root.join("secrets.db.before-v2.db")).unwrap();
        assert_eq!(backup.manifest().key_id.as_deref(), Some(key_id.as_str()));
    }

    #[test]
    fn durable_three_store_migration_reopens_and_finishes_both_first_store_crash_windows() {
        for persist_control_phase in [false, true] {
            let directory = tempdir().unwrap();
            let storage_root = directory.path().join("storage");
            replace_with_legacy_v1_set(&storage_root);
            let live = storage_root.join("live");
            let backup_root = crate::upgrade_backup_root(&storage_root).unwrap();
            let binding = LocalSecretStore::migration_binding(
                &live.join("secrets.db"),
                &storage_root.join("master-key"),
            )
            .unwrap()
            .unwrap();
            let barrier = test_writer_barrier();
            let coordinator = MigrationSetCoordinator::prepare(
                &crate::test_storage_authority(),
                &barrier,
                &live,
                &backup_root,
                Some(&binding),
            )
            .unwrap();
            assert_eq!(database_version(&live.join("control.db")).unwrap(), Some(1));
            assert_eq!(database_version(&live.join("runtime.db")).unwrap(), Some(1));
            assert_eq!(database_version(&live.join("secrets.db")).unwrap(), Some(1));
            assert_eq!(
                BackupSet::open(&crate::test_storage_authority(), &backup_root)
                    .unwrap()
                    .phase(),
                BackupSetPhase::SourcePublished
            );
            drop(coordinator);

            // Losing the in-memory handle after durable source publication is harmless.
            let mut restarted = MigrationSetCoordinator::prepare(
                &crate::test_storage_authority(),
                &barrier,
                &live,
                &backup_root,
                Some(&binding),
            )
            .unwrap();
            let control_uuid = restarted.expected_control_store_uuid().unwrap().to_owned();
            drop(
                ControlStore::open_from_migration_set(
                    &crate::test_storage_authority(),
                    &live.join("control.db"),
                    &backup_root,
                    &control_uuid,
                    LATEST_SCHEMA_VERSION,
                )
                .unwrap(),
            );
            if persist_control_phase {
                restarted.mark_control_migrated(&barrier).unwrap();
            }
            drop(restarted);

            // Restart observes either the source phase or the persisted control phase, proves the
            // already-upgraded control UUID, and deterministically completes the remaining pair.
            let storage = LocalStorageSet::open_migration_component_fixture(
                &crate::test_storage_authority(),
                &barrier,
                &storage_root,
            )
            .unwrap();
            let assert_latest = |connection: &Connection| {
                let version: u32 = connection
                    .query_row("PRAGMA user_version", [], |row| row.get(0))
                    .unwrap();
                assert_eq!(version, LATEST_SCHEMA_VERSION);
            };
            storage.control().with_connection(assert_latest);
            storage.runtime().with_connection(assert_latest);
            storage.secrets().with_connection(|connection| {
                let version: u32 = connection
                    .query_row("PRAGMA user_version", [], |row| row.get(0))
                    .unwrap();
                assert_eq!(version, LATEST_SCHEMA_VERSION);
            });
            drop(storage);

            let completed =
                BackupSet::open(&crate::test_storage_authority(), &backup_root).unwrap();
            assert_eq!(completed.phase(), BackupSetPhase::Completed);
            assert_eq!(
                completed.control.manifest().journal_watermark,
                completed.runtime.manifest().journal_watermark
            );
            assert_eq!(
                completed.control.manifest().journal_watermark,
                completed.secrets.manifest().journal_watermark
            );
            let restored = storage_root.join(format!(
                "restored-source-{}",
                if persist_control_phase {
                    "after"
                } else {
                    "before"
                }
            ));
            completed.restore_to_new_root(&barrier, &restored).unwrap();
            for name in ["control.db", "runtime.db", "secrets.db"] {
                assert_eq!(database_version(&restored.join(name)).unwrap(), Some(1));
            }
            let restored_control = Connection::open(restored.join("control.db")).unwrap();
            let revision: u64 = restored_control
                .query_row(
                    "SELECT target_revision FROM workspace_state
                     WHERE workspace_id = 'personal/default'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(revision, 17);
            drop(restored_control);
            drop(
                LocalStorageSet::open_migration_component_fixture(
                    &crate::test_storage_authority(),
                    &barrier,
                    &storage_root,
                )
                .unwrap(),
            );
        }
    }

    #[test]
    fn incomplete_first_set_manifest_stage_is_cleaned_and_migration_finishes_on_restarts() {
        for (case, bytes, suffix) in [
            ("empty", &b""[..], "writing.stage.json"),
            (
                "truncated",
                &b"{\"schema\":\"hiroute.sqlite-backup-set/v2\""[..],
                "writing.stage.json",
            ),
        ] {
            let directory = tempdir().unwrap();
            let storage_root = directory.path().join("storage");
            replace_with_legacy_v1_set(&storage_root);
            let live = storage_root.join("live");
            let backup_root = crate::upgrade_backup_root(&storage_root).unwrap();
            let binding = LocalSecretStore::migration_binding(
                &live.join("secrets.db"),
                &storage_root.join("master-key"),
            )
            .unwrap()
            .unwrap();
            let barrier = test_writer_barrier();
            let coordinator = MigrationSetCoordinator::prepare(
                &crate::test_storage_authority(),
                &barrier,
                &live,
                &backup_root,
                Some(&binding),
            )
            .unwrap();
            drop(coordinator);
            let original = BackupSet::open(&crate::test_storage_authority(), &backup_root).unwrap();
            let source_digests = [
                original.control.manifest().source_state_digest.clone(),
                original.runtime.manifest().source_state_digest.clone(),
                original.secrets.manifest().source_state_digest.clone(),
            ];
            drop(original);

            fs::remove_file(backup_root.join("backup-set.manifest.json")).unwrap();
            let stage = backup_root.join(format!(".backup-set.manifest.{case}.{suffix}"));
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&stage).unwrap();
            use std::io::Write as _;
            file.write_all(bytes).unwrap();
            file.sync_all().unwrap();
            fs::File::open(&backup_root).unwrap().sync_all().unwrap();

            // The first daemon-shaped restart removes an uncommitted writing generation, reuses
            // the exact three source backups, republishes the set manifest, and finishes all
            // migrations. A second restart proves the stage cannot become a permanent blocker.
            drop(
                LocalStorageSet::open_migration_component_fixture(
                    &crate::test_storage_authority(),
                    &barrier,
                    &storage_root,
                )
                .unwrap(),
            );
            assert!(!stage.exists());
            drop(
                LocalStorageSet::open_migration_component_fixture(
                    &crate::test_storage_authority(),
                    &barrier,
                    &storage_root,
                )
                .unwrap(),
            );
            let completed =
                BackupSet::open(&crate::test_storage_authority(), &backup_root).unwrap();
            assert_eq!(completed.phase(), BackupSetPhase::Completed);
            assert_eq!(
                [
                    completed.control.manifest().source_state_digest.clone(),
                    completed.runtime.manifest().source_state_digest.clone(),
                    completed.secrets.manifest().source_state_digest.clone(),
                ],
                source_digests
            );
            for name in ["control.db", "runtime.db", "secrets.db"] {
                assert_eq!(
                    database_version(&live.join(name)).unwrap(),
                    Some(LATEST_SCHEMA_VERSION)
                );
            }
        }
    }

    #[test]
    fn durable_migration_restore_reconciles_phase_only_and_post_live_rename_crashes() {
        for after_live_rename in [false, true] {
            let directory = tempdir().unwrap();
            let storage_root = directory.path().join("storage");
            replace_with_legacy_v1_set(&storage_root);
            let live = storage_root.join("live");
            let backup_root = crate::upgrade_backup_root(&storage_root).unwrap();
            let binding = LocalSecretStore::migration_binding(
                &live.join("secrets.db"),
                &storage_root.join("master-key"),
            )
            .unwrap()
            .unwrap();
            let barrier = test_writer_barrier();
            let coordinator = MigrationSetCoordinator::prepare(
                &crate::test_storage_authority(),
                &barrier,
                &live,
                &backup_root,
                Some(&binding),
            )
            .unwrap();
            let control_uuid = coordinator
                .expected_control_store_uuid()
                .unwrap()
                .to_owned();
            drop(
                ControlStore::open_from_migration_set(
                    &crate::test_storage_authority(),
                    &live.join("control.db"),
                    &backup_root,
                    &control_uuid,
                    LATEST_SCHEMA_VERSION,
                )
                .unwrap(),
            );
            drop(coordinator);

            let mut set = BackupSet::open(&crate::test_storage_authority(), &backup_root).unwrap();
            set.persist_phase(&barrier, BackupSetPhase::Restoring)
                .unwrap();
            if after_live_rename {
                let ready = storage_root.join(format!(".live.{}.restore-ready", set.set_id()));
                let previous = storage_root.join(format!(".live.{}.pre-migration", set.set_id()));
                set.restore_to_new_root(&barrier, &ready).unwrap();
                fs::rename(&live, &previous).unwrap();
                std::fs::File::open(&storage_root)
                    .unwrap()
                    .sync_all()
                    .unwrap();
            }
            drop(set);

            // Exercise the SQL coordinator component harness. Unsupported sources cannot enter
            // production startup. The
            // first reopen completes the exact group restore and stays fail-closed for that call.
            assert!(
                LocalStorageSet::open_migration_component_fixture(
                    &crate::test_storage_authority(),
                    &barrier,
                    &storage_root,
                )
                .is_err()
            );
            let restored = BackupSet::open(&crate::test_storage_authority(), &backup_root).unwrap();
            assert_eq!(restored.phase(), BackupSetPhase::Restored);
            for (name, backup) in [
                ("control.db", &restored.control),
                ("runtime.db", &restored.runtime),
                ("secrets.db", &restored.secrets),
            ] {
                let path = live.join(name);
                assert_eq!(database_version(&path).unwrap(), Some(1));
                let connection = Connection::open_with_flags(
                    path,
                    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )
                .unwrap();
                assert_eq!(
                    database_state_digest(&connection).unwrap(),
                    backup.manifest().source_state_digest
                );
            }
        }
    }
}
