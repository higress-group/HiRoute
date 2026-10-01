use super::support::open_store;
use crate::LocalObservationStore;
use rusqlite::{StatementStatus, params};
use tempfile::tempdir;

use super::super::retention::GARBAGE_CANDIDATES_SQL;

fn seed(store: &LocalObservationStore, blobs: usize, references: usize) {
    let mut connection = store.connection.lock();
    let tx = connection.transaction().unwrap();
    for id in 0..blobs {
        let digest = format!("blob-{id:06}");
        let path = store.blob_path(&digest);
        std::fs::write(&path, b"content").unwrap();
        tx.execute(
            "INSERT INTO content_blobs_v2 VALUES('workspace',?1,?2,7,'text/plain','complete',1)",
            params![digest, path.to_string_lossy()],
        )
        .unwrap();
        for reference in 0..references {
            tx.execute(
                "INSERT INTO content_instances_v2
                 (workspace_id,content_id,message_instance_id,request_id,direction,fork_id,
                  part_ordinal,content_kind,canonical_media_type,content_blob_digest,
                  has_content_ref,accumulated_byte_count,state,created_at_unix_nanos)
                 VALUES('workspace',?1,'message',?1,'response','fork',0,'text','text/plain',?2,1,7,'complete',1)",
                params![format!("content-{id:06}-{reference}"), digest],
            ).unwrap();
        }
    }
    tx.commit().unwrap();
}

fn idle_query_steps(store: &LocalObservationStore) -> i32 {
    let connection = store.connection.lock();
    let mut statement = connection.prepare(GARBAGE_CANDIDATES_SQL).unwrap();
    assert!(!statement.exists([32]).unwrap());
    statement.get_status(StatementStatus::VmStep)
}

#[test]
fn blob_gc_live_lookup_stays_linear_and_existing_databases_gain_index() {
    let root = tempdir().unwrap();
    let store = open_store(root.path());
    seed(&store, 1024, 2);
    let indexed = idle_query_steps(&store);
    // Count SQLite work rather than asserting machine-dependent wall-clock time.
    assert!(indexed < 1024 * 50, "indexed VM steps: {indexed}");
    store
        .connection
        .lock()
        .execute_batch("DROP INDEX content_instances_v2_live_blob")
        .unwrap();
    let without_index = idle_query_steps(&store);
    assert!(
        without_index > indexed * 20,
        "regression fixture must expose the expensive old lookup"
    );
    drop(store);

    // Opening an existing current-schema database must install the index too;
    // opening it again must remain idempotent and preserve all content.
    for _ in 0..2 {
        let reopened = open_store(root.path());
        assert!(idle_query_steps(&reopened) < 1024 * 50);
        assert_eq!(reopened.collect_garbage_batch(32).unwrap(), 0);
        assert!(reopened.blob_path("blob-000000").exists());
    }
}

#[test]
fn blob_gc_preserves_shared_live_content_until_the_last_reference_is_removed() {
    let root = tempdir().unwrap();
    let store = open_store(root.path());
    seed(&store, 1, 2);
    let path = store.blob_path("blob-000000");
    store
        .connection
        .lock()
        .execute(
            "DELETE FROM content_instances_v2 WHERE content_id='content-000000-0'",
            [],
        )
        .unwrap();
    assert_eq!(store.collect_garbage_batch(32).unwrap(), 0);
    assert!(path.exists());
    store
        .connection
        .lock()
        .execute(
            "DELETE FROM content_instances_v2 WHERE content_id='content-000000-1'",
            [],
        )
        .unwrap();
    assert_eq!(store.collect_garbage_batch(32).unwrap(), 1);
    assert!(!path.exists());
    assert_eq!(store.collect_garbage_batch(32).unwrap(), 0);
}

#[test]
fn blob_gc_full_batch_leaves_a_backlog_and_partial_batch_finishes_it() {
    let root = tempdir().unwrap();
    let store = open_store(root.path());
    seed(&store, 33, 0);
    assert_eq!(store.collect_garbage_batch(32).unwrap(), 32);
    assert_eq!(store.collect_garbage_batch(32).unwrap(), 1);
    assert_eq!(store.collect_garbage_batch(32).unwrap(), 0);
    assert_eq!(std::fs::read_dir(&store.blob_root).unwrap().count(), 0);
}
