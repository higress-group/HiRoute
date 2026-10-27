#![forbid(unsafe_code)]

//! Owner-only local adapters for the Application transaction ports.
//!
//! `control.db`, `runtime.db`, and `secrets.db` remain physically separate. Operations use a
//! durable saga journal; no code claims an ACID transaction across SQLite files or managed
//! artifacts.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod agents;
pub mod backup;
pub mod control;
pub mod migrations;
pub mod runtime;
pub mod secrets;

pub use agents::*;
pub use backup::{BackupSet, SingleWriterBackupBarrier, SqliteBackup};
pub use control::{
    ApplyCapabilityRegistrar, ApplyCapabilityRegistrationV1, ComputeSubscriptionValidationRecordV1,
    ComputeSubscriptionValidationStateV1, ControlReadStamp, ControlStore, ManagedArtifactStore,
    SucceededAgentOperationV1,
};
pub use runtime::RuntimeStore;
pub use secrets::{LocalNativeCredentialAuthority, LocalSecretStore};

use std::path::Path;

/// The only production-shaped entrypoint for opening the three local SQLite stores. Raw
/// per-store open/migrate functions remain crate-private so daemon composition cannot upgrade one
/// database without first publishing the durable source BackupSet.
pub struct LocalStorageSet {
    control: ControlStore,
    runtime: RuntimeStore,
    secrets: LocalSecretStore,
}

#[derive(Clone, Debug, Default)]
pub struct StorageStartupOptions {
    pub gateway_lkg: Option<std::path::PathBuf>,
    /// Seal old Observation unchanged with the complete matching-version source.
    pub preserve_observation_source: bool,
    /// An admitted old package retained by the installer/operator for recovery.
    pub recovery_package: Option<std::path::PathBuf>,
    pub upgrade_progress: Option<hiroute_host_runtime::StorageUpgradeProgress>,
    /// Per-attempt typed facts also reach the caller when admission cannot
    /// acquire the writer lock or write a recovery file.
    pub startup_failure: Option<std::sync::mpsc::Sender<hiroute_host_runtime::StorageRecovery>>,
}
impl StorageStartupOptions {
    pub fn report_failure(&self, recovery: hiroute_host_runtime::StorageRecovery) {
        if let Some(sender) = &self.startup_failure {
            let _ = sender.send(recovery);
        }
    }
}

impl LocalStorageSet {
    /// Retain the storage-owner disposition if fresh Observation initialization
    /// fails after the coordinated core stores opened. `self` keeps their startup
    /// writer exclusion alive while the original batch is verified.
    pub fn record_observation_startup_failure(
        &self,
        storage: &Path,
        cause: hiroute_host_runtime::StorageRecoveryCause,
    ) -> hiroute_host_runtime::StorageRecovery {
        migrations::startup_recovery::publish_failure(
            storage,
            LocalStorageError::InvalidData.with_recovery(
                cause,
                if cause == hiroute_host_runtime::StorageRecoveryCause::MigrationFailed
                    && matches!(std::fs::symlink_metadata(storage.join("observation")),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound)
                {
                    hiroute_host_runtime::StorageRecoveryAction::ResumeCurrent
                } else {
                    hiroute_host_runtime::StorageRecoveryAction::PreserveAndDiagnose
                },
            ),
        )
        .recovery()
    }

    /// Provisional read-only checks; startup repeats admission under its writer lock.
    pub fn preflight_upgrade(
        storage: &Path,
        package: &Path,
        seal: bool,
    ) -> Result<u64, LocalStorageError> {
        if !storage.exists() {
            return Ok(0);
        }
        migrations::validate_startup_format(storage)?;
        if !seal {
            return Ok(0);
        }
        LocalSecretStore::migration_binding(
            &storage.join("live/secrets.db"),
            &storage.join("master-key"),
        )?
        .ok_or(LocalStorageError::Locked)?;
        migrations::validate_owner_file(&storage.join("observation-hmac-key"))?;
        if std::fs::metadata(storage.join("observation-hmac-key"))?.len() != 32 {
            return Err(LocalStorageError::Locked);
        }
        match std::fs::symlink_metadata(package) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(LocalStorageError::InvalidData.with_recovery(
                    hiroute_host_runtime::StorageRecoveryCause::InvalidPackage,
                    hiroute_host_runtime::StorageRecoveryAction::PreserveAndDiagnose,
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(LocalStorageError::Io(error).with_recovery(
                    hiroute_host_runtime::StorageRecoveryCause::MissingPackage,
                    hiroute_host_runtime::StorageRecoveryAction::PreserveAndDiagnose,
                ));
            }
            Err(error) => return Err(LocalStorageError::Io(error)),
        }
        backup::source_snapshot::preflight_space(
            storage,
            None,
            &StorageStartupOptions {
                preserve_observation_source: true,
                recovery_package: Some(package.to_path_buf()),
                ..Default::default()
            },
        )
    }

