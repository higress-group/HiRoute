use std::sync::Arc;

use hiroute_domain::{
    AttemptId, CANONICAL_RESPONSE_CONTENT_MEDIA_TYPE_V1, CONVERSATION_CONTENT_PORT_DIGEST_V2,
    CONVERSATION_CONTENT_SCHEMA_V2, ContentCompleteness, ContentCompletenessDeltaV2,
    ContentCorrelationV2, ContentDownstreamDeliveryV2, ContentId, ContentRefV2,
    ConversationContentChannelV2, ConversationContentDirectionV2, ConversationContentEnvelopeV1,
    ConversationContentPhaseV2, CorrelationProvenance, EventId, LogicalRequestId,
    MessageInstanceId, OBSERVATION_GAP_HEARTBEAT_SCHEMA_V1, ObservationGapHeartbeatV1,
    ObservationNackDetailV1, ObservationProducerV2, ObservationQueryPort, ObservationStreamV1,
    ProducerEpoch, ProducerId, SessionId, SessionScopeV1, StreamId, TranscriptRoot, TurnId,
    WorkspaceId,
};
use rusqlite::Connection;
use tempfile::TempDir;

use crate::writer::{IngestOutcome, ObservationCommitPort, ObservationStoreError};
use crate::{DigestAuthority, LocalObservationStore};

mod batch;
mod pressure;
mod semantic_capture;

struct Fixture {
    _root: TempDir,
    store: Arc<LocalObservationStore>,
    authority: DigestAuthority,
    workspace: WorkspaceId,
    stream: ObservationStreamV1,
}

