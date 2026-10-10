use super::*;

const RESPONSES_INCOMPLETE: &[u8] = br#"{"id":"partial-response","model":"runtime-native","status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[{"type":"message","id":"partial-message","role":"assistant","status":"incomplete","content":[{"type":"output_text","text":"PRIVATE_PARTIAL_BODY"}]}],"usage":{"input_tokens":7,"output_tokens":2,"total_tokens":9}}"#;
const CHAT_INCOMPLETE: &[u8] = br#"{"id":"partial-chat","object":"chat.completion","created":1,"model":"runtime-native","choices":[{"index":0,"message":{"role":"assistant","content":"PRIVATE_PARTIAL_BODY"},"finish_reason":"length"}],"usage":{"prompt_tokens":7,"completion_tokens":2,"total_tokens":9}}"#;
const MESSAGES_INCOMPLETE: &[u8] = br#"{"id":"partial-message","type":"message","role":"assistant","model":"runtime-native","content":[{"type":"text","text":"PRIVATE_PARTIAL_BODY"}],"stop_reason":"max_tokens","stop_sequence":null,"usage":{"input_tokens":7,"output_tokens":2}}"#;

#[test]
fn known_nonstream_incomplete_relays_before_body_and_retains_usage_within_budget() {
    for (protocol, body) in [
        ("responses", RESPONSES_INCOMPLETE),
        ("chat_completions", CHAT_INCOMPLETE),
        ("messages", MESSAGES_INCOMPLETE),
    ] {
        for max_attempts in [1, 2] {
            let first = NativeProvider::start(vec![ProviderReply::Complete {
                status: 200,
                error_kind: None,
                body,
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
                Some(&candidates),
            );
            let response = fixture
                .request_body(br#"{"model":"runtime-model","input":"hello","stream":false}"#);
            let label = format!("{protocol}, max_attempts={max_attempts}");
            let wire = String::from_utf8_lossy(&response.body);
            assert_eq!(
                response.status,
                if max_attempts == 2 { 200 } else { 502 },
                "{label}: {wire}"
            );
            assert_eq!(first.calls(), 1, "{label}");
            assert_eq!(second.calls(), usize::from(max_attempts == 2), "{label}");
            assert!(!wire.contains("PRIVATE_PARTIAL_BODY"), "{label}: {wire}");
            if max_attempts == 2 {
                assert!(wire.contains("fallback"), "{label}: {wire}");
            }
            let facts = wait_execution_facts(&fixture, 1);
            assert!(
                facts
                    .iter()
                    .any(|row| row.pointer("/fact/kind").and_then(|v| v.as_str())
                        == Some("attempt_finished")
                        && row.pointer("/fact/retryable").and_then(|v| v.as_bool()) == Some(true)
                        && row.pointer("/fact/disposition").and_then(|v| v.as_str())
                            == Some("continue")),
                "prebody failure must remain relayable: {label}: {facts:?}"
            );
            assert!(
                facts
                    .iter()
                    .any(|row| row.pointer("/fact/kind").and_then(|v| v.as_str())
                        == Some("usage_and_cache")
                        && row.pointer("/fact/input_tokens").and_then(|v| v.as_u64()) == Some(7)
                        && row.pointer("/fact/output_tokens").and_then(|v| v.as_u64()) == Some(2)),
                "incomplete attempt usage must survive: {label}: {facts:?}"
            );
        }
    }
}