    /// A published source link also protects the intentionally absent live root
    /// between the old-directory rename and fresh Observation initialization.
    pub fn observation_cutover_pending(storage_root: &Path) -> bool {
        storage_root.join("observation-source-backup.json").exists()
            && !storage_root.join("observation/activity.db").exists()
    }

    /// Opens and migrates the coordinated three-store set before daemon listeners or
    /// transaction admission become available. The authority and writer barrier are minted only
    /// inside this crate so daemon composition cannot open an individual store or assert an
    /// ambient backup barrier.
    pub fn open_for_daemon_startup(
        storage_root: impl AsRef<Path>,
    ) -> Result<Self, LocalStorageError> {
        Self::open_for_daemon_startup_with_options(storage_root, &StorageStartupOptions::default())
    }

    pub fn open_for_daemon_startup_with_options(
        storage_root: impl AsRef<Path>,
        options: &StorageStartupOptions,
    ) -> Result<Self, LocalStorageError> {
        let authority = daemon_storage_authority();
        let barrier = backup::daemon_startup_barrier();
        Self::open_with_options(&authority, &barrier, storage_root, options)
    }

    fn open_with_options(
        authority: &DaemonStorageAuthority,
        barrier: &SingleWriterBackupBarrier,
        storage_root: impl AsRef<Path>,
        options: &StorageStartupOptions,
    ) -> Result<Self, LocalStorageError> {
        let storage_root = storage_root.as_ref();
        let result = (|| {
            migrations::prepare_storage_root(storage_root)?;
            let startup_lock = std::sync::Arc::new(migrations::acquire_startup_lock(storage_root)?);
            hiroute_host_runtime::StorageRecovery::clear(storage_root)?;
            let result = migrations::validate_startup_format(storage_root).and_then(|()| {
                Self::open_locked_stores(
                    authority,
                    barrier,
                    storage_root,
                    startup_lock.clone(),
                    Some(options),
                )
            });
            result
                .map_err(|error| migrations::startup_recovery::publish_failure(storage_root, error))
        })();
        result.inspect_err(|error: &LocalStorageError| options.report_failure(error.recovery()))
    }

    // SQL coordinator component harness only: unsupported formats never pass through the
    // production startup entry. Retains the coordinator's crash/backup assertions independently.
    #[cfg(test)]
    pub(crate) fn open_migration_component_fixture(
        authority: &DaemonStorageAuthority,
        barrier: &SingleWriterBackupBarrier,
        storage_root: &Path,
    ) -> Result<Self, LocalStorageError> {
        migrations::prepare_storage_root(storage_root)?;
        let lock = std::sync::Arc::new(migrations::acquire_startup_lock(storage_root)?);
        Self::open_locked_stores(authority, barrier, storage_root, lock, None)
    }

