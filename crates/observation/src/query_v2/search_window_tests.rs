use super::search::{SCAN_WINDOW_SQL, candidate_sql};
use super::search_tests::{index, query, reader, source};
use super::*;
use crate::{DigestAuthority, LocalObservationStore};
use rusqlite::{StatementStatus, params};

fn copies(db: &rusqlite::Connection, count: u32) {
    db.execute("WITH RECURSIVE ids(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM ids WHERE n<?1)
        INSERT INTO content_instances_v2 SELECT workspace_id,'copy-'||n,message_instance_id,request_id,direction,fork_id,part_ordinal,content_kind,canonical_media_type,content_blob_digest,expected_byte_count,has_content_ref,transport_frame_id,downstream_delivery,accumulated_byte_count,state,created_at_unix_nanos FROM content_instances_v2,ids WHERE content_id='content'", [count]).unwrap();
}

#[test]
fn search_directory_work_is_bounded_before_filtering_and_index_readiness() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    source(&store, b"needle");
    index(&store);
    let db = store.connection.lock();
    copies(&db, 12_000);
    let mut window = db.prepare(SCAN_WINDOW_SQL).unwrap();
    let end: i64 = window.query_row(params![0, 12_001], |r| r.get(0)).unwrap();
    assert_eq!(end, 256);
    assert!(window.get_status(StatementStatus::VmStep) < 4_000);
    let mut candidates = db.prepare(&candidate_sql()).unwrap();
    let count = candidates
        .query_map(
            params![
                WorkspaceId::DEFAULT,
                0,
                1000,
                None::<String>,
                None::<String>,
                12001,
                i64::MAX,
                0,
                0,
                None::<String>,
                None::<String>,
                None::<String>,
                None::<String>,
                false,
                1,
                end
            ],
            |_| Ok(()),
        )
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .len();
    assert_eq!(count, 201);
    let bounded = candidates.get_status(StatementStatus::VmStep);
    assert!(
        bounded < 35_000,
        "bounded candidate query used {bounded} VM steps"
    );
    // Negative control: the previous readiness query scanned every occurrence
    // even when all content was ready, before any result could be returned.
    let mut previous = db.prepare("SELECT EXISTS(SELECT 1 FROM content_instances_v2 c
        LEFT JOIN observation_text_index_v2 i ON i.workspace=c.workspace_id AND i.digest=c.content_blob_digest
        WHERE c.workspace_id=?1 AND c.state='complete' AND (i.state IS NULL OR i.state!='ready' OR i.published>?2))").unwrap();
    assert!(
        !previous
            .query_row(params![WorkspaceId::DEFAULT, i64::MAX], |r| r
                .get::<_, bool>(0))
            .unwrap()
    );
    let whole_history = previous.get_status(StatementStatus::VmStep);
    assert!(
        whole_history > 35_000,
        "old readiness scan unexpectedly met the page budget: {whole_history}"
    );
}

#[test]
fn search_pages_preserve_index_gaps_and_freeze_late_content_and_publication() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    source(&store, b"needle");
    index(&store);
    {
        let db = store.connection.lock();
        copies(&db, 600);
        db.execute("UPDATE content_instances_v2 SET content_blob_digest='pending' WHERE content_id='copy-250'", []).unwrap();
    }
    let mut q = query("needle", 200);
    let first = store.search_observed_text(&reader(true), &q, 500).unwrap();
    assert_eq!(first.hits.len(), 200);
    assert!(!first.index_partial);
    q.cursor = first.next_cursor;
    {
        let db = store.connection.lock();
        db.execute(
            "INSERT INTO observation_text_index_v2 VALUES(?1,'pending','ready',2)",
            [WorkspaceId::DEFAULT],
        )
        .unwrap();
        db.execute("INSERT INTO observation_text_blocks_v2 SELECT workspace,'pending',ordinal,original_start,primary_start,folded,offsets,original FROM observation_text_blocks_v2", []).unwrap();
        db.execute("INSERT INTO logical_requests(workspace_id,request_id,session_id,turn_id,started_at_ms) VALUES(?1,'late','session','internal',100)", [WorkspaceId::DEFAULT]).unwrap();
        db.execute("INSERT INTO content_instances_v2 SELECT workspace_id,'late',message_instance_id,'late',direction,fork_id,part_ordinal,content_kind,canonical_media_type,content_blob_digest,expected_byte_count,has_content_ref,transport_frame_id,downstream_delivery,accumulated_byte_count,state,created_at_unix_nanos FROM content_instances_v2 WHERE content_id='content'", []).unwrap();
    }
    let mut ids = first
        .hits
        .into_iter()
        .map(|h| h.content_id)
        .collect::<Vec<_>>();
    let mut saw_partial = false;
    for _ in 0..10 {
        let page = store.search_observed_text(&reader(true), &q, 500).unwrap();
        assert!(
            !saw_partial || page.index_partial,
            "later pages erased a known gap"
        );
        saw_partial |= page.index_partial;
        ids.extend(page.hits.into_iter().map(|h| h.content_id));
        q.cursor = page.next_cursor;
        if q.cursor.is_none() {
            break;
        }
    }
    assert!(q.cursor.is_none());
    assert!(saw_partial);
    assert_eq!(ids.len(), 600);
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 600);
    assert!(!ids.iter().any(|id| id == "late" || id == "copy-250"));
}

#[test]
fn empty_search_windows_advance_without_exposing_hidden_index_gaps() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    source(&store, b"needle");
    index(&store);
    {
        let db = store.connection.lock();
        copies(&db, 600);
        db.execute("INSERT INTO logical_requests(workspace_id,request_id,session_id,turn_id,started_at_ms) VALUES(?1,'visible','session','internal',100)", [WorkspaceId::DEFAULT]).unwrap();
        db.execute(
            "UPDATE content_instances_v2 SET request_id='visible' WHERE content_id='copy-600'",
            [],
        )
        .unwrap();
        db.execute("UPDATE content_instances_v2 SET content_blob_digest='hidden-pending' WHERE request_id='request'", []).unwrap();
    }
    store
        .link_observed_request(&super::tests::link("visible", "allowed"))
        .unwrap();
    let scope = ObservationReaderContext::run_scoped(
        WorkspaceId::default(),
        "worker".into(),
        1,
        10000,
        ["allowed".into()].into_iter().collect(),
        true,
        true,
    )
    .unwrap();
    let mut q = query("needle", 200);
    for _ in 0..2 {
        let page = store.search_observed_text(&scope, &q, 500).unwrap();
        assert!(page.hits.is_empty());
        assert!(!page.index_partial);
        assert!(page.budget_exhausted);
        assert!(page.next_cursor.is_some());
        assert_ne!(q.cursor, page.next_cursor);
        q.cursor = page.next_cursor;
    }
    let page = store.search_observed_text(&scope, &q, 500).unwrap();
    assert_eq!(page.hits.len(), 1);
    assert_eq!(page.hits[0].request_id, "visible");
    assert!(!page.index_partial);
    assert!(!page.budget_exhausted);
    assert!(page.next_cursor.is_none());
}
