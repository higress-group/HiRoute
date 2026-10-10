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
    assert_eq!(serde_json::to_string(&current).unwrap(), released);
}