#[test]
fn unfinished_capture_cannot_borrow_a_previous_requests_complete_status() {
    let fixture = Fixture::new();
    ack(fixture.ingest(&fixture.begin(1)));
    ack(fixture.ingest(&fixture.finish(2, "11")));
    let state = || {
        fixture
            .store
            .get_session(
                &fixture.workspace,
                &SessionId::parse("conversation-1").unwrap(),
                hiroute_domain::ContentMode::None,
            )
            .unwrap()
            .summary
            .content_completeness
    };
    assert_eq!(state(), ContentCompleteness::Complete);
    let mut next_begin = fixture.begin(3);
    next_begin.correlation.request_id = LogicalRequestId::parse("request-2").unwrap();
    ack(fixture.ingest(&next_begin));
    assert_eq!(state(), ContentCompleteness::Unknown);
    let mut next_finish = fixture.finish(4, "22");
    next_finish.correlation.request_id = next_begin.correlation.request_id;
    ack(fixture.ingest(&next_finish));
    assert_eq!(state(), ContentCompleteness::Complete);
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let authority = DigestAuthority::new([7; 32]);
        let store = Arc::new(LocalObservationStore::open(root.path(), authority.clone()).unwrap());
        Self {
            _root: root,
            store,
            authority,
            workspace: WorkspaceId::default(),
            stream: ObservationStreamV1 {
                producer_id: ProducerId::parse("content-producer").unwrap(),
                producer_epoch: ProducerEpoch::parse("epoch-1").unwrap(),
                stream_id: StreamId::parse("stream-1").unwrap(),
            },
        }
    }

    fn base(
        &self,
        sequence: u64,
        phase: ConversationContentPhaseV2,
    ) -> ConversationContentEnvelopeV1 {
        ConversationContentEnvelopeV1 {
            schema_version: CONVERSATION_CONTENT_SCHEMA_V2.into(),
            schema_digest: CONVERSATION_CONTENT_PORT_DIGEST_V2.into(),
            channel: ConversationContentChannelV2::ConversationContent,
            producer: ObservationProducerV2 {
                component: "gateway-content".into(),
                revision: "g-star".into(),
                stream: self.stream.clone(),
            },
            sequence,
            event_id: EventId::parse(format!("event-{sequence}")).unwrap(),
            correlation: ContentCorrelationV2 {
                workspace_id: self.workspace.clone(),
                conversation_id: SessionId::parse("conversation-1").unwrap(),
                session_scope: SessionScopeV1::Conversation,
                correlation_provenance: CorrelationProvenance::AgentSupplied,
                turn_id: TurnId::parse("turn-1").unwrap(),
                request_id: LogicalRequestId::parse("request-1").unwrap(),
            },
            direction: ConversationContentDirectionV2::RequestInput,
            phase,
            attempt_id: None,
            fork_id: "fork-request".into(),
            parent_transcript_root: None,
            result_transcript_root: None,
            message_instance_id: None,
            message_role: None,
            content_kind: None,
            content_id: None,
            content_blob_digest: None,
            message_ordinal: None,
            part_ordinal: None,
            chunk_ordinal: None,
            capture: None,
            transport_frame_id: None,
            canonical_media_type: None,
            canonical_bytes_base64: None,
            content_ref: None,
            downstream_delivery: None,
            abort_reason: None,
            occurred_at_unix_nanos: 1_000_000_000 + sequence,
            loss_watermark: None,
            completeness_delta: None,
        }
    }

    fn begin(&self, sequence: u64) -> ConversationContentEnvelopeV1 {
        self.base(sequence, ConversationContentPhaseV2::Begin)
    }

    #[allow(clippy::too_many_arguments)]
    fn append(
        &self,
        sequence: u64,
        chunk_ordinal: u32,
        message: &str,
        content: &str,
        message_ordinal: u32,
        part_ordinal: u32,
        whole: &[u8],
        chunk: &[u8],
    ) -> ConversationContentEnvelopeV1 {
        let mut value = self.base(sequence, ConversationContentPhaseV2::Append);
        let digest = self
            .authority
            .content_blob_digest("text/plain; charset=utf-8", whole);
        value.message_instance_id = Some(MessageInstanceId::parse(message).unwrap());
        value.message_role = Some("user".into());
        value.content_kind = Some("text".into());
        value.content_id = Some(ContentId::parse(content).unwrap());
        value.content_blob_digest = Some(digest.clone());
        value.message_ordinal = Some(message_ordinal);
        value.part_ordinal = Some(part_ordinal);
        value.chunk_ordinal = Some(chunk_ordinal);
        value.canonical_media_type = Some("text/plain; charset=utf-8".into());
        value.canonical_bytes_base64 = Some(base64(chunk));
        value.content_ref = Some(ContentRefV2 {
            content_id: ContentId::parse(content).unwrap(),
            digest,
            byte_count: whole.len() as u64,
            media_type: "text/plain; charset=utf-8".into(),
        });
        value
    }

    fn finish(&self, sequence: u64, root_byte: &str) -> ConversationContentEnvelopeV1 {
        let mut value = self.base(sequence, ConversationContentPhaseV2::Finish);
        value.result_transcript_root =
            Some(TranscriptRoot::parse(format!("transcript-{}", root_byte.repeat(32))).unwrap());
        value.completeness_delta = Some(ContentCompletenessDeltaV2::Complete);
        value
    }

    fn ingest(&self, value: &ConversationContentEnvelopeV1) -> IngestOutcome {
        self.store.ingest_content(value, &[]).unwrap()
    }
}

#[test]
fn direction_stream_persists_semantic_parts_and_only_the_current_commit_proof() {
    let fixture = Fixture::new();
    let begin = fixture.begin(1);
    let append_one = fixture.append(2, 0, "message-1", "content-1", 0, 0, b"hello", b"hello");
    let append_two = fixture.append(3, 1, "message-2", "content-2", 1, 0, b"world", b"world");
    let finish = fixture.finish(4, "11");

    for event in [&begin, &append_one, &append_two, &finish] {
        assert!(ack(fixture.ingest(event)).content_acknowledgement.is_none());
    }
    let finish_ack = ack(fixture.ingest(&finish));
    assert_eq!(finish_ack.highest_contiguous_sequence, 4);
    let reopened =
        LocalObservationStore::open(fixture._root.path(), fixture.authority.clone()).unwrap();
    assert_eq!(
        finish_ack,
        ack(reopened.ingest_content(&finish, &[]).unwrap())
    );
    let connection = fixture.store.connection.lock();
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE name IN('observation_events','conversation_content_events_v2','observation_feedback_v2','request_content_refs_v2')",[],|r|r.get::<_,u64>(0)).unwrap(),0);
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM content_chunks_v2", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM observation_current_commits_v6",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
    drop(connection);
    let detail = fixture
        .store
        .get_session(
            &fixture.workspace,
            &SessionId::parse("conversation-1").unwrap(),
            hiroute_domain::ContentMode::MessagesAndTools,
        )
        .unwrap();
    assert_eq!(detail.turns[0].messages.len(), 2);
    assert_eq!(detail.turns[0].messages[0].bytes, b"hello");
    assert_eq!(detail.turns[0].messages[1].bytes, b"world");
    assert_eq!(detail.turns[0].messages[1].message_ordinal, 1);
    assert_eq!(
        detail.turns[0].messages[0].transcript_root,
        finish.result_transcript_root
    );
}

