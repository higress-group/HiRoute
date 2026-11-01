use super::*;

#[test]
fn rejection_commits_only_the_verified_prefix_and_keeps_later_records_unprocessed() {
    let fixture = Fixture::new();
    let begin = fixture.begin(1);
    let good = fixture.append(2, 0, "message-1", "content-1", 0, 0, b"first", b"first");
    let bad = fixture.append(3, 1, "message-1", "content-2", 0, 1, b"second", b"wrong");
    let later = fixture.append(4, 2, "message-1", "content-3", 0, 2, b"third", b"third");
    let outcomes = fixture
        .store
        .ingest_content_batch(&[&begin, &good, &bad, &later])
        .unwrap();
    assert_eq!(outcomes.len(), 3);
    assert!(matches!(&outcomes[0], IngestOutcome::Ack(_)));
    let good_ack = ack(outcomes[1].clone());
    assert_eq!(good_ack.highest_contiguous_sequence, 2);
    assert!(matches!(&outcomes[2], IngestOutcome::Nack(_)));
    let connection = fixture.store.connection.lock();
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM content_instances_v2 WHERE content_id='content-3'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    drop(connection);
    assert_eq!(good_ack, ack(fixture.ingest(&good)));
}

#[test]
fn failed_file_barrier_returns_no_ack_and_no_advanced_checkpoint_then_exact_retry_succeeds() {
    let fixture = Fixture::new();
    let begin = fixture.begin(1);
    let append = fixture.append(2, 0, "message-1", "content-1", 0, 0, b"durable", b"durable");
    let finish = fixture.finish(3, "44");
    crate::content::durability::fail_next_flush();
    assert_eq!(
        fixture
            .store
            .ingest_content_batch(&[&begin, &append, &finish]),
        Err(ObservationStoreError::ContentUnavailable)
    );
    let connection = fixture.store.connection.lock();
    let events: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM conversation_content_events_v2",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(events, 0);
    drop(connection);
    let outcomes = fixture
        .store
        .ingest_content_batch(&[&begin, &append, &finish])
        .unwrap();
    assert_eq!(outcomes.len(), 3);
    let expected = ack(outcomes[2].clone());
    drop(fixture.store);
    let reopened = LocalObservationStore::open(fixture._root.path(), fixture.authority).unwrap();
    assert_eq!(
        expected,
        ack(reopened.ingest_content(&finish, &[]).unwrap())
    );
    assert!(matches!(
        reopened.ingest_content(&append, &[]).unwrap(),
        IngestOutcome::Ack(_)
    ));
}