    fn open_locked_stores(
        authority: &DaemonStorageAuthority,
        barrier: &SingleWriterBackupBarrier,
        storage_root: &Path,
        startup_lock: std::sync::Arc<std::fs::File>,
        options: Option<&StorageStartupOptions>,
    ) -> Result<Self, LocalStorageError> {
        let live_root = storage_root.join("live");
        let backup_root = upgrade_backup_root(storage_root)?;
        migrations::prepare_storage_root(
            backup_root.parent().ok_or(LocalStorageError::InvalidData)?,
        )?;
        let previous = storage_root.join("migration-set");
        if previous.exists() {
            if backup_root.exists() {
                return Err(LocalStorageError::InvalidData);
            }
            hiroute_platform::durable::rename(previous, &backup_root)?;
            hiroute_platform::durable::directory_barrier(storage_root)?;
            hiroute_platform::durable::directory_barrier(
                backup_root.parent().ok_or(LocalStorageError::InvalidData)?,
            )?;
        }
        let master_key_path = storage_root.join("master-key");
        let control_path = live_root.join("control.db");
        let runtime_path = live_root.join("runtime.db");
        let secrets_path = live_root.join("secrets.db");
        let secret_binding = LocalSecretStore::migration_binding(&secrets_path, &master_key_path)?;
        let mut migration = migrations::MigrationSetCoordinator::prepare_with_progress(
            authority,
            barrier,
            &live_root,
            &backup_root,
            secret_binding.as_ref(),
            options.and_then(|options| options.upgrade_progress.as_ref()),
        )?;
        if let Some(options) = options {
            migration.preserve_complete_source(
                authority,
                barrier,
                storage_root,
                &backup_root,
                options,
                secret_binding.as_ref(),
            )?;
        }
        let target_schema_version = migration.target_schema_version();
        if (migration.durable_set_prepared()
            || options.is_some_and(|options| options.preserve_observation_source))
            && let Some(progress) = options.and_then(|options| options.upgrade_progress.as_ref())
        {
            progress.report(hiroute_host_runtime::StorageUpgradePhase::Conversion);
        }
        let control_result = if migration.durable_set_prepared() {
            ControlStore::open_from_migration_set(
                authority,
                &control_path,
                &backup_root,
                migration
                    .expected_control_store_uuid()
                    .ok_or(LocalStorageError::InvalidData)?,
                target_schema_version,
            )
        } else {
            ControlStore::open(authority, &control_path, &backup_root)
        };
        let mut control = match control_result {
            Ok(control) => control,
            Err(error) => {
                migration.restore_after_failure(barrier, &live_root)?;
                return Err(error);
            }
        };
        if let Err(error) = migration.mark_control_migrated(barrier) {
            drop(control);
            migration.restore_after_failure(barrier, &live_root)?;
            return Err(error);
        }

        let runtime_result = if migration.durable_set_prepared() {
            RuntimeStore::open_from_migration_set(
                authority,
                &runtime_path,
                &backup_root,
                migration
                    .expected_runtime_store_uuid()
                    .ok_or(LocalStorageError::InvalidData)?,
                target_schema_version,
            )
        } else {
            RuntimeStore::open(authority, &runtime_path, &backup_root)
        };
        let mut runtime = match runtime_result {
            Ok(runtime) => runtime,
            Err(error) => {
                drop(control);
                migration.restore_after_failure(barrier, &live_root)?;
                return Err(error);
            }
        };
        if let Err(error) = migration.mark_runtime_migrated(barrier) {
            drop(runtime);
            drop(control);
            migration.restore_after_failure(barrier, &live_root)?;
            return Err(error);
        }

        let secrets_result = if migration.durable_set_prepared() {
            LocalSecretStore::open_from_migration_set(
                authority,
                &secrets_path,
                &master_key_path,
                &backup_root,
                secret_binding.as_ref().ok_or(LocalStorageError::Locked)?,
                target_schema_version,
            )
        } else {
            LocalSecretStore::open(authority, &secrets_path, &master_key_path, &backup_root)
        };
        let mut secrets = match secrets_result {
            Ok(secrets) => secrets,
            Err(error) => {
                drop(runtime);
                drop(control);
                migration.restore_after_failure(barrier, &live_root)?;
                return Err(error);
            }
        };
        if let Err(error) = migration.mark_secrets_migrated(barrier) {
            drop(secrets);
            drop(runtime);
            drop(control);
            migration.restore_after_failure(barrier, &live_root)?;
            return Err(error);
        }
        if migration.durable_set_prepared()
            && target_schema_version == migrations::LATEST_SCHEMA_VERSION
            && let Some(progress) = options.and_then(|options| options.upgrade_progress.as_ref())
        {
            progress.report(hiroute_host_runtime::StorageUpgradePhase::Validation);
        }
        let integrity = if migration.durable_set_prepared() {
            // A first format conversion audits its complete source/target. Ordinary opens
            // establish only the complete current dependency closure, not all-history health.
            migrations::audit_current_storage(&control, &runtime, &secrets)
                .and_then(|_| migrations::validate_current_storage(&control, &runtime, &secrets))
        } else {
            migrations::validate_current_storage(&control, &runtime, &secrets)
        };
        if let Err(error) =
            integrity.map_err(|error| error.at_upgrade_stage("current storage validation"))
        {
            drop(secrets);
            drop(runtime);
            drop(control);
            migration.restore_after_failure(barrier, &live_root)?;
            return Err(error);
        }
        if let Err(error) = migration.mark_completed(barrier) {
            drop(secrets);
            drop(runtime);
            drop(control);
            migration.restore_after_failure(barrier, &live_root)?;
            return Err(error);
        }
        if target_schema_version < migrations::LATEST_SCHEMA_VERSION {
            // A prior binary's accepted batch must finish under its own target and backup.
            // Keep the startup lock while reopening: prepare archives that completed batch
            // and publishes a fresh source backup for the current version. Admission opens
            // only after the current batch also completes.
            drop(secrets);
            drop(runtime);
            drop(control);
            drop(migration);
            return Self::open_locked_stores(
                authority,
                barrier,
                storage_root,
                startup_lock,
                options,
            );
        }
        control.startup_lock = Some(startup_lock.clone());
        runtime.startup_lock = Some(startup_lock.clone());
        secrets.startup_lock = Some(startup_lock);
        Ok(Self {
            control,
            runtime,
            secrets,
        })
    }

