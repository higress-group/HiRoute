use super::*;
use serde_json::{Value, json};

const CHAT: &[u8] = br#"{"id":"parallel","model":"native","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
const MESSAGES: &[u8] = br#"{"id":"parallel","type":"message","role":"assistant","model":"native","content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}"#;

#[test]
fn responses_parallel_tools_default_and_opt_out_reach_each_upstream() {
    for (protocol, reply) in [
        ("responses", RESPONSES_OK),
        ("chat_completions", CHAT),
        ("messages", MESSAGES),
    ] {
        for setting in [None, Some(false), Some(true)] {
            let provider = NativeProvider::start(vec![
                ProviderReply::Complete {
                    status: 200,
                    error_kind: None,
                    body: reply
                };
                2
            ]);
            let candidates = [PublicationCandidate {
                provider_index: 0,
                upstream_protocol: protocol,
                statically_enabled: true,
            }];
            let mut fixture = RuntimeFixture::launch_with_publication_candidates(
                &[&provider],
                1,
                Some(&candidates),
            );
            let mut publication = fixture.publication();
            publication.aliases[0]
                .routing
                .as_mut()
                .unwrap()
                .codex_parallel_tool_calls = setting;
            publication.publication_revision += 1;
            publication.payload_digest = publication.canonical_digest().unwrap();
            fixture.restart_with_publication(&publication);
            for requested in [true, false] {
                let body = json!({"model":MODEL,"input":"Read two independent facts","stream":false,
                    "parallel_tool_calls":requested,"max_output_tokens":128,
                    "tools":[{"type":"function","name":"lookup","parameters":{"type":"object","properties":{"key":{"type":"string"}}}}]});
                let response = fixture.request_body(&serde_json::to_vec(&body).unwrap());
                assert_eq!(
                    response.status,
                    200,
                    "{protocol} {setting:?}: {}",
                    String::from_utf8_lossy(&response.body)
                );
                let calls = provider.requests();
                let wire = calls.last().unwrap();
                let offset = wire.windows(4).position(|p| p == b"\r\n\r\n").unwrap() + 4;
                let actual: Value = serde_json::from_slice(&wire[offset..]).unwrap();
                let expected = requested && setting != Some(false);
                if protocol == "messages" {
                    assert_eq!(
                        actual["tool_choice"]["disable_parallel_tool_use"],
                        !expected
                    );
                } else {
                    assert_eq!(
                        actual["parallel_tool_calls"], expected,
                        "{protocol} {setting:?}"
                    );
                }
                assert_eq!(actual["tools"].as_array().unwrap().len(), 1);
            }
            assert_eq!(provider.calls(), 2);
        }
    }
}
