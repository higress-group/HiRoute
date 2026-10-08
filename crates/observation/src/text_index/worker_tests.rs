use super::*;
use rusqlite::StatementStatus;

#[test]
fn index_selection_bounds_database_work_even_after_a_large_indexed_history() {
    let root = tempfile::tempdir().unwrap();
    let store =
        LocalObservationStore::open(root.path(), crate::DigestAuthority::new([2; 32])).unwrap();
    let db = store.connection.lock();
    db.execute_batch("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<8192)
        INSERT INTO content_blobs_v2 SELECT 'workspace','blob-'||i,'unused',0,'text/plain','complete',1 FROM n;
        INSERT INTO observation_text_index_v2 SELECT workspace_id,blob_digest,'ready',rowid FROM content_blobs_v2;").unwrap();
    let mut statement = db.prepare(NEXT_SOURCE_SQL).unwrap();
    assert!(!statement.exists(params![0, SCAN_ROWS]).unwrap());
    let bounded = statement.get_status(StatementStatus::VmStep);
    assert!(
        bounded < (SCAN_ROWS * 40) as i32,
        "bounded selection used {bounded} VM steps"
    );
    statement.reset_status(StatementStatus::VmStep);
    // The former scan visited all indexed history for every next blob. Keep a
    // negative control that exposes that cost without wall-clock assertions.
    assert!(!statement.exists(params![0, i64::MAX]).unwrap());
    let whole_history = statement.get_status(StatementStatus::VmStep);
    assert!(whole_history > bounded * 20);
    let publication_steps = |db: &rusqlite::Connection| {
        let mut statement = db
            .prepare("SELECT COALESCE(MAX(published),0)+1 FROM observation_text_index_v2")
            .unwrap();
        assert_eq!(
            statement.query_row([], |row| row.get::<_, i64>(0)).unwrap(),
            8193
        );
        statement.get_status(StatementStatus::VmStep)
    };
    assert!(publication_steps(&db) < 30);
    db.execute_batch("DROP INDEX observation_text_index_published")
        .unwrap();
    assert!(publication_steps(&db) > 8192);
    drop(statement);
    drop(db);
    drop(store);
    for _ in 0..2 {
        let store =
            LocalObservationStore::open(root.path(), crate::DigestAuthority::new([2; 32])).unwrap();
        assert!(publication_steps(&store.connection.lock()) < 30);
    }
}
