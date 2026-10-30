//! Complete stopped-source publication before either core or Observation changes.
use super::*;
use crate::StorageStartupOptions;
use hiroute_platform::durable;

impl MigrationSetCoordinator {
    pub(crate) fn preserve_complete_source(
        &self,
        authority: &DaemonStorageAuthority,
        barrier: &SingleWriterBackupBarrier,
        root: &Path,
        backup_root: &Path,
        options: &StorageStartupOptions,
        binding: Option<&MigrationSecretBinding>,
    ) -> Result<(), LocalStorageError> {
        let link = root.join("observation-source-backup.json");
        if options.preserve_observation_source && link.exists() {
            validate_owner_file(&link)?;
            if fs::metadata(&link)?.len() > 16 * 1024 {
                return Err(LocalStorageError::InvalidData);
            }
            let record: serde_json::Value = serde_json::from_slice(&fs::read(&link)?)
                .map_err(|_| LocalStorageError::InvalidData)?;
            let id = record["set_id"]
                .as_str()
                .ok_or(LocalStorageError::InvalidData)?;
            let name = record["backup_name"]
                .as_str()
                .ok_or(LocalStorageError::InvalidData)?;
            let portable = |s: &str| {
                !s.is_empty()
                    && s.len() <= 128
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
                    && !s.contains("..")
            };
            if record["schema"] != 1 || !portable(id) || !portable(name) {
                return Err(LocalStorageError::InvalidData);
            }
            let parent = backup_root.parent().ok_or(LocalStorageError::InvalidData)?;
            let direct = parent.join(name);
            let archived = parent.join(format!("migration-set.{id}"));
            let path = if direct.exists() {
                let set = BackupSet::open(authority, &direct)?;
                if set.set_id() == id { direct } else { archived }
            } else {
                archived
            };
            let set = BackupSet::open(authority, &path)?;
            if set.set_id() != id {
                return Err(LocalStorageError::InvalidData);
            }
            if path.join("source").exists() {
                crate::backup::source_snapshot::verify_sealed(&path)?;
                crate::backup::source_snapshot::publish_locator(&path, set.set_id())?;
            } else {
                let mut resumed = options.clone();
                resumed.recovery_package = record["recovery_package"].as_str().map(PathBuf::from);
                if resumed.recovery_package.is_none() {
                    return Err(LocalStorageError::InvalidData);
                }
                // The SQL coordinator already admitted this original batch,
                // including any committed core steps. Never re-snapshot a mixed
                // live group or require it to equal the original core sources.
                set.publish_coordinated_source(barrier, root, &resumed)?;
            }
            return Ok(());
        }
        let mut options = options.clone();
        if options.preserve_observation_source && options.recovery_package.is_none() {
            options.recovery_package =
                super::recovery_package::previous_desktop(root).map_err(|error| {
                    error.with_recovery(
                        hiroute_host_runtime::StorageRecoveryCause::InvalidPackage,
                        hiroute_host_runtime::StorageRecoveryAction::PreserveAndDiagnose,
                    )
                })?;
            if options.recovery_package.is_none() {
                return Err(LocalStorageError::InvalidData.at_upgrade_stage("Observation history sealing requires a matching old package: set HIROUTE_RECOVERY_PACKAGE").with_recovery(hiroute_host_runtime::StorageRecoveryCause::MissingPackage, hiroute_host_runtime::StorageRecoveryAction::PreserveAndDiagnose));
            }
        }
        if let Some(package) = &options.recovery_package {
            let valid = package.is_absolute()
                && fs::symlink_metadata(package)
                    .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink());
            if !valid {
                return Err(LocalStorageError::InvalidData.with_recovery(
                    hiroute_host_runtime::StorageRecoveryCause::InvalidPackage,
                    hiroute_host_runtime::StorageRecoveryAction::PreserveAndDiagnose,
                ));
            }
        }
        let source = self.set.as_ref().filter(|set| {
            !matches!(
                set.phase(),
                BackupSetPhase::Completed | BackupSetPhase::Restoring | BackupSetPhase::Restored
            )
        });
        let owned;
        let set = if let Some(set) = source {
            set
        } else if options.preserve_observation_source {
            let path = backup_root
                .parent()
                .ok_or(LocalStorageError::InvalidData)?
                .join("observation-format-5");
            if BackupSet::durable_manifest_present(&path)? && path.join("source").exists() {
                let previous = BackupSet::open(authority, &path)?;
                let archived =
                    path.with_file_name(format!("observation-format-5.{}", previous.set_id()));
                if archived.exists() {
                    return Err(LocalStorageError::InvalidData);
                }
                crate::backup::source_snapshot::archive_guide(&path, &archived, previous.set_id())?;
                durable::rename(&path, &archived)?;
            }
            owned = if BackupSet::durable_manifest_present(&path)? {
                BackupSet::open(authority, &path)?
            } else {
                let binding = binding.ok_or(LocalStorageError::Locked)?;
                let live = root.join("live");
                let control = open_existing_for_snapshot(&live.join("control.db"))?;
                let runtime = open_existing_for_snapshot(&live.join("runtime.db"))?;
                let secrets = open_existing_for_snapshot(&live.join("secrets.db"))?;
                BackupSet::create_migration(
                    barrier,
                    &path,
                    &control,
                    &runtime,
                    &secrets,
                    &binding.key_id,
                    LATEST_SCHEMA_VERSION,
                )?
            };
            &owned
        } else {
            return Ok(());
        };
        if options.preserve_observation_source {
            let name = set
                .directory()
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or(LocalStorageError::InvalidData)?;
            let body=serde_json::to_vec(&serde_json::json!({"schema":1,"backup_name":name,"set_id":set.set_id(),"recovery_package":options.recovery_package})).map_err(|_|LocalStorageError::InvalidData)?;
            durable::write_private_new(&link, &body)?;
            durable::directory_barrier(root)?;
        }
        if let Some(progress) = &options.upgrade_progress {
            progress.report(hiroute_host_runtime::StorageUpgradePhase::Backup);
        }
        set.publish_coordinated_source(barrier, root, &options)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observation_only_startup_preserves_complete_group_options_and_matching_package_once() {
        let directory = crate::test_tempdir().unwrap();
        let root = directory.path().join("storage.custom");
        drop(crate::LocalStorageSet::open_for_daemon_startup(&root).unwrap());
        let observation = root.join("observation");
        crate::prepare_owner_directory(&observation).unwrap();
        let db = Connection::open(observation.join("activity.db")).unwrap();
        db.execute_batch("CREATE TABLE observation_meta(key TEXT PRIMARY KEY,value TEXT); INSERT INTO observation_meta VALUES('schema_version','4'); CREATE TABLE retained(id INTEGER PRIMARY KEY,body BLOB); INSERT INTO retained VALUES(17,X'00ff616263');").unwrap();
        drop(db);
        fs::write(observation.join("body"), b"original content").unwrap();
        let lkg = directory.path().join("custom-gateway.lkg");
        fs::write(&lkg, b"custom recovery input").unwrap();
        let package = directory.path().join("old-package");
        fs::create_dir(&package).unwrap();
        fs::write(package.join("old-daemon"), b"exact original binary fixture").unwrap();
        let options = StorageStartupOptions {
            preserve_observation_source: true,
            gateway_lkg: Some(lkg),
            recovery_package: Some(package),
            ..Default::default()
        };
        drop(
            crate::LocalStorageSet::open_for_daemon_startup_with_options(&root, &options).unwrap(),
        );
        // Fast upgrade moves the original tree; a recursive backup plus a live
        // old directory would still impose history-sized startup work.
        assert!(!observation.exists());
        let link: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("observation-source-backup.json")).unwrap())
                .unwrap();
        let backup = crate::upgrade_backup_root(&root)
            .unwrap()
            .parent()
            .unwrap()
            .join(link["backup_name"].as_str().unwrap());
        crate::backup::source_snapshot::verify(&backup).unwrap();
        // Old binaries discover the adjacent coordinator independently of the
        // restored databases. The manual guide must preserve it away from that
        // active location, including for a non-default storage basename.
        let coordinator = directory.path().join("storage.custom.upgrade-backups");
        for guide in ["RECOVERY.md", "恢复说明.md"] {
            let body = fs::read_to_string(backup.join("source").join(guide)).unwrap();
            assert!(body.contains(coordinator.to_str().unwrap()), "{guide}");
        }
        assert_eq!(
            fs::read(backup.join("source/storage/observation/body")).unwrap(),
            b"original content"
        );
        assert_eq!(
            fs::read(backup.join("source/gateway.lkg")).unwrap(),
            b"custom recovery input"
        );
        assert_eq!(
            fs::read(backup.join("source/recovery-package/old-daemon")).unwrap(),
            b"exact original binary fixture"
        );
        let before = fs::read(backup.join("source/source-backup.json")).unwrap();
        drop(
            crate::LocalStorageSet::open_for_daemon_startup_with_options(&root, &options).unwrap(),
        );
        assert_eq!(
            before,
            fs::read(backup.join("source/source-backup.json")).unwrap()
        );
        // Recovery is the complete old group, not a replacement of activity.db
        // alone. Core stores, key, old Observation and old package stay paired.
        for file in [
            "live/control.db",
            "live/runtime.db",
            "live/secrets.db",
            "master-key",
            "observation/activity.db",
        ] {
            assert!(backup.join("source/storage").join(file).exists());
        }
    }
}

