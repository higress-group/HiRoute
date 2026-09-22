use super::*;
use crate::{DigestAuthority, LocalObservationStore, text_index::TextIndexBuilder};
use hiroute_domain::*;
use rusqlite::params;

fn source(store: &LocalObservationStore, text: &[u8]) -> String {
    let digest = store
        .authority
        .content_blob_digest("text/plain", text)
        .to_string();
    let path = store.activity_path.with_file_name("indexed-source");
    std::fs::write(&path, text).unwrap();
    let connection = store.connection.lock();
    connection.execute("INSERT INTO sessions(workspace_id,session_id,started_at_ms,updated_at_ms) VALUES(?1,'session',100,100)",[WorkspaceId::DEFAULT]).unwrap();
    connection.execute("INSERT INTO logical_requests(workspace_id,request_id,session_id,turn_id,started_at_ms) VALUES(?1,'request','session','internal',100)",[WorkspaceId::DEFAULT]).unwrap();
    connection.execute("INSERT INTO content_blobs_v2(workspace_id,blob_digest,object_path,byte_count,media_type,state,created_at_unix_nanos) VALUES(?1,?2,?3,?4,'text/plain','complete',100000000)",params![WorkspaceId::DEFAULT,digest,path.to_string_lossy(),text.len()]).unwrap();
    connection.execute("INSERT INTO content_instances_v2(workspace_id,content_id,message_instance_id,request_id,direction,fork_id,part_ordinal,content_kind,canonical_media_type,content_blob_digest,has_content_ref,accumulated_byte_count,state,created_at_unix_nanos)
        VALUES(?1,'content','occurrence','request','request','fork',0,'text','text/plain',?2,0,?3,'complete',100000000)",params![WorkspaceId::DEFAULT,digest,text.len()]).unwrap();
    digest
}
fn reader(content: bool) -> ObservationReaderContext {
    ObservationReaderContext::local_user(
        WorkspaceId::default(),
        "user".into(),
        1,
        10000,
        content,
        content,
    )
    .unwrap()
}
fn query(keyword: &str, limit: u16) -> ObservationSearchQueryV2 {
    ObservationSearchQueryV2 {
        agent_id: None,
        plan_id: None,
        native_model: None,
        outcome: None,
        only_model_switch: false,
        from_ms: 0,
        to_ms: 1000,
        session_id: None,
        keyword: keyword.into(),
        limit,
        cursor: None,
    }
}
fn index(store: &LocalObservationStore) {
    let mut builder = TextIndexBuilder::new(store).unwrap();
    for _ in 0..20 {
        builder.cycle(store).unwrap();
        let pending: bool = store
            .connection
            .lock()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM observation_text_index_v2 WHERE state='building')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        if !pending {
            break;
        }
    }
}
#[test]
fn search_indexes_cross_block_literals_with_original_offsets_and_pages_forward() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    let prefix = "a".repeat(64 * 1024 - 4);
    source(
        &store,
        format!("{prefix}NeEdLe 甲İ乙 needle needle").as_bytes(),
    );
    let pending = store
        .search_observed_text(&reader(true), &query("needle", 1), 500)
        .unwrap();
    assert!(pending.index_partial);
    assert!(pending.hits.is_empty());
    index(&store);
    let mut request = query("needle", 1);
    let mut offsets = Vec::new();
    for _ in 0..5 {
        let page = store
            .search_observed_text(&reader(true), &request, 500)
            .unwrap();
        assert!(!page.index_partial);
        for hit in &page.hits {
            offsets.push(hit.original_text_offset);
            assert_eq!(hit.native_turn_id, None);
        }
        request.cursor = page.next_cursor;
        if request.cursor.is_none() {
            break;
        }
    }
    assert_eq!(offsets.len(), 3);
    assert_eq!(offsets[0], (64 * 1024 - 4) as u64);
    assert!(offsets.windows(2).all(|pair| pair[0] < pair[1]));
    let unicode = store
        .search_observed_text(&reader(true), &query("乙", 10), 500)
        .unwrap();
    assert_eq!(
        unicode.hits[0].original_text_offset,
        (prefix.len() + 6 + 1 + 5) as u64
    );
    assert_eq!(
        store.search_observed_text(&reader(false), &query("needle", 1), 500),
        Err(ObservationV2Error::Unauthorized)
    );
}
#[test]
fn deletion_erases_derived_text_and_invalidates_pending_search_pages() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    source(&store, b"needle needle");
    index(&store);
    let mut request = query("needle", 1);
    request.cursor = store
        .search_observed_text(&reader(true), &request, 500)
        .unwrap()
        .next_cursor;
    let principal = ObservationPrincipalV1::local_user(WorkspaceId::default());
    let spec = SessionDeletionSpecV1 {
        workspace_id: WorkspaceId::default(),
        session_id: SessionId::parse("session").unwrap(),
        data_class: DeletionDataClass::ContentOnly,
        delete_rollups: false,
    };
    let preview = store.preview_session_deletion(&principal, &spec).unwrap();
    store
        .apply_session_deletion(
            &principal,
            &spec,
            preview.store_revision,
            &preview.change_digest,
            500,
        )
        .unwrap();
    assert_eq!(
        store.search_observed_text(&reader(true), &request, 500),
        Err(ObservationV2Error::Stale)
    );
    let count: u64 = store
        .connection
        .lock()
        .query_row(
            "SELECT count(*) FROM observation_text_blocks_v2",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    assert!(
        store
            .search_observed_text(&reader(true), &query("needle", 10), 500)
            .unwrap()
            .hits
            .is_empty()
    );
}
#[test]
fn digest_corruption_never_publishes_rebuilt_text() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    source(&store, b"original");
    std::fs::write(
        store.activity_path.with_file_name("indexed-source"),
        b"needle!!",
    )
    .unwrap();
    index(&store);
    let page = store
        .search_observed_text(&reader(true), &query("needle", 10), 500)
        .unwrap();
    assert!(page.index_partial);
    assert!(page.hits.is_empty());
}

