use super::*;
use crate::store::tests::support::open_store;
use rusqlite::StatementStatus;
use tempfile::tempdir;

fn seed_live(store: &LocalObservationStore, count: usize) {
    let mut db = store.connection.lock();
    let tx = db.transaction().unwrap();
    tx.execute(
        "WITH RECURSIVE ids(n) AS (SELECT 0 UNION ALL SELECT n+1 FROM ids WHERE n+1<?1)
         INSERT INTO content_blobs_v2
         SELECT 'workspace',printf('blob-%06d',n),'conversation-content/blobs/missing',7,'text/plain','complete',1 FROM ids",
        [count],
    )
    .unwrap();
    tx.execute_batch(
        "INSERT INTO content_instances_v2
         (workspace_id,content_id,message_instance_id,request_id,direction,fork_id,
          part_ordinal,content_kind,canonical_media_type,content_blob_digest,
          has_content_ref,accumulated_byte_count,state,created_at_unix_nanos)
         SELECT workspace_id,blob_digest,'message',blob_digest,'response','fork',0,
          'text','text/plain',blob_digest,1,7,'complete',1 FROM content_blobs_v2;",
    )
    .unwrap();
    tx.commit().unwrap();
}

fn orphan(store: &LocalObservationStore, digest: &str) {
    let path = store.blob_path(digest);
    std::fs::write(&path, b"content").unwrap();
    store
        .connection
        .lock()
        .execute(
            "INSERT INTO content_blobs_v2 VALUES('workspace',?1,?2,7,'text/plain','complete',1)",
            params![digest, path.to_string_lossy()],
        )
        .unwrap();
}

