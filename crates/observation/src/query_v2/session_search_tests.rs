use super::{
    search_tests::{index, query, reader, source},
    *,
};
use crate::{DigestAuthority, LocalObservationStore};
use hiroute_domain::*;
use rusqlite::params;

#[test]
fn live_search_large_distinct_source_cache_keeps_indexed_anchors_and_complete_pagination() {
    search_scale(30_000, 30_000);
}

#[test]
#[ignore = "targeted 250k-occurrence/60k-source gate; execute explicitly outside ordinary parallel unit tests"]
fn live_search_250k_occurrences_and_60k_sources_under_real_deadline() {
    search_scale(60_000, 250_000);
}

#[test]
#[ignore = "targeted retained-native-history scale gate; execute outside ordinary parallel unit tests"]
fn live_search_450k_occurrences_and_100k_sources_under_real_deadline() {
    search_scale(100_000, 450_000);
}

fn search_scale(distinct: usize, occurrences: usize) {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    {
        let mut db = store.connection.lock();
        let tx = db.transaction().unwrap();
        for n in 0..111 {
            tx.execute("INSERT INTO sessions(workspace_id,session_id,started_at_ms,updated_at_ms,agent_id) VALUES(?1,?2,?3,?3,'scale-agent')",params![WorkspaceId::DEFAULT,format!("scale-{n}"),100+n]).unwrap();
            tx.execute("INSERT INTO logical_requests(workspace_id,request_id,session_id,turn_id,started_at_ms,outcome) VALUES(?1,?2,?3,'internal',?4,'accepted')",params![WorkspaceId::DEFAULT,format!("request-{n}"),format!("scale-{n}"),100+n]).unwrap();
        }
        // Component cache fixture: unlike the repeated single-blob corpus, each
        // row has its own real digest, folded block and production lookup grams.
        // Native acceptance separately proves capture, publication and body reads.
        for n in 0..distinct {
            let body = if n < 51 {
                format!("checksum {n}")
            } else {
                format!("ordinary background e ch 中文 {n}")
            };
            let digest = store
                .authority
                .content_blob_digest("text/plain", body.as_bytes())
                .to_string();
            let (folded, offsets) = crate::text_index::fold(&body);
            tx.execute("INSERT INTO content_instances_v2(workspace_id,content_id,message_instance_id,request_id,direction,fork_id,part_ordinal,content_kind,canonical_media_type,content_blob_digest,has_content_ref,accumulated_byte_count,state,created_at_unix_nanos) VALUES(?1,?2,?2,?3,'request','fork',?4,'text','text/plain',?5,0,?6,'complete',100000000)",params![WorkspaceId::DEFAULT,format!("content-{n}"),format!("request-{}",n%111),n,digest,body.len()]).unwrap();
            tx.execute(
                "INSERT INTO observation_text_index_v2 VALUES(?1,?2,'ready',1)",
                params![WorkspaceId::DEFAULT, digest],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO observation_text_blocks_v2 VALUES(?1,?2,0,0,0,?3,?4,?5)",
                params![WorkspaceId::DEFAULT, digest, folded, offsets, body],
            )
            .unwrap();
            crate::text_index::insert_lookup_fixture(&tx, tx.last_insert_rowid(), &folded).unwrap();
            tx.execute(
                "INSERT INTO observation_text_lookup_sources_v1 VALUES(?1,?2)",
                params![WorkspaceId::DEFAULT, digest],
            )
            .unwrap();
        }
        // The separately selected scale gate uses 60k verified sources and
        // 250k occurrences, including repeated input history. Native acceptance
        // separately reads the preserved installed corpus.
        let mut remaining = occurrences - distinct;
        let mut replica = 0;
        while remaining > 0 {
            let limit = remaining.min(distinct);
            tx.execute("INSERT INTO content_instances_v2(workspace_id,content_id,message_instance_id,request_id,direction,fork_id,part_ordinal,content_kind,canonical_media_type,content_blob_digest,has_content_ref,accumulated_byte_count,state,created_at_unix_nanos) SELECT workspace_id,?1||content_id,message_instance_id,request_id,direction,fork_id,part_ordinal+?2,content_kind,canonical_media_type,content_blob_digest,has_content_ref,accumulated_byte_count,state,created_at_unix_nanos FROM content_instances_v2 WHERE workspace_id=?3 AND content_id LIKE 'content-%' ORDER BY content_id LIMIT ?4",params![format!("repeat-{replica}-"),((replica+1)*distinct) as i64,WorkspaceId::DEFAULT,limit as i64]).unwrap();
            remaining -= limit;
            replica += 1;
        }
        tx.commit().unwrap();
        // Matching reads source identities and covering content metadata, not
        // all folded bodies or every occurrence of a nonmatching request.
        // Gap and final reads retain their supplied-identity lookup boundaries.
        use rusqlite::types::Value;
        let mut bindings = vec![
            Value::Text(WorkspaceId::DEFAULT.into()),
            Value::Integer(0),
            Value::Integer(1000),
        ];
        bindings.extend(std::iter::repeat_n(Value::Null, 6));
        let mut hit_bindings = bindings.clone();
        hit_bindings.extend([
            Value::Text("checksum".into()),
            Value::Text(crate::text_index::lookup::expression("checksum")),
            Value::Integer(i64::MAX),
            Value::Text(String::new()),
            Value::Integer(51),
        ]);
        let mut short_bindings = hit_bindings.clone();
        short_bindings[9] = Value::Text("e".into());
        short_bindings[10] = Value::Text(crate::text_index::lookup::expression("e"));
        let mut live_bindings = bindings.clone();
        live_bindings.push(Value::Text(
            serde_json::to_string(&(0..50).map(|n| format!("content-{n}")).collect::<Vec<_>>())
                .unwrap(),
        ));
        for (sql, bindings, exact_gram) in [
            (
                super::session_search::index_partial_sql(false),
                bindings,
                None,
            ),
            (
                super::session_search::candidate_sql(false, false),
                hit_bindings,
                Some(false),
            ),
            (
                super::session_search::candidate_sql(false, true),
                short_bindings,
                Some(true),
            ),
            (
                super::session_search::live_read_sql(false),
                live_bindings,
                None,
            ),
        ] {
            let returned_identities = bindings.len() == 10;
            let mut stmt = db.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let plan = stmt
                .query_map(rusqlite::params_from_iter(&bindings), |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(3)?,
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            if !returned_identities {
                assert!(
                    plan.iter().any(|(_, _, step)| step.starts_with(
                        "SEARCH i USING COVERING INDEX observation_text_index_ready "
                    ) && step.contains("(workspace=?")),
                    "source readiness must not load every index table row: {plan:?}"
                );
            }
            if let Some(exact_gram) = exact_gram {
                assert!(
                    plan.iter().any(|(_, _, step)| step.starts_with(
                        "SEARCH c USING COVERING INDEX content_instances_v2_search_blob "
                    ) && step
                        .contains("(workspace_id=? AND content_blob_digest=?)")),
                    "matching metadata must remain covered: {plan:?}"
                );
                assert_eq!(
                    plan.iter()
                        .filter(|(_, _, step)| step
                            .contains("observation_text_lookup_v1 VIRTUAL TABLE"))
                        .count(),
                    1,
                    "the shared keyword candidate set must be built once: {plan:?}"
                );
                assert!(
                    plan.iter().any(|(_, _, step)| step
                        .starts_with("SEARCH b USING INTEGER PRIMARY KEY (rowid=?)")),
                    "keyword candidates must drive direct block identities, not every workspace source: {plan:?}"
                );
                let _ = exact_gram;
            } else if !returned_identities {
                assert!(
                    plan.iter().any(|(_, _, step)| step
                        .starts_with("SEARCH content_instances_v2 USING COVERING INDEX content_instances_v2_live_blob ")
                        && step.contains("(workspace_id=?)")),
                    "deduplication must read the covering source index, not large metadata pages: {plan:?}"
                );
                assert!(
                    plan.iter().any(|(_, _, step)| step == "MERGE (EXCEPT)")
                        && plan
                            .iter()
                            .any(|(_, _, step)| step == "MATERIALIZE unready_sources"),
                    "ready history must be subtracted by ordered merge, without materializing complete source sets: {plan:?}"
                );
                assert!(
                    plan.iter().any(|(_, _, step)| step.starts_with(
                        "SEARCH c USING COVERING INDEX content_instances_v2_search_blob "
                    ) && step
                        .contains("(workspace_id=? AND content_blob_digest=?)")),
                    "only unready source identities may open candidate occurrences: {plan:?}"
                );
            }
            assert!(
                plan.iter().any(|(_, _, step)| step.starts_with("SEARCH r ")
                    && step.contains("(workspace_id=? AND request_id=?)")),
                "request identity lookup required: {plan:?}"
            );
            if returned_identities {
                assert!(
                    plan.iter().any(|(_, _, step)| step.starts_with("SEARCH c ")
                        && step.contains("(workspace_id=? AND content_id=?)")),
                    "returned content identity lookup required: {plan:?}"
                );
            }
        }
    }
    let mut q = query("checksum", 50);
    let first = store
        .search_observed_sessions(&reader(true), &q, 500)
        .unwrap();
    assert_eq!(first.hits.len(), 50);
    assert!(!first.index_partial);
    assert!(first.hits.iter().all(|h| h.original_text_offset == 0));
    q.cursor = first.next_cursor;
    assert!(q.cursor.is_some());
    let last = store
        .search_observed_sessions(&reader(true), &q, 500)
        .unwrap();
    assert_eq!(last.hits.len(), 1);
    assert_eq!(last.hits[0].session_id, "scale-0");
    assert!(last.next_cursor.is_none() && !last.index_partial);
    // A common one-character keyword exercises nearly every distinct block,
    // including the final current-visibility read under the same query budget.
    for word in ["e", "ch", "o", "中文"] {
        let common = store
            .search_observed_sessions(&reader(true), &query(word, 200), 500)
            .unwrap_or_else(|error| panic!("common keyword {word} failed: {error:?}"));
        assert_eq!(common.hits.len(), 111, "{word}");
        assert!(
            common.next_cursor.is_none() && !common.index_partial,
            "{word}"
        );
        let mut analysis = ObservationAnalysisQueryV1::new(AnalysisViewV1::Conversations);
        analysis.query = Some(word.into());
        analysis.limit = 200;
        let range = AnalysisTimeRangeV1 {
            requested_from_ms: 0,
            requested_to_ms: 1000,
            effective_from_ms: 0,
            effective_to_ms: 1000,
            from_utc_offset_seconds: 0,
            to_utc_offset_seconds: 0,
            clipped_for_retention: false,
        };
        let analyzed = store
            .observed_analysis(&reader(true), &analysis, Some(&range), None, 500)
            .unwrap_or_else(|error| panic!("analysis keyword {word} failed: {error:?}"));
        assert_eq!(analyzed.conversations.len(), 111, "{word}");
        assert!(analyzed.next_cursor.is_none(), "{word}");
    }
    let mut short = query("o", 200);
    short.limit = 50;
    let common = store
        .search_observed_sessions(&reader(true), &short, 500)
        .unwrap();
    assert_eq!(common.hits.len(), 50);
    assert!(common.next_cursor.is_some() && !common.index_partial);
    let missing = store
        .search_observed_sessions(&reader(true), &query("absent", 50), 500)
        .unwrap();
    assert!(missing.hits.is_empty() && !missing.index_partial);
    store.connection.lock().execute("DELETE FROM observation_text_lookup_sources_v1 WHERE digest=(SELECT content_blob_digest FROM content_instances_v2 WHERE content_id='content-29999')",[]).unwrap();
    let pending = store
        .search_observed_sessions(&reader(true), &query("absent", 50), 500)
        .unwrap();
    assert!(pending.hits.is_empty() && pending.index_partial);
    let mut hidden = query("absent", 50);
    hidden.agent_id = Some("other-agent".into());
    let hidden = store
        .search_observed_sessions(&reader(true), &hidden, 500)
        .unwrap();
    assert!(hidden.hits.is_empty() && !hidden.index_partial);
}

fn add(store: &LocalObservationStore, session: &str, request: &str, at: i64, text: &str) {
    let digest = store
        .authority
        .content_blob_digest("text/plain", text.as_bytes())
        .to_string();
    let path = store.blob_path(&digest);
    std::fs::write(&path, text).unwrap();
    let db = store.connection.lock();
    db.execute("INSERT OR IGNORE INTO sessions(workspace_id,session_id,started_at_ms,updated_at_ms,agent_id) VALUES(?1,?2,?3,?3,'test-agent')",params![WorkspaceId::DEFAULT,session,at]).unwrap();
    db.execute("INSERT INTO logical_requests(workspace_id,request_id,session_id,turn_id,started_at_ms,outcome) VALUES(?1,?2,?3,'internal',?4,'accepted')",params![WorkspaceId::DEFAULT,request,session,at]).unwrap();
    db.execute("INSERT OR IGNORE INTO content_blobs_v2(workspace_id,blob_digest,object_path,byte_count,media_type,state,created_at_unix_nanos) VALUES(?1,?2,?3,?4,'text/plain','complete',?5)",params![WorkspaceId::DEFAULT,digest,path.to_string_lossy(),text.len(),at*1_000_000]).unwrap();
    db.execute("INSERT INTO content_instances_v2(workspace_id,content_id,message_instance_id,request_id,direction,fork_id,part_ordinal,content_kind,canonical_media_type,content_blob_digest,has_content_ref,accumulated_byte_count,state,created_at_unix_nanos)
        VALUES(?1,?2,?3,?3,'request','fork',0,'text','text/plain',?4,0,?5,'complete',?6)",params![WorkspaceId::DEFAULT,format!("content-{request}"),request,digest,text.len(),at*1_000_000]).unwrap();
}

#[test]
fn session_search_reaches_late_matches_without_directory_pages_and_survives_new_worker_links() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    source(&store, b"background");
    store.connection.lock().execute_batch("WITH RECURSIVE ids(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM ids WHERE n<27800)
      INSERT INTO content_instances_v2 SELECT workspace_id,'copy-'||n,message_instance_id,request_id,direction,fork_id,part_ordinal,content_kind,canonical_media_type,content_blob_digest,expected_byte_count,has_content_ref,transport_frame_id,downstream_delivery,accumulated_byte_count,state,created_at_unix_nanos FROM content_instances_v2,ids WHERE content_id='content';").unwrap();
    add(
        &store,
        "match-new",
        "new",
        300,
        "checksum checksum checksum",
    );
    add(&store, "match-middle", "middle", 200, "checksum");
    add(&store, "match-old", "old", 150, "checksum");
    index(&store);
    let legacy = store
        .search_observed_text(&reader(true), &query("checksum", 1), 500)
        .unwrap();
    assert!(legacy.hits.is_empty() && legacy.budget_exhausted && legacy.next_cursor.is_some());
    let mut q = query("checksum", 1);
    let first = store
        .search_observed_sessions(&reader(true), &q, 500)
        .unwrap();
    assert_eq!(first.hits.len(), 1);
    assert_eq!(first.hits[0].session_id, "match-new");
    assert!(!first.index_partial);
    q.cursor = first.next_cursor;
    add(&store, "unrelated", "worker", 800, "another job");
    store
        .link_observed_request(&super::tests::link("worker", "another-run"))
        .unwrap();
    let renewed = ObservationReaderContext::local_user(
        WorkspaceId::default(),
        "user".into(),
        2,
        12000,
        true,
        true,
    )
    .unwrap();
    let second = store.search_observed_sessions(&renewed, &q, 500).unwrap();
    assert_eq!(
        second
            .hits
            .iter()
            .map(|h| h.session_id.as_str())
            .collect::<Vec<_>>(),
        ["match-middle"]
    );
    q.cursor = second.next_cursor;
    // A newly published old match is live, rather than excluded by a frozen
    // content/publication watermark. It is behind the consumed sort position.
    add(&store, "match-late-index", "late-index", 175, "checksum");
    index(&store);
    let third = store.search_observed_sessions(&renewed, &q, 500).unwrap();
    assert_eq!(third.hits[0].session_id, "match-late-index");
    q.cursor = third.next_cursor;
    let last = store.search_observed_sessions(&renewed, &q, 500).unwrap();
    assert_eq!(last.hits[0].session_id, "match-old");
    assert!(last.next_cursor.is_none());
    // The exact pre-change occurrence reader exhibits the original stale
    // behavior after the same real relation mutation.
    let old = ObservationSearchQueryV2 {
        cursor: legacy.next_cursor,
        ..query("checksum", 1)
    };
    assert_eq!(
        store.search_observed_text(&reader(true), &old, 500),
        Err(ObservationV2Error::Stale)
    );
}

#[test]
fn session_search_literal_grams_preserve_short_unicode_cross_block_and_original_body_anchors() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    let prefix = "a".repeat(64 * 1024 - 4);
    let long_chinese = "中文".repeat(128);
    let body = format!("{prefix}NeEdLe 甲İ乙 中文检索 abcbcd . OR \" ΣΟΣ\0尾字 {long_chinese}");
    source(&store, body.as_bytes());
    index(&store);
    for (keyword, literal) in [
        ("needle", "NeEdLe"),
        ("乙", "乙"),
        ("中文", "中文"),
        ("文检索", "文检索"),
        ("İ", "İ"),
        ("ΣΟΣ", "ΣΟΣ"),
        (".", "."),
        ("OR \"", "OR \""),
        ("尾字", "尾字"),
    ] {
        let page = store
            .search_observed_sessions(&reader(true), &query(keyword, 10), 500)
            .unwrap();
        assert_eq!(page.hits.len(), 1, "{keyword}");
        assert!(page.next_cursor.is_none());
        let hit = &page.hits[0];
        assert_eq!(
            hit.original_text_offset,
            body.find(literal).unwrap() as u64,
            "{keyword}"
        );
        let content = store
            .observed_content_page(
                &reader(true),
                &ObservationContentQueryV2 {
                    request_id: LogicalRequestId::parse(hit.request_id.clone()).unwrap(),
                    content_id: hit.content_id.clone(),
                    anchor_offset: Some(hit.original_text_offset),
                    cursor: None,
                },
                500,
            )
            .unwrap();
        let start = content.chunks[0].original_byte_offset;
        let visible: String = content.chunks.iter().map(|c| c.text.as_str()).collect();
        assert!(visible[(hit.original_text_offset - start) as usize..].starts_with(literal));
    }
    let page = store
        .search_observed_sessions(&reader(true), &query(&long_chinese, 10), 500)
        .unwrap();
    assert_eq!(
        page.hits[0].original_text_offset,
        body.find(&long_chinese).unwrap() as u64
    );
    assert_eq!(
        store.search_observed_sessions(&reader(true), &query(&"字".repeat(257), 10), 500),
        Err(ObservationV2Error::Invalid)
    );
    // Trigram intersection is only candidate evidence, never an exact hit.
    assert!(
        store
            .search_observed_sessions(&reader(true), &query("abcd", 10), 500)
            .unwrap()
            .hits
            .is_empty()
    );
}

#[test]
fn folded_scalar_count_keeps_long_trigram_intersections_out_of_session_ranking() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    // Two input scalars expand to four folded scalars. The newer request has
    // both trigrams without the full substring; it must not hide the older hit.
    add(&store, "folded", "false-grams", 200, "i̇i / ̇i̇");
    add(&store, "folded", "exact-folded", 100, "İİ");
    index(&store);
    let page = store
        .search_observed_sessions(&reader(true), &query("İİ", 10), 500)
        .unwrap();
    assert_eq!(page.hits.len(), 1);
    assert_eq!(page.hits[0].request_id, "exact-folded");
    assert_eq!(page.hits[0].original_text_offset, 0);
    assert!(page.next_cursor.is_none() && !page.index_partial);
}

