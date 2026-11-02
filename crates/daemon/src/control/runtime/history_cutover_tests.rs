use super::*;
use rusqlite::Connection;
use std::fs;

#[test]
fn core_upgrade_reuses_current_history_and_snapshots_only_authority() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::history_cutover_tests::core_upgrade_reuses_current_history_and_snapshots_only_authority",
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
            "retained-grant",
            1,
            "retained-connection",
            "agent_codex_default",
        )
        .unwrap();
    original
        .observation
        .set_capacity_limit(Some(10_000_000_000))
        .unwrap();
    drop(original);
    let hmac = fs::read(root.join("observation-hmac-key")).unwrap();
    let master = fs::read(root.join("master-key")).unwrap();
    let observation = root.join("observation/activity.db");
    let db = Connection::open(&observation).unwrap();
    db.execute(
        "INSERT INTO observation_meta VALUES('generation_probe','retained-current-history')",
        [],
    )
    .unwrap();
    drop(db);
    fs::write(
        root.join("observation/retained-body"),
        b"exact retained contents",
    )
    .unwrap();
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(&observation).unwrap();
        (metadata.dev(), metadata.ino())
    };
    // Synthetic main schema 35 source; Observation remains current.
    // Released-package upgrade acceptance is separate from this entry regression.
    let db = Connection::open(root.join("live/control.db")).unwrap();
    db.execute_batch(
        "DROP TABLE compute_key_budget_effects;
         DROP TABLE compute_key_spend_charges;
         DROP TABLE compute_key_spend_reservations;
         DROP TABLE compute_key_budget_bindings;
         DROP TABLE compute_key_budgets;
         ALTER TABLE compute_management_effects DROP COLUMN key_budget_witness_digest;
         ALTER TABLE compute_management_effects DROP COLUMN key_budget_accounting_digest;
         ALTER TABLE compute_management_effects DROP COLUMN key_budget_compensation_root;
         DROP TABLE compute_subscription_api_links;
         ALTER TABLE compute_management_effects DROP COLUMN api_link_before;
         ALTER TABLE compute_management_effects DROP COLUMN api_link_after;",
    )
    .unwrap();
    // The synthetic shape change must retain a valid locator stamp, as the
    // genuine main v35 source did before the additive v36/v37 steps.
    let schema: i64 = db
        .pragma_query_value(None, "schema_version", |row| row.get(0))
        .unwrap();
    db.execute(
        "UPDATE agent_locator_coverage SET schema_version=?1 WHERE singleton=1 AND tampered=0",
        [schema],
    )
    .unwrap();
    drop(db);
    for name in ["control.db", "runtime.db", "secrets.db"] {
        Connection::open(root.join("live").join(name))
            .unwrap()
            .pragma_update(None, "user_version", 35)
            .unwrap();
    }
    let package = temporary.path().join("old-package");
    fs::create_dir(&package).unwrap();
    fs::write(package.join("old-program"), b"matching package fixture").unwrap();
    let runtime = open(hiroute_local_storage::StorageStartupOptions {
        recovery_package: Some(package),
        ..Default::default()
    })
    .unwrap();
    let backup = hiroute_local_storage::upgrade_backup_root(&root).unwrap();
    let source = backup.join("source");
    assert!(!source.join("storage/observation").exists());
    assert_eq!(
        fs::read(source.join("recovery-package/old-program")).unwrap(),
        b"matching package fixture"
    );
    for key in ["master-key", "observation-hmac-key"] {
        assert_eq!(
            fs::read(root.join(key)).unwrap(),
            fs::read(source.join("storage").join(key)).unwrap()
        );
    }
    for name in ["control.db", "runtime.db", "secrets.db"] {
        let old = Connection::open(source.join("storage/live").join(name)).unwrap();
        assert_eq!(
            old.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            35
        );
    }
    drop(runtime);
    let source_manifest = fs::read(source.join("source-backup.json")).unwrap();
    let runtime = open(Default::default()).unwrap();
    assert_eq!(
        fs::read(source.join("source-backup.json")).unwrap(),
        source_manifest
    );
    assert_eq!(
        runtime
            .observation
            .capacity_status()
            .unwrap()
            .max_active_bytes,
        Some(10_000_000_000)
    );
    assert_eq!(fs::read(root.join("observation-hmac-key")).unwrap(), hmac);
    assert_eq!(fs::read(root.join("master-key")).unwrap(), master);
    assert_eq!(
        fs::read(root.join("observation/retained-body")).unwrap(),
        b"exact retained contents"
    );
    let db = Connection::open(&observation).unwrap();
    assert_eq!(grant_count(&db, "retained-grant"), 1);
    assert_eq!(
        db.query_row(
            "SELECT value FROM observation_meta WHERE key='generation_probe'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "retained-current-history"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(&observation).unwrap();
        assert_eq!((metadata.dev(), metadata.ino()), identity);
    }
}

#[test]
fn production_startup_resets_old_observation_and_preserves_the_new_generation_on_restart() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::history_cutover_tests::production_startup_resets_old_observation_and_preserves_the_new_generation_on_restart",
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
    original
        .observation
        .set_capacity_limit(Some(10_000_000_000))
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
    let runtime = open(Default::default()).unwrap();
    assert!(!root.join("observation-source-backup.json").exists());
    let progress: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join(".observation.format6-reset.json")).unwrap())
            .unwrap();
    let source = root.join(format!(
        ".observation.retired-{}",
        progress["token"].as_str().unwrap()
    ));
    assert_eq!(
        fs::read(source.join("original-object")).unwrap(),
        b"unmodified old contents"
    );
    let db = Connection::open(source.join("activity.db")).unwrap();
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
        Some(10_000_000_000)
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
        "6"
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
