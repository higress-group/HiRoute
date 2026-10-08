//! Upgrade consumers use frozen schema-23 journals produced by the previous product.
use super::*;
use crate::{LocalStorageSet, backup::test_writer_barrier, test_tempdir as tempdir};
use hiroute_domain::{
    BeginOperationOutcome, CompensationOutcome, ControlRepositoryPort, EffectReconciliation,
    OperationId, OperationState, OperationStepKind, OwnedEffectV1,
};
use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/schema23-worker-operations.json");
const TABLES: [&str; 5] = [
    "operations",
    "operation_steps",
    "worker_dependency_selections",
    "worker_dependency_selection_effects",
    "writer_claim",
];

pub(super) fn cases() -> Vec<Value> {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    assert_eq!(fixture["schema_version"], 23);
    let cases = fixture["cases"].as_array().unwrap();
    let matrix: std::collections::BTreeSet<_> = cases
        .iter()
        .map(|case| {
            (
                case["harness"].as_str().unwrap(),
                case["stage"].as_str().unwrap(),
            )
        })
        .collect();
    let expected = ["codex_cli", "claude_code"]
        .into_iter()
        .flat_map(|harness| {
            ["staged", "activated", "succeeded"]
                .into_iter()
                .map(move |stage| (harness, stage))
        })
        .collect();
    assert_eq!(cases.len(), 6, "all original producer cases are required");
    assert_eq!(matrix, expected);
    cases.clone()
}

fn row_values(row: &Value) -> Vec<rusqlite::types::Value> {
    row.as_array()
        .unwrap()
        .iter()
        .map(|value| match value {
            Value::Null => rusqlite::types::Value::Null,
            Value::Number(number) => number.as_i64().unwrap().into(),
            Value::String(text) => text.clone().into(),
            _ => panic!("unexpected frozen SQL value"),
        })
        .collect()
}

pub(super) fn schema23_source(root: &Path, case: &Value) {
    // Initialize only fresh key/store identities. Reconstruct the published v20 tables and
    // rewind the no-op/runtime format revision; no current serializer produces legacy data.
    drop(LocalStorageSet::open_for_daemon_startup(root).unwrap());
    for path in live_database_paths(&root.join("live")) {
        let connection = Connection::open(&path).unwrap();
        if path.file_name().unwrap() == "control.db" {
            connection
                .execute_batch("DROP TABLE decision_service_operations; DROP TABLE decision_services; DROP TABLE agent_surface_checks")
                .unwrap();
            connection
                .execute_batch(agent_surface_checks_v22::CONTROL)
                .unwrap();
            connection
                .execute_batch(
                    "DROP TABLE worker_dependency_selection_effects;
                 DROP TABLE worker_dependency_selections;",
                )
                .unwrap();
            connection
                .execute_batch(worker_dependencies_v20::CONTROL)
                .unwrap();
            for table in TABLES {
                let data = &case["tables"][table];
                let columns: Vec<_> = data["columns"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap())
                    .collect();
                let parameters = vec!["?"; columns.len()].join(",");
                let sql = format!(
                    "INSERT INTO {table} ({}) VALUES ({parameters})",
                    columns.join(",")
                );
                for row in data["rows"].as_array().unwrap() {
                    connection
                        .execute(&sql, rusqlite::params_from_iter(row_values(row)))
                        .unwrap();
                }
            }
        }
        connection
            .execute("DELETE FROM schema_migrations WHERE version > 23", [])
            .unwrap();
        connection.pragma_update(None, "user_version", 23).unwrap();
    }
}

