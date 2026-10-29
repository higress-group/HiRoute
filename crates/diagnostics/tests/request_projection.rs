use hiroute_diagnostics::event::{
    DiagnosticEvent, RequestProjection, RequestProjectionOutcome, RequestProjectionReason,
    RequestProjectionStage,
};
use hiroute_diagnostics::level::DiagnosticLevel;

#[test]
fn request_projection_events_roundtrip_only_closed_payload_free_facts() {
    let encoded = r#"{"request_projection":{"request_token":null,"binding_token":null,"ingress_protocol":"open_ai_responses","upstream_protocol":"open_ai_chat","stage":"attempt_materialization","outcome":"declared_loss","reason":"message_phase_omitted","first_message_index":3,"affected_items":2}}"#;
    let event: DiagnosticEvent = serde_json::from_str(encoded).unwrap();
    assert_eq!(event.level(), DiagnosticLevel::Warn);
    let DiagnosticEvent::RequestProjection(projection) = &event else {
        panic!("projection event")
    };
    assert_eq!(
        projection.stage,
        RequestProjectionStage::AttemptMaterialization
    );
    assert_eq!(projection.outcome, RequestProjectionOutcome::DeclaredLoss);
    assert_eq!(
        projection.reason,
        RequestProjectionReason::MessagePhaseOmitted
    );
    assert_eq!(projection.first_message_index, Some(3));
    assert_eq!(projection.affected_items, Some(2));
    assert_eq!(serde_json::to_string(&event).unwrap(), encoded);
    let record = serde_json::json!({
        "schema":"hiroute.diagnostic-event/v1","timestamp_ms":1,"monotonic_ms":1,
        "component":"gateway","boot_id":"00000000000000000000000000000000",
        "sequence":1,"level":"warn","level_revision":0,"event":event,
    });
    let record =
        hiroute_diagnostics::DiagnosticRecordV1::parse_line(&serde_json::to_vec(&record).unwrap())
            .unwrap();
    assert_eq!(
        hiroute_diagnostics::DiagnosticRecordV1::parse_line(&record.encode_jsonl().unwrap())
            .unwrap(),
        record
    );
    for invalid in [
        encoded.replace("message_phase_omitted", "PRIVATE_UNKNOWN_REASON"),
        encoded.replace(
            "\"affected_items\":2",
            "\"affected_items\":2,\"body\":\"PRIVATE_TEXT\"",
        ),
    ] {
        assert!(serde_json::from_str::<DiagnosticEvent>(&invalid).is_err());
    }
    let unknown_count = RequestProjection {
        affected_items: None,
        ..projection.clone()
    };
    let encoded = serde_json::to_string(&unknown_count).unwrap();
    assert_eq!(
        serde_json::from_str::<RequestProjection>(&encoded).unwrap(),
        unknown_count
    );
}