#[test]
fn accepted_response_abort_preserves_attempt_frame_ref_parent_and_query_coordinates() {
    let fixture = Fixture::new();
    let begin = fixture.begin(1);
    let mut request = fixture.append(
        2,
        0,
        "message-request",
        "content-request",
        0,
        0,
        b"prompt",
        b"prompt",
    );
    request.content_ref = None;
    let request_finish = fixture.finish(3, "66");
    for event in [&begin, &request, &request_finish] {
        assert!(matches!(fixture.ingest(event), IngestOutcome::Ack(_)));
    }
    let parent = request_finish.result_transcript_root.clone().unwrap();

    let mut response_begin = fixture.base(4, ConversationContentPhaseV2::Begin);
    response_begin.direction = ConversationContentDirectionV2::ResponseDelivered;
    response_begin.attempt_id = Some(AttemptId::parse("attempt-accepted").unwrap());
    response_begin.fork_id = "fork-response".into();
    response_begin.parent_transcript_root = Some(parent.clone());
    let begin_ack = ack(fixture.ingest(&response_begin));
    assert!(begin_ack.content_acknowledgement.is_none());

    let bytes = br#"{"event":{"kind":"text_delta","delta":"answer"}}"#;
    let digest = fixture
        .authority
        .content_blob_digest(CANONICAL_RESPONSE_CONTENT_MEDIA_TYPE_V1, bytes);
    let mut response = fixture.base(5, ConversationContentPhaseV2::Append);
    response.direction = ConversationContentDirectionV2::ResponseDelivered;
    response.attempt_id = Some(AttemptId::parse("attempt-accepted").unwrap());
    response.fork_id = "fork-response".into();
    response.parent_transcript_root = Some(parent.clone());
    response.message_instance_id = Some(MessageInstanceId::parse("message-response").unwrap());
    response.message_role = Some("assistant".into());
    response.content_kind = Some("text_delta".into());
    response.content_id = Some(ContentId::parse("content-response").unwrap());
    response.content_blob_digest = Some(digest.clone());
    response.message_ordinal = Some(0);
    response.part_ordinal = Some(7);
    response.chunk_ordinal = Some(0);
    response.transport_frame_id = Some("frame-accepted-7".into());
    response.canonical_media_type = Some(CANONICAL_RESPONSE_CONTENT_MEDIA_TYPE_V1.into());
    response.canonical_bytes_base64 = Some(base64(bytes));
    response.content_ref = Some(ContentRefV2 {
        content_id: ContentId::parse("content-response").unwrap(),
        digest: digest.clone(),
        byte_count: bytes.len() as u64,
        media_type: CANONICAL_RESPONSE_CONTENT_MEDIA_TYPE_V1.into(),
    });
    response.downstream_delivery = Some(ContentDownstreamDeliveryV2::FullFrameTransportAccepted);
    let response_ack = ack(fixture.ingest(&response));
    assert!(response_ack.content_acknowledgement.is_none());
    assert_eq!(
        fixture
            .store
            .connection
            .lock()
            .query_row(
                "SELECT attempt_id FROM content_instances_v2 WHERE content_id='content-response'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "attempt-accepted"
    );

    let mut abort = fixture.base(6, ConversationContentPhaseV2::Abort);
    abort.direction = ConversationContentDirectionV2::ResponseDelivered;
    abort.attempt_id = Some(AttemptId::parse("attempt-accepted").unwrap());
    abort.fork_id = "fork-response".into();
    abort.parent_transcript_root = Some(parent.clone());
    abort.result_transcript_root =
        Some(TranscriptRoot::parse(format!("transcript-{}", "77".repeat(32))).unwrap());
    abort.downstream_delivery = Some(ContentDownstreamDeliveryV2::FullFrameTransportAccepted);
    abort.abort_reason = Some("accepted_stream_ended_partial".into());
    abort.completeness_delta = Some(ContentCompletenessDeltaV2::Partial);
    let abort_ack = ack(fixture.ingest(&abort));
    assert!(abort_ack.content_acknowledgement.is_none());

    let detail = fixture
        .store
        .get_session(
            &fixture.workspace,
            &SessionId::parse("conversation-1").unwrap(),
            hiroute_domain::ContentMode::MessagesAndTools,
        )
        .unwrap();
    assert_eq!(
        detail.summary.content_completeness,
        ContentCompleteness::Partial
    );
    let response = detail.turns[0]
        .messages
        .iter()
        .find(|message| message.direction == ConversationContentDirectionV2::ResponseDelivered)
        .unwrap();
    assert_eq!(
        response.attempt_id.as_ref().map(AttemptId::as_str),
        Some("attempt-accepted")
    );
    assert_eq!(response.fork_id, "fork-response");
    assert_eq!(
        response.transport_frame_id.as_deref(),
        Some("frame-accepted-7")
    );
    assert_eq!(
        response.content_ref.as_ref().unwrap().byte_count,
        bytes.len() as u64
    );
    assert_eq!(
        response.downstream_delivery,
        Some(ContentDownstreamDeliveryV2::FullFrameTransportAccepted)
    );
    assert_eq!(response.bytes, bytes);
    assert_eq!(
        response
            .parent_transcript_root
            .as_ref()
            .map(TranscriptRoot::as_str),
        Some(parent.as_str())
    );
}

#[test]
fn wrong_schema_digest_sequence_identity_and_terminal_state_are_durable_typed_nacks() {
    let fixture = Fixture::new();
    let mut version = fixture.begin(1);
    version.schema_version = "hiroute.observation.conversation-content-envelope/v999".into();
    assert!(matches!(nack(fixture.ingest(&version)).detail,
        ObservationNackDetailV1::UnsupportedSchema { ref rejected_schema_version, .. }
            if rejected_schema_version.ends_with("/v3")));

    let fixture = Fixture::new();
    let mut digest = fixture.begin(1);
    digest.schema_digest = format!("sha256:{}", "00".repeat(32));
    let digest_nack = nack(fixture.ingest(&digest));
    assert!(matches!(digest_nack.detail,
        ObservationNackDetailV1::DigestMismatch {
            subject: hiroute_domain::ObservationDigestSubjectV1::EnvelopeSchema,
            ref expected_digest, ref rejected_digest, ..
        } if expected_digest == CONVERSATION_CONTENT_PORT_DIGEST_V2
            && rejected_digest == &format!("sha256:{}", "00".repeat(32))));
    let reopened =
        LocalObservationStore::open(fixture._root.path(), fixture.authority.clone()).unwrap();
    assert_eq!(
        digest_nack,
        nack(reopened.ingest_content(&digest, &[]).unwrap())
    );

    let fixture = Fixture::new();
    assert!(matches!(
        fixture.ingest(&fixture.begin(1)),
        IngestOutcome::Ack(_)
    ));
    let mut conflicting = fixture.begin(1);
    conflicting.event_id = EventId::parse("different-event-1").unwrap();
    assert!(matches!(nack(fixture.ingest(&conflicting)).detail,
        ObservationNackDetailV1::SequenceEventConflict {
            sequence: 1, ref expected_event_id, ref rejected_event_id
        } if expected_event_id == "event-1" && rejected_event_id == "different-event-1"));

    let mut wrong_correlation = fixture.append(2, 0, "message-1", "content-1", 0, 0, b"x", b"x");
    wrong_correlation.correlation.session_scope = SessionScopeV1::RequestScoped;
    wrong_correlation.correlation.correlation_provenance = CorrelationProvenance::ProtocolState;
    assert!(matches!(nack(fixture.ingest(&wrong_correlation)).detail,
        ObservationNackDetailV1::ImmutableProjectionConflict { ref projection_key, .. }
            if projection_key.contains("content_stream")));

    let append = fixture.append(2, 0, "message-1", "content-1", 0, 0, b"x", b"x");
    assert!(matches!(fixture.ingest(&append), IngestOutcome::Ack(_)));
    let conflicting_content =
        fixture.append(3, 1, "message-1", "content-conflict", 0, 0, b"y", b"y");
    assert!(matches!(nack(fixture.ingest(&conflicting_content)).detail,
        ObservationNackDetailV1::ImmutableProjectionConflict { ref projection_key, .. }
            if projection_key.contains("message_part")));
    assert!(matches!(
        fixture.ingest(&fixture.finish(3, "88")),
        IngestOutcome::Ack(_)
    ));
    let mut after_terminal = fixture.append(4, 1, "message-2", "content-2", 1, 0, b"y", b"y");
    after_terminal.event_id = EventId::parse("append-after-finish").unwrap();
    assert!(matches!(
        nack(fixture.ingest(&after_terminal)).detail,
        ObservationNackDetailV1::ContentStateConflict {
            expected_phase: ConversationContentPhaseV2::Finish,
            rejected_phase: ConversationContentPhaseV2::Append,
            ..
        }
    ));
}

#[test]
fn empty_canonical_chunk_is_persisted_as_an_exact_zero_byte_blob() {
    let fixture = Fixture::new();
    assert!(matches!(
        fixture.ingest(&fixture.begin(1)),
        IngestOutcome::Ack(_)
    ));
    let append = fixture.append(2, 0, "message-empty", "content-empty", 0, 0, b"", b"");
    let ack = ack(fixture.ingest(&append));
    assert!(ack.content_acknowledgement.is_none());
    assert!(matches!(
        fixture.ingest(&fixture.finish(3, "99")),
        IngestOutcome::Ack(_)
    ));
    let detail = fixture
        .store
        .get_session(
            &fixture.workspace,
            &SessionId::parse("conversation-1").unwrap(),
            hiroute_domain::ContentMode::MessagesAndTools,
        )
        .unwrap();
    assert!(detail.turns[0].messages[0].bytes.is_empty());
}

#[test]
fn current_append_retry_checks_bytes_but_terminal_commit_ack_is_not_a_body_availability_claim() {
    let fixture = Fixture::new();
    let begin = fixture.begin(1);
    let append = fixture.append(2, 0, "message-1", "content-1", 0, 0, b"x", b"x");
    let finish = fixture.finish(3, "aa");
    ack(fixture.ingest(&begin));
    ack(fixture.ingest(&append));
    let path: String = fixture
        .store
        .connection
        .lock()
        .query_row("SELECT object_path FROM content_blobs_v2", [], |r| r.get(0))
        .unwrap();
    let path = fixture.store.object_path(&path).unwrap();
    std::fs::write(&path, b"z").unwrap();
    assert_eq!(
        fixture.store.ingest_content(&append, &[]).unwrap_err(),
        ObservationStoreError::Corrupt
    );
    std::fs::write(&path, b"x").unwrap();
    let committed = ack(fixture.ingest(&finish));
    std::fs::write(&path, b"z").unwrap();
    assert_eq!(ack(fixture.ingest(&finish)), committed);
    assert_eq!(
        fixture.store.ingest_content(&append, &[]).unwrap_err(),
        ObservationStoreError::Corrupt
    );
    assert!(
        fixture
            .store
            .get_session(
                &fixture.workspace,
                &SessionId::parse("conversation-1").unwrap(),
                hiroute_domain::ContentMode::MessagesAndTools
            )
            .is_err()
    );
}

#[test]
fn legacy_schema_is_not_converted_by_the_current_store() {
    let root = tempfile::tempdir().unwrap();
    let connection = Connection::open(root.path().join("activity.db")).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE observation_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         INSERT INTO observation_meta(key, value) VALUES
           ('schema_version', '3'), ('store_revision', '7');
         CREATE TABLE observation_gaps (
           gap_id INTEGER PRIMARY KEY AUTOINCREMENT,
           channel TEXT NOT NULL,
           producer_id TEXT NOT NULL,
           producer_epoch TEXT NOT NULL,
           stream_id TEXT NOT NULL,
           first_sequence INTEGER NOT NULL,
           last_sequence INTEGER NOT NULL,
           known_loss INTEGER NOT NULL,
           workspace_id TEXT NOT NULL,
           session_id TEXT
         );
         CREATE TABLE content_streams (content_id TEXT);
         CREATE TABLE content_blobs (blob_digest TEXT);
         CREATE TABLE message_instances (message_instance_id TEXT);
         CREATE TABLE transcript_roots (transcript_root TEXT);",
        )
        .unwrap();
    drop(connection);

    let mut historical = Connection::open(root.path().join("activity.db")).unwrap();
    assert!(super::super::schema::migrate(&mut historical).is_err());
    assert_eq!(
        historical
            .query_row(
                "SELECT value FROM observation_meta WHERE key='schema_version'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "3"
    );
    assert!(LocalObservationStore::open(root.path(), DigestAuthority::new([7; 32])).is_err());
}

#[test]
fn populated_fake_content_contract_fails_closed_without_inventing_v2_coordinates() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("activity.db");
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE observation_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO observation_meta(key, value) VALUES
               ('schema_version', '3'), ('store_revision', '7');
             CREATE TABLE content_streams (content_id TEXT);
             INSERT INTO content_streams(content_id) VALUES ('legacy-content');",
        )
        .unwrap();
    drop(connection);

    assert!(matches!(
        LocalObservationStore::open(&root, DigestAuthority::new([7; 32])),
        Err(ObservationStoreError::Corrupt)
    ));
    let connection = Connection::open(database).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT value FROM observation_meta WHERE key='schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "3",
    );
    assert_eq!(
        connection
            .query_row("SELECT content_id FROM content_streams", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
        "legacy-content",
    );
}

