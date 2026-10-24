//! The listener must report actual projection consumption while stripping private hints.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn production_native_evidence_distinguishes_header_adoption_from_window_use() {
    for case in [
        "native_evidence_applied",
        "native_evidence_rejected",
        "native_evidence_disabled",
    ] {
        run_child_case(case).unwrap();
    }
}

pub(super) fn headers(case: &str) -> String {
    if !case.starts_with("native_evidence_") {
        return String::new();
    }
    let message = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&["SENTINEL_REQUEST_BODY"]).unwrap())
    );
    let scope = if case == "native_evidence_rejected" {
        "0".repeat(64)
    } else {
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&[&message]).unwrap())
        )
    };
    let context = json!({"version":2,"agent":"opencode","purpose":"main",
        "session_id":"SENTINEL_NATIVE_SESSION","turn_id":"SENTINEL_NATIVE_TURN",
        "input":{"kind":"native_messages","event_id":"SENTINEL_NATIVE_EVENT",
        "coverage":"window","messages":[{"index":0,"sha256":message}],"scope_digest":scope}});
    format!(
        "X-HiRoute-Agent: opencode\r\nX-OpenCode-Session-ID: SENTINEL_NATIVE_SESSION\r\nX-HiRoute-Routing-Context: {context}\r\n"
    )
}

pub(super) fn assert_evidence(case: &str, records: &[Value], upstream: &[u8], response: &[u8]) {
    if !case.starts_with("native_evidence_") {
        return;
    }
    let request = String::from_utf8_lossy(upstream).to_ascii_lowercase();
    assert!(!request.contains("x-hiroute-routing-context"));
    assert!(!request.contains("sentinel_native_"));
    let response = String::from_utf8_lossy(response);
    assert!(response.contains("SENTINEL_PROVIDER_TEXT"));
    assert!(response.contains("event: message_stop"));
    let events: Vec<_> = records
        .iter()
        .filter_map(|r| payload(r, "native_routing_evidence"))
        .collect();
    if case == "native_evidence_disabled" {
        assert!(events.is_empty());
        return;
    }
    assert_eq!(events.len(), 1);
    let event = events[0];
    assert_eq!(event["header"], "present");
    assert_eq!(event["adapter"]["disposition"], "applied");
    assert_eq!(event["adapter"]["agent"], "open_code");
    assert_eq!(event["turn_applied"], true);
    assert_eq!(event["track_history"], true);
    assert_eq!(event["origin"], "observed");
    assert_eq!(
        event["projection"],
        if case == "native_evidence_applied" {
            "window_applied"
        } else {
            "window_rejected"
        }
    );
    assert!(!event["context_token"].is_null());
    let end = records
        .iter()
        .find_map(|r| payload(r, "request_end"))
        .unwrap();
    assert!(!event["request_token"].is_null());
    assert_eq!(event["request_token"], end["request_token"]);
}
