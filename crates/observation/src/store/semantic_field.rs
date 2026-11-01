//! Operator-staged offline evidence, not an upgrade reader or converter.
//! The Gateway field exporter supplies current envelopes; a small stopped
//! fixture supplies unchanged business evidence for the same selected window.
use super::*;
use crate::{
    text_index::TextIndexBuilder,
    writer::{IngestOutcome, ObservationCommitPort},
};
use hiroute_domain::{
    ConversationContentEnvelopeV1, ObservationCatalogQueryV2, ObservationContentQueryV2,
    ObservationReaderContext,
};
use rusqlite::types::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader},
};

const TABLES: &[&str] = &[
    "execution_fact_events",
    "logical_requests",
    "observation_analysis_attempts_v1",
    "observation_analysis_models_v1",
    "observation_analysis_reasoning_v1",
    "observation_analysis_requests_v1",
    "observation_attempt_models_v2",
    "observation_safe_facts_v2",
    "observation_sensitive_payloads_v2",
    "plan_quality_segments",
    "routing_receipts",
    "sessions",
    "turns",
    "valuation_attempt_inputs_v2",
    "valuation_records_v2",
    "valuation_requests_v2",
];
const FROZEN: &[&str] = &[
    "execution_fact_events",
    "observation_attempt_models_v2",
    "observation_safe_facts_v2",
    "observation_sensitive_payloads_v2",
    "plan_quality_segments",
    "routing_receipts",
    "valuation_attempt_inputs_v2",
    "valuation_records_v2",
    "valuation_requests_v2",
];