#[test]
fn envelope_loss_reason_is_preserved_in_the_query_projection() {
    let fixture = Fixture::new();
    let mut begin = fixture.begin(2);
    begin.loss_watermark = Some(hiroute_domain::ContentLossWatermarkV2 {
        first_sequence: 1,
        last_sequence: 1,
        reason: "queue_events_exceeded".into(),
    });
    begin.completeness_delta = Some(ContentCompletenessDeltaV2::Partial);
    let outcome = ack(fixture.ingest(&begin));
    assert_eq!(outcome.highest_contiguous_sequence, 0);
    assert_eq!(outcome.highest_accounted_sequence, 2);

    let status = fixture.store.get_status(&fixture.workspace).unwrap();
    assert_eq!(status.gaps.len(), 1);
    assert_eq!(status.gaps[0].first_sequence, 1);
    assert_eq!(status.gaps[0].last_sequence, 1);
    assert_eq!(
        status.gaps[0].reason.as_deref(),
        Some("queue_events_exceeded")
    );

    let fixture = Fixture::new();
    assert!(matches!(
        fixture.ingest(&fixture.begin(1)),
        IngestOutcome::Ack(_)
    ));
    let mut finish = fixture.finish(3, "ab");
    finish.loss_watermark = Some(hiroute_domain::ContentLossWatermarkV2 {
        first_sequence: 2,
        last_sequence: 2,
        reason: "queue_bytes_exceeded".into(),
    });
    finish.completeness_delta = Some(ContentCompletenessDeltaV2::Partial);
    let finish_ack = ack(fixture.ingest(&finish));
    assert_eq!(finish_ack.highest_contiguous_sequence, 1);
    assert_eq!(finish_ack.highest_accounted_sequence, 3);
    assert_eq!(
        fixture
            .store
            .get_session(
                &fixture.workspace,
                &SessionId::parse("conversation-1").unwrap(),
                hiroute_domain::ContentMode::None,
            )
            .unwrap()
            .summary
            .content_completeness,
        ContentCompleteness::Partial
    );
}

