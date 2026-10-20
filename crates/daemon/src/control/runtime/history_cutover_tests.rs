use super::*;
use rusqlite::Connection;
use std::fs;

#[test]
fn production_startup_seals_old_history_and_preserves_the_new_generation_on_restart() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::history_cutover_tests::production_startup_seals_old_history_and_preserves_the_new_generation_on_restart",
    ) {
        return;
    }
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("storage");
    let open = |options| {
        ProductionControlRuntime::prepare_for_role_all_with_codex_desktop_engine(
            &root,
            crate::release_catalog::fixture_catalog(),
            None,
            None,
            options,
        )
    };
    let original = open(Default::default()).unwrap();
    original
        .observation
        .register_agent_grant(
            &WorkspaceId::default(),
            "original-grant",
            1,
            "original-connection",
            "agent_codex_default",
        )
        .unwrap();
    drop(original);
    let key = fs::read(root.join("observation-hmac-key")).unwrap();
    let old_db = root.join("observation/activity.db");
    let db = Connection::open(&old_db).unwrap();
    // Synthetic admission coverage only; real released old bytes/packages are
    // verified separately by installed field acceptance.
    db.execute(
        "UPDATE observation_meta SET value='4' WHERE key='schema_version'",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO observation_meta VALUES('generation_probe','original-history')",
        [],
    )
    .unwrap();
    drop(db);
    fs::write(
        root.join("observation/original-object"),
        b"unmodified old contents",
    )
    .unwrap();
    let package = temporary.path().join("old-package");
    fs::create_dir(&package).unwrap();
    fs::write(package.join("old-program"), b"synthetic matching package").unwrap();
    let runtime = open(hiroute_local_storage::StorageStartupOptions {
        recovery_package: Some(package),
        ..Default::default()
    })
    .unwrap();
    let link: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("observation-source-backup.json")).unwrap())
            .unwrap();
    let source = hiroute_local_storage::upgrade_backup_root(&root)
        .unwrap()
        .parent()
        .unwrap()
        .join(link["backup_name"].as_str().unwrap())
        .join("source/storage");
    assert_eq!(
        fs::read(source.join("observation/original-object")).unwrap(),
        b"unmodified old contents"
    );
    let db = Connection::open(source.join("observation/activity.db")).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT value FROM observation_meta WHERE key='generation_probe'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "original-history"
    );
    assert_attribution_tables(&db);
    assert_eq!(grant_count(&db, "original-grant"), 1);
    drop(db);
    assert_eq!(fs::read(root.join("observation-hmac-key")).unwrap(), key);
    assert!(!root.join("observation/original-object").exists());
    {
        let current = Connection::open(&old_db).unwrap();
        assert_attribution_tables(&current);
        assert_eq!(grant_count(&current, "original-grant"), 0);
    }
    runtime
        .observation
        .register_agent_grant(
            &WorkspaceId::default(),
            "current-grant",
            2,
            "current-connection",
            "agent_claude_default",
        )
        .unwrap();
    assert_eq!(
        runtime
            .observation
            .capacity_status()
            .unwrap()
            .max_active_bytes,
        Some(20_000_000_000)
    );
    runtime
        .observation
        .set_capacity_limit(Some(20_000_000_000))
        .unwrap();
    drop(runtime);
    let db = Connection::open(&old_db).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT value FROM observation_meta WHERE key='schema_version'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "5"
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM observation_meta WHERE key='generation_probe'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute(
        "INSERT INTO observation_meta VALUES('generation_probe','new-current-history')",
        [],
    )
    .unwrap();
    drop(db);
    let runtime = open(Default::default()).unwrap();
    assert_eq!(
        runtime
            .observation
            .capacity_status()
            .unwrap()
            .max_active_bytes,
        Some(20_000_000_000)
    );
    let db = Connection::open(&old_db).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT value FROM observation_meta WHERE key='generation_probe'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "new-current-history"
    );
    assert_attribution_tables(&db);
    assert_eq!(grant_count(&db, "current-grant"), 1);
    assert_eq!(fs::read(root.join("observation-hmac-key")).unwrap(), key);
    drop(db);
    runtime.observation.set_capacity_limit(None).unwrap();
    drop(runtime);
    let runtime = open(Default::default()).unwrap();
    assert_eq!(
        runtime
            .observation
            .capacity_status()
            .unwrap()
            .max_active_bytes,
        None
    );
}

fn assert_attribution_tables(db: &Connection) {
    let count: u64 = db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type='table'
             AND name IN ('observation_agent_grants_v1','observation_session_agents_v1')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 2,
        "both attribution tables belong to their source generation"
    );
}

fn grant_count(db: &Connection, grant: &str) -> u64 {
    db.query_row(
        "SELECT COUNT(*) FROM observation_agent_grants_v1 WHERE grant_id=?1",
        [grant],
        |row| row.get(0),
    )
    .unwrap()
}