#[test]
fn released_block_cache_backfills_without_changing_bodies_offsets_or_occurrence_search() {
    let root = tempfile::tempdir().unwrap();
    let authority = DigestAuthority::new([2; 32]);
    let store = LocalObservationStore::open(root.path(), authority.clone()).unwrap();
    let digest = source(&store, "xx甲İNeedle乙".as_bytes());
    {
        let db = store.connection.lock();
        db.execute_batch(
            "DROP TRIGGER observation_text_lookup_delete_block;
          DROP TRIGGER observation_text_lookup_delete_source;
          DROP INDEX content_instances_v2_search_blob;
          DROP TABLE observation_text_lookup_v1; DROP TABLE observation_text_lookup_sources_v1;
          DROP TABLE observation_text_blocks_v2; DROP TABLE observation_text_index_v2;",
        )
        .unwrap();
        db.execute_batch(include_str!(
            "../../tests/fixtures/released-v0.3.1-text-index.sql"
        ))
        .unwrap();
        db.execute(
            "INSERT INTO observation_text_index_v2 VALUES(?1,?2,'ready',1)",
            params![WorkspaceId::DEFAULT, digest],
        )
        .unwrap();
        // Frozen output of the released per-character lowercase producer for
        // xx甲İNeedle乙. In particular İ expands while its original bytes do not.
        db.execute("INSERT INTO observation_text_blocks_v2 VALUES(?1,?2,0,0,0,'xx甲i̇needle乙',X'00000000000000000100000001000000020000000200000005000000050000000600000005000000080000000700000009000000080000000a000000090000000b0000000a0000000c0000000b0000000d0000000c0000000e0000000d0000001100000010000000','xx甲İNeedle乙')",params![WorkspaceId::DEFAULT,digest]).unwrap();
    }
    drop(store);
    let store = LocalObservationStore::open(root.path(), authority.clone()).unwrap();
    let pending = store
        .search_observed_sessions(&reader(true), &query("needle", 10), 500)
        .unwrap();
    assert!(pending.index_partial && pending.hits.is_empty());
    assert_eq!(
        store
            .search_observed_text(&reader(true), &query("needle", 10), 500)
            .unwrap()
            .hits[0]
            .original_text_offset,
        7
    );
    // A crash may leave unpublished lookup rows. They grant no hit and restart
    // replaces them idempotently before publishing this complete source.
    store.connection.lock().execute_batch("INSERT INTO observation_text_lookup_v1(rowid,grams) SELECT rowid,'g62' FROM observation_text_blocks_v2;").unwrap();
    index(&store);
    let page = store
        .search_observed_sessions(&reader(true), &query("needle", 10), 500)
        .unwrap();
    assert!(!page.index_partial);
    assert_eq!(page.hits[0].original_text_offset, 7);
    drop(store);
    let store = LocalObservationStore::open(root.path(), authority).unwrap();
    index(&store);
    for name in [
        "content_instances_v2_search_blob",
        "observation_text_index_ready",
    ] {
        assert_eq!(
            store
                .connection
                .lock()
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?1",
                    [name],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
            1
        );
    }
    assert_eq!(
        store
            .search_observed_sessions(&reader(true), &query("needle", 10), 500)
            .unwrap(),
        page
    );
    assert_eq!(
        store
            .connection
            .lock()
            .query_row("SELECT COUNT(*) FROM observation_text_lookup_v1", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        1
    );
}

thread_local! {
    static INDEX_CHECKPOINTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn yield_first_index_cycle(sql: &str) {
    if sql == "SELECT value FROM observation_meta WHERE key='store_revision'" {
        let first = INDEX_CHECKPOINTS.with(|count| {
            let previous = count.get();
            count.set(previous + 1);
            previous == 0
        });
        if first {
            // Inject one preemption after cycle timing starts, before a source
            // enters building. This exhausts the existing 500 ms cycle budget;
            // it is the fault trigger, not a readiness wait or a new deadline.
            std::thread::sleep(std::time::Duration::from_millis(510));
        }
    }
}

#[test]
fn index_fixture_drains_sources_after_a_cycle_yields_before_building() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    for n in 0..3 {
        add(
            &store,
            &format!("session-{n}"),
            &format!("request-{n}"),
            100 + n,
            &format!("needle {n}"),
        );
    }
    INDEX_CHECKPOINTS.with(|count| count.set(0));
    store.connection.lock().trace(Some(yield_first_index_cycle));
    index(&store);
    store.connection.lock().trace(None);
    INDEX_CHECKPOINTS.with(|count| {
        assert!(
            count.get() >= 2,
            "fixture stopped after the injected empty cycle"
        );
    });
    let mut q = query("needle", 2);
    let first = store
        .search_observed_sessions(&reader(true), &q, 500)
        .unwrap();
    assert!(
        !first.index_partial,
        "fixture left sources unpublished after returning {} sessions",
        first.hits.len()
    );
    assert_eq!(first.hits.len(), 2);
    assert!(first.next_cursor.is_some());
    q.cursor = first.next_cursor;
    let last = store
        .search_observed_sessions(&reader(true), &q, 500)
        .unwrap();
    assert!(!last.index_partial);
    assert_eq!(last.hits.len(), 1);
    assert_eq!(last.hits[0].session_id, "session-0");
    assert!(last.next_cursor.is_none());
}

#[test]
fn live_search_deletion_removes_lookup_and_body_without_invalidating_other_sessions() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    add(&store, "new", "new", 300, "needle new");
    add(&store, "middle", "middle", 200, "needle deleted");
    add(&store, "old", "old", 100, "needle old");
    index(&store);
    let mut q = query("needle", 1);
    let first = store
        .search_observed_sessions(&reader(true), &q, 500)
        .unwrap();
    assert!(!first.index_partial);
    assert_eq!(first.hits.len(), 1);
    assert_eq!(first.hits[0].session_id, "new");
    assert!(first.next_cursor.is_some());
    q.cursor = first.next_cursor;
    let principal = ObservationPrincipalV1::local_user(WorkspaceId::default());
    let spec = SessionDeletionSpecV1 {
        workspace_id: WorkspaceId::default(),
        session_id: SessionId::parse("middle").unwrap(),
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
    let next = store
        .search_observed_sessions(&reader(true), &q, 500)
        .unwrap();
    assert!(!next.index_partial);
    assert_eq!(next.hits.len(), 1);
    assert_eq!(next.hits[0].session_id, "old");
    assert!(next.next_cursor.is_none());
    assert_eq!(store.connection.lock().query_row("SELECT count(*) FROM observation_text_lookup_v1 WHERE observation_text_lookup_v1 MATCH ?1",[crate::text_index::lookup::expression("deleted")],|r|r.get::<_,u64>(0)).unwrap(),0);
    let body = store.observed_content_page(
        &reader(true),
        &ObservationContentQueryV2 {
            request_id: LogicalRequestId::parse("middle").unwrap(),
            content_id: "content-middle".into(),
            anchor_offset: Some(0),
            cursor: None,
        },
        500,
    );
    assert!(!body.is_ok_and(|p| !p.chunks.is_empty()));
}

#[test]
fn live_search_filters_current_run_authority_and_never_reports_hidden_gaps() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    add(&store, "new", "new", 300, "needle");
    add(&store, "old", "old", 100, "needle");
    index(&store);
    add(&store, "hidden", "hidden", 250, "not indexed");
    for request in ["new", "old"] {
        store
            .link_observed_request(&super::tests::link(request, "allowed"))
            .unwrap();
    }
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
    let mut q = query("needle", 1);
    let first = store.search_observed_sessions(&scope, &q, 500).unwrap();
    assert_eq!(first.hits[0].session_id, "new");
    assert!(!first.index_partial);
    q.cursor = first.next_cursor;
    assert_eq!(
        store.link_observed_request(&super::tests::link("old", "other")),
        Err(ObservationV2Error::RelationshipConflict)
    );
    let next = store.search_observed_sessions(&scope, &q, 500).unwrap();
    assert!(next.hits.is_empty() && next.next_cursor.is_none() && !next.index_partial);
    assert_eq!(
        store.search_observed_sessions(&reader(false), &q, 500),
        Err(ObservationV2Error::Unauthorized)
    );
    let changed = ObservationReaderContext::run_scoped(
        WorkspaceId::default(),
        "worker".into(),
        2,
        10000,
        ["other".into()].into_iter().collect(),
        true,
        true,
    )
    .unwrap();
    assert_eq!(
        store.search_observed_sessions(&changed, &q, 500),
        Err(ObservationV2Error::Stale)
    );
    let mut changed_query = q.clone();
    changed_query.keyword = "different".into();
    assert_eq!(
        store.search_observed_sessions(&scope, &changed_query, 500),
        Err(ObservationV2Error::Stale)
    );
    scope.cancel();
    assert_eq!(
        store.search_observed_sessions(&scope, &q, 500),
        Err(ObservationV2Error::Unavailable)
    );
}