#[test]
fn typed_nacks_are_coordinate_complete_and_durable() {
    let fixture = Fixture::new();
    let missing_sequence = fixture.append(2, 0, "message-1", "content-1", 0, 0, b"x", b"x");
    let missing = nack(fixture.ingest(&missing_sequence));
    assert!(
        matches!(missing.detail, ObservationNackDetailV1::MissingSequenceRanges { ref ranges }
        if ranges[0].first_sequence == 1 && ranges[0].last_sequence == 1)
    );
    assert!(missing.retryable);
    assert_eq!(missing, nack(fixture.ingest(&missing_sequence)));

    let mut unknown = fixture.begin(1);
    unknown.event_id = EventId::parse("unknown-root").unwrap();
    unknown.parent_transcript_root =
        Some(TranscriptRoot::parse(format!("transcript-{}", "22".repeat(32))).unwrap());
    let rejected = nack(fixture.ingest(&unknown));
    assert!(
        matches!(rejected.detail, ObservationNackDetailV1::UnknownTranscriptRoot {
        ref request_id, direction: ConversationContentDirectionV2::RequestInput,
        ref fork_id, ref transcript_root
    } if request_id == "request-1" && fork_id == "fork-request" && transcript_root.ends_with(&"22".repeat(32)))
    );
    assert_eq!(rejected, nack(fixture.ingest(&unknown)));
}

