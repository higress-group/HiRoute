use super::*;
use serde_json::{Value, json};

const OVERFLOW: &[u8] = br#"{"error":{"type":"invalid_request_error","code":"context_length_exceeded","message":"PRIVATE_INPUT_MUST_NOT_ESCAPE"}}"#;
const MESSAGES_OVERFLOW: &[u8] = br#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 123456 tokens > 100000 maximum"}}"#;

fn overflow_request(fixture: &RuntimeFixture, protocol: &str) -> WireResponse {
    let (path, body) = match protocol {
        "responses" => (
            "/v1/responses",
            json!({"model":MODEL,"input":"long native input","stream":false}),
        ),
        "chat_completions" => (
            "/v1/chat/completions",
            json!({"model":MODEL,"messages":[{"role":"user","content":"long native input"}],"stream":false}),
        ),
        "messages" => (
            "/v1/messages",
            json!({"model":MODEL,"messages":[{"role":"user","content":"long native input"}],"max_tokens":32,"stream":false}),
        ),
        _ => unreachable!(),
    };
    request(
        fixture.address,
        "POST",
        path,
        &[(
            "X-HiRoute-Token",
            match protocol {
                "chat_completions" => CHAT_TOKEN,
                "messages" => MESSAGES_TOKEN,
                _ => TOKEN,
            },
        )],
        &serde_json::to_vec(&body).unwrap(),
    )
}

fn assert_overflow(response: &WireResponse) {
    assert_eq!(
        response.status,
        400,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&response.body).unwrap(),
        json!({
            "error": {"type":"invalid_request_error", "code":"context_length_exceeded",
                "message":"Input exceeds the context window."}
        })
    );
}

#[test]
fn native_context_overflow_survives_sanitization_and_attempt_limit_without_skipping_relay() {
    for protocol in ["responses", "chat_completions", "messages"] {
        for (candidate_count, max_attempts) in [(1, 1), (2, 1), (2, 2)] {
            let first = NativeProvider::start(vec![ProviderReply::Complete {
                status: 400,
                error_kind: None,
                body: if protocol == "messages" {
                    MESSAGES_OVERFLOW
                } else {
                    OVERFLOW
                },
            }]);
            let second = NativeProvider::start(vec![ProviderReply::Complete {
                status: 200,
                error_kind: None,
                body: FALLBACK_OK,
            }]);
            let candidates = [
                PublicationCandidate {
                    provider_index: 0,
                    upstream_protocol: protocol,
                    statically_enabled: true,
                },
                PublicationCandidate {
                    provider_index: 1,
                    upstream_protocol: "responses",
                    statically_enabled: true,
                },
            ];
            let fixture = RuntimeFixture::launch_with_observation_and_publication_candidates(
                &[&first, &second],
                max_attempts,
                ObservationFaults::healthy(),
                Some(&candidates[..candidate_count]),
            );
            let response = overflow_request(&fixture, protocol);
            assert_eq!(
                first.calls(),
                1,
                "{protocol}, {candidate_count}, {max_attempts}: {} {}",
                response.status,
                String::from_utf8_lossy(&response.body)
            );
            assert_eq!(second.calls(), usize::from(max_attempts == 2));
            if max_attempts == 2 {
                assert_eq!(
                    response.status,
                    200,
                    "{protocol}: {}",
                    String::from_utf8_lossy(&response.body)
                );
                assert!(String::from_utf8_lossy(&response.body).contains("fallback"));
            } else {
                assert_overflow(&response);
            }
            assert!(
                !String::from_utf8_lossy(&response.body).contains("PRIVATE_INPUT_MUST_NOT_ESCAPE")
            );
            assert!(!String::from_utf8_lossy(&response.body).contains("123456"));
        }
    }
}

#[test]
fn ordinary_client_errors_and_later_failures_never_reuse_a_stale_overflow_cause() {
    for body in [PERMANENT_ERROR, br#"{"error":{"type":"invalid_request_error","message":"context_length_exceeded quoted in an unrelated error"}}"#] {
        let first = NativeProvider::start(vec![ProviderReply::Complete {status:400,error_kind:None,body}]);
        let second = NativeProvider::start(vec![ProviderReply::Complete {status:200,error_kind:None,body:FALLBACK_OK}]);
        let fixture = RuntimeFixture::launch(&[&first, &second], 1);
        let response = fixture.request();
        assert_eq!(response.status, 502);
        assert_eq!(response.body, b"no upstream candidate");
        assert_eq!((first.calls(),second.calls()),(1,0));
    }
    let first = NativeProvider::start(vec![ProviderReply::Complete {
        status: 400,
        error_kind: None,
        body: OVERFLOW,
    }]);
    let second = NativeProvider::start(vec![ProviderReply::Complete {
        status: 503,
        error_kind: None,
        body: b"private later failure",
    }]);
    let third = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: FALLBACK_OK,
    }]);
    let fixture = RuntimeFixture::launch(&[&first, &second, &third], 2);
    let response = fixture.request();
    assert_eq!(response.status, 502);
    assert_eq!(response.body, b"no upstream candidate");
    assert_eq!((first.calls(), second.calls(), third.calls()), (1, 1, 0));
}