#[cfg(test)]
#[test]
fn missing_old_package_refuses_observation_sealing_before_mutation() {
    let directory = crate::test_tempdir().unwrap();
    let root = directory.path().join("storage");
    drop(crate::LocalStorageSet::open_for_daemon_startup(&root).unwrap());
    let path = root.join("live/control.db");
    let before = fs::read(&path).unwrap();
    let error = crate::LocalStorageSet::open_for_daemon_startup_with_options(
        &root,
        &StorageStartupOptions {
            preserve_observation_source: true,
            ..Default::default()
        },
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("HIROUTE_RECOVERY_PACKAGE"));
    assert_eq!(before, fs::read(&path).unwrap());
    assert!(!root.join("observation-source-backup.json").exists());
}

#[cfg(test)]
#[cfg(unix)]
#[test]
fn recovery_space_reservation_stops_before_sealing_observation() {
    let directory = crate::test_tempdir().unwrap();
    let root = directory.path().join("storage");
    drop(crate::LocalStorageSet::open_for_daemon_startup(&root).unwrap());
    let original = fs::read(root.join("live/control.db")).unwrap();
    let package = directory.path().join("old-package");
    fs::create_dir(&package).unwrap();
    fs::write(package.join("daemon"), b"retained old package").unwrap();
    let pad = fs::File::create(package.join("sparse-recovery-input")).unwrap();
    pad.set_len(
        fs2::available_space(&root)
            .unwrap()
            .saturating_add(512 * 1024 * 1024),
    )
    .unwrap();
    let observation = root.join("observation");
    fs::create_dir(&observation).unwrap();
    fs::write(observation.join("body"), b"do not move on failure").unwrap();
    let error = crate::LocalStorageSet::open_for_daemon_startup_with_options(
        &root,
        &StorageStartupOptions {
            preserve_observation_source: true,
            recovery_package: Some(package),
            ..Default::default()
        },
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("Insufficient space"));
    assert_eq!(fs::read(root.join("live/control.db")).unwrap(), original);
    assert_eq!(
        fs::read(observation.join("body")).unwrap(),
        b"do not move on failure"
    );
    let backup = crate::upgrade_backup_root(&root)
        .unwrap()
        .parent()
        .unwrap()
        .join("observation-format-5");
    assert!(!backup.join("source-staging").exists());
    assert!(!backup.join("source").exists());
}