    /// Validate cross-store grant/publication equality after Operation recovery and before
    /// serving admission opens. Store opening deliberately admits recoverable saga checkpoints.
    pub fn validate_recovered_grant_publications(&self) -> Result<(), LocalStorageError> {
        migrations::validate_recovered_grant_publications(&self.control, &self.secrets)
    }

    pub fn control(&self) -> &ControlStore {
        &self.control
    }

    pub fn runtime(&self) -> &RuntimeStore {
        &self.runtime
    }

    pub fn secrets(&self) -> &LocalSecretStore {
        &self.secrets
    }

    /// Transfers the three independently owned stores to their final composition adapters.
    pub fn into_parts(self) -> (ControlStore, RuntimeStore, LocalSecretStore) {
        (self.control, self.runtime, self.secrets)
    }

    /// Narrow capability persistence seam for the daemon's protected launcher only.
    pub fn apply_capability_registrar(&self) -> ApplyCapabilityRegistrar<'_> {
        ApplyCapabilityRegistrar::new(&self.control)
    }

    /// Opens the operation journal's managed publication/Agent-artifact adapter after the
    /// coordinated store set has completed startup. The daemon receives the narrow adapter, not
    /// the raw storage authority used to construct it.
    pub fn open_managed_artifacts(
        &self,
        root: impl AsRef<Path>,
        restore_root: impl AsRef<Path>,
    ) -> Result<ManagedArtifactStore, LocalStorageError> {
        ManagedArtifactStore::open(&daemon_storage_authority(), root, restore_root)
    }

    /// Register all native targets, then authenticate current recovery roots and unresolved
    /// work. Optional historical restore records are authenticated on explicit access.
    pub fn open_managed_artifacts_with_external_targets(
        &self,
        root: impl AsRef<Path>,
        restore_root: impl AsRef<Path>,
        targets: impl IntoIterator<Item = (String, std::path::PathBuf)>,
    ) -> Result<ManagedArtifactStore, LocalStorageError> {
        let dependencies = self
            .control
            .artifact_dependencies()
            .map_err(|_| LocalStorageError::InvalidData)?;
        let store = ManagedArtifactStore::open_registered(
            &daemon_storage_authority(),
            root,
            restore_root,
            targets,
        )?;
        store.validate_dependencies(&dependencies)?;
        Ok(store)
    }

    /// Restart-safe factory for a registered native Agent configuration target. Its exact path
    /// binding must be present before durable rendered-effect markers are validated.
    pub fn open_managed_artifacts_with_external_target(
        &self,
        root: impl AsRef<Path>,
        restore_root: impl AsRef<Path>,
        target: impl Into<String>,
        path: impl Into<std::path::PathBuf>,
    ) -> Result<ManagedArtifactStore, LocalStorageError> {
        ManagedArtifactStore::open_with_external_target(
            &daemon_storage_authority(),
            root,
            restore_root,
            target,
            path,
        )
    }
}
/// Upgrade backups live beside the directory restored as a whole, never inside it.
pub fn upgrade_backup_root(storage_root: &Path) -> Result<std::path::PathBuf, LocalStorageError> {
    let mut name = storage_root
        .file_name()
        .ok_or(LocalStorageError::InvalidData)?
        .to_os_string();
    name.push(".upgrade-backups");
    Ok(storage_root
        .parent()
        .ok_or(LocalStorageError::InvalidData)?
        .join(name)
        .join("migration-set"))
}

#[cfg(test)]
mod startup_tests {
    use super::*;
    use hiroute_domain::{
        CanonicalDigest, ControlRepositoryPort, PortErrorCode, ProtectedApplyCapability,
        RevisionSetV1, WorkspaceId,
    };

