use hiroute_diagnostics::{event::DiagnosticEvent, level::DiagnosticLevel};

#[test]
fn product_read_outcomes_preserve_old_control_events_and_reject_payloads() {
    let old = r#"{"control_call_end":{"operation":"get_client_service_status","submission":"sent","elapsed_ms":4}}"#;
    assert_eq!(
        serde_json::to_string(&serde_json::from_str::<DiagnosticEvent>(old).unwrap()).unwrap(),
        old
    );
    for code in [
        "resident_unavailable",
        "client_context_invalid",
        "read_snapshot_changed",
        "confirmation_stale",
        "response_data_missing",
    ] {
        let encoded = format!(
            r#"{{"product_read_end":{{"operation":"desktop_snapshot","error":"{code}","elapsed_ms":5}}}}"#
        );
        let event: DiagnosticEvent = serde_json::from_str(&encoded).unwrap();
        assert_eq!(event.level(), DiagnosticLevel::Warn);
        assert_eq!(event.kind(), "product_read_end");
        assert_eq!(serde_json::to_string(&event).unwrap(), encoded);
    }
    let success = r#"{"product_read_end":{"operation":"observation_read","elapsed_ms":5}}"#;
    assert_eq!(
        serde_json::from_str::<DiagnosticEvent>(success)
            .unwrap()
            .level(),
        DiagnosticLevel::Debug
    );
    for invalid in [
        success.replace("observation_read", "PRIVATE_OPERATION"),
        success.replace(
            "\"elapsed_ms\":5",
            "\"elapsed_ms\":5,\"error\":\"PRIVATE_FAILURE\"",
        ),
        success.replace(
            "\"elapsed_ms\":5",
            "\"elapsed_ms\":5,\"body\":\"PRIVATE_CONTENT\"",
        ),
    ] {
        assert!(serde_json::from_str::<DiagnosticEvent>(&invalid).is_err());
    }
}
