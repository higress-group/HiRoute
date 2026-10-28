use hiroute_diagnostics::{event::DiagnosticEvent, level::DiagnosticLevel};

#[test]
fn info_summary_is_closed_bounded_and_old_request_events_still_read() {
    let old = r#"{"request_end":{"outcome":"completed","elapsed_ms":42}}"#;
    assert!(serde_json::from_str::<DiagnosticEvent>(old).is_ok());
    let summary = serde_json::json!({"request_timing_summary": {
        "request_token": null, "protocol": "open_ai_responses", "outcome": "completed",
        "http_status": 200, "elapsed_us": 42,
        "local": {"ingress_us": 0, "replay_us": 0, "parse_us": 0, "externalize_us": 0, "prevalidate_us": 0, "history_us": 0,
            "decision_us": 0, "plan_us": 0, "prepare_us": 0, "stream_us": 0, "other_us": 0},
        "wait": {"client_read_us": 0, "decision_us": 0, "connect_us": 0, "upstream_write_us": 0,
            "upstream_read_us": 42, "downstream_write_us": 0, "local_work_us": 0, "other_us": 0},
        "asynchronous": {"records": 0, "delivered": 0, "dropped": 0, "queue_wait_max_us": 0,
            "sink_work_us": 0, "after_response_us": 0}
    }});
    let event: DiagnosticEvent = serde_json::from_value(summary.clone()).unwrap();
    assert_eq!(event.level(), DiagnosticLevel::Info);
    assert_eq!(event.kind(), "request_timing_summary");
    assert!(serde_json::to_vec(&event).unwrap().len() < 2_048);
    for path in ["payload", "headers", "prompt"] {
        let mut invalid = summary.clone();
        invalid["request_timing_summary"]["local"][path] = "private".into();
        assert!(serde_json::from_value::<DiagnosticEvent>(invalid).is_err());
    }
}
