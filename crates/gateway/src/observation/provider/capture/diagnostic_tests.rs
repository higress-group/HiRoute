//! Synthetic strict-decoder failures through the real off-path capture worker.
//! These fixtures diagnose rejection; they do not relax or certify the input.
use std::time::{Duration, Instant};

use hiroute_diagnostics::{
    DiagnosticLevel,
    event::ProcessRole,
    record::Component,
    runtime::{DiagnosticRuntime, RuntimeConfig},
};
use serde_json::{Value, json};

use super::super::{CanonicalCaptureProducer, CaptureJob, reserve};
use super::*;
use crate::server::core_runtime::observation::{
    GatewayObservation, GatewayObservationSinks, OtelContentPolicy, RequestObservation,
    RequestObservationMetadata, pricing, request::RequestObservationCapture,
};
use crate::server::core_runtime::profiles::fixed_reasoning;

fn request(runtime: &DiagnosticRuntime) -> RequestObservation {
    let gateway = GatewayObservation::with_sinks_and_policy(
        true,
        64 * 1024,
        GatewayObservationSinks::discard(),
        OtelContentPolicy::Disabled,
    );
    RequestObservation::new(
        RequestObservationCapture {
            enabled: true,
            content: true,
        },
        gateway.key,
        gateway.channels.clone(),
        OtelContentPolicy::Disabled,
        pricing::capture(None, "workspace:test", &[], 0),
        RequestObservationMetadata {
            workspace_id: "workspace:test".into(),
            conversation_id: "conversation:test".into(),
            session_scope: "request_scoped".into(),
            correlation_provenance: "unproven".into(),
            turn_id: "turn:test".into(),
            request_id: "request:test".into(),
            authority_id: "authority:test".into(),
            authority_epoch: 1,
            publication_revision: 1,
            publication_digest: "sha256:publication".into(),
            agent_plan_id: Some("plan/test".into()),
            route: hiroute_domain::ModelRequestRouteV2::Plan {
                revision: 1,
                semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(b"plan"),
            },
            plan_display_name: None,
            served_model_id: "alias".into(),
            grant_id: "grant:test".into(),
            grant_generation: 1,
            ingress_protocol: "responses".into(),
        },
        runtime.port(),
    )
}

fn run_failure(events: Vec<Value>) -> (Value, String) {
    let root = std::env::temp_dir().join(format!(
        "hiroute-capture-diagnostic-{}-{}",
        std::process::id(),
        crate::server::core_runtime::observation::unix_nanos()
    ));
    let runtime = DiagnosticRuntime::start(RuntimeConfig {
        root: root.clone(),
        role: ProcessRole::Daemon,
        component: Component::Gateway,
        parent_session_id: None,
        level_override: Some(DiagnosticLevel::Info),
    });
    let request = request(&runtime);
    request.lock_state().next_attempt_ordinal = 3;
    let producer = CanonicalCaptureProducer::new(true, 4 * 1024 * 1024);
    let profile = Arc::new(CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    ));
    let handle = producer
        .begin(
            request.clone(),
            profile,
            ToolIdProjection::new(IngressProtocol::Responses),
            None,
            Default::default(),
            true,
        )
        .unwrap();
    // A later request-local attempt must not capture this worker's diagnostic.
    request.lock_state().next_attempt_ordinal = 9;
    let bytes = events
        .into_iter()
        .flat_map(|event| format!("data: {event}\n\n").into_bytes())
        .collect::<Vec<_>>();
    for event in [
        CapturedNativeEvent::Head(200),
        CapturedNativeEvent::Body(bytes),
        CapturedNativeEvent::EndStream,
    ] {
        handle
            .try_send(CaptureJob::Native {
                id: handle.id,
                event,
                reservation: reserve(&handle.shared.budget, 32 * 1024).unwrap(),
            })
            .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while !handle
        .status
        .finished
        .load(std::sync::atomic::Ordering::Acquire)
    {
        assert!(Instant::now() < deadline, "capture worker did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        handle.failure_reason(),
        Some("canonical_response_decode_failed")
    );
    drop(handle);
    drop(producer);
    runtime.shutdown();
    let log = std::fs::read_to_string(root.join("daemon/current.jsonl")).unwrap();
    let failures = log
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|row| row["event"].get("response_failure").cloned())
        .collect::<Vec<_>>();
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(
        failures.len(),
        1,
        "first capture error requires exactly one structured diagnostic"
    );
    (failures.into_iter().next().unwrap(), log)
}