pub(super) fn assert_frozen_rows(connection: &Connection, case: &Value) {
    for table in TABLES {
        let data = &case["tables"][table];
        let mut statement = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1,2"))
            .unwrap();
        let count = statement.column_count();
        let actual: Vec<Vec<rusqlite::types::Value>> = statement
            .query_map([], |row| (0..count).map(|i| row.get(i)).collect())
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let expected: Vec<_> = data["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(row_values)
            .collect();
        assert_eq!(
            actual, expected,
            "{table}: migration must retain raw bytes and ownership"
        );
    }
}

#[test]
fn schema23_worker_journals_replay_and_compensate_after_production_startup() {
    for case in cases() {
        let directory = tempdir().unwrap();
        let root = directory.path().join("storage");
        schema23_source(&root, &case);
        let key = fs::read(root.join("master-key")).unwrap();
        let stores = LocalStorageSet::open_for_daemon_startup(&root).unwrap();
        assert_eq!(fs::read(root.join("master-key")).unwrap(), key);
        let store = stores.control();
        store.with_connection(|connection| assert_frozen_rows(connection, &case));
        let id: OperationId = serde_json::from_value(case["operation_id"].clone()).unwrap();
        let mut operation = store.load_operation(&id).unwrap().unwrap();
        assert!(store.operation_is_current(&operation).unwrap());
        let recoverable = store.recoverable_operations().unwrap();
        if operation.state.is_terminal() {
            assert!(recoverable.is_empty());
        } else {
            assert_eq!(recoverable, vec![operation.clone()]);
        }
        assert_eq!(
            store
                .operation_for_idempotency(&operation.workspace_id, &operation.idempotency)
                .unwrap(),
            Some(operation.clone())
        );
        assert_eq!(
            store.begin_local_operation(&operation).unwrap(),
            BeginOperationOutcome::ExistingSame(Box::new(operation.clone()))
        );
        let effect: OwnedEffectV1 = serde_json::from_value(case["effect"].clone()).unwrap();
        assert_eq!(
            operation
                .step(OperationStepKind::MaterializeSources)
                .effects,
            vec![effect.clone()]
        );
        let change = operation.plan.worker_dependency_selection().unwrap();
        let harness = change.after_selection.harness;
        let expected_revision = if case["stage"] == "staged" { 1 } else { 2 };
        assert_eq!(
            store
                .worker_dependency_selection_revision(&operation.workspace_id, harness)
                .unwrap(),
            expected_revision
        );
        assert_eq!(
            store
                .apply_worker_dependency_selection(&id, &operation.workspace_id, change)
                .unwrap(),
            effect
        );
        let expected_state = if case["stage"] == "staged" {
            EffectReconciliation::Staged(effect.clone())
        } else {
            EffectReconciliation::Applied(effect.clone())
        };
        assert_eq!(
            store.observe_control(&id, &operation.workspace_id).unwrap(),
            expected_state
        );
        // Idempotent journal/effect reads above must not even reserialize historical JSON.
        store.with_connection(|connection| assert_frozen_rows(connection, &case));
        for _ in 0..2 {
            assert_eq!(store.activate_control(&effect).unwrap(), effect);
        }
        assert_eq!(
            store
                .worker_dependency_selection(&operation.workspace_id, harness)
                .unwrap(),
            Some((change.after_selection.clone(), 2))
        );
        let previous: OwnedEffectV1 =
            serde_json::from_value(case["previous_effect"].clone()).unwrap();
        assert_eq!(
            store.compensate_control(&previous).unwrap(),
            CompensationOutcome::OwnershipLost
        );
        if !operation.state.is_terminal() {
            assert_eq!(
                store.compensate_control(&effect).unwrap(),
                CompensationOutcome::Compensated
            );
            assert_eq!(
                store.compensate_control(&effect).unwrap(),
                CompensationOutcome::AlreadyCompensated
            );
            assert_eq!(
                store
                    .worker_dependency_selection_revision(&operation.workspace_id, harness)
                    .unwrap(),
                1
            );
            let frozen_effect = case["tables"]["worker_dependency_selection_effects"]["rows"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row[0] == case["operation_id"])
                .unwrap();
            store.with_connection(|connection| {
                let restored: (String, String) = connection.query_row(
                    "SELECT selection_json, owner_operation_id FROM worker_dependency_selections",
                    [], |row| Ok((row.get(0)?, row.get(1)?)),
                ).unwrap();
                assert_eq!(
                    restored,
                    (
                        frozen_effect[4].as_str().unwrap().to_owned(),
                        frozen_effect[5].as_str().unwrap().to_owned()
                    )
                );
            });
            operation.transition(OperationState::RollingBack).unwrap();
            operation.transition(OperationState::RolledBack).unwrap();
            store.finish_operation(&mut operation).unwrap();
            assert!(store.operation_is_current(&operation).unwrap());
            assert!(!store.writer_recovery_required().unwrap());
        }
        drop(stores);
        let reopened = LocalStorageSet::open_for_daemon_startup(&root).unwrap();
        assert_eq!(
            reopened.control().load_operation(&id).unwrap(),
            Some(operation)
        );
        assert_eq!(
            BackupSet::open(
                &crate::test_storage_authority(),
                crate::upgrade_backup_root(&root).unwrap()
            )
            .unwrap()
            .phase(),
            BackupSetPhase::Completed
        );
        for path in live_database_paths(&root.join("live")) {
            assert_eq!(
                database_version(&path).unwrap(),
                Some(LATEST_SCHEMA_VERSION)
            );
        }
    }
}

#[test]
fn schema23_interrupted_set_needs_its_complete_backup_and_resumes_at_production_startup() {
    for persist_phase in [false, true] {
        let directory = tempdir().unwrap();
        let root = directory.path().join("storage");
        let case = &cases()[0];
        schema23_source(&root, case);
        let live = root.join("live");
        let backup = crate::upgrade_backup_root(&root).unwrap();
        let binding = crate::LocalSecretStore::migration_binding(
            &live.join("secrets.db"),
            &root.join("master-key"),
        )
        .unwrap()
        .unwrap();
        let barrier = test_writer_barrier();
        let mut coordinator = MigrationSetCoordinator::prepare(
            &crate::test_storage_authority(),
            &barrier,
            &live,
            &backup,
            Some(&binding),
        )
        .unwrap();
        drop(
            crate::ControlStore::open_from_migration_set(
                &crate::test_storage_authority(),
                &live.join("control.db"),
                &backup,
                coordinator.expected_control_store_uuid().unwrap(),
                coordinator.target_schema_version(),
            )
            .unwrap(),
        );
        if persist_phase {
            coordinator.mark_control_migrated(&barrier).unwrap();
        }
        drop(coordinator);
        // Both no manifest and a manifest whose source set is incomplete must fail closed.
        let withheld = if persist_phase {
            backup.join("runtime.db")
        } else {
            backup.clone()
        };
        let saved_backup = backup.with_file_name("withheld-source-backup");
        fs::rename(&withheld, &saved_backup).unwrap();
        let before = live_database_paths(&live).map(|path| fs::read(path).unwrap());
        assert!(LocalStorageSet::open_for_daemon_startup(&root).is_err());
        assert_eq!(
            live_database_paths(&live).map(|path| fs::read(path).unwrap()),
            before
        );
        fs::rename(saved_backup, &withheld).unwrap();
        let stores = LocalStorageSet::open_for_daemon_startup(&root).unwrap();
        stores
            .control()
            .with_connection(|connection| assert_frozen_rows(connection, case));
        assert_eq!(
            BackupSet::open(&crate::test_storage_authority(), backup)
                .unwrap()
                .phase(),
            BackupSetPhase::Completed
        );
    }
}

#[test]
fn completed_schema23_backup_is_archived_before_the_next_migration_batch() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("storage");
    schema23_source(&root, &cases()[0]);
    let live = root.join("live");
    let backup = crate::upgrade_backup_root(&root).unwrap();
    let paths = live_database_paths(&live);
    let binding = crate::LocalSecretStore::migration_binding(&paths[2], &root.join("master-key"))
        .unwrap()
        .unwrap();
    let barrier = test_writer_barrier();
    let mut previous = BackupSet::create_migration(
        &barrier,
        &backup,
        &open_existing_for_snapshot(&paths[0]).unwrap(),
        &open_existing_for_snapshot(&paths[1]).unwrap(),
        &open_existing_for_snapshot(&paths[2]).unwrap(),
        &binding.key_id,
        23,
    )
    .unwrap();
    for phase in [
        BackupSetPhase::ControlMigrated,
        BackupSetPhase::RuntimeMigrated,
        BackupSetPhase::SecretsMigrated,
        BackupSetPhase::Completed,
    ] {
        previous.persist_phase(&barrier, phase).unwrap();
    }
    let previous_id = previous.set_id().to_owned();
    let before = fs::read(backup.join("control.db")).unwrap();
    drop(previous);
    drop(LocalStorageSet::open_for_daemon_startup(&root).unwrap());
    let archived = backup
        .parent()
        .unwrap()
        .join(format!("migration-set.{previous_id}"));
    assert_eq!(fs::read(archived.join("control.db")).unwrap(), before);
    let current = BackupSet::open(&crate::test_storage_authority(), &backup).unwrap();
    assert_ne!(current.set_id(), previous_id);
    assert_eq!(current.target_schema_version(), LATEST_SCHEMA_VERSION);
    assert_eq!(current.phase(), BackupSetPhase::Completed);
    drop(LocalStorageSet::open_for_daemon_startup(&root).unwrap());
}

#[test]
fn unmarked_schema23_is_refused_before_rotating_backups_or_creating_keys() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("storage");
    schema23_source(&root, &cases()[0]);
    Connection::open(root.join("live/control.db"))
        .unwrap()
        .execute("DROP TABLE stable_storage_format", [])
        .unwrap();
    let paths = live_database_paths(&root.join("live"));
    let before = paths.clone().map(|path| fs::read(path).unwrap());
    let key = fs::read(root.join("master-key")).unwrap();
    assert!(matches!(
        LocalStorageSet::open_for_daemon_startup(&root),
        Err(LocalStorageError::UpgradeSourceUnsupported)
    ));
    assert_eq!(paths.map(|path| fs::read(path).unwrap()), before);
    assert_eq!(fs::read(root.join("master-key")).unwrap(), key);
    assert!(!crate::upgrade_backup_root(&root).unwrap().exists());
}