fn scan_steps(store: &LocalObservationStore, after: &str) -> i32 {
    let db = store.connection.lock();
    let mut statement = db.prepare(SCAN_SQL).unwrap();
    let rows = statement
        .query_map(params!["workspace", after, SCAN_ROWS], |row| {
            row.get::<_, bool>(2)
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows.len(), SCAN_ROWS);
    assert!(rows.iter().all(|orphan| !orphan));
    statement.get_status(StatementStatus::VmStep)
}

#[test]
fn idle_scan_work_is_bounded_independently_of_history_and_cursor_position() {
    let root = tempdir().unwrap();
    let store = open_store(root.path());
    seed_live(&store, 32_768);
    let first = scan_steps(&store, "");
    let middle = scan_steps(&store, "blob-016383");
    for steps in [first, middle] {
        assert!(steps < 20_000, "one 256-row page used {steps} VM steps");
    }
    assert!(
        (first - middle).abs() < 200,
        "keyset seek must skip the prefix"
    );

    // The original LIMIT bounded orphan matches, so even an empty pass consumed
    // all live history. Keep this negative control to prove fixture sensitivity.
    let db = store.connection.lock();
    let mut old = db
        .prepare(super::super::retention::GARBAGE_CANDIDATES_SQL)
        .unwrap();
    assert!(!old.exists([DELETE_ROWS]).unwrap());
    assert!(old.get_status(StatementStatus::VmStep) > first * 20);
    let mut plan = db
        .prepare(&format!("EXPLAIN QUERY PLAN {SCAN_SQL}"))
        .unwrap();
    let details = plan
        .query_map(params!["workspace", "blob-016383", SCAN_ROWS], |r| {
            r.get::<_, String>(3)
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        details
            .iter()
            .any(|s| s.contains("SEARCH b USING COVERING INDEX")),
        "{details:?}"
    );
    assert!(
        !details
            .iter()
            .any(|s| s.contains("SCAN c") || s.contains("TEMP B-TREE")),
        "{details:?}"
    );
}

#[test]
fn incremental_scan_yields_past_live_prefix_and_eventually_collects_tail() {
    let root = tempdir().unwrap();
    let store = open_store(root.path());
    seed_live(&store, SCAN_ROWS * 2);
    orphan(&store, "tail");
    let mut cursor = BlobGcCursor::default();
    for _ in 0..2 {
        assert_eq!(
            store.collect_garbage_incremental(&mut cursor).unwrap(),
            (0, true)
        );
        assert!(store.blob_path("tail").exists());
    }
    assert_eq!(
        store.collect_garbage_incremental(&mut cursor).unwrap(),
        (1, false)
    );
    assert!(cursor.after.is_none());
    assert!(!store.blob_path("tail").exists());
}

#[test]
fn deletion_limit_never_skips_remaining_orphans_in_the_scanned_page() {
    let root = tempdir().unwrap();
    let store = open_store(root.path());
    for n in 0..70 {
        orphan(&store, &format!("orphan-{n:03}"));
    }
    let mut cursor = BlobGcCursor::default();
    for expected in [(32, true), (32, true), (6, false), (0, false)] {
        assert_eq!(
            store.collect_garbage_incremental(&mut cursor).unwrap(),
            expected
        );
    }
    assert_eq!(std::fs::read_dir(&store.blob_root).unwrap().count(), 0);
}

#[test]
fn changes_behind_cursor_and_interrupted_sweeps_are_revisited() {
    let root = tempdir().unwrap();
    let store = open_store(root.path());
    seed_live(&store, 300);
    let mut cursor = BlobGcCursor::default();
    assert_eq!(
        store.collect_garbage_incremental(&mut cursor).unwrap(),
        (0, true)
    );
    store
        .connection
        .lock()
        .execute(
            "DELETE FROM content_instances_v2 WHERE content_id='blob-000000'",
            [],
        )
        .unwrap();
    assert_eq!(
        store.collect_garbage_incremental(&mut cursor).unwrap(),
        (0, false)
    );
    assert_eq!(
        store.collect_garbage_incremental(&mut cursor).unwrap(),
        (1, true)
    );
    // A process-local cursor is deliberately not persisted. Restarting mid-sweep
    // must begin at the first identity and retain all remaining live rows.
    drop(store);
    let reopened = open_store(root.path());
    let mut restarted = BlobGcCursor::default();
    assert_eq!(
        reopened
            .collect_garbage_incremental(&mut restarted)
            .unwrap(),
        (0, true)
    );
    assert_eq!(
        reopened
            .collect_garbage_incremental(&mut restarted)
            .unwrap(),
        (0, false)
    );
    assert_eq!(
        reopened
            .connection
            .lock()
            .query_row("SELECT count(*) FROM content_blobs_v2", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        299
    );
}

#[test]
fn idle_incremental_discovery_does_not_take_capture_writer() {
    let root = tempdir().unwrap();
    let store = std::sync::Arc::new(open_store(root.path()));
    seed_live(&store, 300);
    let writer = store.connection.lock();
    std::thread::scope(|threads| {
        let (send, recv) = std::sync::mpsc::channel();
        let store = std::sync::Arc::clone(&store);
        threads.spawn(move || {
            send.send(store.collect_garbage_incremental(&mut BlobGcCursor::default()))
                .unwrap();
        });
        let result = recv.recv_timeout(std::time::Duration::from_millis(500));
        drop(writer);
        assert_eq!(result.unwrap().unwrap(), (0, true));
    });
}

#[test]
fn failed_deletion_keeps_cursor_for_retry() {
    let root = tempdir().unwrap();
    let store = open_store(root.path());
    orphan(&store, "orphan");
    let path = store.blob_path("orphan");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let mut cursor = BlobGcCursor::default();
    assert!(store.collect_garbage_incremental(&mut cursor).is_err());
    assert!(cursor.after.is_none());
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, b"content").unwrap();
    assert_eq!(
        store.collect_garbage_incremental(&mut cursor).unwrap(),
        (1, false)
    );
}

#[test]
fn incremental_scan_preserves_capacity_protection_until_released() {
    let root = tempdir().unwrap();
    let store = open_store(root.path());
    orphan(&store, "protected");
    store
        .connection
        .lock()
        .execute(
            "INSERT INTO observation_capacity_blobs_v5 VALUES('workspace','protected')",
            [],
        )
        .unwrap();
    let mut cursor = BlobGcCursor::default();
    assert_eq!(
        store.collect_garbage_incremental(&mut cursor).unwrap(),
        (0, false)
    );
    assert_eq!(
        std::fs::read(store.blob_path("protected")).unwrap(),
        b"content"
    );
    store
        .connection
        .lock()
        .execute("DELETE FROM observation_capacity_blobs_v5", [])
        .unwrap();
    assert_eq!(
        store.collect_garbage_incremental(&mut cursor).unwrap(),
        (1, false)
    );
}

#[test]
fn incremental_collection_keeps_shared_bytes_in_another_workspace() {
    use crate::store::tests::support::{Fixture, install_content};
    let root = tempdir().unwrap();
    let store = open_store(root.path());
    let expired = Fixture::new("incremental-expired");
    let mut live = Fixture::new("incremental-live");
    live.workspace = hiroute_domain::WorkspaceId::parse("other-workspace").unwrap();
    let body = b"one file shared by independent workspace catalogs";
    install_content(&store, &expired, body, 100);
    install_content(&store, &live, body, 200);
    store
        .connection
        .lock()
        .execute(
            "UPDATE content_instances_v2 SET state='installing' WHERE workspace_id=?1",
            [expired.workspace.as_str()],
        )
        .unwrap();
    let mut cursor = BlobGcCursor::default();
    assert_eq!(
        store.collect_garbage_incremental(&mut cursor).unwrap(),
        (1, false)
    );
    let digest = store.authority.content_blob_digest("text/plain", body);
    assert_eq!(
        std::fs::read(store.blob_path(digest.as_str())).unwrap(),
        body
    );
    assert_eq!(
        store.collect_garbage_incremental(&mut cursor).unwrap(),
        (0, false)
    );
}

#[test]
fn retained_gc_reader_releases_each_snapshot_and_observes_later_commits() {
    let root = tempdir().unwrap();
    let store = open_store(root.path());
    seed_live(&store, 300);
    let mut cursor = BlobGcCursor::default();
    assert_eq!(
        store.collect_garbage_incremental(&mut cursor).unwrap(),
        (0, true)
    );
    assert!(cursor.connection.as_ref().unwrap().is_autocommit());
    orphan(&store, "tail");
    // A retained connection must not leave a snapshot pinning the WAL between
    // ticks. Capture commits and the existing page-release path remain usable.
    let busy = store
        .connection
        .lock()
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
    assert_eq!(busy, 0);
    assert_eq!(
        store.collect_garbage_incremental(&mut cursor).unwrap(),
        (1, false)
    );
    assert!(!store.blob_path("tail").exists());
    assert!(cursor.connection.as_ref().unwrap().is_autocommit());
    // Reuse remains valid over a completed sweep and sees a new later commit.
    orphan(&store, "tail-next");
    assert_eq!(
        store.collect_garbage_incremental(&mut cursor).unwrap(),
        (0, true)
    );
    assert_eq!(
        store.collect_garbage_incremental(&mut cursor).unwrap(),
        (1, false)
    );
}