#[test]
fn capture_first_decode_failure_reports_safe_shape_and_frozen_attempt_without_active_request() {
    let (failure, log) = run_failure(vec![
        json!({"type":"response.created","response":{"id":"PRIVATE_ID","model":"physical"}}),
        json!({"type":"response.reasoning_summary_text.delta","output_index":0,
            "item_id":"PRIVATE_ID","summary_index":1,"delta":"PRIVATE_BODY_TOKEN"}),
        json!({"type":"PRIVATE_UNKNOWN_EVENT","secret":"PRIVATE_LATER_BODY"}),
    ]);
    assert_eq!(failure["attempt_index"], 3);
    assert_eq!(failure["stage"], "decode");
    assert_eq!(failure["reason"], "invalid_field");
    assert_eq!(failure["frame_index"], 2);
    assert!(failure["received_bytes"].as_u64().unwrap() > 0);
    assert_eq!(failure["capture"]["ingress_protocol"], "open_ai_responses");
    assert_eq!(failure["capture"]["upstream_protocol"], "open_ai_responses");
    assert_eq!(failure["capture"]["last_event"]["event"], "reasoning_delta");
    assert_eq!(failure["capture"]["last_event"]["summary_index"], 1);
    for private in [
        "PRIVATE_ID",
        "PRIVATE_BODY_TOKEN",
        "PRIVATE_UNKNOWN_EVENT",
        "PRIVATE_LATER_BODY",
    ] {
        assert!(
            !log.contains(private),
            "private data reached diagnostic log"
        );
    }
}

#[test]
fn capture_multi_summary_rejection_reports_counts_without_summary_text_or_encrypted_state() {
    let (failure, log) = run_failure(vec![
        json!({"type":"response.created","response":{"id":"PRIVATE_ID","model":"physical"}}),
        json!({"type":"response.output_item.done","output_index":0,"item":{
            "type":"reasoning","id":"PRIVATE_REASONING_ID","status":"completed",
            "encrypted_content":"PRIVATE_ENCRYPTED_STATE",
            "summary":[{"type":"summary_text","text":"PRIVATE_FIRST"},
                {"type":"summary_text","text":"PRIVATE_SECOND"}]}}),
    ]);
    assert_eq!(failure["reason"], "unsupported_field");
    assert_eq!(failure["frame_index"], 2);
    assert_eq!(
        failure["capture"]["last_event"]["event"],
        "output_item_done"
    );
    assert_eq!(failure["capture"]["last_event"]["item"], "reasoning");
    assert_eq!(failure["capture"]["last_event"]["summary_parts"], 2);
    for private in [
        "PRIVATE_ID",
        "PRIVATE_REASONING_ID",
        "PRIVATE_FIRST",
        "PRIVATE_SECOND",
        "PRIVATE_ENCRYPTED_STATE",
    ] {
        assert!(!log.contains(private));
    }
}

#[test]
fn capture_unknown_event_and_dynamic_error_never_serialize_their_values() {
    let (failure, log) = run_failure(vec![
        json!({"type":"response.created","response":{"id":"PRIVATE_ID","model":"physical"}}),
        json!({"type":"PRIVATE_UNKNOWN_EVENT","authorization":"PRIVATE_CREDENTIAL",
            "path":"/private/PRIVATE_PATH","content":"PRIVATE_TEXT"}),
    ]);
    assert_eq!(failure["reason"], "unsupported_value");
    assert_eq!(failure["capture"]["last_event"]["event"], "unknown");
    for private in [
        "PRIVATE_UNKNOWN_EVENT",
        "PRIVATE_ID",
        "PRIVATE_CREDENTIAL",
        "PRIVATE_PATH",
        "PRIVATE_TEXT",
    ] {
        assert!(!log.contains(private));
    }
}
