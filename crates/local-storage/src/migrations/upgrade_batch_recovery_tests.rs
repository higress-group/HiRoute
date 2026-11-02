//! Product startup recovery at fixed schema23/24/25 crash boundaries.
//! Reuses frozen old Worker journals; SQL commits are real historical migration steps.
use super::*;
use crate::{LocalStorageSet, backup::test_writer_barrier};
use worker_dependencies_v24_tests::{assert_frozen_rows, cases, schema23_source};

fn source_batch(root: &Path, target: u32) -> BackupSet {
    source_batch_from_version(root, 23, target)
}

fn source_batch_from_version(root: &Path, source: u32, target: u32) -> BackupSet {
    schema23_source(root, &cases()[0]);
    let paths = live_database_paths(&root.join("live"));
    for version in 24..=source {
        for (index, kind) in [
            DatabaseKind::Control,
            DatabaseKind::Runtime,
            DatabaseKind::Secrets,
        ]
        .into_iter()
        .enumerate()
        {
            commit_step(&paths[index], kind, version);
        }
    }
    let binding = crate::LocalSecretStore::migration_binding(&paths[2], &root.join("master-key"))
        .unwrap()
        .unwrap();
    BackupSet::create_migration(
        &test_writer_barrier(),
        &crate::upgrade_backup_root(root).unwrap(),
        &open_existing_for_snapshot(&paths[0]).unwrap(),
        &open_existing_for_snapshot(&paths[1]).unwrap(),
        &open_existing_for_snapshot(&paths[2]).unwrap(),
        &binding.key_id,
        target,
    )
    .unwrap()
}

fn commit_step(path: &Path, kind: DatabaseKind, version: u32) {
    let mut connection = Connection::open(path).unwrap();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    if kind == DatabaseKind::Control && matches!(version, 34..=38) {
        crate::control::agent_locator::validate_coverage(&transaction).unwrap();
    }
    transaction
        .execute_batch(migration_sql(kind, version).unwrap())
        .unwrap();
    if kind == DatabaseKind::Control && version == 32 {
        // The crash fixture commits the complete current migration transaction,
        // including its semantic initialization before publishing the schema marker.
        current_dependencies_v32::initialize(&transaction).unwrap();
    }
    if kind == DatabaseKind::Control && version == 33 {
        agent_locator_v33::initialize(&transaction).unwrap();
    }
    if kind == DatabaseKind::Control && matches!(version, 34 | 35) {
        super::decision_sources_v34::refresh_locator(&transaction).unwrap();
    }
    if kind == DatabaseKind::Control && version == 36 {
        subscription_api_v36::initialize(&transaction).unwrap();
    }
    if kind == DatabaseKind::Control && version == 37 {
        key_budgets_v37::initialize(&transaction).unwrap();
    }
    if kind == DatabaseKind::Control && version == 38 {
        decision_sources_v34::refresh_locator(&transaction).unwrap();
    }
    transaction
        .pragma_update(None, "user_version", version)
        .unwrap();
    transaction.execute(
        "INSERT INTO schema_migrations(version, applied_at, binary_version) VALUES (?1, 1, 'interrupted-producer')",
        [version],
    ).unwrap();
    transaction.commit().unwrap();
}

fn assert_latest_and_preserved(root: &Path, key: &[u8]) {
    let stores = LocalStorageSet::open_for_daemon_startup(root).unwrap();
    stores
        .control()
        .with_connection(|connection| assert_frozen_rows(connection, &cases()[0]));
    assert_eq!(fs::read(root.join("master-key")).unwrap(), key);
    for path in live_database_paths(&root.join("live")) {
        assert_eq!(
            database_version(&path).unwrap(),
            Some(LATEST_SCHEMA_VERSION)
        );
    }
}