fn fingerprints(db: &Connection) -> BTreeMap<String, (usize, String)> {
    FROZEN
        .iter()
        .map(|table| {
            let mut statement = db.prepare(&format!("SELECT * FROM {table}")).unwrap();
            let count = statement.column_count();
            let mut rows = statement
                .query_map([], |row| {
                    Ok((0..count)
                        .map(|i| format!("{:?}", row.get::<_, Value>(i).unwrap()))
                        .collect::<Vec<_>>()
                        .join("\0"))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows.sort();
            let mut hash = Sha256::new();
            for row in &rows {
                hash.update((row.len() as u64).to_be_bytes());
                hash.update(row.as_bytes());
            }
            (
                table.to_string(),
                (rows.len(), format!("{:x}", hash.finalize())),
            )
        })
        .collect()
}

fn allocated(path: &Path) -> u64 {
    let metadata = fs::symlink_metadata(path).unwrap();
    if metadata.is_dir() {
        return fs::read_dir(path)
            .unwrap()
            .map(|e| allocated(&e.unwrap().path()))
            .sum();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.blocks() * 512
    }
    #[cfg(not(unix))]
    {
        metadata.len()
    }
}

#[test]
#[ignore = "requires explicitly staged current field envelopes and a private stopped business fixture"]
fn current_writer_storage_and_reads_for_private_semantic_field_corpus() {
    let directory =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/semantic-observation-field");
    assert!(
        directory.join("manifest.json").is_file(),
        "explicitly staged field fixture only"
    );
    let root = directory.join("current-observation");
    assert!(
        !root.exists(),
        "never overwrite evidence from a previous field run"
    );
    let store = LocalObservationStore::open(&root, DigestAuthority::new([7; 32])).unwrap();
    store.set_capacity_limit(Some(10_000_000_000)).unwrap();
    {
        let db = store.connection.lock();
        let source = directory.join("business.db").canonicalize().unwrap();
        db.execute(
            "ATTACH DATABASE ?1 AS field_source",
            [format!("file:{}?mode=ro", source.display())],
        )
        .unwrap();
        for table in TABLES {
            let columns = db
                .prepare(&format!("PRAGMA field_source.table_info({table})"))
                .unwrap()
                .query_map([], |r| r.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(!columns.is_empty());
            assert!(
                columns
                    .iter()
                    .all(|name| name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
            );
            let columns = columns.join(",");
            db.execute(
                &format!(
                    "INSERT INTO {table}({columns}) SELECT {columns} FROM field_source.{table}"
                ),
                [],
            )
            .unwrap();
        }
        db.execute_batch("DETACH DATABASE field_source").unwrap();
    }
    let before = fingerprints(&store.connection.lock());
    let mut records = 0;
    for line in BufReader::new(fs::File::open(directory.join("capture.jsonl")).unwrap()).lines() {
        let event: ConversationContentEnvelopeV1 = serde_json::from_str(&line.unwrap()).unwrap();
        assert!(matches!(
            store.ingest_content(&event, &[]).unwrap(),
            IngestOutcome::Ack(_)
        ));
        records += 1;
    }
    assert!(records > 0);
    assert_eq!(
        fingerprints(&store.connection.lock()),
        before,
        "frozen score/rubric/membership/model/cost facts must be unchanged"
    );
    let mut index = TextIndexBuilder::new(&store).unwrap();
    let mut complete = false;
    for _ in 0..2000 {
        index.cycle(&store).unwrap();
        let pending:bool=store.connection.lock().query_row("SELECT EXISTS(SELECT 1 FROM content_blobs_v2 b WHERE b.state='complete' AND EXISTS(SELECT 1 FROM content_instances_v2 c WHERE c.workspace_id=b.workspace_id AND c.content_blob_digest=b.blob_digest AND c.state='complete') AND NOT EXISTS(SELECT 1 FROM observation_text_index_v2 i WHERE i.workspace=b.workspace_id AND i.digest=b.blob_digest AND i.state IN('ready','failed')))",[],|r|r.get(0)).unwrap();
        if !pending {
            complete = true;
            break;
        }
    }
    assert!(complete, "all current semantic bodies must finish indexing");
    drop(index);
    let requests:Vec<(String,String,i64)>=store.connection.lock().prepare("SELECT workspace_id,request_id,started_at_ms FROM logical_requests ORDER BY workspace_id,request_id").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap().collect::<Result<_,_>>().unwrap();
    let now = requests.iter().map(|r| r.2).max().unwrap() + 1;
    let mut bodies = 0;
    for (workspace, request, _) in &requests {
        let reader = ObservationReaderContext::local_user(
            WorkspaceId::parse(workspace).unwrap(),
            "field".into(),
            1,
            i64::MAX,
            true,
            true,
        )
        .unwrap();
        let mut query = ObservationCatalogQueryV2 {
            request_id: hiroute_domain::LogicalRequestId::parse(request).unwrap(),
            limit: 200,
            cursor: None,
            include_input_selection: true,
            include_semantic_metadata: true,
        };
        loop {
            let page = store.observed_catalog(&reader, &query, now).unwrap();
            assert!(
                !page.roots_partial,
                "ordinary selected complete requests must retain complete current captures"
            );
            for part in page.contents {
                let mut query = ObservationContentQueryV2 {
                    request_id: query.request_id.clone(),
                    content_id: part.content_id,
                    cursor: None,
                    anchor_offset: None,
                };
                let mut bytes = 0;
                loop {
                    let page = store.observed_content_page(&reader, &query, now).unwrap();
                    assert!(matches!(page.state.as_str(), "available" | "complete"));
                    bytes += page.chunks.iter().map(|c| c.text.len() as u64).sum::<u64>();
                    query.cursor = page.next_cursor;
                    if query.cursor.is_none() {
                        break;
                    }
                }
                assert_eq!(bytes, part.byte_count);
                bodies += 1;
            }
            query.cursor = page.next_cursor;
            if query.cursor.is_none() {
                break;
            }
        }
    }
    let counts = {
        let db = store.connection.lock();
        let chunks: u64 = db
            .query_row("SELECT COUNT(*) FROM content_chunks_v2", [], |r| r.get(0))
            .unwrap();
        assert_eq!(chunks, 0);
        let proofs: u64 = db
            .query_row(
                "SELECT COUNT(*) FROM observation_current_batch_v6",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(proofs <= crate::writer::MAX_CONTENT_BATCH_RECORDS as u64);
        let missing:u64=db.query_row("SELECT COUNT(*) FROM content_instances_v2 WHERE direction='response_delivered' AND attempt_id IS NULL",[],|r|r.get(0)).unwrap();
        assert_eq!(missing, 0);
        db.execute_batch(
            "PRAGMA wal_checkpoint(TRUNCATE); VACUUM; PRAGMA wal_checkpoint(TRUNCATE)",
        )
        .unwrap();
        serde_json::json!({"chunks":chunks,"bounded_commit_proofs":proofs,"output_without_attempt":missing})
    };
    drop(store);
    let store = LocalObservationStore::open(&root, DigestAuthority::new([7; 32])).unwrap();
    assert_eq!(fingerprints(&store.connection.lock()), before);
    assert_eq!(
        store.capacity_status().unwrap().max_active_bytes,
        Some(10_000_000_000)
    );
    drop(store);
    let result = serde_json::json!({"requests":requests.len(),"body_reads":bodies,"ingested_records":records,"database_bytes":fs::metadata(root.join("activity.db")).unwrap().len(),"allocated_total_bytes":allocated(&root),"integrity_and_reopen":true,"frozen_business_facts_equal":true,"counts":counts});
    fs::write(
        directory.join("result.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    println!("{result}");
}
