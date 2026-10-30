//! Recovery disposition is decided while the startup writer lock is held.
use super::*;
use hiroute_host_runtime::StorageRecoveryAction as Action;

pub(crate) fn publish_failure(root: &Path, error: LocalStorageError) -> LocalStorageError {
    let mut recovery = error.recovery();
    let initial = recovery.clone();
    // Missing package and matching SQL markers cannot establish that a complete
    // original group is readable by the exact old program. In particular,
    // current Gateway payloads can be incompatible without a SQL version change.
    // No unproven program-only downgrade is authorized here.
    if matches!(
        recovery.action,
        Action::PreserveAndDiagnose | Action::ResumeCurrent
    ) {
        let resume = recovery.action == Action::ResumeCurrent;
        // A verified original group proves a complete restoration target. It
        // does not prove that arbitrary current corruption can resume. Only a
        // caller holding the successfully opened core stores may request resume
        // while the fresh Observation root has not yet been published.
        match verified_original_source(root) {
            Ok((name, id, phase)) => {
                recovery.backup_name = Some(name);
                recovery.backup_batch = Some(id);
                recovery.action = if resume
                    && !matches!(phase, BackupSetPhase::Restored | BackupSetPhase::Restoring)
                {
                    Action::ResumeCurrent
                } else {
                    Action::RestoreCompleteGroup
                };
            }
            Err(_) => {
                recovery.action = Action::PreserveAndDiagnose;
                recovery.backup_name = None;
                recovery.backup_batch = None;
            }
        }
    }
    let _ = recovery.store(root);
    // Preserve the existing public Rust error variants when the additional
    // disposition carries no different facts. A changed batch/action must still
    // travel with the error instead of depending on a writable report file.
    if recovery == initial && !matches!(error, LocalStorageError::Recovery { .. }) {
        return error;
    }
    LocalStorageError::Recovery {
        recovery: Box::new(recovery),
        source: Box::new(match error {
            LocalStorageError::Recovery { source, .. } => *source,
            error => error,
        }),
    }
}