    #[test]
    fn daemon_startup_opens_the_coordinated_store_set() {
        let directory = crate::test_tempdir().unwrap();
        let storage_root = directory.path().join("storage");
        let stores = LocalStorageSet::open_for_daemon_startup(&storage_root).unwrap();
        let _ = stores.control();
        let _ = stores.runtime();
        let _ = stores.secrets();
        assert!(storage_root.join("live/control.db").is_file());
        assert!(storage_root.join("live/runtime.db").is_file());
        assert!(storage_root.join("live/secrets.db").is_file());
        #[cfg(windows)]
        {
            use hiroute_platform::windows::security;
            for name in [
                "startup.lock",
                "master-key",
                "live/control.db",
                "live/runtime.db",
                "live/secrets.db",
            ] {
                security::verify_owner_only(&storage_root.join(name), false)
                    .unwrap_or_else(|error| panic!("new managed {name} is not private: {error:?}"));
            }
            for name in ["live", ""] {
                security::verify_owner_only(&storage_root.join(name), true).unwrap();
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn daemon_startup_accepts_accessible_existing_root_without_chmod() {
        use std::os::unix::fs::PermissionsExt;

        let directory = crate::test_tempdir().unwrap();
        let root = directory.path().join("storage");
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        LocalStorageSet::open_for_daemon_startup(&root).unwrap();
        assert!(root.join("live").is_dir());
        assert_eq!(
            std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[test]
    fn protected_launcher_registrar_persists_only_a_short_lived_exact_grant() {
        let directory = crate::test_tempdir().unwrap();
        let stores =
            LocalStorageSet::open_for_daemon_startup(directory.path().join("storage")).unwrap();
        let capability = "0123456789abcdef0123456789abcdef0123456789abcdef".to_owned();
        let workspace = WorkspaceId::default();
        let revisions = RevisionSetV1 {
            target: 0,
            dependencies: Default::default(),
        };
        let accepted = CanonicalDigest::of_bytes(b"accepted-change");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let registration = ApplyCapabilityRegistrationV1::from_protected_launcher(
            capability.clone(),
            "interactive-user",
            workspace.clone(),
            "ApplyComputeConnection",
            accepted.clone(),
            revisions.clone(),
            now + 60,
        )
        .unwrap();
        stores
            .apply_capability_registrar()
            .register(registration)
            .unwrap();
        let protected = ProtectedApplyCapability::new(capability).unwrap();
        let verified = stores
            .control()
            .verify_apply_authorization(
                &protected,
                &workspace,
                "interactive-user",
                "ApplyComputeConnection",
                &accepted,
                &revisions,
            )
            .unwrap();
        assert_eq!(verified.scope(), "apply:one-shot");

        let too_long = ApplyCapabilityRegistrationV1::from_protected_launcher(
            "abcdef0123456789abcdef0123456789abcdef0123456789".into(),
            "interactive-user",
            workspace,
            "ApplyComputeConnection",
            accepted,
            revisions,
            // A stale now + 301 can enter the permitted 300-second window while the
            // preceding registration/verification runs. An unbounded expiry cannot.
            i64::MAX,
        )
        .unwrap();
        let error = stores
            .apply_capability_registrar()
            .register(too_long)
            .unwrap_err();
        assert_eq!(error.code, PortErrorCode::PermissionDenied);
        assert_eq!(error.context, "control.capability.expiry");
    }

    #[test]
    fn coordinated_set_is_the_only_public_managed_artifact_factory() {
        let directory = crate::test_tempdir().unwrap();
        let stores =
            LocalStorageSet::open_for_daemon_startup(directory.path().join("storage")).unwrap();
        let artifacts = stores
            .open_managed_artifacts(
                directory.path().join("artifacts"),
                directory.path().join("restore"),
            )
            .unwrap();
        assert!(artifacts.read_target("current").unwrap().is_none());
    }
}

/// Seals raw storage adapters until the authenticated daemon composition owns their lifecycle.
/// There is intentionally no public constructor in this PROCESS.
pub struct DaemonStorageAuthority {
    _private: (),
}

pub(crate) fn daemon_storage_authority() -> DaemonStorageAuthority {
    DaemonStorageAuthority { _private: () }
}

/// Test-owned roots must satisfy the same permission contract under every caller umask.
#[cfg(all(test, not(windows)))]
pub(crate) type TestTempDir = tempfile::TempDir;

#[cfg(all(test, windows))]
pub(crate) struct TestTempDir {
    _cleanup: tempfile::TempDir,
    path: std::path::PathBuf,
}

#[cfg(all(test, windows))]
impl TestTempDir {
    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }
}

#[cfg(test)]
pub(crate) fn test_tempdir() -> std::io::Result<TestTempDir> {
    let builder = tempfile::Builder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::PermissionsExt;
        let mut builder = builder;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
        builder
    };
    let cleanup = builder.tempdir()?;
    #[cfg(not(windows))]
    {
        Ok(cleanup)
    }
    #[cfg(windows)]
    {
        let path = cleanup.path().join("private");
        drop(hiroute_platform::windows::security::create_owner_only_directory(&path)?);
        Ok(TestTempDir {
            _cleanup: cleanup,
            path,
        })
    }
}

#[cfg(test)]
pub(crate) fn test_create_dir_all(path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        drop(hiroute_platform::windows::security::ensure_private_directory(path.as_ref())?);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(path)
    }
}

#[cfg(test)]
pub(crate) fn test_storage_authority() -> DaemonStorageAuthority {
    daemon_storage_authority()
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalStorageCapability {
    ControlRepository,
    SecretStore,
    RuntimeStateStore,
    MigrationBackup,
    ConditionalArtifactCompensation,
}

pub const IMPLEMENTATION_STATUS: &str = "transaction-v1";

#[derive(Debug, Error)]
pub enum LocalStorageError {
    #[error(
        "UPGRADE_SOURCE_UNSUPPORTED: this source format has no validated upgrade and recovery path"
    )]
    UpgradeSourceUnsupported,
    #[error("storage upgrade requires recovery: {recovery:?}: {source}")]
    Recovery {
        recovery: Box<hiroute_host_runtime::StorageRecovery>,
        source: Box<LocalStorageError>,
    },
    #[error("stable storage upgrade failed at {stage}: {source}")]
    UpgradeStage {
        stage: &'static str,
        source: Box<LocalStorageError>,
    },
    #[error("local storage I/O failed")]
    Io(#[from] std::io::Error),
    #[error("local SQLite operation failed")]
    Sqlite(#[from] rusqlite::Error),
    #[error("local storage data is corrupt or incompatible")]
    InvalidData,
    #[error("owner-only storage permission validation failed")]
    Permission,
    #[error("Secret cryptography failed closed")]
    Crypto,
    #[error("encrypted local state is locked because its exact key binding is unavailable")]
    Locked,
}

fn prepare_owner_directory(path: &Path) -> Result<(), LocalStorageError> {
    use hiroute_platform::private_fs::{self, DirectoryError};
    private_fs::prepare_storage_directory(path).map_err(|error| match error {
        DirectoryError::NotDirectory => LocalStorageError::Permission,
        DirectoryError::Io(error) => LocalStorageError::Io(error),
    })
}

impl LocalStorageError {
    pub fn recovery(&self) -> hiroute_host_runtime::StorageRecovery {
        use hiroute_host_runtime::{
            StorageRecovery, StorageRecoveryAction as A, StorageRecoveryCause as C,
        };
        match self {
            Self::Recovery { recovery, .. } => recovery.as_ref().clone(),
            Self::UpgradeStage { source, .. } => {
                let mut report = source.recovery();
                if report.cause == C::UnknownState {
                    report.cause = C::MigrationFailed;
                }
                report
            }
            Self::UpgradeSourceUnsupported => {
                StorageRecovery::new(C::UnsupportedFormat, A::PreserveAndDiagnose)
            }
            Self::Permission => StorageRecovery::new(C::AccessDenied, A::RetryCurrent),
            Self::Locked | Self::Crypto => {
                StorageRecovery::new(C::MissingKey, A::PreserveAndDiagnose)
            }
            Self::Io(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || (cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33))) =>
            {
                StorageRecovery::new(C::Locked, A::RetryCurrent)
            }
            Self::Io(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                StorageRecovery::new(C::AccessDenied, A::RetryCurrent)
            }
            _ => StorageRecovery::new(C::UnknownState, A::PreserveAndDiagnose),
        }
    }
    pub(crate) fn with_recovery(
        self,
        cause: hiroute_host_runtime::StorageRecoveryCause,
        action: hiroute_host_runtime::StorageRecoveryAction,
    ) -> Self {
        Self::Recovery {
            recovery: Box::new(hiroute_host_runtime::StorageRecovery::new(cause, action)),
            source: Box::new(self),
        }
    }
    pub(crate) fn at_upgrade_stage(self, stage: &'static str) -> Self {
        Self::UpgradeStage {
            stage,
            source: Box::new(self),
        }
    }
}