#[test]
fn upgrade_progress_tracks_real_conversion_and_is_silent_after_completion() {
    use hiroute_host_runtime::StorageUpgradePhase::*;
    for target in [24, 36, 37, LATEST_SCHEMA_VERSION] {
        let directory = crate::test_tempdir().unwrap();
        let root = directory.path().join("storage");
        source_batch(&root, target);
        let (sender, receiver) = std::sync::mpsc::channel();
        let options = crate::StorageStartupOptions {
            upgrade_progress: Some(sender.into()),
            ..Default::default()
        };
        drop(LocalStorageSet::open_for_daemon_startup_with_options(&root, &options).unwrap());
        assert_eq!(
            receiver.try_iter().collect::<Vec<_>>(),
            [SourceCheck, Backup, Conversion, Validation]
        );
        let (sender, receiver) = std::sync::mpsc::channel();
        let options = crate::StorageStartupOptions {
            upgrade_progress: Some(sender.into()),
            ..Default::default()
        };
        drop(LocalStorageSet::open_for_daemon_startup_with_options(&root, &options).unwrap());
        assert!(
            receiver.try_recv().is_err(),
            "completed upgrade is ordinary startup"
        );
    }
}

#[test]
fn failed_conversion_keeps_last_entered_phase_and_original_group() {
    use hiroute_host_runtime::StorageUpgradePhase::*;
    let directory = crate::test_tempdir().unwrap();
    let root = directory.path().join("storage");
    schema23_source(&root, &cases()[0]);
    let path = root.join("live/control.db");
    let db = Connection::open(&path).unwrap();
    assert!(
        db.execute("UPDATE operations SET operation_json='{}'", [])
            .unwrap()
            > 0
    );
    let before = database_state_digest(&db).unwrap();
    drop(db);
    let (sender, receiver) = std::sync::mpsc::channel();
    let options = crate::StorageStartupOptions {
        upgrade_progress: Some(sender.into()),
        ..Default::default()
    };
    assert!(LocalStorageSet::open_for_daemon_startup_with_options(&root, &options).is_err());
    assert_eq!(
        receiver.try_iter().collect::<Vec<_>>(),
        [SourceCheck, Backup, Conversion]
    );
    // Recovery may rebuild SQLite pages; compare the original logical group.
    let backup = crate::upgrade_backup_root(&root).unwrap();
    assert!(backup.join("control.db").exists());
    assert_eq!(database_version(&path).unwrap(), Some(23));
    let db = Connection::open(&path).unwrap();
    assert_eq!(database_state_digest(&db).unwrap(), before);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM operations WHERE operation_json!='{}'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn completed_upgrade_keeps_current_binding_and_unfinished_recovery_strict() {
    for fault in [
        "unfinished",
        "unknown_manifest",
        "foreign_store",
        "current_membership",
    ] {
        let directory = crate::test_tempdir().unwrap();
        let root = directory.path().join("storage");
        schema23_source(&root, &cases()[0]);
        drop(LocalStorageSet::open_for_daemon_startup(&root).unwrap());
        let backup = crate::upgrade_backup_root(&root).unwrap();
        match fault {
            "unfinished" => {
                let path = backup.join("backup-set.manifest.json");
                let mut manifest: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                manifest["phase"] = "secrets_migrated".into();
                fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
                fs::write(backup.join("control.db"), b"required backup damaged").unwrap();
            }
            "unknown_manifest" => {
                fs::write(backup.join("backup-set.manifest.json"), b"{}").unwrap()
            }
            "foreign_store" => {
                Connection::open(root.join("live/runtime.db"))
                    .unwrap()
                    .execute("UPDATE storage_meta SET store_uuid='foreign-store'", [])
                    .unwrap();
            }
            "current_membership" => {
                Connection::open(root.join("live/control.db"))
                    .unwrap()
                    .execute("DELETE FROM operation_recovery_coverage", [])
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            LocalStorageSet::open_for_daemon_startup(&root).is_err(),
            "{fault}"
        );
    }
}

#[test]
fn missing_original_backup_for_old_mixed_versions_refuses_without_adopting_data() {
    let directory = crate::test_tempdir().unwrap();
    let root = directory.path().join("storage");
    let set = source_batch(&root, 24);
    let backup = set.directory().to_owned();
    commit_step(&root.join("live/control.db"), DatabaseKind::Control, 24);
    let withheld = backup.with_file_name("withheld-original-batch");
    fs::rename(&backup, &withheld).unwrap();
    let paths = live_database_paths(&root.join("live"));
    let before = paths.clone().map(|path| fs::read(path).unwrap());
    let key = fs::read(root.join("master-key")).unwrap();
    assert!(LocalStorageSet::open_for_daemon_startup(&root).is_err());
    assert_eq!(paths.map(|path| fs::read(path).unwrap()), before);
    assert_eq!(fs::read(root.join("master-key")).unwrap(), key);
    assert!(
        !backup.exists(),
        "a missing original batch must not be replaced"
    );
    assert_eq!(
        BackupSet::open(&crate::test_storage_authority(), withheld)
            .unwrap()
            .set_id(),
        set.set_id()
    );
}

#[test]
fn prior_target_batch_finishes_and_archives_before_a_new_current_batch() {
    for target in [24, 25, 32, 33, 34, 35, 37] {
        for persist_phase in [false, true] {
            let directory = crate::test_tempdir().unwrap();
            let root = directory.path().join("storage");
            let mut set = source_batch(&root, target);
            let backup = set.directory().to_owned();
            let source_bytes = fs::read(backup.join("control.db")).unwrap();
            let old_id = set.set_id().to_owned();
            let key = fs::read(root.join("master-key")).unwrap();
            for version in 24..=target {
                commit_step(
                    &root.join("live/control.db"),
                    DatabaseKind::Control,
                    version,
                );
            }
            if persist_phase {
                set.persist_phase(&test_writer_barrier(), BackupSetPhase::ControlMigrated)
                    .unwrap();
            }
            drop(set);
            assert_latest_and_preserved(&root, &key);
            let archived = backup
                .parent()
                .unwrap()
                .join(format!("migration-set.{old_id}"));
            let previous = BackupSet::open(&crate::test_storage_authority(), &archived).unwrap();
            assert_eq!(previous.target_schema_version(), target);
            assert_eq!(previous.phase(), BackupSetPhase::Completed);
            assert_eq!(previous.set_id(), old_id);
            assert_eq!(fs::read(archived.join("control.db")).unwrap(), source_bytes);
            let current = BackupSet::open(&crate::test_storage_authority(), &backup).unwrap();
            assert_ne!(current.set_id(), old_id);
            assert_eq!(current.control.manifest().schema_version, target);
            assert_eq!(current.target_schema_version(), LATEST_SCHEMA_VERSION);
            assert_eq!(current.phase(), BackupSetPhase::Completed);
            assert_latest_and_preserved(&root, &key);
            assert_eq!(
                BackupSet::open(&crate::test_storage_authority(), backup)
                    .unwrap()
                    .set_id(),
                current.set_id()
            );
        }
    }
}

#[test]
fn retained_schema29_to30_batch_finishes_before_current_upgrade() {
    for committed in 0..=3 {
        for persist_last_phase in [false, true] {
            if committed == 0 && persist_last_phase {
                continue;
            }
            let directory = crate::test_tempdir().unwrap();
            let root = directory.path().join("storage");
            let mut set = source_batch_from_version(&root, 29, 30);
            let backup = set.directory().to_owned();
            let original_id = set.set_id().to_owned();
            let original_sources = [
                set.control.manifest().clone(),
                set.runtime.manifest().clone(),
                set.secrets.manifest().clone(),
            ];
            let original_bytes = ["control.db", "runtime.db", "secrets.db"]
                .map(|name| fs::read(backup.join(name)).unwrap());
            let key = fs::read(root.join("master-key")).unwrap();
            let kinds = [
                DatabaseKind::Control,
                DatabaseKind::Runtime,
                DatabaseKind::Secrets,
            ];
            let phases = [
                BackupSetPhase::ControlMigrated,
                BackupSetPhase::RuntimeMigrated,
                BackupSetPhase::SecretsMigrated,
            ];
            let paths = live_database_paths(&root.join("live"));
            for (index, kind) in kinds.into_iter().take(committed).enumerate() {
                commit_step(&paths[index], kind, 30);
                if index + 1 < committed || persist_last_phase {
                    set.persist_phase(&test_writer_barrier(), phases[index])
                        .unwrap();
                }
            }
            drop(set);
            assert_latest_and_preserved(&root, &key);
            let archived = backup
                .parent()
                .unwrap()
                .join(format!("migration-set.{original_id}"));
            let previous = BackupSet::open(&crate::test_storage_authority(), &archived).unwrap();
            assert_eq!(previous.set_id(), original_id);
            assert_eq!(previous.target_schema_version(), 30);
            assert_eq!(previous.phase(), BackupSetPhase::Completed);
            assert_eq!(
                [
                    previous.control.manifest().clone(),
                    previous.runtime.manifest().clone(),
                    previous.secrets.manifest().clone(),
                ],
                original_sources
            );
            for (index, name) in ["control.db", "runtime.db", "secrets.db"]
                .iter()
                .enumerate()
            {
                assert_eq!(
                    fs::read(archived.join(name)).unwrap(),
                    original_bytes[index]
                );
            }
            let current = BackupSet::open(&crate::test_storage_authority(), &backup).unwrap();
            assert_ne!(current.set_id(), original_id);
            assert_eq!(current.control.manifest().schema_version, 30);
            assert_eq!(current.target_schema_version(), LATEST_SCHEMA_VERSION);
            assert_eq!(current.phase(), BackupSetPhase::Completed);
            assert_latest_and_preserved(&root, &key);
            assert_eq!(
                BackupSet::open(&crate::test_storage_authority(), &backup)
                    .unwrap()
                    .set_id(),
                current.set_id()
            );
        }
    }
}

#[test]
fn retained_target30_refuses_other_sources_and_changed_live_data() {
    for source in [23, 29] {
        let directory = crate::test_tempdir().unwrap();
        let root = directory.path().join("storage");
        let set = source_batch_from_version(&root, source, 30);
        let original_id = set.set_id().to_owned();
        let backup = set.directory().to_owned();
        drop(set);
        if source == 29 {
            Connection::open(root.join("live/control.db"))
                .unwrap()
                .execute_batch("CREATE TABLE unverified_source_change(value TEXT);")
                .unwrap();
        }
        let paths = live_database_paths(&root.join("live"));
        let before = paths.each_ref().map(|path| fs::read(path).unwrap());
        assert!(LocalStorageSet::open_for_daemon_startup(&root).is_err());
        for (index, path) in paths.iter().enumerate() {
            assert_eq!(database_version(path).unwrap(), Some(source));
            assert_eq!(fs::read(path).unwrap(), before[index]);
        }
        let original = BackupSet::open(&crate::test_storage_authority(), &backup).unwrap();
        assert_eq!(original.set_id(), original_id);
        assert_eq!(original.phase(), BackupSetPhase::SourcePublished);
    }
}

#[test]
fn current_batch_resumes_schema24_commit_for_each_recorded_next_store() {
    for next in 0..3 {
        let directory = crate::test_tempdir().unwrap();
        let root = directory.path().join("storage");
        let mut set = source_batch(&root, LATEST_SCHEMA_VERSION);
        let backup = set.directory().to_owned();
        let id = set.set_id().to_owned();
        let source_bytes = fs::read(backup.join("control.db")).unwrap();
        let key = fs::read(root.join("master-key")).unwrap();
        let paths = live_database_paths(&root.join("live"));
        for (index, kind) in [
            DatabaseKind::Control,
            DatabaseKind::Runtime,
            DatabaseKind::Secrets,
        ]
        .into_iter()
        .enumerate()
        .take(next + 1)
        {
            commit_step(&paths[index], kind, 24);
            if index < next {
                for version in 25..=LATEST_SCHEMA_VERSION {
                    commit_step(&paths[index], kind, version);
                }
                let phase = [
                    BackupSetPhase::ControlMigrated,
                    BackupSetPhase::RuntimeMigrated,
                ][index];
                set.persist_phase(&test_writer_barrier(), phase).unwrap();
            }
        }
        assert_eq!(
            database_version(&paths[next]).unwrap(),
            Some(24),
            "pin the actual intermediate revision"
        );
        drop(set);
        assert_latest_and_preserved(&root, &key);
        let current = BackupSet::open(&crate::test_storage_authority(), &backup).unwrap();
        assert_eq!(current.set_id(), id);
        assert_eq!(current.phase(), BackupSetPhase::Completed);
        assert_eq!(fs::read(backup.join("control.db")).unwrap(), source_bytes);
        assert_latest_and_preserved(&root, &key);
    }
}

#[test]
fn intermediate_revision_without_its_identity_ledger_or_next_phase_is_preserved_and_refused() {
    for fault in [
        "identity",
        "ledger",
        "ahead_of_phase",
        "phase_ahead",
        "source",
    ] {
        let directory = crate::test_tempdir().unwrap();
        let root = directory.path().join("storage");
        let mut set = source_batch(&root, 25);
        let paths = live_database_paths(&root.join("live"));
        let index = usize::from(fault == "ahead_of_phase");
        let kind = [DatabaseKind::Control, DatabaseKind::Runtime][index];
        commit_step(&paths[index], kind, 24);
        let connection = Connection::open(&paths[index]).unwrap();
        match fault {
            "identity" => {
                connection
                    .execute("UPDATE storage_meta SET store_uuid='foreign-store'", [])
                    .unwrap();
            }
            "ledger" => {
                connection
                    .execute("DELETE FROM schema_migrations WHERE version=24", [])
                    .unwrap();
            }
            "phase_ahead" => {
                set.persist_phase(&test_writer_barrier(), BackupSetPhase::ControlMigrated)
                    .unwrap();
            }
            "source" => {
                Connection::open(&paths[1])
                    .unwrap()
                    .execute("CREATE TABLE changed_source (value TEXT)", [])
                    .unwrap();
            }
            _ => {}
        }
        drop(connection);
        let before = paths.clone().map(|path| fs::read(path).unwrap());
        let key = fs::read(root.join("master-key")).unwrap();
        let manifest = fs::read(set.directory().join("backup-set.manifest.json")).unwrap();
        assert!(
            LocalStorageSet::open_for_daemon_startup(&root).is_err(),
            "{fault}"
        );
        assert_eq!(paths.map(|path| fs::read(path).unwrap()), before, "{fault}");
        assert_eq!(fs::read(root.join("master-key")).unwrap(), key, "{fault}");
        assert_eq!(
            fs::read(set.directory().join("backup-set.manifest.json")).unwrap(),
            manifest,
            "{fault}"
        );
    }
}

#[test]
fn completed_prior_batch_with_missing_or_blank_live_identity_is_not_archived() {
    for (index, sql) in [
        (0, "DELETE FROM storage_meta"),
        (0, "UPDATE storage_meta SET store_uuid=''"),
        (1, "DELETE FROM storage_meta"),
        (1, "UPDATE storage_meta SET store_uuid=''"),
    ] {
        let directory = crate::test_tempdir().unwrap();
        let root = directory.path().join("storage");
        let mut set = source_batch(&root, 24);
        let paths = live_database_paths(&root.join("live"));
        for (path, kind) in paths.iter().zip([
            DatabaseKind::Control,
            DatabaseKind::Runtime,
            DatabaseKind::Secrets,
        ]) {
            commit_step(path, kind, 24);
        }
        for phase in [
            BackupSetPhase::ControlMigrated,
            BackupSetPhase::RuntimeMigrated,
            BackupSetPhase::SecretsMigrated,
            BackupSetPhase::Completed,
        ] {
            set.persist_phase(&test_writer_barrier(), phase).unwrap();
        }
        Connection::open(&paths[index])
            .unwrap()
            .execute(sql, [])
            .unwrap();
        let before = paths.clone().map(|path| fs::read(path).unwrap());
        let key = fs::read(root.join("master-key")).unwrap();
        let backup = set.directory().to_owned();
        let manifest = fs::read(backup.join("backup-set.manifest.json")).unwrap();
        let source_bytes = fs::read(backup.join("control.db")).unwrap();
        let archived = backup
            .parent()
            .unwrap()
            .join(format!("migration-set.{}", set.set_id()));
        assert!(LocalStorageSet::open_for_daemon_startup(&root).is_err());
        assert_eq!(paths.map(|path| fs::read(path).unwrap()), before);
        assert_eq!(fs::read(root.join("master-key")).unwrap(), key);
        assert_eq!(
            fs::read(backup.join("backup-set.manifest.json")).unwrap(),
            manifest
        );
        assert_eq!(fs::read(backup.join("control.db")).unwrap(), source_bytes);
        assert!(
            !archived.exists(),
            "invalid identity must fail before any backup rotation"
        );
    }
}
