//! Independent content may reach SQLite after the execution terminal.
use crate::WriterCycleOutcome;
use hiroute_domain::{
    AttemptId, CANONICAL_RESPONSE_CONTENT_MEDIA_TYPE_V1, ContentCompleteness,
    ContentDownstreamDeliveryV2, ContentId, ContentMode, ContentRefV2,
    ConversationContentDirectionV2, EventId, MessageInstanceId, ObservationQueryPort, ReceiptId,
    TranscriptRoot,
};

use super::support::*;

#[test]
fn delayed_content_after_request_finished_preserves_receipt_and_reopens_exactly() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new("late-content");
    let store = open_store(root.path());
    install_value_request(
        &store,
        &fixture,
        fixture.request.as_str(),
        "accepted-attempt",
        finished("accepted-attempt"),
        100,
    );
    let receipt_id = ReceiptId::parse(fixture.request.as_str()).unwrap();
    let receipt = store.get_receipt(&fixture.workspace, &receipt_id).unwrap();
    assert_eq!(receipt.ordered_facts.len(), 10);
    // Capture timestamps describe the request, not the later ingestion time.
    // A finished request is not a tombstone for its independent content lane.
    let text = format!("late capture: {}", "escape \\\"\n路由🦀".repeat(2048));
    install_content(&store, &fixture, text.as_bytes(), 101);
    let partial = store
        .get_session(
            &fixture.workspace,
            &fixture.session,
            ContentMode::MessagesAndTools,
        )
        .unwrap();
    assert_eq!(
        partial.summary.content_completeness,
        ContentCompleteness::Partial
    );
    let parent = partial.turns[0].messages[0]
        .transcript_root
        .clone()
        .unwrap();
    let response_bytes = br#"{"event":{"kind":"text_delta","delta":"42"}}"#;
    let mut response = fixture.request_content_events(response_bytes, 104);
    for event in &mut response {
        event.sequence += 3;
        event.event_id = EventId::parse(format!("late-response-{}", event.sequence)).unwrap();
        event.direction = ConversationContentDirectionV2::ResponseDelivered;
        event.attempt_id = Some(AttemptId::parse("accepted-attempt").unwrap());
        event.fork_id = "response-fork".into();
        event.parent_transcript_root = Some(parent.clone());
        if event.message_instance_id.is_some() {
            event.message_instance_id = Some(MessageInstanceId::parse("response-message").unwrap());
            event.content_id = Some(ContentId::parse("response-content").unwrap());
            event.message_role = Some("assistant".into());
            event.content_kind = Some("text_delta".into());
            event.canonical_media_type = Some(CANONICAL_RESPONSE_CONTENT_MEDIA_TYPE_V1.into());
            event.content_blob_digest = Some(
                authority()
                    .content_blob_digest(CANONICAL_RESPONSE_CONTENT_MEDIA_TYPE_V1, response_bytes),
            );
            event.content_ref = Some(ContentRefV2 {
                content_id: event.content_id.clone().unwrap(),
                digest: event.content_blob_digest.clone().unwrap(),
                byte_count: response_bytes.len() as u64,
                media_type: CANONICAL_RESPONSE_CONTENT_MEDIA_TYPE_V1.into(),
            });
            event.transport_frame_id = Some("accepted-frame".into());
            event.downstream_delivery =
                Some(ContentDownstreamDeliveryV2::FullFrameTransportAccepted);
        }
        if event.result_transcript_root.is_some() {
            event.result_transcript_root =
                Some(TranscriptRoot::parse(format!("transcript-{}", "42".repeat(32))).unwrap());
            event.downstream_delivery =
                Some(ContentDownstreamDeliveryV2::FullFrameTransportAccepted);
        }
    }
    let persist_response = |store: &std::sync::Arc<crate::LocalObservationStore>| {
        let writer = writer(store);
        let channel = content_channel(&fixture, 128 * 1024);
        for event in response.clone() {
            let outcome = offer_content(&writer, &channel, event);
            assert!(matches!(outcome, WriterCycleOutcome::Ack(_)), "{outcome:?}");
        }
    };
    persist_response(&store);
    let detail = store
        .get_session(
            &fixture.workspace,
            &fixture.session,
            ContentMode::MessagesAndTools,
        )
        .unwrap();
    assert_eq!(
        detail.summary.content_completeness,
        ContentCompleteness::Complete
    );
    assert_eq!(detail.turns[0].messages[0].bytes, text.as_bytes());
    assert_eq!(detail.turns[0].messages[1].bytes, response_bytes);
    assert_eq!(
        store.get_receipt(&fixture.workspace, &receipt_id).unwrap(),
        receipt
    );
    drop(store);

    let reopened = open_store(root.path());
    assert_eq!(
        reopened
            .get_session(
                &fixture.workspace,
                &fixture.session,
                ContentMode::MessagesAndTools
            )
            .unwrap(),
        detail
    );
    assert_eq!(
        reopened
            .get_receipt(&fixture.workspace, &receipt_id)
            .unwrap(),
        receipt
    );
    // An idempotent replay after reopen retains the existing ACK/duplicate path.
    install_content(&reopened, &fixture, text.as_bytes(), 101);
    persist_response(&reopened);
    assert_eq!(
        reopened
            .get_session(
                &fixture.workspace,
                &fixture.session,
                ContentMode::MessagesAndTools
            )
            .unwrap(),
        detail
    );
}