#[test]
fn indexed_body_pages_are_bounded_and_never_duplicate_overlap_bytes() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    let original = "中".repeat(90_000);
    source(&store, original.as_bytes());
    index(&store);
    let mut query = ObservationContentQueryV2 {
        anchor_offset: None,
        request_id: LogicalRequestId::parse("request").unwrap(),
        content_id: "content".into(),
        cursor: None,
    };
    let mut bytes = Vec::new();
    for _ in 0..10 {
        let page = store
            .observed_content_page(&reader(true), &query, 500)
            .unwrap();
        assert_eq!(page.state, "available");
        assert!(serde_json::to_vec(&page).unwrap().len() < 1024 * 1024);
        for chunk in page.chunks {
            assert_eq!(chunk.original_byte_offset, bytes.len() as u64);
            bytes.extend_from_slice(chunk.text.as_bytes());
        }
        query.cursor = page.next_cursor;
        if query.cursor.is_none() {
            break;
        }
    }
    assert_eq!(bytes, original.as_bytes());
    assert_eq!(
        store.observed_content_page(&reader(false), &query, 500),
        Err(ObservationV2Error::Unauthorized)
    );
}

#[test]
fn catalog_preserves_occurrences_and_ancestry_stops_at_cycles() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    source(&store, b"same user question");
    {
        let db = store.connection.lock();
        for (occurrence, ordinal) in [("occurrence", 0), ("again", 1)] {
            db.execute("INSERT INTO content_message_instances_v2(workspace_id,message_instance_id,conversation_id,request_id,direction,fork_id,message_ordinal,message_role,occurred_at_unix_nanos) VALUES(?1,?2,'session','request','request','fork',?3,'user',100000000)",params![WorkspaceId::DEFAULT,occurrence,ordinal]).unwrap();
        }
        db.execute("INSERT INTO content_instances_v2 SELECT workspace_id,'content-again','again',request_id,direction,fork_id,part_ordinal,content_kind,canonical_media_type,content_blob_digest,expected_byte_count,has_content_ref,transport_frame_id,downstream_delivery,accumulated_byte_count,state,created_at_unix_nanos FROM content_instances_v2 WHERE content_id='content'",[]).unwrap();
        db.execute("INSERT INTO transcript_roots_v2 VALUES(?1,'root','session','request','request','fork','root','finish',100000000)",[WorkspaceId::DEFAULT]).unwrap();
    }
    let mut q = ObservationCatalogQueryV2 {
        request_id: LogicalRequestId::parse("request").unwrap(),
        limit: 1,
        cursor: None,
    };
    assert_eq!(
        store.observed_catalog(&reader(false), &q, 500),
        Err(ObservationV2Error::Unauthorized)
    );
    let first = store.observed_catalog(&reader(true), &q, 500).unwrap();
    assert_eq!(first.contents[0].message_occurrence_id, "occurrence");
    assert_eq!(first.contents[0].kind, "text");
    assert_eq!(first.transcript_roots, ["root"]);
    q.cursor = first.next_cursor;
    let second = store.observed_catalog(&reader(true), &q, 500).unwrap();
    assert_eq!(second.contents[0].message_occurrence_id, "again");
    assert_eq!(second.contents[0].kind, "text");
    assert!(second.next_cursor.is_none());
    let ancestry = store
        .observed_ancestry(
            &reader(true),
            &ObservationAncestryQueryV2 {
                request_id: q.request_id,
                transcript_root: "root".into(),
            },
            500,
        )
        .unwrap();
    assert_eq!(ancestry.roots.len(), 1);
    assert_eq!(ancestry.gap.as_deref(), Some("cycle"));
}

