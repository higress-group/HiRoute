use hiroute_diagnostics::event::{ResponseFailure, ResponseFailureReason, ResponseFailureStage};

#[test]
fn supported_release_response_failure_retains_meaning_and_defaults_extensions() {
    // Frozen from the serializer at supported release d625e653048b2306c88da07826d52027e9891675:
    // crates/diagnostics/src/event/model.rs::ResponseFailure. Registered in compatibility support.
    let released =
        r#"{"request_token":null,"attempt_index":3,"stage":"decode","reason":"other_protocol"}"#;
    let current: ResponseFailure = serde_json::from_str(released).unwrap();
    assert_eq!(current.attempt_index, 3);
    assert_eq!(current.stage, ResponseFailureStage::Decode);
    assert_eq!(current.reason, ResponseFailureReason::OtherProtocol);
    assert!(current.request_token.is_none());
    assert!(current.attempt_token.is_none());
    assert!(current.field.is_none());
    assert!(current.frame_index.is_none());
    assert!(current.received_bytes.is_none());
    assert!(current.capture.is_none());
    assert_eq!(serde_json::to_string(&current).unwrap(), released);
}

#[test]
fn current_capture_failure_roundtrips_only_closed_shape_fields() {
    let current = r#"{"request_token":null,"attempt_index":2,"stage":"decode","reason":"invalid_field","frame_index":7,"received_bytes":1024,"capture":{"ingress_protocol":"open_ai_responses","upstream_protocol":"open_ai_responses","last_event":{"event":"reasoning_delta","summary_index":1}}}"#;
    let parsed: ResponseFailure = serde_json::from_str(current).unwrap();
    assert_eq!(serde_json::to_string(&parsed).unwrap(), current);
    for invalid in [
        current.replace("reasoning_delta", "PRIVATE_UNKNOWN_EVENT"),
        current.replace(
            "\"summary_index\":1",
            "\"summary_index\":1,\"text\":\"PRIVATE_BODY\"",
        ),
    ] {
        assert!(serde_json::from_str::<ResponseFailure>(&invalid).is_err());
    }
}
