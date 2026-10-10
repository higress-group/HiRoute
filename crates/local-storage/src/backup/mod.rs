pub(crate) mod source_snapshot;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use hiroute_domain::CanonicalDigest;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::{DaemonStorageAuthority, LocalStorageError};

const BACKUP_MANIFEST_SCHEMA: &str = "hiroute.sqlite-backup-manifest/v2";
const BACKUP_SET_MANIFEST_SCHEMA: &str = "hiroute.sqlite-backup-set/v2";
const BACKUP_SET_MANIFEST_NAME: &str = "backup-set.manifest.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackupManifest {
    pub schema: String,
    pub database_kind: String,
    pub store_uuid: String,
    pub schema_version: u32,
    pub source_digest: CanonicalDigest,
    pub source_state_digest: CanonicalDigest,
    pub key_id: Option<String>,
    pub journal_watermark: String,
    pub attempt_id: String,
}

#[derive(Clone, Debug)]
pub struct SqliteBackup {
    path: PathBuf,
    manifest: BackupManifest,
}

impl SqliteBackup {
    pub fn create(
        _authority: &DaemonStorageAuthority,
        connection: &Connection,
        path: impl AsRef<Path>,
    ) -> Result<Self, LocalStorageError> {
        Self::create_bound(connection, path, None)
    }

    pub(crate) fn create_bound(
        connection: &Connection,
        path: impl AsRef<Path>,
        key_id: Option<&CanonicalDigest>,
    ) -> Result<Self, LocalStorageError> {
        let watermark = standalone_watermark(connection)?;
        Self::create_at_watermark(connection, path.as_ref(), &watermark, key_id)
    }

