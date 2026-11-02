use super::*;

fn legacy(root: &Path, version: &str) {
    fs::create_dir_all(root).unwrap();
    let db = Connection::open(root.join("activity.db")).unwrap();
    db.execute_batch("CREATE TABLE observation_meta(key TEXT PRIMARY KEY,value TEXT); CREATE TABLE logical_requests(id TEXT); CREATE TABLE observation_events(id TEXT); CREATE TABLE observation_checkpoints(id TEXT); CREATE TABLE observation_feedback_v2(id TEXT); CREATE TABLE conversation_content_events_v2(id TEXT); CREATE TABLE content_blobs_v2(id TEXT); CREATE TABLE content_instances_v2(id TEXT);").unwrap();
    db.execute(
        "INSERT INTO observation_meta VALUES('schema_version',?1)",
        [version],
    )
    .unwrap();
    db.execute(
        "INSERT INTO observation_events VALUES('original history')",
        [],
    )
    .unwrap();
    fs::write(root.join("original-object"), b"retained bytes").unwrap();
}

#[test]
fn old_formats_are_classified_for_reset_without_conversion_or_new_files() {
    for version in ["1", "2", "3", "4", "5"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("observation");
        legacy(&root, version);
        let before = fs::read(root.join("activity.db")).unwrap();
        assert!(required(&root).unwrap());
        assert!(prepare(&root).is_err());
        assert_eq!(fs::read(root.join("activity.db")).unwrap(), before);
        assert_eq!(
            fs::read(root.join("original-object")).unwrap(),
            b"retained bytes"
        );
        assert!(!root.join("conversation-content").exists());
        assert!(!paths(&root).unwrap().1.exists());
    }
}

#[test]
fn interrupted_converter_roots_and_unknown_formats_remain_preserved() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("observation");
    legacy(&root, "999");
    let before = fs::read(root.join("activity.db")).unwrap();
    assert!(prepare(&root).is_err());
    assert_eq!(fs::read(root.join("activity.db")).unwrap(), before);
    let (old, target, journal) = paths(&root).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(target.join("data"), b"real intermediate data").unwrap();
    fs::write(&journal, b"old converter progress").unwrap();
    assert!(required(&root).is_err());
    fs::rename(&root, &old).unwrap();
    assert!(prepare(&root).is_err());
    assert!(!root.exists());
    assert_eq!(
        fs::read(target.join("data")).unwrap(),
        b"real intermediate data"
    );
    assert_eq!(fs::read(old.join("activity.db")).unwrap(), before);
}

#[test]
fn missing_database_with_retained_files_is_never_fresh() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("observation");
    fs::create_dir_all(root.join("conversation-content/blobs")).unwrap();
    fs::write(
        root.join("conversation-content/blobs/retained"),
        b"do not reset",
    )
    .unwrap();
    assert!(prepare(&root).is_err());
    assert!(!root.join("activity.db").exists());
}

#[test]
fn current_history_and_capacity_policy_survive_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("observation");
    let authority = crate::DigestAuthority::new([7; 32]);
    let store = crate::LocalObservationStore::open(&root, authority.clone()).unwrap();
    store.set_capacity_limit(Some(20_000_000_000)).unwrap();
    store
        .connection
        .lock()
        .execute(
            "INSERT INTO observation_meta VALUES('retained_probe','original-current-data')",
            [],
        )
        .unwrap();
    drop(store);
    assert!(!required(&root).unwrap());
    let store = crate::LocalObservationStore::open(&root, authority).unwrap();
    assert_eq!(
        store
            .connection
            .lock()
            .query_row(
                "SELECT value FROM observation_meta WHERE key='retained_probe'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "original-current-data"
    );
    assert_eq!(
        store.capacity_status().unwrap().max_active_bytes,
        Some(20_000_000_000)
    );
}

#[cfg(unix)]
#[test]
fn a_link_cannot_supply_the_live_observation_root() {
    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("original");
    legacy(&original, "4");
    let root = directory.path().join("observation");
    std::os::unix::fs::symlink(&original, &root).unwrap();
    assert!(required(&root).is_err());
    assert_eq!(
        fs::read(original.join("original-object")).unwrap(),
        b"retained bytes"
    );
}

#[test]
fn fresh_initialization_preserves_unpublished_material_and_publishes_only_current_format() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("observation");
    let interrupted = directory.path().join(".observation-fresh-0000000000000000");
    fs::create_dir(&interrupted).unwrap();
    fs::write(
        interrupted.join("partial-db"),
        b"unpublished initialization",
    )
    .unwrap();
    let authority = crate::DigestAuthority::new([7; 32]);
    let store =
        crate::LocalObservationStore::open_fresh_after_seal(&root, authority.clone()).unwrap();
    assert_eq!(version(&root).unwrap().as_deref(), Some("6"));
    assert_eq!(
        fs::read(interrupted.join("partial-db")).unwrap(),
        b"unpublished initialization"
    );
    store.set_capacity_limit(Some(20_000_000_000)).unwrap();
    drop(store);
    let reopened = crate::LocalObservationStore::open_fresh_after_seal(&root, authority).unwrap();
    assert_eq!(
        reopened.capacity_status().unwrap().max_active_bytes,
        Some(20_000_000_000)
    );
}
