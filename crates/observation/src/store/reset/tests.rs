use super::*;

fn legacy(parent: &Path, setting: Option<&str>) -> PathBuf {
    let root = parent.join("observation");
    private_fs::create_directory(&root).unwrap();
    // Keep SQLite's original inherited permissions: admission must not require
    // that old data was written through the current atomic-file primitive.
    let db = Connection::open(root.join("activity.db")).unwrap();
    db.execute_batch("CREATE TABLE observation_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL); INSERT INTO observation_meta VALUES('schema_version','3'); CREATE TABLE old_history(body TEXT); INSERT INTO old_history VALUES('old observations');").unwrap();
    if let Some(value) = setting {
        db.execute(
            "INSERT INTO observation_meta VALUES('max_active_bytes',?1)",
            [value],
        )
        .unwrap();
    }
    drop(db);
    fs::write(root.join("old-body"), b"do not copy me").unwrap();
    root
}

fn authority() -> DigestAuthority {
    DigestAuthority::new([42; 32])
}

#[test]
fn fresh_database_reopens_through_private_admission_without_process_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("observation");
    let store = LocalObservationStore::open(&root, authority()).unwrap();
    store.set_capacity_limit(Some(10_000_000_000)).unwrap();
    drop(store);
    private_fs::open_private_file(&root.join("activity.db")).unwrap();
    let reopened = LocalObservationStore::open(&root, authority()).unwrap();
    assert_eq!(
        reopened.capacity_status().unwrap().max_active_bytes,
        Some(10_000_000_000)
    );
}

