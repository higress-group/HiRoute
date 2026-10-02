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
    ComputeSubscriptionValidationStateV1, ControlStore, ManagedArtifactStore,
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
    pub upgrade_progress:
        Option<std::sync::mpsc::Sender<hiroute_host_runtime::StorageUpgradePhase>>,
}

impl LocalStorageSet {
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
        _options: &StorageStartupOptions,
    ) -> Result<Self, LocalStorageError> {
        let storage_root = storage_root.as_ref();
        migrations::prepare_storage_root(storage_root)?;
        migrations::validate_startup_format(storage_root)?;
        let startup_lock = std::sync::Arc::new(migrations::acquire_startup_lock(storage_root)?);
        migrations::validate_startup_format(storage_root)?;
        Self::open_locked_stores(authority, barrier, storage_root, startup_lock)
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
        Self::open_locked_stores(authority, barrier, storage_root, lock)
    }

    fn open_locked_stores(
        authority: &DaemonStorageAuthority,
        barrier: &SingleWriterBackupBarrier,
        storage_root: &Path,
        startup_lock: std::sync::Arc<std::fs::File>,
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
            std::fs::rename(previous, &backup_root)?;
            std::fs::File::open(storage_root)?.sync_all()?;
            std::fs::File::open(backup_root.parent().ok_or(LocalStorageError::InvalidData)?)?
                .sync_all()?;
        }
        let master_key_path = storage_root.join("master-key");
        let control_path = live_root.join("control.db");
        let runtime_path = live_root.join("runtime.db");
        let secrets_path = live_root.join("secrets.db");
        let secret_binding = LocalSecretStore::migration_binding(&secrets_path, &master_key_path)?;
        let mut migration = migrations::MigrationSetCoordinator::prepare(
            authority,
            barrier,
            &live_root,
            &backup_root,
            secret_binding.as_ref(),
        )?;
        let control_result = if migration.durable_set_prepared() {
            ControlStore::open_from_migration_set(
                authority,
                &control_path,
                &backup_root,
                migration
                    .expected_control_store_uuid()
                    .ok_or(LocalStorageError::InvalidData)?,
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
        if let Err(error) = migrations::validate_current_storage(&control, &runtime, &secrets)
            .map_err(|error| error.at_upgrade_stage("current storage validation"))
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

    /// Register every native target together, before validating any persisted effect marker.
    /// Sequential single-target opening cannot recover a store containing both Agent families.
    pub fn open_managed_artifacts_with_external_targets(
        &self,
        root: impl AsRef<Path>,
        restore_root: impl AsRef<Path>,
        targets: impl IntoIterator<Item = (String, std::path::PathBuf)>,
    ) -> Result<ManagedArtifactStore, LocalStorageError> {
        ManagedArtifactStore::open_with_external_targets(
            &daemon_storage_authority(),
            root,
            restore_root,
            targets,
        )
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
        CanonicalDigest, ControlRepositoryPort, ProtectedApplyCapability, RevisionSetV1,
        WorkspaceId,
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
    }

    #[cfg(unix)]
    #[test]
    fn daemon_startup_rejects_an_insecure_existing_root_before_migration() {
        use std::os::unix::fs::PermissionsExt;

        let directory = crate::test_tempdir().unwrap();
        let root = directory.path().join("storage");
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            LocalStorageSet::open_for_daemon_startup(&root),
            Err(LocalStorageError::Permission)
        ));
        assert!(!root.join("live").exists());
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
            now + 301,
        )
        .unwrap();
        assert!(
            stores
                .apply_capability_registrar()
                .register(too_long)
                .is_err()
        );
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
#[cfg(test)]
pub(crate) fn test_tempdir() -> std::io::Result<tempfile::TempDir> {
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    builder.tempdir()
}

#[cfg(test)]
pub(crate) fn test_create_dir_all(path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
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

impl LocalStorageError {
    pub(crate) fn at_upgrade_stage(self, stage: &'static str) -> Self {
        Self::UpgradeStage {
            stage,
            source: Box::new(self),
        }
    }
}