#[test]
fn catalog_keeps_canonical_kind_but_search_excludes_internal_state() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    source(&store, b"private-signature-needle");
    store.connection.lock().execute("INSERT INTO content_message_instances_v2(workspace_id,message_instance_id,conversation_id,request_id,direction,fork_id,message_ordinal,message_role,occurred_at_unix_nanos) VALUES(?1,'occurrence','session','request','request','fork',0,'assistant',100000000)",[WorkspaceId::DEFAULT]).unwrap();
    index(&store);
    assert_eq!(
        store
            .search_observed_text(&reader(true), &query("private-signature-needle", 10), 500)
            .unwrap()
            .hits
            .len(),
        1
    );
    for kind in ["provider_state", "reasoning_delta", "reasoning_finished"] {
        store
            .connection
            .lock()
            .execute(
                "UPDATE content_instances_v2 SET content_kind=?1 WHERE content_id='content'",
                [kind],
            )
            .unwrap();
        let catalog = store
            .observed_catalog(
                &reader(true),
                &ObservationCatalogQueryV2 {
                    request_id: LogicalRequestId::parse("request").unwrap(),
                    limit: 10,
                    cursor: None,
                },
                500,
            )
            .unwrap();
        assert_eq!(catalog.contents[0].kind, kind);
        let search = store
            .search_observed_text(&reader(true), &query("private-signature-needle", 10), 500)
            .unwrap();
        assert!(
            search.hits.is_empty(),
            "{kind} must not be a searchable conversation hit"
        );
        assert!(!search.index_partial);
    }
}

#[test]
fn index_owner_is_exclusive_and_released_after_drop() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    let first = TextIndexBuilder::new(&store).unwrap();
    assert!(TextIndexBuilder::new(&store).is_err());
    drop(first);
    assert!(TextIndexBuilder::new(&store).is_ok());
}

#[test]
fn far_search_anchor_starts_at_its_block_and_binds_followup_cursor() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    let prefix = "a".repeat(3 * 64 * 1024 + 17);
    source(
        &store,
        format!("{prefix}needle{}", "z".repeat(4 * 64 * 1024)).as_bytes(),
    );
    index(&store);
    let search = store
        .search_observed_text(&reader(true), &query("needle", 1), 500)
        .unwrap();
    assert_eq!(search.hits.len(), 1);
    let hit = &search.hits[0];
    let mut anchored = ObservationContentQueryV2 {
        request_id: LogicalRequestId::parse("request").unwrap(),
        content_id: hit.content_id.clone(),
        anchor_offset: Some(hit.original_text_offset),
        cursor: None,
    };
    let page = store
        .observed_content_page(&reader(true), &anchored, 500)
        .unwrap();
    assert!(page.chunks[0].original_byte_offset > 0);
    let relative = (hit.original_text_offset - page.chunks[0].original_byte_offset) as usize;
    assert!(page.chunks[0].text[relative..].starts_with("needle"));
    assert!(page.next_cursor.is_some());
    anchored.cursor = page.next_cursor;
    anchored.anchor_offset = Some(0);
    assert_eq!(
        store.observed_content_page(&reader(true), &anchored, 500),
        Err(ObservationV2Error::Stale)
    );
    let mut filtered = query("needle", 1);
    filtered.agent_id = Some("other-agent".into());
    assert!(
        store
            .search_observed_text(&reader(true), &filtered, 500)
            .unwrap()
            .hits
            .is_empty()
    );
}
