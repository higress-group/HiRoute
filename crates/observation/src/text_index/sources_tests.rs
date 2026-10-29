use super::{sources::take_dirty, worker::TextIndexBuilder};
use crate::{DigestAuthority, LocalObservationStore, query_v2::search_tests};

fn store(root: &std::path::Path) -> LocalObservationStore {
    LocalObservationStore::open(root, DigestAuthority::new([2; 32])).unwrap()
}

fn hits(store: &LocalObservationStore) -> usize {
    store
        .search_observed_text(
            &search_tests::reader(true),
            &search_tests::query("needle", 10),
            500,
        )
        .unwrap()
        .hits
        .len()
}

#[test]
fn source_notifications_commit_with_peer_completion_and_rollback_with_it() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path());
    search_tests::source(&store, b"late needle");
    store
        .connection
        .lock()
        .execute("UPDATE content_instances_v2 SET state='installing'", [])
        .unwrap();
    let mut builder = TextIndexBuilder::new(&store).unwrap();
    assert_eq!(builder.cycle(&store).unwrap(), 0);
    let mut peer = rusqlite::Connection::open(&store.activity_path).unwrap();
    {
        let tx = peer.transaction().unwrap();
        tx.execute("UPDATE content_instances_v2 SET state='complete'", [])
            .unwrap();
        // Drop rolls back both the liveness change and its notification.
    }
    assert_eq!(take_dirty(&store).unwrap(), None);
    assert_eq!(builder.cycle(&store).unwrap(), 0);
    peer.execute("UPDATE content_instances_v2 SET state='complete'", [])
        .unwrap();
    assert_eq!(builder.cycle(&store).unwrap(), 1);
    assert_eq!(hits(&store), 1);
    assert_eq!(take_dirty(&store).unwrap(), None);
}

#[test]
fn blob_completion_after_its_reference_wakes_the_idle_builder() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path());
    search_tests::source(&store, b"late blob needle");
    store
        .connection
        .lock()
        .execute("UPDATE content_blobs_v2 SET state='installing'", [])
        .unwrap();
    let mut builder = TextIndexBuilder::new(&store).unwrap();
    assert_eq!(builder.cycle(&store).unwrap(), 0);
    store
        .connection
        .lock()
        .execute("UPDATE content_blobs_v2 SET state='complete'", [])
        .unwrap();
    assert_eq!(builder.cycle(&store).unwrap(), 1);
    assert_eq!(hits(&store), 1);
}

#[test]
fn source_publication_after_a_complete_reference_notifies_without_a_revision_change() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path());
    search_tests::source(&store, b"restored blob needle");
    let db = store.connection.lock();
    db.execute_batch(
        "CREATE TEMP TABLE held_blob AS SELECT * FROM content_blobs_v2;
        DELETE FROM content_blobs_v2;",
    )
    .unwrap();
    drop(db);
    let mut builder = TextIndexBuilder::new(&store).unwrap();
    assert_eq!(builder.cycle(&store).unwrap(), 0);
    store
        .connection
        .lock()
        .execute("INSERT INTO content_blobs_v2 SELECT * FROM held_blob", [])
        .unwrap();
    assert_eq!(builder.cycle(&store).unwrap(), 1);
    assert_eq!(hits(&store), 1);
}

#[test]
fn deleting_a_derived_index_rechecks_the_live_source_and_restart_is_idempotent() {
    let root = tempfile::tempdir().unwrap();
    {
        let store = store(root.path());
        search_tests::source(&store, b"retained needle");
        let mut builder = TextIndexBuilder::new(&store).unwrap();
        assert_eq!(builder.cycle(&store).unwrap(), 1);
        store
            .connection
            .lock()
            .execute_batch(
                "DELETE FROM observation_text_blocks_v2;
            DELETE FROM observation_text_index_v2;",
            )
            .unwrap();
        assert_eq!(builder.cycle(&store).unwrap(), 1);
        assert_eq!(hits(&store), 1);
    }
    for _ in 0..2 {
        let store = store(root.path());
        let mut builder = TextIndexBuilder::new(&store).unwrap();
        assert_eq!(builder.cycle(&store).unwrap(), 0);
        assert_eq!(hits(&store), 1);
        assert_eq!(take_dirty(&store).unwrap(), None);
    }
}

#[test]
fn restart_recovers_a_notification_consumed_before_indexing() {
    let root = tempfile::tempdir().unwrap();
    {
        let store = store(root.path());
        search_tests::source(&store, b"recoverable needle");
        assert_eq!(take_dirty(&store).unwrap(), Some(1));
    }
    let store = store(root.path());
    let mut builder = TextIndexBuilder::new(&store).unwrap();
    assert_eq!(builder.cycle(&store).unwrap(), 1);
    assert_eq!(hits(&store), 1);
}

#[test]
fn notifications_coalesce_at_the_earliest_source_and_ignore_already_indexed_references() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path());
    search_tests::source(&store, b"shared needle");
    for _ in 0..4 {
        store
            .connection
            .lock()
            .execute_batch(
                "UPDATE content_instances_v2 SET state='installing';
            UPDATE content_instances_v2 SET state='complete';",
            )
            .unwrap();
    }
    assert_eq!(take_dirty(&store).unwrap(), Some(1));
    assert_eq!(take_dirty(&store).unwrap(), None);
    let mut builder = TextIndexBuilder::new(&store).unwrap();
    assert_eq!(builder.cycle(&store).unwrap(), 1);
    store
        .connection
        .lock()
        .execute_batch(
            "UPDATE content_instances_v2 SET state='installing';
        UPDATE content_instances_v2 SET state='complete';",
        )
        .unwrap();
    assert_eq!(take_dirty(&store).unwrap(), None);
    assert_eq!(hits(&store), 1);
}