#[test]
fn chunk_digest_missing_blob_state_and_sequence_conflicts_fail_closed() {
    let fixture = Fixture::new();
    assert!(matches!(
        fixture.ingest(&fixture.begin(1)),
        IngestOutcome::Ack(_)
    ));

    let wrong_ordinal = fixture.append(2, 1, "message-1", "content-1", 0, 0, b"x", b"x");
    assert!(matches!(
        nack(fixture.ingest(&wrong_ordinal)).detail,
        ObservationNackDetailV1::ChunkOrdinalConflict {
            expected_chunk_ordinal: 0,
            rejected_chunk_ordinal: 1,
            ..
        }
    ));

    let mut wrong_digest = fixture.append(2, 0, "message-1", "content-1", 0, 0, b"x", b"x");
    wrong_digest.event_id = EventId::parse("wrong-digest").unwrap();
    wrong_digest.content_blob_digest = Some(
        hiroute_domain::ContentBlobDigest::parse(format!("blob-{}", "33".repeat(32))).unwrap(),
    );
    wrong_digest.content_ref.as_mut().unwrap().digest =
        wrong_digest.content_blob_digest.clone().unwrap();
    assert!(matches!(
        nack(fixture.ingest(&wrong_digest)).detail,
        ObservationNackDetailV1::DigestMismatch {
            subject: hiroute_domain::ObservationDigestSubjectV1::ContentBlob,
            ..
        }
    ));

    let fixture = Fixture::new();
    fixture.ingest(&fixture.begin(1));
    let mut partial = fixture.append(2, 0, "message-1", "content-1", 0, 0, b"two", b"t");
    partial.content_ref.as_mut().unwrap().byte_count = 3;
    fixture.ingest(&partial);
    assert!(
        matches!(nack(fixture.ingest(&fixture.finish(3, "44"))).detail,
        ObservationNackDetailV1::MissingBlob { ref blobs, .. } if blobs.len() == 1)
    );
}