    fn create_at_watermark(
        connection: &Connection,
        path: &Path,
        journal_watermark: &str,
        key_id: Option<&CanonicalDigest>,
    ) -> Result<Self, LocalStorageError> {
        let parent = path.parent().ok_or(LocalStorageError::InvalidData)?;
        prepare_owner_directory(parent)?;
        let attempt_id = random_attempt_id()?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(LocalStorageError::InvalidData)?;
        // `VACUUM INTO` creates the destination before this process can tighten its mode. A crash
        // in that small window leaves a current-uid regular file in an already owner-only
        // directory. Reconcile those exact attempt names before allocating the next attempt so a
        // restart never treats a recoverable 0644 SQLite stage as a permanent permission fault.
        cleanup_orphan_stages(parent, file_name)?;
        let stage = parent.join(format!(".{file_name}.{attempt_id}.stage.db"));
        let stage_manifest = parent.join(format!(".{file_name}.{attempt_id}.stage.manifest.json"));
        if stage.exists() || stage_manifest.exists() {
            return Err(LocalStorageError::InvalidData);
        }

        connection.execute_batch("PRAGMA wal_checkpoint(FULL)")?;
        let source_state_digest = database_state_digest(connection)?;
        connection.execute(
            "VACUUM main INTO ?1",
            params![stage.to_string_lossy().as_ref()],
        )?;
        set_owner_file_permissions(&stage)?;
        File::open(&stage)?.sync_all()?;
        validate_database(&stage)?;
        let identity = database_identity(connection)?;
        if identity
            .key_id
            .as_deref()
            .is_some_and(|stored| key_id.is_some_and(|requested| stored != requested.as_str()))
        {
            return Err(LocalStorageError::Locked);
        }
        let manifest = BackupManifest {
            schema: BACKUP_MANIFEST_SCHEMA.to_owned(),
            database_kind: identity.kind,
            store_uuid: identity.store_uuid,
            schema_version: identity.schema_version,
            source_digest: file_digest(&stage)?,
            source_state_digest,
            key_id: identity.key_id.or_else(|| key_id.map(ToString::to_string)),
            journal_watermark: journal_watermark.to_owned(),
            attempt_id: attempt_id.clone(),
        };
        write_manifest(&stage_manifest, &manifest)?;
        File::open(parent)?.sync_all()?;

        let final_manifest = manifest_path(path);
        if path.exists() || final_manifest.exists() {
            if path.exists()
                && final_manifest.exists()
                && Self::open(path).is_ok_and(|existing| {
                    manifest_exact_except_attempt(&existing.manifest, &manifest)
                })
            {
                fs::remove_file(&stage)?;
                fs::remove_file(&stage_manifest)?;
                cleanup_orphan_stages(parent, file_name)?;
                return Self::open(path);
            }
            // Preserve stale or half-published finals for forensic recovery; they never block a
            // fresh exact attempt and are never silently reused for a different source snapshot.
            if path.exists() {
                fs::rename(
                    path,
                    parent.join(format!(".{file_name}.{attempt_id}.stale.db")),
                )?;
            }
            if final_manifest.exists() {
                fs::rename(
                    &final_manifest,
                    parent.join(format!(".{file_name}.{attempt_id}.stale.manifest.json")),
                )?;
            }
            File::open(parent)?.sync_all()?;
        }

        fs::rename(&stage, path)?;
        File::open(parent)?.sync_all()?;
        fs::rename(&stage_manifest, &final_manifest)?;
        File::open(parent)?.sync_all()?;
        let backup = Self::open(path)?;
        cleanup_orphan_stages(parent, file_name)?;
        Ok(backup)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, LocalStorageError> {
        let path = path.as_ref();
        validate_owner_file(path)?;
        validate_database(path)?;
        let manifest_path = manifest_path(path);
        validate_owner_file(&manifest_path)?;
        let manifest: BackupManifest = serde_json::from_slice(&fs::read(&manifest_path)?)
            .map_err(|_| LocalStorageError::InvalidData)?;
        if manifest.schema != BACKUP_MANIFEST_SCHEMA || manifest.source_digest != file_digest(path)?
        {
            return Err(LocalStorageError::InvalidData);
        }
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let identity = database_identity(&connection)?;
        if manifest.database_kind != identity.kind
            || manifest.store_uuid != identity.store_uuid
            || manifest.schema_version != identity.schema_version
            || identity
                .key_id
                .as_ref()
                .is_some_and(|key_id| manifest.key_id.as_ref() != Some(key_id))
            || manifest
                .key_id
                .as_deref()
                .is_some_and(|key_id| CanonicalDigest::parse(key_id).is_err())
            || manifest.source_state_digest != database_state_digest(&connection)?
        {
            return Err(LocalStorageError::InvalidData);
        }
        Ok(Self {
            path: path.to_path_buf(),
            manifest,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn manifest(&self) -> &BackupManifest {
        &self.manifest
    }

    /// Restore is intentionally create-new. The exact DB and its manifest are verified before and
    /// after publication; replacing a live database remains a separate reversible owner action.
    pub(crate) fn restore_to(
        &self,
        destination: impl AsRef<Path>,
    ) -> Result<(), LocalStorageError> {
        Self::open(&self.path)?;
        let destination = destination.as_ref();
        if destination.exists() {
            return Err(LocalStorageError::InvalidData);
        }
        let parent = destination.parent().ok_or(LocalStorageError::InvalidData)?;
        prepare_owner_directory(parent)?;
        let attempt = random_attempt_id()?;
        let temporary = parent.join(format!(".restore-{attempt}.stage.db"));
        fs::copy(&self.path, &temporary)?;
        set_owner_file_permissions(&temporary)?;
        File::open(&temporary)?.sync_all()?;
        validate_database(&temporary)?;
        if file_digest(&temporary)? != self.manifest.source_digest {
            return Err(LocalStorageError::InvalidData);
        }
        fs::rename(&temporary, destination)?;
        File::open(parent)?.sync_all()?;
        validate_database(destination)?;
        if file_digest(destination)? != self.manifest.source_digest {
            return Err(LocalStorageError::InvalidData);
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct BackupSet {
    pub control: SqliteBackup,
    pub runtime: SqliteBackup,
    pub secrets: SqliteBackup,
    root: PathBuf,
    set_id: String,
    attempt_id: String,
    phase: BackupSetPhase,
    target_schema_version: u32,
    watermark: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupSetPhase {
    SourcePublished,
    ControlMigrated,
    RuntimeMigrated,
    SecretsMigrated,
    Completed,
    Restoring,
    Restored,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct BackupSetManifest {
    schema: String,
    set_id: String,
    attempt_id: String,
    phase: BackupSetPhase,
    target_schema_version: u32,
    watermark: String,
    control: BackupManifest,
    runtime: BackupManifest,
    secrets: BackupManifest,
}

/// Proof that the daemon-owned writer actor is paused for the full three-store snapshot.
///
/// The field is intentionally private and no constructor is exposed in this PROCESS. The formal
/// daemon composition owner will mint it only while holding the same lock used by transaction
/// admission; raw callers cannot claim a consistent set from three unrelated connections.
pub struct SingleWriterBackupBarrier {
    _private: (),
}

/// Crate-sealed proof used only while daemon startup still has listeners and transaction
/// admission closed. No production caller can mint or retain the raw barrier.
pub(crate) fn daemon_startup_barrier() -> SingleWriterBackupBarrier {
    SingleWriterBackupBarrier { _private: () }
}

#[cfg(test)]
pub(crate) fn test_writer_barrier() -> SingleWriterBackupBarrier {
    daemon_startup_barrier()
}

impl BackupSet {
    /// Saves the complete stopped storage authority beside this verified three-store backup.
    /// The caller must retain the startup barrier and directory ownership until publication ends.
    pub fn publish_stopped_source(
        &self,
        _barrier: &SingleWriterBackupBarrier,
        storage_root: &Path,
        options: &crate::StorageStartupOptions,
    ) -> Result<(), LocalStorageError> {
        if self
            .root
            .canonicalize()?
            .starts_with(storage_root.canonicalize()?)
        {
            return Err(LocalStorageError::InvalidData);
        }
        for (name, backup) in [
            ("control.db", &self.control),
            ("runtime.db", &self.runtime),
            ("secrets.db", &self.secrets),
        ] {
            let connection = Connection::open_with_flags(
                storage_root.join("live").join(name),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            validate_restored_source_identity(&connection, backup.manifest())?;
            if database_state_digest(&connection)? != backup.manifest().source_state_digest {
                return Err(LocalStorageError::InvalidData);
            }
        }
        let metadata = serde_json::json!({
            "batch_id": self.set_id(), "target_schema_version": self.target_schema_version(),
            "source_stores": {"control":self.control.manifest(),"runtime":self.runtime.manifest(),"secrets":self.secrets.manifest()}
        });
        source_snapshot::publish(storage_root, &self.root, &metadata, options)
    }

    pub(crate) fn directory(&self) -> &Path {
        &self.root
    }
    /// Creates a local three-store snapshot under the hirouted single-writer barrier. A durable
    /// writer claim proves that the barrier is not held and causes a fail-closed refusal.
    pub fn create(
        barrier: &SingleWriterBackupBarrier,
        backup_root: impl AsRef<Path>,
        control: &Connection,
        runtime: &Connection,
        secrets: &Connection,
    ) -> Result<Self, LocalStorageError> {
        Self::create_inner(
            barrier,
            backup_root.as_ref(),
            control,
            runtime,
            secrets,
            None,
            crate::migrations::LATEST_SCHEMA_VERSION,
            false,
        )
    }

    pub(crate) fn create_migration(
        barrier: &SingleWriterBackupBarrier,
        backup_root: &Path,
        control: &Connection,
        runtime: &Connection,
        secrets: &Connection,
        secret_key_id: &CanonicalDigest,
        target_schema_version: u32,
    ) -> Result<Self, LocalStorageError> {
        Self::create_inner(
            barrier,
            backup_root,
            control,
            runtime,
            secrets,
            Some(secret_key_id),
            target_schema_version,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_inner(
        _barrier: &SingleWriterBackupBarrier,
        backup_root: &Path,
        control: &Connection,
        runtime: &Connection,
        secrets: &Connection,
        secret_key_id: Option<&CanonicalDigest>,
        target_schema_version: u32,
        startup_migration: bool,
    ) -> Result<Self, LocalStorageError> {
        prepare_owner_directory(backup_root)?;
        if reconcile_set_manifest_publication(backup_root)? {
            let existing = Self::open_inner(backup_root)?;
            if existing.target_schema_version != target_schema_version
                || !existing.sources_match_connections(control, runtime, secrets, secret_key_id)?
            {
                return Err(LocalStorageError::InvalidData);
            }
            return Ok(existing);
        }
        let writer_table: bool = control.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'writer_claim'
            )",
            [],
            |row| row.get(0),
        )?;
        let writer_claim = writer_table
            && control.query_row("SELECT EXISTS(SELECT 1 FROM writer_claim)", [], |row| {
                row.get::<_, bool>(0)
            })?;
        if writer_claim && !startup_migration {
            return Err(LocalStorageError::InvalidData);
        }
        control.execute_batch("PRAGMA wal_checkpoint(FULL)")?;
        runtime.execute_batch("PRAGMA wal_checkpoint(FULL)")?;
        secrets.execute_batch("PRAGMA wal_checkpoint(FULL)")?;
        let watermark = control_watermark(control)?;
        let control = SqliteBackup::create_at_watermark(
            control,
            &backup_root.join("control.db"),
            &watermark,
            None,
        )?;
        let runtime = SqliteBackup::create_at_watermark(
            runtime,
            &backup_root.join("runtime.db"),
            &watermark,
            None,
        )?;
        let secrets = SqliteBackup::create_at_watermark(
            secrets,
            &backup_root.join("secrets.db"),
            &watermark,
            secret_key_id,
        )?;
        if control.manifest.journal_watermark != runtime.manifest.journal_watermark
            || control.manifest.journal_watermark != secrets.manifest.journal_watermark
        {
            return Err(LocalStorageError::InvalidData);
        }
        let manifest = BackupSetManifest {
            schema: BACKUP_SET_MANIFEST_SCHEMA.to_owned(),
            set_id: format!("set_{}", random_attempt_id()?),
            attempt_id: random_attempt_id()?,
            phase: BackupSetPhase::SourcePublished,
            target_schema_version,
            watermark,
            control: control.manifest.clone(),
            runtime: runtime.manifest.clone(),
            secrets: secrets.manifest.clone(),
        };
        publish_set_manifest(backup_root, &manifest, false)?;
        Self::from_manifest(backup_root, manifest)
    }

    /// Reopens a durable three-store source set after process loss and validates every exact
    /// per-store backup before returning the handle.
    pub fn open(
        _authority: &DaemonStorageAuthority,
        backup_root: impl AsRef<Path>,
    ) -> Result<Self, LocalStorageError> {
        Self::open_inner(backup_root.as_ref())
    }

    pub(crate) fn open_inner(backup_root: &Path) -> Result<Self, LocalStorageError> {
        prepare_owner_directory(backup_root)?;
        if !reconcile_set_manifest_publication(backup_root)? {
            return Err(LocalStorageError::InvalidData);
        }
        let path = set_manifest_path(backup_root);
        validate_owner_file(&path)?;
        let manifest: BackupSetManifest =
            serde_json::from_slice(&fs::read(path)?).map_err(|_| LocalStorageError::InvalidData)?;
        Self::from_manifest(backup_root, manifest)
    }

    pub(crate) fn durable_manifest_present(backup_root: &Path) -> Result<bool, LocalStorageError> {
        if !backup_root.exists() {
            return Ok(false);
        }
        prepare_owner_directory(backup_root)?;
        reconcile_set_manifest_publication(backup_root)
    }

    fn from_manifest(
        backup_root: &Path,
        manifest: BackupSetManifest,
    ) -> Result<Self, LocalStorageError> {
        validate_set_manifest_shape(&manifest)?;
        let (control, runtime, secrets) =
            validate_manifest_against_backups(backup_root, &manifest)?;
        Ok(Self {
            control,
            runtime,
            secrets,
            root: backup_root.to_path_buf(),
            set_id: manifest.set_id,
            attempt_id: manifest.attempt_id,
            phase: manifest.phase,
            target_schema_version: manifest.target_schema_version,
            watermark: manifest.watermark,
        })
    }

    pub fn watermark(&self) -> &str {
        &self.watermark
    }

    pub fn set_id(&self) -> &str {
        &self.set_id
    }

    pub const fn phase(&self) -> BackupSetPhase {
        self.phase
    }

    pub const fn target_schema_version(&self) -> u32 {
        self.target_schema_version
    }

    pub(crate) fn persist_phase(
        &mut self,
        _barrier: &SingleWriterBackupBarrier,
        phase: BackupSetPhase,
    ) -> Result<(), LocalStorageError> {
        if phase == self.phase {
            return Ok(());
        }
        if !valid_phase_transition(self.phase, phase) {
            return Err(LocalStorageError::InvalidData);
        }
        let mut manifest = self.set_manifest();
        manifest.phase = phase;
        publish_set_manifest(&self.root, &manifest, true)?;
        self.phase = phase;
        Ok(())
    }

    /// Restores the already-validated set into one new directory. All three databases and their
    /// manifests are prepared beneath a unique sibling directory, fsynced, re-opened, and only
    /// then published with one directory rename. A crash cannot expose a partial final set.
    pub fn restore_to_new_root(
        &self,
        _barrier: &SingleWriterBackupBarrier,
        destination: impl AsRef<Path>,
    ) -> Result<(), LocalStorageError> {
        self.validate_source_set()?;
        let destination = destination.as_ref();
        let expected = self.source_manifest();
        if destination.exists() {
            return validate_restored_set(destination, &expected);
        }
        let parent = destination.parent().ok_or(LocalStorageError::InvalidData)?;
        prepare_owner_directory(parent)?;
        let name = destination
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(LocalStorageError::InvalidData)?;
        let attempt = random_attempt_id()?;
        let stage = parent.join(format!(".{name}.{attempt}.restore-set"));
        fs::create_dir(&stage)?;
        set_owner_directory_permissions(&stage)?;

        for (backup, name) in [
            (&self.control, "control.db"),
            (&self.runtime, "runtime.db"),
            (&self.secrets, "secrets.db"),
        ] {
            let database = stage.join(name);
            backup.restore_to(&database)?;
            write_manifest(&manifest_path(&database), backup.manifest())?;
        }
        write_set_manifest(&stage.join(BACKUP_SET_MANIFEST_NAME), &expected)?;
        validate_restored_set(&stage, &expected)?;
        File::open(&stage)?.sync_all()?;
        fs::rename(&stage, destination)?;
        File::open(parent)?.sync_all()?;
        validate_restored_set(destination, &expected)
    }

    /// Recovery-only replacement of the sealed live directory. Publication is a directory-level
    /// swap with durable phase and fixed set-owned paths, so a restart can finish the swap without
    /// ever selecting files from two generations.
    pub(crate) fn restore_over_live_root(
        &mut self,
        barrier: &SingleWriterBackupBarrier,
        live_root: &Path,
    ) -> Result<(), LocalStorageError> {
        if self.phase != BackupSetPhase::Restoring {
            self.persist_phase(barrier, BackupSetPhase::Restoring)?;
        }
        let parent = live_root.parent().ok_or(LocalStorageError::InvalidData)?;
        prepare_owner_directory(parent)?;
        let live_name = live_root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(LocalStorageError::InvalidData)?;
        let ready = parent.join(format!(".{live_name}.{}.restore-ready", self.set_id));
        let previous = parent.join(format!(".{live_name}.{}.pre-migration", self.set_id));
        let expected = self.source_manifest();

        if live_root.exists() && previous.exists() {
            validate_restored_set(live_root, &expected)?;
            self.persist_phase(barrier, BackupSetPhase::Restored)?;
            return Ok(());
        }
        if !ready.exists() {
            self.restore_to_new_root(barrier, &ready)?;
        } else {
            validate_restored_set(&ready, &expected)?;
        }
        if live_root.exists() {
            if previous.exists() {
                return Err(LocalStorageError::InvalidData);
            }
            fs::rename(live_root, &previous)?;
            File::open(parent)?.sync_all()?;
        }
        if !live_root.exists() {
            fs::rename(&ready, live_root)?;
            File::open(parent)?.sync_all()?;
        }
        validate_restored_set(live_root, &expected)?;
        self.persist_phase(barrier, BackupSetPhase::Restored)
    }

    fn set_manifest(&self) -> BackupSetManifest {
        BackupSetManifest {
            schema: BACKUP_SET_MANIFEST_SCHEMA.to_owned(),
            set_id: self.set_id.clone(),
            attempt_id: self.attempt_id.clone(),
            phase: self.phase,
            target_schema_version: self.target_schema_version,
            watermark: self.watermark.clone(),
            control: self.control.manifest.clone(),
            runtime: self.runtime.manifest.clone(),
            secrets: self.secrets.manifest.clone(),
        }
    }

    fn source_manifest(&self) -> BackupSetManifest {
        let mut manifest = self.set_manifest();
        manifest.phase = BackupSetPhase::SourcePublished;
        manifest
    }

    fn sources_match_connections(
        &self,
        control: &Connection,
        runtime: &Connection,
        secrets: &Connection,
        secret_key_id: Option<&CanonicalDigest>,
    ) -> Result<bool, LocalStorageError> {
        Ok(
            self.control.manifest.source_state_digest == database_state_digest(control)?
                && self.runtime.manifest.source_state_digest == database_state_digest(runtime)?
                && self.secrets.manifest.source_state_digest == database_state_digest(secrets)?
                && self.secrets.manifest.key_id.as_deref()
                    == secret_key_id.map(CanonicalDigest::as_str),
        )
    }

    fn validate_source_set(&self) -> Result<(), LocalStorageError> {
        let control = SqliteBackup::open(&self.control.path)?;
        let runtime = SqliteBackup::open(&self.runtime.path)?;
        let secrets = SqliteBackup::open(&self.secrets.path)?;
        if control.manifest != self.control.manifest
            || runtime.manifest != self.runtime.manifest
            || secrets.manifest != self.secrets.manifest
            || control.manifest.journal_watermark != self.watermark
            || runtime.manifest.journal_watermark != self.watermark
            || secrets.manifest.journal_watermark != self.watermark
        {
            return Err(LocalStorageError::InvalidData);
        }
        Ok(())
    }
}

fn validate_restored_set(
    root: &Path,
    expected: &BackupSetManifest,
) -> Result<(), LocalStorageError> {
    validate_set_manifest_shape(expected)?;
    let manifest_path = root.join(BACKUP_SET_MANIFEST_NAME);
    validate_owner_file(&manifest_path)?;
    let manifest: BackupSetManifest = serde_json::from_slice(&fs::read(manifest_path)?)
        .map_err(|_| LocalStorageError::InvalidData)?;
    if &manifest != expected || validate_set_manifest_shape(&manifest).is_err() {
        return Err(LocalStorageError::InvalidData);
    }
    for (name, expected_manifest) in [
        ("control.db", &manifest.control),
        ("runtime.db", &manifest.runtime),
        ("secrets.db", &manifest.secrets),
    ] {
        let restored = SqliteBackup::open(root.join(name))?;
        if restored.manifest() != expected_manifest {
            return Err(LocalStorageError::InvalidData);
        }
    }
    Ok(())
}

fn validate_set_manifest_shape(manifest: &BackupSetManifest) -> Result<(), LocalStorageError> {
    let valid_id = |value: &str| {
        !value.is_empty()
            && value.len() <= 96
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    };
    if manifest.schema != BACKUP_SET_MANIFEST_SCHEMA
        || !valid_id(&manifest.set_id)
        || !valid_id(&manifest.attempt_id)
        || manifest.target_schema_version == 0
        || manifest.watermark.is_empty()
        || manifest.control.database_kind != "control"
        || manifest.runtime.database_kind != "runtime"
        || manifest.secrets.database_kind != "secrets"
        || manifest.control.journal_watermark != manifest.watermark
        || manifest.runtime.journal_watermark != manifest.watermark
        || manifest.secrets.journal_watermark != manifest.watermark
    {
        return Err(LocalStorageError::InvalidData);
    }
    Ok(())
}

fn validate_manifest_against_backups(
    root: &Path,
    manifest: &BackupSetManifest,
) -> Result<(SqliteBackup, SqliteBackup, SqliteBackup), LocalStorageError> {
    let control = SqliteBackup::open(root.join("control.db"))?;
    let runtime = SqliteBackup::open(root.join("runtime.db"))?;
    let secrets = SqliteBackup::open(root.join("secrets.db"))?;
    if control.manifest != manifest.control
        || runtime.manifest != manifest.runtime
        || secrets.manifest != manifest.secrets
    {
        return Err(LocalStorageError::InvalidData);
    }
    Ok((control, runtime, secrets))
}

fn valid_phase_transition(from: BackupSetPhase, to: BackupSetPhase) -> bool {
    matches!(
        (from, to),
        (
            BackupSetPhase::SourcePublished,
            BackupSetPhase::ControlMigrated
        ) | (
            BackupSetPhase::ControlMigrated,
            BackupSetPhase::RuntimeMigrated
        ) | (
            BackupSetPhase::RuntimeMigrated,
            BackupSetPhase::SecretsMigrated
        ) | (BackupSetPhase::SecretsMigrated, BackupSetPhase::Completed)
            | (
                BackupSetPhase::SourcePublished
                    | BackupSetPhase::ControlMigrated
                    | BackupSetPhase::RuntimeMigrated
                    | BackupSetPhase::SecretsMigrated,
                BackupSetPhase::Restoring
            )
            | (BackupSetPhase::Restoring, BackupSetPhase::Restored)
    )
}

struct DatabaseIdentity {
    kind: String,
    store_uuid: String,
    schema_version: u32,
    key_id: Option<String>,
}

pub(crate) fn database_state_digest(
    connection: &Connection,
) -> Result<CanonicalDigest, LocalStorageError> {
    let user_version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let mut hasher = Sha256::new();
    digest_field(&mut hasher, b"hiroute.sqlite-logical-state/v1");
    digest_field(&mut hasher, &user_version.to_le_bytes());

    let schema = {
        let mut statement = connection.prepare(
            "SELECT type, name, coalesce(sql, '') FROM sqlite_schema
             WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
        )?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    for (kind, name, sql) in &schema {
        digest_field(&mut hasher, kind.as_bytes());
        digest_field(&mut hasher, name.as_bytes());
        digest_field(&mut hasher, sql.as_bytes());
    }

    for (_, table, _) in schema.iter().filter(|(kind, _, _)| kind == "table") {
        let columns = {
            let mut statement =
                connection.prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")?;
            statement
                .query_map([table], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        if columns.is_empty() {
            return Err(LocalStorageError::InvalidData);
        }
        digest_field(&mut hasher, table.as_bytes());
        for column in &columns {
            digest_field(&mut hasher, column.as_bytes());
        }
        let projection = columns
            .iter()
            .map(|column| quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        let query = format!(
            "SELECT {projection} FROM {} ORDER BY {projection}",
            quote_identifier(table)
        );
        let mut statement = connection.prepare(&query)?;
        let column_count = statement.column_count();
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            digest_field(&mut hasher, b"row");
            for index in 0..column_count {
                match row.get_ref(index)? {
                    ValueRef::Null => digest_field(&mut hasher, b"null"),
                    ValueRef::Integer(value) => {
                        digest_field(&mut hasher, b"integer");
                        digest_field(&mut hasher, &value.to_le_bytes());
                    }
                    ValueRef::Real(value) => {
                        digest_field(&mut hasher, b"real");
                        digest_field(&mut hasher, &value.to_bits().to_le_bytes());
                    }
                    ValueRef::Text(value) => {
                        digest_field(&mut hasher, b"text");
                        digest_field(&mut hasher, value);
                    }
                    ValueRef::Blob(value) => {
                        digest_field(&mut hasher, b"blob");
                        digest_field(&mut hasher, value);
                    }
                }
            }
        }
    }
    CanonicalDigest::parse(format!("sha256:{}", encode_hex(&hasher.finalize())))
        .map_err(|_| LocalStorageError::InvalidData)
}

fn digest_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value);
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn database_identity(connection: &Connection) -> Result<DatabaseIdentity, LocalStorageError> {
    let schema_version = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let table_exists = |name: &str| -> Result<bool, LocalStorageError> {
        Ok(connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            params![name],
            |row| row.get(0),
        )?)
    };
    if table_exists("operations")? {
        let (store_uuid, key_id) = generic_store_binding(connection)?;
        Ok(DatabaseIdentity {
            kind: "control".to_owned(),
            store_uuid,
            schema_version,
            key_id,
        })
    } else if table_exists("runtime_state")? {
        let (store_uuid, key_id) = generic_store_binding(connection)?;
        Ok(DatabaseIdentity {
            kind: "runtime".to_owned(),
            store_uuid,
            schema_version,
            key_id,
        })
    } else if table_exists("secret_store_meta")? {
        let binding = if schema_version >= 3 {
            connection
                .query_row(
                    "SELECT store_uuid, key_id FROM secret_store_meta WHERE singleton = 1",
                    [],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?
        } else {
            None
        };
        Ok(DatabaseIdentity {
            kind: "secrets".to_owned(),
            store_uuid: binding
                .as_ref()
                .map(|binding| binding.0.clone())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "legacy-unbound".to_owned()),
            schema_version,
            key_id: binding
                .map(|binding| binding.1)
                .filter(|value| !value.is_empty()),
        })
    } else {
        Err(LocalStorageError::InvalidData)
    }
}

/// A manually restored, completed source may have new business rows. Match its identity,
/// rather than the old state digest; interrupted migration still uses its exact source digest.
pub(crate) fn validate_restored_source_identity(
    connection: &Connection,
    source: &BackupManifest,
) -> Result<(), LocalStorageError> {
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    let identity = database_identity(connection)?;
    if integrity != "ok"
        || source.store_uuid.is_empty()
        || source.store_uuid == "legacy-unbound"
        || identity.kind != source.database_kind
        || identity.store_uuid != source.store_uuid
        || identity.schema_version != source.schema_version
        || identity.key_id != source.key_id
    {
        return Err(LocalStorageError::InvalidData);
    }
    Ok(())
}

fn generic_store_binding(
    connection: &Connection,
) -> Result<(String, Option<String>), LocalStorageError> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'storage_meta')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(("legacy-unbound".to_owned(), None));
    }
    Ok(connection
        .query_row(
            "SELECT store_uuid, key_id FROM storage_meta WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .unwrap_or_else(|| ("legacy-unbound".to_owned(), None)))
}

fn standalone_watermark(connection: &Connection) -> Result<String, LocalStorageError> {
    let identity = database_identity(connection)?;
    if identity.kind == "control" {
        control_watermark(connection)
    } else {
        Ok(format!(
            "standalone:{}:{}",
            identity.kind, identity.schema_version
        ))
    }
}

fn control_watermark(connection: &Connection) -> Result<String, LocalStorageError> {
    let (count, generation): (u64, u64) = connection.query_row(
        "SELECT count(*), coalesce(max(generation), 0) FROM operations",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(format!("control:{count}:{generation}"))
}

fn manifest_exact_except_attempt(left: &BackupManifest, right: &BackupManifest) -> bool {
    left.schema == right.schema
        && left.database_kind == right.database_kind
        && left.store_uuid == right.store_uuid
        && left.schema_version == right.schema_version
        && left.source_digest == right.source_digest
        && left.source_state_digest == right.source_state_digest
        && left.key_id == right.key_id
        && left.journal_watermark == right.journal_watermark
}

fn manifest_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(".manifest.json");
    PathBuf::from(value)
}

fn set_manifest_path(root: &Path) -> PathBuf {
    root.join(BACKUP_SET_MANIFEST_NAME)
}

fn set_manifest_stage_prefix() -> &'static str {
    ".backup-set.manifest."
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SetManifestStageKind {
    Writing,
    Committed,
}

fn set_manifest_stages(
    root: &Path,
) -> Result<Vec<(PathBuf, SetManifestStageKind)>, LocalStorageError> {
    let mut stages = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(set_manifest_stage_prefix()) || !name.ends_with(".stage.json") {
            continue;
        }
        let remainder = &name[set_manifest_stage_prefix().len()..];
        let (attempt, kind) = if let Some(attempt) = remainder.strip_suffix(".committed.stage.json")
        {
            (attempt, SetManifestStageKind::Committed)
        } else if let Some(attempt) = remainder.strip_suffix(".writing.stage.json") {
            (attempt, SetManifestStageKind::Writing)
        } else if let Some(attempt) = remainder.strip_suffix(".stage.json") {
            // Legacy stages were visible from file creation through fsync and therefore carry no
            // proof that their bytes were ever committed. Never promote them, even when parsing
            // happens to succeed.
            (attempt, SetManifestStageKind::Writing)
        } else {
            return Err(LocalStorageError::InvalidData);
        };
        if attempt.is_empty()
            || attempt.len() > 96
            || !attempt
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(LocalStorageError::InvalidData);
        }
        stages.push((entry.path(), kind));
    }
    Ok(stages)
}

fn publish_set_manifest(
    root: &Path,
    manifest: &BackupSetManifest,
    replace: bool,
) -> Result<(), LocalStorageError> {
    validate_set_manifest_shape(manifest)?;
    let attempt = random_attempt_id()?;
    let writing = root.join(format!(
        "{}{attempt}.writing.stage.json",
        set_manifest_stage_prefix()
    ));
    let committed = root.join(format!(
        "{}{attempt}.committed.stage.json",
        set_manifest_stage_prefix()
    ));
    write_set_manifest(&writing, manifest)?;
    // The committed name is the durable proof that the complete JSON bytes were fsynced. A
    // writing name can therefore always be discarded after a crash without attempting to parse
    // bytes that may be empty or truncated.
    File::open(root)?.sync_all()?;
    fs::rename(&writing, &committed)?;
    File::open(root)?.sync_all()?;
    let final_path = set_manifest_path(root);
    if !replace && final_path.exists() {
        return Err(LocalStorageError::InvalidData);
    }
    fs::rename(&committed, &final_path)?;
    File::open(root)?.sync_all()?;
    validate_owner_file(&final_path)
}

fn reconcile_set_manifest_publication(root: &Path) -> Result<bool, LocalStorageError> {
    let final_path = set_manifest_path(root);
    let stages = set_manifest_stages(root)?;
    if final_path.exists() {
        validate_owner_file(&final_path)?;
        for (stage, _) in stages {
            tighten_and_remove_owner_orphan(&stage)?;
        }
        File::open(root)?.sync_all()?;
        return Ok(true);
    }

    let mut committed = Vec::new();
    for (stage, kind) in stages {
        match kind {
            SetManifestStageKind::Writing => tighten_and_remove_owner_orphan(&stage)?,
            SetManifestStageKind::Committed => committed.push(stage),
        }
    }
    if committed.is_empty() {
        File::open(root)?.sync_all()?;
        return Ok(false);
    }
    if committed.len() != 1 {
        return Err(LocalStorageError::InvalidData);
    }
    let stage = committed.pop().expect("length checked");
    validate_owner_file(&stage)?;
    let manifest: BackupSetManifest =
        serde_json::from_slice(&fs::read(&stage)?).map_err(|_| LocalStorageError::InvalidData)?;
    validate_set_manifest_shape(&manifest)?;
    validate_manifest_against_backups(root, &manifest)?;
    fs::rename(stage, &final_path)?;
    File::open(root)?.sync_all()?;
    validate_owner_file(&final_path)?;
    Ok(true)
}

fn write_manifest(path: &Path, manifest: &BackupManifest) -> Result<(), LocalStorageError> {
    let bytes = serde_json::to_vec(manifest).map_err(|_| LocalStorageError::InvalidData)?;
    write_owner_file(path, &bytes)
}

fn write_set_manifest(path: &Path, manifest: &BackupSetManifest) -> Result<(), LocalStorageError> {
    let bytes = serde_json::to_vec(manifest).map_err(|_| LocalStorageError::InvalidData)?;
    write_owner_file(path, &bytes)
}

fn write_owner_file(path: &Path, bytes: &[u8]) -> Result<(), LocalStorageError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    use std::io::Write as _;
    file.write_all(bytes)?;
    file.sync_all()?;
    validate_owner_file(path)
}

fn file_digest(path: &Path) -> Result<CanonicalDigest, LocalStorageError> {
    Ok(CanonicalDigest::of_bytes(&fs::read(path)?))
}

fn random_attempt_id() -> Result<String, LocalStorageError> {
    let mut bytes = [0_u8; 12];
    getrandom::fill(&mut bytes).map_err(|_| LocalStorageError::Crypto)?;
    Ok(encode_hex(&bytes))
}

fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn cleanup_orphan_stages(parent: &Path, file_name: &str) -> Result<(), LocalStorageError> {
    let prefix = format!(".{file_name}.");
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(&prefix) && name.contains(".stage.") {
            tighten_and_remove_owner_orphan(&entry.path())?;
        }
    }
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn tighten_and_remove_owner_orphan(path: &Path) -> Result<(), LocalStorageError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(LocalStorageError::Permission);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(LocalStorageError::Permission);
        }
    }
    validate_owner_file(path)?;
    fs::remove_file(path)?;
    Ok(())
}

fn validate_database(path: &Path) -> Result<(), LocalStorageError> {
    let connection = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let result: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if result == "ok" {
        Ok(())
    } else {
        Err(LocalStorageError::InvalidData)
    }
}

fn prepare_owner_directory(path: &Path) -> Result<(), LocalStorageError> {
    if path.exists() {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
            return Err(LocalStorageError::Permission);
        }
    } else {
        fs::create_dir_all(path)?;
        set_owner_directory_permissions(path)?;
    }
    Ok(())
}

fn set_owner_directory_permissions(path: &Path) -> Result<(), LocalStorageError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
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

fn validate_owner_file(path: &Path) -> Result<(), LocalStorageError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(LocalStorageError::Permission);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::test_tempdir as tempdir;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::{ControlStore, LocalSecretStore, RuntimeStore};

    #[test]
    fn unique_stage_and_stale_final_do_not_block_exact_publication_and_restore() {
        let directory = tempdir().unwrap();
        let root = directory.path().join("data");
        let control = ControlStore::open(
            &crate::test_storage_authority(),
            root.join("control.db"),
            root.join("migration"),
        )
        .unwrap();
        let backup_path = root.join("backups/control.db");
        let first = control
            .with_connection(|connection| {
                SqliteBackup::create(&crate::test_storage_authority(), connection, &backup_path)
            })
            .unwrap();
        fs::write(manifest_path(&backup_path), b"stale-final").unwrap();
        let parent = backup_path.parent().unwrap();
        let orphan = parent.join(".control.db.deadbeef.stage.db");
        fs::write(&orphan, b"orphan").unwrap();
        set_owner_file_permissions(&orphan).unwrap();
        let second = control
            .with_connection(|connection| {
                SqliteBackup::create(&crate::test_storage_authority(), connection, &backup_path)
            })
            .unwrap();
        assert!(second.manifest().attempt_id != first.manifest().attempt_id);
        assert!(!orphan.exists());
        let restored = root.join("restored/control.db");
        second.restore_to(&restored).unwrap();
        validate_database(&restored).unwrap();
    }

    #[test]
    fn half_published_database_or_manifest_is_reconciled_by_a_fresh_attempt() {
        let directory = tempdir().unwrap();
        let root = directory.path().join("data");
        let control = ControlStore::open(
            &crate::test_storage_authority(),
            root.join("control.db"),
            root.join("migration"),
        )
        .unwrap();
        let backup_path = root.join("backups/control.db");

        control
            .with_connection(|connection| {
                SqliteBackup::create(&crate::test_storage_authority(), connection, &backup_path)
            })
            .unwrap();
        fs::remove_file(manifest_path(&backup_path)).unwrap();
        let recovered_database_only = control
            .with_connection(|connection| {
                SqliteBackup::create(&crate::test_storage_authority(), connection, &backup_path)
            })
            .unwrap();
        SqliteBackup::open(recovered_database_only.path()).unwrap();

        fs::remove_file(&backup_path).unwrap();
        let recovered_manifest_only = control
            .with_connection(|connection| {
                SqliteBackup::create(&crate::test_storage_authority(), connection, &backup_path)
            })
            .unwrap();
        SqliteBackup::open(recovered_manifest_only.path()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn vacuum_created_0644_stage_is_tightened_and_reconciled_across_two_restarts() {
        use std::os::unix::fs::MetadataExt;

        let directory = tempdir().unwrap();
        let root = directory.path().join("data");
        let control = ControlStore::open(
            &crate::test_storage_authority(),
            root.join("control.db"),
            root.join("migration"),
        )
        .unwrap();
        control.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO dependency_revisions(workspace_id, dependency_key, revision)
                     VALUES ('personal/default', 'prices', 9)",
                    [],
                )
                .unwrap();
        });
        let backup_path = root.join("backups/control.db");
        let parent = backup_path.parent().unwrap();
        prepare_owner_directory(parent).unwrap();

        for crash_id in ["after-vacuum-one", "after-vacuum-two"] {
            let orphan = parent.join(format!(".control.db.{crash_id}.stage.db"));
            control.with_connection(|connection| {
                connection
                    .execute_batch("PRAGMA wal_checkpoint(FULL)")
                    .unwrap();
                connection
                    .execute("VACUUM main INTO ?1", [orphan.to_string_lossy().as_ref()])
                    .unwrap();
            });
            fs::set_permissions(&orphan, fs::Permissions::from_mode(0o644)).unwrap();
            let metadata = fs::symlink_metadata(&orphan).unwrap();
            assert_eq!(metadata.mode() & 0o777, 0o644);
            assert_eq!(metadata.uid(), rustix::process::getuid().as_raw());

            let backup = control
                .with_connection(|connection| {
                    SqliteBackup::create(&crate::test_storage_authority(), connection, &backup_path)
                })
                .unwrap();
            assert!(!orphan.exists());
            assert_eq!(
                backup.manifest().source_state_digest,
                control.with_connection(|connection| database_state_digest(connection).unwrap())
            );
        }

        let reopened = SqliteBackup::open(&backup_path).unwrap();
        let restored = root.join("restored/control.db");
        reopened.restore_to(&restored).unwrap();
        let restored = Connection::open_with_flags(
            restored,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        assert_eq!(
            database_state_digest(&restored).unwrap(),
            reopened.manifest().source_state_digest
        );
    }

    #[cfg(unix)]
    #[test]
    fn orphan_stage_cleanup_rejects_a_symlink_without_touching_its_target() {
        use std::os::unix::fs::symlink;

        let directory = tempdir().unwrap();
        let root = directory.path().join("data");
        let control = ControlStore::open(
            &crate::test_storage_authority(),
            root.join("control.db"),
            root.join("migration"),
        )
        .unwrap();
        let backup_path = root.join("backups/control.db");
        let parent = backup_path.parent().unwrap();
        prepare_owner_directory(parent).unwrap();
        let target = root.join("must-not-change");
        fs::write(&target, b"owner-data").unwrap();
        set_owner_file_permissions(&target).unwrap();
        let orphan = parent.join(".control.db.symlink.stage.db");
        symlink(&target, &orphan).unwrap();

        assert!(
            control
                .with_connection(|connection| {
                    SqliteBackup::create(&crate::test_storage_authority(), connection, &backup_path)
                })
                .is_err()
        );
        assert!(
            fs::symlink_metadata(&orphan)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(target).unwrap(), b"owner-data");
    }

    #[test]
    fn backup_set_manifest_reopens_publish_boundaries_and_rejects_mismatch() {
        let directory = tempdir().unwrap();
        let root = directory.path().join("data");
        let control = ControlStore::open(
            &crate::test_storage_authority(),
            root.join("control.db"),
            root.join("migration"),
        )
        .unwrap();
        let runtime = RuntimeStore::open(
            &crate::test_storage_authority(),
            root.join("runtime.db"),
            root.join("migration"),
        )
        .unwrap();
        let secrets = LocalSecretStore::open(
            &crate::test_storage_authority(),
            root.join("secrets.db"),
            root.join("master-key"),
            root.join("migration"),
        )
        .unwrap();
        let backup_root = root.join("backup-set");
        let set = control
            .with_connection(|control_connection| {
                runtime.with_connection(|runtime_connection| {
                    secrets.with_connection(|secret_connection| {
                        BackupSet::create(
                            &test_writer_barrier(),
                            &backup_root,
                            control_connection,
                            runtime_connection,
                            secret_connection,
                        )
                    })
                })
            })
            .unwrap();
        let set_id = set.set_id().to_owned();
        drop(set);

        let final_manifest = set_manifest_path(&backup_root);
        let stage_manifest = backup_root.join(format!(
            "{}crash-before-rename.committed.stage.json",
            set_manifest_stage_prefix()
        ));
        fs::rename(&final_manifest, &stage_manifest).unwrap();
        File::open(&backup_root).unwrap().sync_all().unwrap();
        let reopened = BackupSet::open(&crate::test_storage_authority(), &backup_root).unwrap();
        assert_eq!(reopened.set_id(), set_id);
        assert_eq!(reopened.phase(), BackupSetPhase::SourcePublished);
        assert!(final_manifest.exists());
        assert!(!stage_manifest.exists());

        let mut staged_phase = reopened.set_manifest();
        staged_phase.phase = BackupSetPhase::ControlMigrated;
        let phase_stage = backup_root.join(format!(
            "{}phase-before-rename.committed.stage.json",
            set_manifest_stage_prefix()
        ));
        write_set_manifest(&phase_stage, &staged_phase).unwrap();
        let reopened = BackupSet::open(&crate::test_storage_authority(), &backup_root).unwrap();
        assert_eq!(reopened.phase(), BackupSetPhase::SourcePublished);
        assert!(!phase_stage.exists());

        let original = fs::read(&final_manifest).unwrap();
        let mut mismatch: BackupSetManifest = serde_json::from_slice(&original).unwrap();
        mismatch.control.source_digest = CanonicalDigest::of_bytes(b"mismatched-set");
        fs::write(&final_manifest, serde_json::to_vec(&mismatch).unwrap()).unwrap();
        assert!(BackupSet::open(&crate::test_storage_authority(), &backup_root).is_err());
        fs::write(&final_manifest, original).unwrap();

        let exact = BackupSet::open(&crate::test_storage_authority(), &backup_root).unwrap();
        let restored = root.join("restored-source-set");
        exact
            .restore_to_new_root(&test_writer_barrier(), &restored)
            .unwrap();
        for (name, backup) in [
            ("control.db", &exact.control),
            ("runtime.db", &exact.runtime),
            ("secrets.db", &exact.secrets),
        ] {
            let connection = Connection::open_with_flags(
                restored.join(name),
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .unwrap();
            assert_eq!(
                database_state_digest(&connection).unwrap(),
                backup.manifest().source_state_digest
            );
        }
    }

    #[test]
    fn backup_set_has_one_watermark_and_refuses_an_admitted_writer() {
        let directory = tempdir().unwrap();
        let root = directory.path().join("data");
        let control = ControlStore::open(
            &crate::test_storage_authority(),
            root.join("control.db"),
            root.join("migration"),
        )
        .unwrap();
        let runtime = RuntimeStore::open(
            &crate::test_storage_authority(),
            root.join("runtime.db"),
            root.join("migration"),
        )
        .unwrap();
        let secrets = LocalSecretStore::open(
            &crate::test_storage_authority(),
            root.join("secrets.db"),
            root.join("master-key"),
            root.join("migration"),
        )
        .unwrap();
        let set = control
            .with_connection(|control_connection| {
                runtime.with_connection(|runtime_connection| {
                    secrets.with_connection(|secret_connection| {
                        BackupSet::create(
                            &test_writer_barrier(),
                            root.join("backup-set"),
                            control_connection,
                            runtime_connection,
                            secret_connection,
                        )
                    })
                })
            })
            .unwrap();
        assert_eq!(set.control.manifest().journal_watermark, set.watermark());
        assert_eq!(set.runtime.manifest().journal_watermark, set.watermark());
        assert_eq!(set.secrets.manifest().journal_watermark, set.watermark());
        let restored = root.join("restored-set");
        set.restore_to_new_root(&test_writer_barrier(), &restored)
            .unwrap();
        set.restore_to_new_root(&test_writer_barrier(), &restored)
            .unwrap();
        for name in ["control.db", "runtime.db", "secrets.db"] {
            SqliteBackup::open(restored.join(name)).unwrap();
        }
        control.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO writer_claim(singleton, operation_id, admitted_at)
                     VALUES (1, 'op_11111111111111111111111111111111', unixepoch())",
                    [],
                )
                .unwrap();
        });
        assert!(
            control
                .with_connection(|control_connection| {
                    runtime.with_connection(|runtime_connection| {
                        secrets.with_connection(|secret_connection| {
                            BackupSet::create(
                                &test_writer_barrier(),
                                root.join("blocked-set"),
                                control_connection,
                                runtime_connection,
                                secret_connection,
                            )
                        })
                    })
                })
                .is_err()
        );
    }
}
