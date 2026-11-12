use super::*;
use serde_json::{Value, json};

const ROLE_REJECTION: &[u8] = br#"{"type":"error","error":{"type":"invalid_request_error","message":"messages.1: Unexpected role \"system\". PRIVATE_PROVIDER_DETAIL"},"request_id":"private-provider-request"}"#;
const MESSAGES_OK: &[u8] = br#"{"id":"messages-ok","type":"message","role":"assistant","model":"runtime-native","content":[{"type":"text","text":"recovered"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}"#;

fn candidate(provider_index: usize) -> PublicationCandidate {
    PublicationCandidate {
        provider_index,
        upstream_protocol: "messages",
        statically_enabled: true,
    }
}

fn complete(status: u16, body: &'static [u8]) -> ProviderReply {
    ProviderReply::Complete {
        status,
        error_kind: None,
        body,
    }
}

fn messages_request(fixture: &RuntimeFixture, inline_system: bool) -> WireResponse {
    let reminder = "synthetic reminder";
    let messages = if inline_system {
        json!([
            {"role":"user","content":"synthetic input"},
            {"role":"system","content":reminder}
        ])
    } else {
        json!([{"role":"user","content":format!("synthetic input\n{reminder}")}])
    };
    let body = json!({
        "model": MODEL,
        "system": "stable instructions",
        "messages": messages,
        "max_tokens": 32,
        "stream": false
    });
    request(
        fixture.address,
        "POST",
        "/v1/messages",
        &[("X-HiRoute-Token", MESSAGES_TOKEN)],
        &serde_json::to_vec(&body).unwrap(),
    )
}

fn assert_generic_error(response: &WireResponse) {
    assert_eq!(
        response.status,
        400,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&response.body).unwrap(),
        json!({"error":{"type":"upstream_error","code":"UPSTREAM_ATTEMPT_FAILED"}})
    );
}

#[test]
fn legacy_messages_role_rejection_preserves_the_client_retry() {
    let provider = NativeProvider::start(vec![
        complete(400, ROLE_REJECTION),
        complete(200, MESSAGES_OK),
    ]);
    let fallback = NativeProvider::start(vec![complete(200, MESSAGES_OK)]);
    // The next candidate is eligible, but the one-attempt limit is exhausted.
    // The native error has to survive the decision session's exhaustion path.
    let fixture = RuntimeFixture::launch_with_observation_and_publication_candidates(
        &[&provider, &fallback],
        1,
        ObservationFaults::healthy(),
        Some(&[candidate(0), candidate(1)]),
    );
    let rejected = messages_request(&fixture, true);
    assert_eq!(
        rejected.status,
        400,
        "{}",
        String::from_utf8_lossy(&rejected.body)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&rejected.body).unwrap(),
        json!({"type":"error","error":{
            "type":"invalid_request_error",
            "message":"Unexpected role \"system\". Use a supported input message role."
        }})
    );
    assert_eq!(provider.calls(), 1);

    let retried = messages_request(&fixture, false);
    assert_eq!(retried.status, 200);
    assert!(String::from_utf8_lossy(&retried.body).contains("recovered"));
    assert_eq!(fallback.calls(), 0);
    let attempts = provider.requests();
    assert_eq!(attempts.len(), 2);
    let outbound = |bytes: &[u8]| -> Value {
        let body_start = bytes.windows(4).position(|s| s == b"\r\n\r\n").unwrap() + 4;
        serde_json::from_slice(&bytes[body_start..]).unwrap()
    };
    let first = outbound(&attempts[0]);
    let retry = outbound(&attempts[1]);
    assert_eq!(first["system"], "stable instructions");
    assert_eq!(
        first["messages"][1],
        json!({"role":"system","content":"synthetic reminder"})
    );
    assert_eq!(retry["system"], first["system"]);
    assert_eq!(
        retry["messages"],
        json!([{"role":"user","content":"synthetic input\nsynthetic reminder"}])
    );
}

#[test]
fn messages_system_role_recovery_requires_matching_history_and_latest_response() {
    let provider = NativeProvider::start(vec![
        complete(400, ROLE_REJECTION),
        complete(400, PERMANENT_ERROR),
        complete(400, ROLE_REJECTION),
        complete(400, ROLE_REJECTION),
    ]);
    let fallback = NativeProvider::start(vec![
        complete(400, PERMANENT_ERROR),
        complete(200, MESSAGES_OK),
    ]);
    let fixture = RuntimeFixture::launch_with_observation_and_publication_candidates(
        &[&provider],
        1,
        ObservationFaults::healthy(),
        Some(&[candidate(0)]),
    );
    for inline_system in [false, true] {
        assert_generic_error(&messages_request(&fixture, inline_system));
    }
    drop(fixture);

    let fixture = RuntimeFixture::launch_with_observation_and_publication_candidates(
        &[&provider, &fallback],
        2,
        ObservationFaults::healthy(),
        Some(&[candidate(0), candidate(1)]),
    );
    assert_generic_error(&messages_request(&fixture, true));
    let recovered_by_relay = messages_request(&fixture, true);
    assert_eq!(recovered_by_relay.status, 200);
    assert_eq!((provider.calls(), fallback.calls()), (4, 2));
}
