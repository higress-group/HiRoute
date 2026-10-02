use super::*;
use crate::LocalStorageSet;
use crate::backup::test_writer_barrier;
use crate::test_tempdir as tempdir;

#[test]
fn fresh_current_contains_worker_subscription_and_concurrency_formats_and_reopens() {
    let root = tempdir().unwrap();
    let storage = root.path().join("storage");
    drop(
        LocalStorageSet::open_migration_component_fixture(
            &crate::test_storage_authority(),
            &test_writer_barrier(),
            &storage,
        )
        .unwrap(),
    );
    for (index, name) in ["control.db", "runtime.db", "secrets.db"]
        .iter()
        .enumerate()
    {
        let path = storage.join("live").join(name);
        assert_eq!(
            database_version(&path).unwrap(),
            Some(LATEST_SCHEMA_VERSION)
        );
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let marker: i64 = conn
            .query_row(
                "SELECT format_version FROM worker_authorization_format WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marker, 1);
        let subscriptions: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='compute_subscription_validations'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(subscriptions, if index == 0 { 1 } else { 0 });
        let worker_settings: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='worker_settings'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(worker_settings, if index == 1 { 1 } else { 0 });
    }
    drop(
        LocalStorageSet::open_migration_component_fixture(
            &crate::test_storage_authority(),
            &test_writer_barrier(),
            &storage,
        )
        .unwrap(),
    );
}

#[derive(Clone, Copy, Debug)]
enum LegacyLayout {
    V15,
    WorkerV16,
    SubscriptionV16,
}

#[test]
fn v15_and_both_experimental_v16_layouts_converge_without_losing_data_or_keys() {
    for layout in [
        LegacyLayout::V15,
        LegacyLayout::WorkerV16,
        LegacyLayout::SubscriptionV16,
    ] {
        let root = tempdir().unwrap();
        let storage = root.path().join("storage");
        let live = storage.join("live");
        drop(
            LocalStorageSet::open_migration_component_fixture(
                &crate::test_storage_authority(),
                &test_writer_barrier(),
                &storage,
            )
            .unwrap(),
        );
        let master_key_before = fs::read(storage.join("master-key")).unwrap();
        let version = match layout {
            LegacyLayout::V15 => 15,
            LegacyLayout::WorkerV16 | LegacyLayout::SubscriptionV16 => 16,
        };
        for (index, name) in ["control.db", "runtime.db", "secrets.db"]
            .iter()
            .enumerate()
        {
            let path = live.join(name);
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS convergence_sentinel(value TEXT NOT NULL);
                 DELETE FROM convergence_sentinel;
                 INSERT INTO convergence_sentinel(value) VALUES ('preserve-me');
                 DROP TABLE IF EXISTS stable_storage_format;
                 DROP TABLE IF EXISTS worker_authorization_format;
                 DROP TABLE IF EXISTS worker_settings;",
            )
            .unwrap();
            if index == 0 {
                conn.execute_batch(
                    "DROP INDEX IF EXISTS compute_subscription_validations_candidate_idx;
                     DROP TABLE IF EXISTS compute_subscription_validations;
                     DROP INDEX IF EXISTS worker_dependency_selection_effects_state;
                     DROP TABLE IF EXISTS worker_dependency_selection_effects;
                     DROP TABLE IF EXISTS worker_dependency_selections;
                     DROP TABLE IF EXISTS agent_surface_checks;",
                )
                .unwrap();
            }
            if index == 1 {
                conn.execute_batch(
                    "DROP INDEX IF EXISTS delegation_continuation_releases_pending;
                     DROP TABLE IF EXISTS delegation_continuation_releases;
                     DROP INDEX IF EXISTS delegation_native_uses_scope;
                     DROP INDEX IF EXISTS delegation_native_uses_root;
                     DROP TABLE IF EXISTS delegation_native_uses;
                     DROP TABLE IF EXISTS delegation_native_roots;",
                )
                .unwrap();
            }
            match layout {
                LegacyLayout::V15 => {}
                LegacyLayout::WorkerV16 => {
                    conn.execute_batch(WORKER_AUTHORIZATION_FORMAT_V16).unwrap();
                }
                LegacyLayout::SubscriptionV16 if index == 0 => {
                    conn.execute_batch(subscription_v16::CONTROL).unwrap();
                }
                LegacyLayout::SubscriptionV16 => {}
            }
            conn.pragma_update(None, "user_version", version).unwrap();
            drop(conn);
        }

        drop(
            LocalStorageSet::open_migration_component_fixture(
                &crate::test_storage_authority(),
                &test_writer_barrier(),
                &storage,
            )
            .unwrap(),
        );
        assert_eq!(
            fs::read(storage.join("master-key")).unwrap(),
            master_key_before
        );
        let backup = BackupSet::open(
            &crate::test_storage_authority(),
            crate::upgrade_backup_root(&storage).unwrap(),
        )
        .unwrap();
        assert_eq!(backup.phase(), BackupSetPhase::Completed);
        assert_eq!(backup.target_schema_version(), LATEST_SCHEMA_VERSION);

        for (index, name) in ["control.db", "runtime.db", "secrets.db"]
            .iter()
            .enumerate()
        {
            let path = live.join(name);
            assert_eq!(
                database_version(&path).unwrap(),
                Some(LATEST_SCHEMA_VERSION)
            );
            let conn =
                Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            let sentinel: String = conn
                .query_row("SELECT value FROM convergence_sentinel", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(
                sentinel, "preserve-me",
                "layout {layout:?}, database {name}"
            );
            let marker: i64 = conn
                .query_row(
                    "SELECT format_version FROM worker_authorization_format WHERE singleton=1",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(marker, 1);
            let subscriptions: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master
                     WHERE type='table' AND name='compute_subscription_validations'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(subscriptions, if index == 0 { 1 } else { 0 });
            let worker_settings: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master
                     WHERE type='table' AND name='worker_settings'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(worker_settings, if index == 1 { 1 } else { 0 });
        }

        // Completed migration metadata and converged schemas are restart-idempotent.
        drop(
            LocalStorageSet::open_migration_component_fixture(
                &crate::test_storage_authority(),
                &test_writer_barrier(),
                &storage,
            )
            .unwrap(),
        );
    }
}
