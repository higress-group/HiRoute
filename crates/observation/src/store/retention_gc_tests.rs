use super::*;

#[test]
fn ordinary_gc_retains_a_blob_still_referenced_from_another_workspace() {
    use crate::store::tests::support::{Fixture, install_content, open_store};
    let root = tempfile::tempdir().unwrap();
    let store = open_store(root.path());
    let expired = Fixture::new("gc-expired-workspace");
    let mut live = Fixture::new("gc-live-workspace");
    live.workspace = hiroute_domain::WorkspaceId::parse("other-workspace").unwrap();
    let body = b"shared across independent workspace catalogs";
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
    let candidates = store.garbage_candidates(32).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(
        store.collect_unreferenced_candidates(&candidates).unwrap(),
        1
    );
    let digest = store.authority.content_blob_digest("text/plain", body);
    assert_eq!(fs::read(store.blob_path(digest.as_str())).unwrap(), body);
    assert!(store.connection.lock().query_row("SELECT EXISTS(SELECT 1 FROM content_blobs_v2 WHERE workspace_id=?1 AND blob_digest=?2)",rusqlite::params![live.workspace.as_str(),digest.as_str()],|r|r.get::<_,bool>(0)).unwrap());
}

#[test]
fn blob_made_live_after_gc_candidate_discovery_keeps_its_file_and_metadata() {
    let root = tempfile::tempdir().unwrap();
    let store =
        LocalObservationStore::open(root.path(), crate::DigestAuthority::new([2; 32])).unwrap();
    let digest = crate::query_v2::search_tests::source(&store, b"captured content");
    store
        .connection
        .lock()
        .execute("UPDATE content_instances_v2 SET state='installing'", [])
        .unwrap();
    let candidates = store.garbage_candidates(32).unwrap();
    assert_eq!(candidates.len(), 1);
    // Reproduce a capture publication between the read snapshot and removal.
    store
        .connection
        .lock()
        .execute("UPDATE content_instances_v2 SET state='complete'", [])
        .unwrap();
    assert_eq!(
        store.collect_unreferenced_candidates(&candidates).unwrap(),
        0
    );
    assert_eq!(
        std::fs::read(store.blob_path(&digest)).unwrap(),
        b"captured content"
    );
    assert!(
        store
            .connection
            .lock()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM content_blobs_v2 WHERE blob_digest=?1)",
                [digest],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
}

#[test]
fn capacity_protection_added_after_discovery_wins_the_deletion_recheck() {
    let root = tempfile::tempdir().unwrap();
    let store =
        LocalObservationStore::open(root.path(), crate::DigestAuthority::new([2; 32])).unwrap();
    let digest = crate::query_v2::search_tests::source(&store, b"protected content");
    store
        .connection
        .lock()
        .execute("UPDATE content_instances_v2 SET state='installing'", [])
        .unwrap();
    let candidates = store.garbage_candidates(32).unwrap();
    assert_eq!(candidates.len(), 1);
    store
        .connection
        .lock()
        .execute(
            "INSERT INTO observation_capacity_blobs_v5(workspace,digest) VALUES(?1,?2)",
            params![candidates[0].0, candidates[0].1],
        )
        .unwrap();
    assert_eq!(
        store.collect_unreferenced_candidates(&candidates).unwrap(),
        0
    );
    assert_eq!(
        std::fs::read(store.blob_path(&digest)).unwrap(),
        b"protected content"
    );
}