#[test]
fn live_search_respects_query_filters_and_does_not_call_unindexed_content_complete_empty() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    add(&store, "session", "request", 100, "needle");
    let pending = store
        .search_observed_sessions(&reader(true), &query("needle", 10), 500)
        .unwrap();
    assert!(pending.index_partial && pending.hits.is_empty() && pending.next_cursor.is_none());
    index(&store);
    // A building source with a leftover lookup marker remains incomplete;
    // a workspace-local ready set must not hide an unpublished source.
    store
        .connection
        .lock()
        .execute("UPDATE observation_text_index_v2 SET state='building'", [])
        .unwrap();
    let building = store
        .search_observed_sessions(&reader(true), &query("needle", 10), 500)
        .unwrap();
    assert!(building.hits.is_empty() && building.index_partial);
    store
        .connection
        .lock()
        .execute("UPDATE observation_text_index_v2 SET state='ready'", [])
        .unwrap();
    for keyword in ["missing", "needle"] {
        let mut q = query(keyword, 10);
        q.agent_id = Some("other".into());
        let page = store
            .search_observed_sessions(&reader(true), &q, 500)
            .unwrap();
        assert!(page.hits.is_empty() && !page.index_partial);
    }
    for kind in ["provider_state", "reasoning_delta", "reasoning_finished"] {
        store
            .connection
            .lock()
            .execute("UPDATE content_instances_v2 SET content_kind=?1", [kind])
            .unwrap();
        let page = store
            .search_observed_sessions(&reader(true), &query("needle", 10), 500)
            .unwrap();
        assert!(page.hits.is_empty() && !page.index_partial);
    }
    store
        .connection
        .lock()
        .execute("UPDATE content_instances_v2 SET content_kind='text'", [])
        .unwrap();
    let mut q = query("needle", 10);
    q.only_model_switch = true;
    assert!(
        store
            .search_observed_sessions(&reader(true), &q, 500)
            .unwrap()
            .hits
            .is_empty()
    );
    store.connection.lock().execute("INSERT INTO observation_attempt_models_v2(workspace_id,request_id,ordinal,model_id) VALUES(?1,'request',0,'one'),(?1,'request',1,'two')",[WorkspaceId::DEFAULT]).unwrap();
    assert_eq!(
        store
            .search_observed_sessions(&reader(true), &q, 500)
            .unwrap()
            .hits
            .len(),
        1
    );
    q.native_model = Some("absent".into());
    assert!(
        store
            .search_observed_sessions(&reader(true), &q, 500)
            .unwrap()
            .hits
            .is_empty()
    );
}
