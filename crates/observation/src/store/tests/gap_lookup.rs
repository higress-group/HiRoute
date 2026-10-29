use hiroute_domain::{ObservationChannel, ObservationQueryPort, ReceiptId};
use rusqlite::{Connection, StatementStatus, params};
use tempfile::tempdir;

use super::super::gap::{RESOLVE_SEQUENCE_SQL, resolve_sequence};
use super::support::*;
use crate::{LocalObservationStore, WriterCycleOutcome};

fn seed_old_gaps(store: &LocalObservationStore, count: i64) {
    store
        .connection
        .lock()
        .execute(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<?1)
         INSERT INTO observation_gaps
         (channel, producer_id, producer_epoch, stream_id, first_sequence, last_sequence,
          known_loss, workspace_id, session_id, reason)
         SELECT 'content','producer','epoch','stream',i,i,0,'workspace','session','old'
         FROM n UNION ALL
         SELECT 'content','producer','other-epoch','stream',i,i,1,'workspace',NULL,'epoch'
         FROM n UNION ALL
         SELECT 'content','producer','epoch','other-stream',i,i,0,'workspace',NULL,'stream'
         FROM n UNION ALL
         SELECT 'fact','producer','epoch','stream',i,i,0,'workspace',NULL,'channel'
         FROM n",
            [count],
        )
        .unwrap();
}

fn lookup_work(connection: &Connection, sequence: i64) -> (Vec<i64>, i32, i32) {
    let mut statement = connection.prepare(RESOLVE_SEQUENCE_SQL).unwrap();
    let ids = statement
        .query_map(
            params!["content", "producer", "epoch", "stream", sequence],
            |row| row.get(0),
        )
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    (
        ids,
        statement.get_status(StatementStatus::VmStep),
        statement.get_status(StatementStatus::FullscanStep),
    )
}

#[test]
fn gap_lookup_skips_unrelated_streams_and_already_ended_ranges() {
    for count in [1_001, 25_001] {
        let root = tempdir().unwrap();
        let store = open_store(root.path());
        seed_old_gaps(&store, count);
        let connection = store.connection.lock();
        let (ids, steps, scanned) = lookup_work(&connection, count + 1);
        assert!(ids.is_empty());
        assert_eq!(scanned, 0);
        assert!(steps < 128, "{} old gaps took {steps} VM steps", count * 4);

        let mut plan = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {RESOLVE_SEQUENCE_SQL}"))
            .unwrap();
        let details = plan
            .query_map(
                params!["content", "producer", "epoch", "stream", count + 1],
                |row| row.get::<_, String>(3),
            )
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            details
                .iter()
                .any(|line| line.contains("observation_gaps_stream_end")
                    && line.contains("last_sequence>?")),
            "must seek past earlier gaps, rather than scan first_sequence: {details:?}"
        );
    }
}

fn gap_rows(connection: &Connection) -> Vec<String> {
    let mut statement = connection
        .prepare(
            "SELECT json_array(gap_id,channel,producer_id,producer_epoch,stream_id,
             first_sequence,last_sequence,known_loss,workspace_id,session_id,reason)
             FROM observation_gaps ORDER BY gap_id",
        )
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn current_store_reopen_adds_gap_index_without_changing_history_or_receipts() {
    let root = tempdir().unwrap();
    let fixture = Fixture::new("gap-reopen");
    let store = open_store(root.path());
    let facts = request_facts(&fixture, "attempt-gap", finished("attempt-gap"), 100);
    {
        let writer = writer(&store);
        let channel = fact_channel(&fixture, 512 * 1024);
        for fact in facts.clone() {
            assert!(matches!(
                offer_fact(&writer, &channel, fact),
                WriterCycleOutcome::Ack(_)
            ));
        }
    }
    let receipt_id = ReceiptId::parse(fixture.request.as_str()).unwrap();
    let receipt = store.get_receipt(&fixture.workspace, &receipt_id).unwrap();
    seed_old_gaps(&store, 25_001);
    let original_gaps = {
        let connection = store.connection.lock();
        connection
            .execute_batch("DROP INDEX observation_gaps_stream_end")
            .unwrap();
        let (ids, steps, scanned) = lookup_work(&connection, 25_002);
        assert!(ids.is_empty());
        assert!(
            steps > 100_000 && scanned > 100_000,
            "negative control: {steps}/{scanned}"
        );
        gap_rows(&connection)
    };
    drop(store);

    let mut index_root = None;
    for _ in 0..2 {
        let store = open_store(root.path());
        assert_eq!(
            store.get_receipt(&fixture.workspace, &receipt_id).unwrap(),
            receipt
        );
        {
            let connection = store.connection.lock();
            assert_eq!(gap_rows(&connection), original_gaps);
            let (ids, steps, scanned) = lookup_work(&connection, 25_002);
            assert!(ids.is_empty());
            assert!(steps < 128);
            assert_eq!(scanned, 0);
            let root: i64 = connection
                .query_row(
                    "SELECT rootpage FROM sqlite_schema WHERE name='observation_gaps_stream_end'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                *index_root.get_or_insert(root),
                root,
                "reopen must reuse the index"
            );
        }
        let writer = writer(&store);
        let channel = fact_channel(&fixture, 512 * 1024);
        let WriterCycleOutcome::Ack(ack) = offer_fact(&writer, &channel, facts[9].clone()) else {
            panic!("historical replay must retain its acknowledgement")
        };
        assert_eq!(ack.highest_contiguous_sequence, 10);
        assert_eq!(ack.highest_accounted_sequence, 10);
    }
}

#[test]
fn indexed_resolution_preserves_every_overlapping_gap_scope_and_reason() {
    let root = tempdir().unwrap();
    let fixture = Fixture::new("gap-split");
    let store = open_store(root.path());
    let mut connection = store.connection.lock();
    let tx = connection.transaction().unwrap();
    for (first, last, known, session, reason) in [
        (7, 9, false, Some("session-a"), Some("missing")),
        (8, 10, true, Some("session-b"), Some("declared-loss")),
        (8, 8, false, None, None),
    ] {
        tx.execute(
            "INSERT INTO observation_gaps
             (channel,producer_id,producer_epoch,stream_id,first_sequence,last_sequence,
              known_loss,workspace_id,session_id,reason) VALUES('content',?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![fixture.content_stream.producer_id.as_str(),
                fixture.content_stream.producer_epoch.as_str(), fixture.content_stream.stream_id.as_str(),
                first, last, known, fixture.workspace.as_str(), session, reason],
        ).unwrap();
    }
    resolve_sequence(&tx, ObservationChannel::Content, &fixture.content_stream, 8).unwrap();
    let fragments: Vec<(i64, i64, bool, String, String)> = tx
        .prepare(
            "SELECT first_sequence,last_sequence,known_loss,session_id,reason FROM observation_gaps
         ORDER BY session_id,first_sequence",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        fragments,
        vec![
            (7, 7, false, "session-a".into(), "missing".into()),
            (9, 9, false, "session-a".into(), "missing".into()),
            (9, 10, true, "session-b".into(), "declared-loss".into()),
        ]
    );
    // Both fragments of an overlap must survive, and replay must be idempotent.
    resolve_sequence(&tx, ObservationChannel::Content, &fixture.content_stream, 8).unwrap();
    assert_eq!(gap_rows(&tx).len(), 3);
    for sequence in [7, 9, 10] {
        resolve_sequence(
            &tx,
            ObservationChannel::Content,
            &fixture.content_stream,
            sequence,
        )
        .unwrap();
    }
    assert!(gap_rows(&tx).is_empty());
    tx.rollback().unwrap();
}
