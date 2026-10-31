use hiroute_diagnostics::event::{DiagnosticEvent, ExecutionFailureReason};
use hiroute_diagnostics::level::DiagnosticLevel;

#[test]
fn pre_attempt_failure_records_roundtrip_and_reject_unstructured_data() {
    let encoded = r#"{"request_execution_failure":{"request_token":null,"source":"provider","reason":"memory_budget_exceeded","request":{"raw_body_bytes":2000000,"canonical_messages":724,"tools":21,"replay_disk_backed":true},"attempt_started":false,"response_started":false,"last_budget_rejection":{"role":"attempt_wire","requested_bytes":4096,"stream_live_bytes":8192,"stream_limit_bytes":10000,"worker_live_bytes":8192,"worker_limit_bytes":20000,"process_live_bytes":8192,"process_limit_bytes":40000}}}"#;
    let event: DiagnosticEvent = serde_json::from_str(encoded).unwrap();
    assert_eq!(event.level(), DiagnosticLevel::Warn);
    let DiagnosticEvent::RequestExecutionFailure(ref failure) = event else {
        panic!("execution failure")
    };
    assert_eq!(failure.reason, ExecutionFailureReason::MemoryBudgetExceeded);
    assert!(!failure.attempt_started && !failure.response_started);
    assert_eq!(serde_json::to_string(&event).unwrap(), encoded);
    let invalid = encoded.replace(
        "\"response_started\":false",
        "\"response_started\":false,\"raw_error\":\"private credential\"",
    );
    assert!(serde_json::from_str::<DiagnosticEvent>(&invalid).is_err());
    assert!(
        serde_json::from_str::<DiagnosticEvent>(
            &encoded.replace("memory_budget_exceeded", "private_error")
        )
        .is_err()
    );
}