#[test]
fn format_reset_preserves_capacity_choices_without_copying_observations_or_native_history() {
    for (setting, expected) in [
        (None, Some(20_000_000_000)),
        (Some("10000000000"), Some(10_000_000_000)),
        (Some("null"), None),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = legacy(dir.path(), setting);
        let native = dir.path().join("native-history.db");
        fs::write(&native, b"original native conversation").unwrap();
        let old_id = directory_identity(&root).unwrap();
        #[cfg(windows)]
        let old_permissions = hiroute_platform::file_security::capture(
            &private_fs::open_regular_file(&root.join("activity.db")).unwrap(),
        )
        .unwrap();
        let store = LocalObservationStore::open_for_daemon_startup(&root, authority()).unwrap();
        assert_eq!(store.capacity_status().unwrap().max_active_bytes, expected);
        assert_eq!(upgrade::version(&root).unwrap().as_deref(), Some("6"));
        assert!(!root.join("old-body").exists());
        let record = read_record(&journal(&root).unwrap()).unwrap();
        let (retired, _) = paths(&root, &record).unwrap();
        assert_eq!(directory_identity(&retired).unwrap(), old_id);
        #[cfg(windows)]
        assert_eq!(
            hiroute_platform::file_security::capture(
                &private_fs::open_regular_file(&retired.join("activity.db")).unwrap(),
            )
            .unwrap(),
            old_permissions,
            "reset must not rewrite the old SQLite file's owner or DACL"
        );
        assert_eq!(
            fs::read(retired.join("old-body")).unwrap(),
            b"do not copy me"
        );
        store.cleanup_retired_format().unwrap();
        assert!(retired.exists(), "SQL open is not service readiness");
        assert_eq!(fs::read(native).unwrap(), b"original native conversation");
        // A restart must keep newly written data and a newer setting.
        store.set_capacity_limit(Some(12345)).unwrap();
        store
            .connection
            .lock()
            .execute(
                "INSERT INTO observation_meta VALUES('new-data','survives')",
                [],
            )
            .unwrap();
        drop(store);
        let reopened = LocalObservationStore::open_for_daemon_startup(&root, authority()).unwrap();
        assert_eq!(
            reopened.capacity_status().unwrap().max_active_bytes,
            Some(12345)
        );
        assert_eq!(
            reopened
                .connection
                .lock()
                .query_row(
                    "SELECT value FROM observation_meta WHERE key='new-data'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "survives"
        );
        reopened.service_ready();
        reopened.cleanup_retired_format().unwrap();
        assert!(!retired.exists());
        assert!(!journal(&root).unwrap().exists());
    }
}

fn interrupted(parent: &Path) -> (PathBuf, Record) {
    let root = legacy(parent, Some("10000000000"));
    let token = "0123456789abcdef0123456789abcdef".to_owned();
    let stage = parent.join(format!(".observation.fresh-{token}"));
    private_fs::create_directory(&stage).unwrap();
    let store = LocalObservationStore::open(&stage, authority()).unwrap();
    store
        .connection
        .lock()
        .execute(
            "UPDATE observation_meta SET value=?1 WHERE key='format_generation'",
            [&token],
        )
        .unwrap();
    store.set_capacity_limit(Some(10_000_000_000)).unwrap();
    store
        .connection
        .lock()
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    drop(store);
    let record = Record {
        schema: 1,
        parent: directory_identity(parent).unwrap(),
        source: directory_identity(&root).unwrap(),
        fresh: directory_identity(&stage).unwrap(),
        token,
    };
    durable::write_private_new(
        &journal(&root).unwrap(),
        &serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    (root, record)
}

#[test]
fn format_reset_resumes_each_rename_boundary_and_never_republishes_over_current_data() {
    for phase in 0..3 {
        let dir = tempfile::tempdir().unwrap();
        let (root, record) = interrupted(dir.path());
        let (retired, stage) = paths(&root, &record).unwrap();
        if phase >= 1 {
            durable::rename(&root, &retired).unwrap();
        }
        if phase >= 2 {
            durable::rename(&stage, &root).unwrap();
        }
        let store = LocalObservationStore::open_for_daemon_startup(&root, authority()).unwrap();
        assert_eq!(directory_identity(&root).unwrap(), record.fresh);
        assert_eq!(
            store.capacity_status().unwrap().max_active_bytes,
            Some(10_000_000_000)
        );
        assert_eq!(
            fs::read(retired.join("old-body")).unwrap(),
            b"do not copy me"
        );
    }
}

#[test]
fn format_reset_preserves_unknown_corrupt_and_replaced_generations() {
    for version in ["6", "999", "broken"] {
        let dir = tempfile::tempdir().unwrap();
        let root = legacy(dir.path(), None);
        Connection::open(root.join("activity.db"))
            .unwrap()
            .execute(
                "UPDATE observation_meta SET value=?1 WHERE key='schema_version'",
                [version],
            )
            .unwrap();
        assert!(LocalObservationStore::open_for_daemon_startup(&root, authority()).is_err());
        assert_eq!(fs::read(root.join("old-body")).unwrap(), b"do not copy me");
        assert!(!journal(&root).unwrap().exists());
    }
    let dir = tempfile::tempdir().unwrap();
    let (root, record) = interrupted(dir.path());
    let original = dir.path().join("unrelated-recovery");
    fs::rename(&root, &original).unwrap();
    private_fs::create_directory(&root).unwrap();
    fs::write(root.join("foreign"), b"preserve").unwrap();
    assert!(LocalObservationStore::open_for_daemon_startup(&root, authority()).is_err());
    assert_eq!(fs::read(root.join("foreign")).unwrap(), b"preserve");
    let (_, stage) = paths(&root, &record).unwrap();
    assert!(stage.exists());
}

#[test]
fn retired_cleanup_is_bounded_and_rejects_replaced_paths() {
    let dir = tempfile::tempdir().unwrap();
    let root = legacy(dir.path(), None);
    for i in 0..400 {
        fs::write(root.join(format!("body-{i}")), b"x").unwrap();
    }
    let store = LocalObservationStore::open_for_daemon_startup(&root, authority()).unwrap();
    let record = read_record(&journal(&root).unwrap()).unwrap();
    let (retired, _) = paths(&root, &record).unwrap();
    store.service_ready();
    store.cleanup_retired_format().unwrap();
    assert!(retired.exists());
    assert!(fs::read_dir(&retired).unwrap().count() >= 270);
    let protected = dir.path().join("protected-original");
    fs::rename(&retired, &protected).unwrap();
    private_fs::create_directory(&retired).unwrap();
    fs::write(retired.join("foreign"), b"preserve").unwrap();
    store.reset_cleanup.lock().due = Instant::now();
    assert!(store.cleanup_retired_format().is_err());
    assert_eq!(fs::read(retired.join("foreign")).unwrap(), b"preserve");
}

#[test]
fn query_cursors_are_bound_to_the_observation_generation_not_only_the_installation_key() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let a = LocalObservationStore::open(first.path(), authority()).unwrap();
    let b = LocalObservationStore::open(second.path(), authority()).unwrap();
    let cursor = a
        .encode_observation_cursor(&serde_json::json!({"same_query":true}))
        .unwrap();
    assert!(
        a.decode_observation_cursor::<serde_json::Value>(&cursor)
            .is_ok()
    );
    assert!(
        b.decode_observation_cursor::<serde_json::Value>(&cursor)
            .is_err()
    );
    drop(a);
    let a = LocalObservationStore::open(first.path(), authority()).unwrap();
    assert!(
        a.decode_observation_cursor::<serde_json::Value>(&cursor)
            .is_ok()
    );
}

#[test]
fn current_format_missing_immutable_capture_state_is_preserved_as_corrupt() {
    for table in [
        "observation_current_batch_v6",
        "observation_capture_scopes_v6",
        "observation_input_anchors_v6",
        "retired_execution_identities_v6",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("observation");
        let store = LocalObservationStore::open(&root, authority()).unwrap();
        store.set_capacity_limit(Some(10_000_000_000)).unwrap();
        drop(store);
        let db = Connection::open(root.join("activity.db")).unwrap();
        db.execute_batch(&format!("DROP TABLE {table}")).unwrap();
        drop(db);
        assert!(matches!(upgrade::required(&root), Err(Error::Corrupt)));
        assert!(matches!(
            LocalObservationStore::open_for_daemon_startup(&root, authority()),
            Err(Error::Corrupt)
        ));
        assert!(!journal(&root).unwrap().exists());
        let db = Connection::open(root.join("activity.db")).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT value FROM observation_meta WHERE key='max_active_bytes'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "10000000000"
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name=?1",
                [table],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
    }
}