fn verified_original_source(
    root: &Path,
) -> Result<(String, String, BackupSetPhase), LocalStorageError> {
    let link = root.join("observation-source-backup.json");
    validate_owner_file(&link)?;
    if fs::metadata(&link)?.len() > 16 * 1024 {
        return Err(LocalStorageError::InvalidData);
    }
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(link)?).map_err(|_| LocalStorageError::InvalidData)?;
    let name = record["backup_name"]
        .as_str()
        .ok_or(LocalStorageError::InvalidData)?;
    let id = record["set_id"]
        .as_str()
        .ok_or(LocalStorageError::InvalidData)?;
    let portable = |s: &str| {
        !s.is_empty()
            && s.len() <= 128
            && !s.contains("..")
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    if record["schema"] != 1 || !portable(name) || !portable(id) {
        return Err(LocalStorageError::InvalidData);
    }
    let parent = crate::upgrade_backup_root(root)?
        .parent()
        .ok_or(LocalStorageError::InvalidData)?
        .to_path_buf();
    for name in [name.to_owned(), format!("migration-set.{id}")] {
        let path = parent.join(&name);
        if !path.exists() {
            continue;
        }
        let set = BackupSet::open(&crate::daemon_storage_authority(), &path)?;
        if set.set_id() != id {
            continue;
        }
        crate::backup::source_snapshot::verify_sealed(&path)?;
        return Ok((name, id.to_owned(), set.phase()));
    }
    Err(LocalStorageError::InvalidData)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_host_runtime::{StorageRecovery, StorageRecoveryCause as Cause};
    #[test]
    fn early_writer_and_root_refusals_report_the_attempt_without_replacing_writer_facts() {
        let directory = crate::test_tempdir().unwrap();
        let root = directory.path().join("storage");
        let stores = crate::LocalStorageSet::open_for_daemon_startup(&root).unwrap();
        let prior = StorageRecovery::new(Cause::UnknownState, Action::PreserveAndDiagnose);
        prior.store(&root).unwrap();
        let before = fs::read(root.join(hiroute_host_runtime::STORAGE_RECOVERY_FILE)).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let options = crate::StorageStartupOptions {
            startup_failure: Some(sender),
            ..Default::default()
        };
        let error = crate::LocalStorageSet::open_for_daemon_startup_with_options(&root, &options)
            .err()
            .unwrap();
        assert_eq!(error.recovery().cause, Cause::Locked);
        let report = receiver.try_recv().unwrap();
        assert_eq!(report.cause, Cause::Locked);
        assert_eq!(report.action, Action::RetryCurrent);
        assert_eq!(
            fs::read(root.join(hiroute_host_runtime::STORAGE_RECOVERY_FILE)).unwrap(),
            before
        );
        drop(stores);
        let blocked = directory.path().join("not-a-directory");
        hiroute_platform::durable::write_private_new(&blocked, b"preserve foreign input").unwrap();
        assert!(
            crate::LocalStorageSet::open_for_daemon_startup_with_options(&blocked, &options)
                .is_err()
        );
        assert_eq!(receiver.try_recv().unwrap().cause, Cause::AccessDenied);
        assert_eq!(fs::read(blocked).unwrap(), b"preserve foreign input");
    }

    #[test]
    fn a_verified_old_source_does_not_make_current_corruption_resumable() {
        let directory = crate::test_tempdir().unwrap();
        let root = directory.path().join("storage");
        drop(crate::LocalStorageSet::open_for_daemon_startup(&root).unwrap());
        hiroute_platform::durable::write_private_new(&root.join("observation-hmac-key"), &[7; 32])
            .unwrap();
        crate::prepare_owner_directory(&root.join("observation")).unwrap();
        let db = Connection::open(root.join("observation/activity.db")).unwrap();
        db.execute_batch("CREATE TABLE observation_meta(key TEXT PRIMARY KEY,value TEXT); INSERT INTO observation_meta VALUES('schema_version','4');").unwrap();
        drop(db);
        let package = directory.path().join("old-program");
        crate::prepare_owner_directory(&package).unwrap();
        hiroute_platform::durable::write_private_new(
            &package.join("hirouted"),
            b"original program fixture",
        )
        .unwrap();
        let stores = crate::LocalStorageSet::open_for_daemon_startup_with_options(
            &root,
            &crate::StorageStartupOptions {
                preserve_observation_source: true,
                recovery_package: Some(package),
                ..Default::default()
            },
        )
        .unwrap();
        // Successfully opened core + unpublished fresh Observation is a known
        // initialization boundary. It may continue with the accepted program.
        stores.record_observation_startup_failure(&root, Cause::MigrationFailed);
        assert_eq!(
            StorageRecovery::read(&root).unwrap().unwrap().action,
            Action::ResumeCurrent
        );
        for (kind, cause) in [
            (std::io::ErrorKind::WouldBlock, Cause::Locked),
            (std::io::ErrorKind::PermissionDenied, Cause::AccessDenied),
        ] {
            let failure = publish_failure(
                &root,
                LocalStorageError::Io(std::io::Error::new(kind, "file prerequisite")),
            );
            let report = failure.recovery();
            assert_eq!(report.cause, cause);
            assert_eq!(
                report.action,
                Action::RetryCurrent,
                "a retained old source must not turn a correctable prerequisite into restoration"
            );
            assert!(report.backup_batch.is_none());
            assert_eq!(
                StorageRecovery::read(&root).unwrap().unwrap().action,
                Action::RetryCurrent
            );
        }
        drop(stores);
        // The retained source still exists after a later generation is present.
        // Arbitrary damage to that current generation cannot inherit resume.
        crate::prepare_owner_directory(&root.join("observation")).unwrap();
        hiroute_platform::durable::write_private_new(
            &root.join("observation/activity.db"),
            b"new generation must remain",
        )
        .unwrap();
        fs::write(root.join("live/control.db"), b"damaged current control").unwrap();
        assert!(crate::LocalStorageSet::open_for_daemon_startup(&root).is_err());
        let report = StorageRecovery::read(&root).unwrap().unwrap();
        assert_eq!(report.action, Action::RestoreCompleteGroup);
        assert!(report.backup_batch.is_some());
        assert_eq!(
            fs::read(root.join("live/control.db")).unwrap(),
            b"damaged current control"
        );
        assert_eq!(
            fs::read(root.join("observation/activity.db")).unwrap(),
            b"new generation must remain"
        );
    }

    #[test]
    fn missing_package_never_infers_original_program_compatibility_from_sql_markers() {
        for prior_upgrade in [false, true] {
            let directory = crate::test_tempdir().unwrap();
            let root = directory.path().join("storage");
            drop(crate::LocalStorageSet::open_for_daemon_startup(&root).unwrap());
            hiroute_platform::durable::write_private_new(
                &root.join("observation-hmac-key"),
                &[7; 32],
            )
            .unwrap();
            crate::prepare_owner_directory(&root.join("observation")).unwrap();
            let path = root.join("observation/activity.db");
            hiroute_platform::durable::write_private_new(&path, b"").unwrap();
            let db = Connection::open(&path).unwrap();
            db.execute_batch("CREATE TABLE observation_meta(key TEXT PRIMARY KEY,value TEXT); INSERT INTO observation_meta VALUES('schema_version','4'); CREATE TABLE original_content(id INTEGER PRIMARY KEY,body BLOB); INSERT INTO original_content VALUES(1,X'00ff');").unwrap();
            drop(db);
            super::super::set_owner_file_permissions(&path).unwrap();
            let paths = [
                "live/control.db",
                "live/runtime.db",
                "live/secrets.db",
                "master-key",
                "observation-hmac-key",
                "observation/activity.db",
            ];
            let original: Vec<_> = paths
                .iter()
                .map(|p| fs::read(root.join(p)).unwrap())
                .collect();
            if prior_upgrade {
                hiroute_platform::durable::write_private_new(
                    &root.join("observation.upgrade.json"),
                    b"unknown prior phase",
                )
                .unwrap();
            }
            assert!(
                crate::LocalStorageSet::open_for_daemon_startup_with_options(
                    &root,
                    &crate::StorageStartupOptions {
                        preserve_observation_source: true,
                        ..Default::default()
                    }
                )
                .is_err()
            );
            let recovery = StorageRecovery::read(&root).unwrap().unwrap();
            assert_eq!(recovery.cause, Cause::MissingPackage);
            assert_eq!(recovery.action, Action::PreserveAndDiagnose);
            for (path, before) in paths.iter().zip(original) {
                assert_eq!(fs::read(root.join(path)).unwrap(), before, "changed {path}");
            }
        }
    }
}