#[test]
fn gap_heartbeat_is_a_durable_stream_entry_and_does_not_invent_correlation() {
    let fixture = Fixture::new();
    let heartbeat = ObservationGapHeartbeatV1 {
        schema_version: OBSERVATION_GAP_HEARTBEAT_SCHEMA_V1.into(),
        channel: "conversation_content".into(),
        producer: ObservationProducerV2 {
            component: "gateway-content".into(),
            revision: "g-star".into(),
            stream: fixture.stream.clone(),
        },
        sequence: 2,
        event_id: EventId::parse("gap:stream-1:1:2:queue_bytes_exceeded").unwrap(),
        loss_watermark: hiroute_domain::ContentLossWatermarkV2 {
            first_sequence: 1,
            last_sequence: 2,
            reason: "queue_bytes_exceeded".into(),
        },
        completeness_delta: ContentCompletenessDeltaV2::Partial,
    };
    let first = fixture.store.ingest_gap_heartbeat(&heartbeat).unwrap();
    let heartbeat_ack = ack(first.clone());
    assert_eq!(heartbeat_ack.highest_contiguous_sequence, 0);
    assert_eq!(heartbeat_ack.highest_accounted_sequence, 2);
    assert!(heartbeat_ack.content_acknowledgement.is_none());
    assert_eq!(
        first,
        fixture.store.ingest_gap_heartbeat(&heartbeat).unwrap()
    );
    let reopened =
        LocalObservationStore::open(fixture._root.path(), fixture.authority.clone()).unwrap();
    assert_eq!(first, reopened.ingest_gap_heartbeat(&heartbeat).unwrap());
    drop(reopened);
    let connection = fixture.store.connection.lock();
    let scope: (String, Option<String>, Option<String>) = connection
        .query_row(
            "SELECT workspace_id, session_id, reason FROM observation_gaps WHERE first_sequence=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        scope,
        (String::new(), None, Some("queue_bytes_exceeded".into()))
    );
    drop(connection);

    let mut conflicting = heartbeat.clone();
    conflicting.event_id = EventId::parse("different-gap-event").unwrap();
    assert!(matches!(
        nack(fixture.store.ingest_gap_heartbeat(&conflicting).unwrap()).detail,
        ObservationNackDetailV1::SequenceEventConflict {
            sequence: 2,
            ref expected_event_id,
            ref rejected_event_id,
        } if expected_event_id == "gap:stream-1:1:2:queue_bytes_exceeded"
            && rejected_event_id == "different-gap-event"
    ));

    let content_ack = ack(fixture.ingest(&fixture.begin(3)));
    assert_eq!(content_ack.highest_contiguous_sequence, 0);
    assert_eq!(content_ack.highest_accounted_sequence, 3);
    let connection = fixture.store.connection.lock();
    let durable_gap_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM observation_gaps
             WHERE first_sequence=1 AND last_sequence=2 AND reason='queue_bytes_exceeded'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(durable_gap_count, 1);

    let unsupported_fixture = Fixture::new();
    let mut unsupported = heartbeat;
    unsupported.schema_version = "hiroute.observation.gap-heartbeat/v2".into();
    assert!(matches!(
        nack(unsupported_fixture
            .store
            .ingest_gap_heartbeat(&unsupported)
            .unwrap())
        .detail,
        ObservationNackDetailV1::UnsupportedSchema {
            ref rejected_schema_version,
            ref supported_schema_versions,
        } if rejected_schema_version == "hiroute.observation.gap-heartbeat/v2"
            && supported_schema_versions == &[OBSERVATION_GAP_HEARTBEAT_SCHEMA_V1]
    ));
}

