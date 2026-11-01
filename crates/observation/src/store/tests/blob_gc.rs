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
                 VALUES('workspace',?1,?1,?1,'response','fork',0,'text','text/plain',?2,1,7,'complete',1)",
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
fn idle_blob_gc_does_not_take_the_capture_writer_for_candidate_metadata() {
    let root = tempdir().unwrap();
    let store = std::sync::Arc::new(open_store(root.path()));
    seed(&store, 32, 1);
    let writer = store.connection.lock();
    std::thread::scope(|threads| {
        let (finished, result) = std::sync::mpsc::channel();
        let worker_store = std::sync::Arc::clone(&store);
        threads.spawn(move || {
            finished
                .send(worker_store.collect_garbage_batch(32))
                .unwrap();
        });
        let idle = result.recv_timeout(std::time::Duration::from_millis(500));
        // Release even on failure, so the negative control cannot leave a
        // blocked maintenance thread behind when the assertion unwinds.
        drop(writer);
        assert_eq!(idle.unwrap().unwrap(), 0);
    });
}

#[test]
fn blob_gc_candidate_metadata_merges_covering_identities_before_opening_blob_rows() {
    let root = tempdir().unwrap();
    let store = open_store(root.path());
    let db = store.connection.lock();
    let mut statement = db
        .prepare(&format!("EXPLAIN QUERY PLAN {GARBAGE_CANDIDATES_SQL}"))
        .unwrap();
    let plan = statement
        .query_map([32], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        plan.iter().any(|step| step == "MERGE (EXCEPT)"),
        "idle candidate work must merge ordered covering sources: {plan:?}"
    );
    assert!(
        plan.iter().any(|step| step
            .contains("content_blobs_v2 USING COVERING INDEX sqlite_autoindex_content_blobs_v2_1")),
        "live blob metadata must not be opened: {plan:?}"
    );
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
        // Each current index can satisfy this lookup. Remove all alternatives
        // so the negative control still represents the original unindexed path.
        .execute_batch(
            "DROP INDEX content_instances_v2_live_blob;
             DROP INDEX content_instances_v2_search_blob;
             DROP INDEX content_instances_v2_live_blob_global;",
        )
        .unwrap();
    // Negative control for the original correlated lookup. The current merge
    // algorithm also avoids its quadratic work when an index is unavailable.
    let without_index = {
        let db = store.connection.lock();
        let mut statement = db.prepare(
            "SELECT b.workspace_id,b.blob_digest FROM content_blobs_v2 b WHERE NOT EXISTS(
             SELECT 1 FROM content_instances_v2 c WHERE c.workspace_id=b.workspace_id AND c.content_blob_digest=b.blob_digest AND c.state='complete') ORDER BY b.rowid LIMIT ?1",
        ).unwrap();
        assert!(!statement.exists([32]).unwrap());
        statement.get_status(StatementStatus::VmStep)
    };
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
