//! Historical SQL layout evidence, not a synthetic native model-call receipt.
use super::*;
use crate::LocalStorageSet;

fn rows(connection: &Connection) -> Vec<(String, String, String, i64, String, i64)> {
    connection.prepare("SELECT workspace_id, context_id, surface, applied_revision, record_json, updated_at FROM agent_surface_checks ORDER BY surface")
        .unwrap().query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)))
        .unwrap().map(Result::unwrap).collect()
}

#[test]
fn schema24_surface_upgrade_preserves_raw_results_and_existing_key() {
    let directory = crate::test_tempdir().unwrap();
    let root = directory.path().join("storage");
    drop(LocalStorageSet::open_for_daemon_startup(&root).unwrap());
    let mut expected = Vec::new();
    for path in live_database_paths(&root.join("live")) {
        let connection = Connection::open(&path).unwrap();
        if path.file_name().unwrap() == "control.db" {
            connection
                .execute_batch("DROP TABLE agent_surface_checks")
                .unwrap();
            connection
                .execute_batch(agent_surface_checks_v22::CONTROL)
                .unwrap();
            for surface in ["codex_cli", "codex_desktop", "claude_cli"] {
                // Opaque historical bytes intentionally do not use the current record serializer.
                let raw =
                    format!(" {{ \"historical_surface\" : \"{surface}\", \"opaque\" : [ ] }}\n");
                connection.execute("INSERT INTO agent_surface_checks VALUES ('personal/default', 'historical/context', ?1, 7, ?2, 123)", params![surface, raw]).unwrap();
            }
            expected = rows(&connection);
        }
        connection
            .execute("DELETE FROM schema_migrations WHERE version > 24", [])
            .unwrap();
        connection.pragma_update(None, "user_version", 24).unwrap();
    }
    let key = fs::read(root.join("master-key")).unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(&root).unwrap();
    assert_eq!(fs::read(root.join("master-key")).unwrap(), key);
    stores.control().with_connection(|connection| {
        assert_eq!(rows(connection), expected);
        // Only the named new surface is admitted; the storage constraint remains closed.
        connection.execute("INSERT INTO agent_surface_checks VALUES ('personal/default', 'new/context', 'qoder_cli', 8, '{}', 124)", []).unwrap();
        assert!(connection.execute("INSERT INTO agent_surface_checks VALUES ('personal/default', 'new/context', 'unknown_cli', 8, '{}', 124)", []).is_err());
        connection.execute("DELETE FROM agent_surface_checks WHERE context_id='new/context'", []).unwrap();
        assert_eq!(rows(connection), expected);
    });
    drop(stores);
    let reopened = LocalStorageSet::open_for_daemon_startup(&root).unwrap();
    reopened
        .control()
        .with_connection(|connection| assert_eq!(rows(connection), expected));
    for path in live_database_paths(&root.join("live")) {
        assert_eq!(
            database_version(&path).unwrap(),
            Some(LATEST_SCHEMA_VERSION)
        );
    }
}
