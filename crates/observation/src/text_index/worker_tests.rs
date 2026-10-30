use super::*;
use rusqlite::StatementStatus;

thread_local! {
    static STATEMENTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static SOURCE_WINDOWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn count_statement(_: &str) {
    STATEMENTS.with(|count| count.set(count.get() + 1));
}

fn count_source_windows(sql: &str) {
    if sql.starts_with("SELECT MAX(rowid) FROM (SELECT rowid FROM content_blobs_v2") {
        SOURCE_WINDOWS.with(|count| count.set(count.get() + 1));
    }
}

#[test]
fn idle_indexer_does_not_rescan_ready_history_after_settlement_or_analysis_reads() {
    let root = tempfile::tempdir().unwrap();
    let store =
        LocalObservationStore::open(root.path(), crate::DigestAuthority::new([2; 32])).unwrap();
    store.connection.lock().execute_batch(
        "WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<8192)
         INSERT INTO content_blobs_v2 SELECT 'workspace','blob-'||i,'unused',0,'text/plain','complete',1 FROM n;
         INSERT INTO observation_text_index_v2 SELECT workspace_id,blob_digest,'ready',rowid FROM content_blobs_v2;
         INSERT INTO observation_text_lookup_sources_v1 SELECT workspace,digest FROM observation_text_index_v2;",
    ).unwrap();
    store.settle_pending_valuations(16).unwrap();
    store.reconcile_analysis_batch(500).unwrap();
    let mut builder = TextIndexBuilder::new(&store).unwrap();
    store.connection.lock().trace(Some(count_statement));
    for _ in 0..32 {
        builder.cycle(&store).unwrap();
        if builder.idle {
            break;
        }
    }
    assert!(builder.idle);
    for _ in 0..3 {
        // These are the ordinary idle maintenance and query paths that share
        // the production store; neither changes content needing an index.
        let before = store.connection.lock().total_changes();
        assert_eq!(store.settle_pending_valuations(16).unwrap(), 0);
        assert_eq!(store.reconcile_analysis_batch(500).unwrap(), 0);
        assert_eq!(store.connection.lock().total_changes(), before);
        let reader = crate::query_v2::search_tests::reader(true);
        let q = hiroute_domain::ObservationAnalysisQueryV1::new(
            hiroute_domain::AnalysisViewV1::Conversations,
        );
        let range = hiroute_domain::AnalysisTimeRangeV1 {
            requested_from_ms: 0,
            requested_to_ms: 500,
            effective_from_ms: 0,
            effective_to_ms: 500,
            from_utc_offset_seconds: 0,
            to_utc_offset_seconds: 0,
            clipped_for_retention: false,
        };
        store
            .observed_analysis(&reader, &q, Some(&range), None, 500)
            .unwrap();
        STATEMENTS.with(|count| count.set(0));
        assert_eq!(builder.cycle(&store).unwrap(), 0);
        assert!(STATEMENTS.with(|count| count.get()) < 8);
    }
    // A generic revision (including accepted facts) cannot dirty text sources.
    crate::store::increment_store_revision(&store.connection.lock()).unwrap();
    STATEMENTS.with(|count| count.set(0));
    assert_eq!(builder.cycle(&store).unwrap(), 0);
    assert!(STATEMENTS.with(|count| count.get()) < 8);
}

#[test]
fn indexer_rechecks_a_late_live_source_changed_by_a_peer_writer() {
    let root = tempfile::tempdir().unwrap();
    let store =
        LocalObservationStore::open(root.path(), crate::DigestAuthority::new([2; 32])).unwrap();
    crate::query_v2::search_tests::source(&store, b"late needle");
    store
        .connection
        .lock()
        .execute("UPDATE content_instances_v2 SET state='installing'", [])
        .unwrap();
    let mut builder = TextIndexBuilder::new(&store).unwrap();
    assert_eq!(builder.cycle(&store).unwrap(), 0);
    assert!(builder.idle);
    let peer = rusqlite::Connection::open(&store.activity_path).unwrap();
    peer.execute("UPDATE content_instances_v2 SET state='complete'", [])
        .unwrap();
    assert_eq!(builder.cycle(&store).unwrap(), 1);
    let hits = store
        .search_observed_text(
            &crate::query_v2::search_tests::reader(true),
            &crate::query_v2::search_tests::query("needle", 10),
            500,
        )
        .unwrap();
    assert_eq!(hits.hits.len(), 1);
}

#[test]
fn indexer_does_not_declare_idle_when_a_source_changes_behind_the_cursor() {
    let root = tempfile::tempdir().unwrap();
    let store =
        LocalObservationStore::open(root.path(), crate::DigestAuthority::new([2; 32])).unwrap();
    crate::query_v2::search_tests::source(&store, b"late needle");
    store
        .connection
        .lock()
        .execute("UPDATE content_instances_v2 SET state='installing'", [])
        .unwrap();
    let mut builder = TextIndexBuilder::new(&store).unwrap();
    // Completion races a pass that already visited the old source. Its
    // notification survives exhaustion and rewinds the next bounded cycle.
    super::super::sources::take_dirty(&store).unwrap();
    builder.scan_after = 1;
    assert!(next(&store, &mut builder.scan_after).unwrap().1);
    store
        .connection
        .lock()
        .execute("UPDATE content_instances_v2 SET state='complete'", [])
        .unwrap();
    builder.idle = true;
    assert_eq!(builder.cycle(&store).unwrap(), 1);
}

#[test]
fn a_new_source_seeks_past_indexed_history_without_scanning_it_again() {
    let root = tempfile::tempdir().unwrap();
    let store =
        LocalObservationStore::open(root.path(), crate::DigestAuthority::new([2; 32])).unwrap();
    store.connection.lock().execute_batch(
        "WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<16384)
         INSERT INTO content_blobs_v2 SELECT 'workspace','blob-'||i,'unused',0,'text/plain','complete',1 FROM n;
         INSERT INTO observation_text_index_v2 SELECT workspace_id,blob_digest,'ready',rowid FROM content_blobs_v2;
         INSERT INTO observation_text_lookup_sources_v1 SELECT workspace,digest FROM observation_text_index_v2;",
    ).unwrap();
    let mut builder = TextIndexBuilder::new(&store).unwrap();
    for _ in 0..64 {
        builder.cycle(&store).unwrap();
        if builder.idle {
            break;
        }
    }
    assert!(builder.idle);
    crate::query_v2::search_tests::source(&store, b"fresh needle");
    store.connection.lock().trace(Some(count_source_windows));
    SOURCE_WINDOWS.with(|count| count.set(0));
    assert_eq!(builder.cycle(&store).unwrap(), 1);
    assert!(SOURCE_WINDOWS.with(|count| count.get()) <= 2);
    let hits = store
        .search_observed_text(
            &crate::query_v2::search_tests::reader(true),
            &crate::query_v2::search_tests::query("needle", 10),
            500,
        )
        .unwrap();
    assert_eq!(hits.hits.len(), 1);
    // Negative control: the former broad-revision wakeup restarted at zero.
    let mut after = 0;
    SOURCE_WINDOWS.with(|count| count.set(0));
    while !next(&store, &mut after).unwrap().1 {}
    assert!(SOURCE_WINDOWS.with(|count| count.get()) > 64);
}

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