#[test]
fn many_chunks_install_without_a_whole_stream_buffer() {
    let fixture = Fixture::new();
    fixture.ingest(&fixture.begin(1));
    let whole = vec![b'x'; 512 * 1024];
    for (index, chunk) in whole.chunks(8 * 1024).enumerate() {
        let value = fixture.append(
            index as u64 + 2,
            index as u32,
            "message-large",
            "content-large",
            0,
            0,
            &whole,
            chunk,
        );
        assert!(matches!(fixture.ingest(&value), IngestOutcome::Ack(_)));
    }
    let finish_sequence = whole.chunks(8 * 1024).count() as u64 + 2;
    let finish = fixture.finish(finish_sequence, "55");
    let acknowledgement = ack(fixture.ingest(&finish));
    assert!(acknowledgement.content_acknowledgement.is_none());
    let connection = fixture.store.connection.lock();
    let byte_count: i64 = connection
        .query_row("SELECT byte_count FROM content_blobs_v2", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(byte_count, whole.len() as i64);
}

fn ack(outcome: IngestOutcome) -> hiroute_domain::ObservationAckV2 {
    match outcome {
        IngestOutcome::Ack(value) => value,
        other => panic!("expected ACK, got {other:?}"),
    }
}

fn nack(outcome: IngestOutcome) -> hiroute_domain::ObservationNackV1 {
    match outcome {
        IngestOutcome::Nack(value) => value,
        other => panic!("expected NACK, got {other:?}"),
    }
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(ALPHABET[(first >> 2) as usize] as char);
        output.push(ALPHABET[(((first & 3) << 4) | (second >> 4)) as usize] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[(((second & 15) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[(third & 63) as usize] as char
        } else {
            '='
        });
    }
    output
}
