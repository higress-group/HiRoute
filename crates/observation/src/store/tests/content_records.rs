use super::*;
use crate::store::{record_projection, records};

#[test]
fn schema_markers_remain_original_record_data() {
    let fixture = Fixture::new();
    let begin = fixture.begin(1);
    let shared = record_projection::ContentColumns::from_envelope(&begin).shared();
    let mut original = serde_json::to_value(&begin).unwrap();
    original
        .as_object_mut()
        .unwrap()
        .remove("canonical_bytes_base64");
    original["schema_version"] = serde_json::json!("retained-original-schema");
    original["schema_digest"] = serde_json::json!("retained-original-digest");
    let frame = record_projection::encode(original.clone(), &shared).unwrap();
    assert_eq!(
        record_projection::decode(&frame, &shared).unwrap(),
        original
    );
}

#[test]
fn mixed_old_and_projected_records_reopen_with_original_feedback_and_history() {
    let fixture = Fixture::new();
    let begin = fixture.begin(1);
    let append = fixture.append(2, 0, "message-1", "content-1", 0, 0, b"history", b"history");
    let finish = fixture.finish(3, "11");
    let events = [&begin, &append, &finish];
    let feedback: Vec<_> = events.iter().map(|e| ack(fixture.ingest(e))).collect();
    {
        let db = fixture.store.connection.lock();
        // This is the former serializer and tag0/1 codec, not the new projection.
        let mut metadata = serde_json::to_value(&begin).unwrap();
        metadata
            .as_object_mut()
            .unwrap()
            .remove("canonical_bytes_base64");
        db.execute(
            "UPDATE conversation_content_events_v2 SET metadata_json=?1 WHERE sequence=1",
            [records::encode(&serde_json::to_vec(&metadata).unwrap()).unwrap()],
        )
        .unwrap();
        db.execute("UPDATE observation_feedback_v2 SET feedback_json=?1 WHERE rejected_or_acked_sequence=1",
            [records::encode(&serde_json::to_vec(&feedback[0]).unwrap()).unwrap()]).unwrap();
    }
    let reopened =
        LocalObservationStore::open(fixture._root.path(), fixture.authority.clone()).unwrap();
    for (e, expected) in events.iter().zip(&feedback) {
        assert_eq!(ack(reopened.ingest_content(e, &[]).unwrap()), *expected);
    }
    let history = reopened
        .get_session(
            &fixture.workspace,
            &SessionId::parse("conversation-1").unwrap(),
            hiroute_domain::ContentMode::MessagesAndTools,
        )
        .unwrap();
    assert_eq!(history.turns[0].messages[0].bytes, b"history");
    assert_eq!(
        feedback[0].highest_contiguous_sequence, 1,
        "replay must not borrow the later checkpoint"
    );
}

#[test]
fn projected_content_metadata_is_bound_to_its_original_sql_columns() {
    for column in [
        "workspace_id",
        "request_id",
        "direction",
        "fork_id",
        "phase",
    ] {
        let fixture = Fixture::new();
        let begin = fixture.begin(1);
        ack(fixture.ingest(&begin));
        fixture
            .store
            .connection
            .lock()
            .execute(
                &format!(
                    "UPDATE conversation_content_events_v2 SET {column}='changed' WHERE sequence=1"
                ),
                [],
            )
            .unwrap();
        assert_eq!(
            fixture.store.ingest_content(&begin, &[]).unwrap_err(),
            ObservationStoreError::Corrupt,
            "changed column {column}"
        );
    }
}

#[test]
fn projected_ack_and_nack_keep_their_original_complete_values() {
    let fixture = Fixture::new();
    let bad = fixture.append(1, 0, "message-1", "content-1", 0, 0, b"x", b"x");
    let rejected = fixture.ingest(&bad);
    assert!(matches!(rejected, IngestOutcome::Nack(_)));
    let reopened =
        LocalObservationStore::open(fixture._root.path(), fixture.authority.clone()).unwrap();
    assert_eq!(reopened.ingest_content(&bad, &[]).unwrap(), rejected);
    let db = reopened.connection.lock();
    let frame: Vec<u8> = db
        .query_row(
            "SELECT feedback_json FROM observation_feedback_v2 ORDER BY feedback_id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(matches!(frame[0], 2 | 3));
    assert_eq!(
        record_projection::decode(
            &frame,
            &record_projection::feedback_fields("conversation_content", &fixture.stream)
        )
        .unwrap(),
        match rejected {
            IngestOutcome::Ack(v) => serde_json::to_value(v).unwrap(),
            IngestOutcome::Nack(v) => serde_json::to_value(v).unwrap(),
        }
    );
}
