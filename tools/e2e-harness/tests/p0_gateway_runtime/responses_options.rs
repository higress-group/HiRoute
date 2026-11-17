use super::*;
use serde_json::{Value, json};

const CHAT_OK: &[u8] = br#"{"id":"chat-options","model":"runtime-native-model-1","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;

#[test]
fn native_search_extensions_reach_upstream_and_retain_provider_outcomes() {
    for upstream_status in [200, 400] {
        let provider = NativeProvider::start(vec![ProviderReply::Complete {
            status: upstream_status,
            error_kind: None,
            body: if upstream_status == 200 {
                RESPONSES_OK
            } else {
                PERMANENT_ERROR
            },
        }]);
        let fixture = RuntimeFixture::launch(&[&provider], 1);
        let body = json!({"model":MODEL,"stream":false,"input":[
            {"role":"user","content":"Inspect the example"},
            {"type":"web_search_call","id":"ws-native","status":"completed",
             "action":{"type":"provider_future_action","query":"Example"}},
            {"role":"user","content":"Continue"}
        ],"tools":[{"type":"web_search","external_web_access":true,
            "filters":{"allowed_domains":["example.com"]}}]});
        let response = fixture.request_body(&serde_json::to_vec(&body).unwrap());
        assert_eq!(
            provider.calls(),
            1,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        assert_eq!(response.status, upstream_status);
        let requests = provider.requests();
        let wire = &requests[0];
        assert_eq!(
            wire_header(wire, "authorization"),
            Some("Bearer provider-secret-1".into())
        );
        assert!(wire_header(wire, "x-hiroute-token").is_none());
        let offset = wire
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .unwrap()
            + 4;
        let actual: Value = serde_json::from_slice(&wire[offset..]).unwrap();
        assert_eq!(actual["tools"], body["tools"]);
        assert_eq!(actual["input"], body["input"]);
        if upstream_status == 400 {
            assert_eq!(
                serde_json::from_slice::<Value>(&response.body).unwrap(),
                json!({"error":{"code":"UPSTREAM_ATTEMPT_FAILED","type":"upstream_error"}})
            );
        }
    }
}

#[test]
fn native_tool_extensions_reach_each_protocol_and_retain_provider_outcomes() {
    const MESSAGES_OK: &[u8] = br#"{"id":"msg-native","type":"message","role":"assistant","model":"runtime-native-model-1","content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}"#;
    for (protocol, path, token, success, body) in [
        (
            "responses",
            "/v1/responses",
            TOKEN,
            RESPONSES_OK,
            json!({"model":MODEL,"stream":false,"input":[
                {"role":"user","content":"Read the fixture"},
                {"type":"function_call","call_id":"call-1","name":"read","arguments":"{}","provider_options":{"origin":"model"}},
                {"type":"function_call_output","call_id":"call-1","output":"done"}
            ],"tools":[{"type":"function","name":"read","parameters":{"type":"object"},"provider_options":{"mode":"native"}}]}),
        ),
        (
            "chat_completions",
            "/v1/chat/completions",
            CHAT_TOKEN,
            CHAT_OK,
            json!({"model":MODEL,"stream":false,"messages":[
                {"role":"user","content":"Read the fixture"},
                {"role":"assistant","tool_calls":[{"type":"function","id":"call-1","function":{"name":"read","arguments":"{}"},"provider_options":{"origin":"model"}}]},
                {"role":"tool","tool_call_id":"call-1","content":"done"}
            ],"tools":[{"type":"function","function":{"name":"read","parameters":{"type":"object"},"provider_options":{"mode":"native"}}}]}),
        ),
        (
            "messages",
            "/v1/messages",
            MESSAGES_TOKEN,
            MESSAGES_OK,
            json!({"model":MODEL,"stream":false,"max_tokens":128,"messages":[
                {"role":"user","content":"Read the fixture"},
                {"role":"assistant","content":[{"type":"tool_use","id":"call-1","name":"read","input":{},"provider_options":{"origin":"model"}}]},
                {"role":"user","content":[{"type":"tool_result","tool_use_id":"call-1","content":"done"}]}
            ],"tools":[{"type":"custom","name":"read","input_schema":{"type":"object"},"defer_loading":true}]}),
        ),
    ] {
        for status in [200, 400] {
            let provider = NativeProvider::start(vec![ProviderReply::Complete {
                status,
                error_kind: None,
                body: if status == 200 {
                    success
                } else {
                    PERMANENT_ERROR
                },
            }]);
            let candidates = [PublicationCandidate {
                provider_index: 0,
                upstream_protocol: protocol,
                statically_enabled: true,
            }];
            let fixture = RuntimeFixture::launch_with_observation_and_publication_candidates(
                &[&provider],
                1,
                ObservationFaults::healthy(),
                Some(&candidates),
            );
            let response = request(
                fixture.address,
                "POST",
                path,
                &[("X-HiRoute-Token", token)],
                &serde_json::to_vec(&body).unwrap(),
            );
            assert_eq!(
                provider.calls(),
                1,
                "{protocol}: {}",
                String::from_utf8_lossy(&response.body)
            );
            assert_eq!(
                response.status,
                status,
                "{protocol}: {}",
                String::from_utf8_lossy(&response.body)
            );
            let requests = provider.requests();
            let wire = &requests[0];
            assert_eq!(
                wire_header(wire, "authorization"),
                Some("Bearer provider-secret-1".into())
            );
            assert!(wire_header(wire, "x-hiroute-token").is_none());
            let offset = wire
                .windows(4)
                .position(|part| part == b"\r\n\r\n")
                .unwrap()
                + 4;
            let actual: Value = serde_json::from_slice(&wire[offset..]).unwrap();
            let history = if protocol == "responses" {
                "input"
            } else {
                "messages"
            };
            assert_eq!(actual[history], body[history]);
            assert_eq!(actual["tools"], body["tools"]);
            if status == 400 {
                assert_eq!(
                    serde_json::from_slice::<Value>(&response.body).unwrap(),
                    json!({"error":{"code":"UPSTREAM_ATTEMPT_FAILED","type":"upstream_error"}})
                );
            }
        }
    }
}

#[test]
fn pi_options_reach_real_gateway_chat_upstream_with_exact_auth_and_tools() {
    for options in [
        json!({}),
        json!({"store":false}),
        json!({"prompt_cache_key":"pi-session"}),
        json!({"store":false,"prompt_cache_key":"pi-session"}),
    ] {
        let provider = NativeProvider::start(vec![ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: CHAT_OK,
        }]);
        let candidates = [PublicationCandidate {
            provider_index: 0,
            upstream_protocol: "chat_completions",
            statically_enabled: true,
        }];
        let fixture =
            RuntimeFixture::launch_with_publication_candidates(&[&provider], 1, Some(&candidates));
        let mut body = json!({"model":MODEL,"input":"hello","stream":false,
            "max_output_tokens":128,"tools":[{"type":"function","name":"lookup",
            "parameters":{"type":"object","properties":{"key":{"type":"string"}}}}]});
        body.as_object_mut()
            .unwrap()
            .extend(options.as_object().unwrap().clone());
        let response = fixture.request_body(&serde_json::to_vec(&body).unwrap());
        assert_eq!(
            response.status,
            200,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        assert_eq!(provider.calls(), 1);
        let requests = provider.requests();
        let wire = &requests[0];
        assert!(wire.starts_with(b"POST /v1/chat/completions "));
        assert_eq!(
            wire_header(wire, "authorization"),
            Some("Bearer provider-secret-1".into())
        );
        assert!(wire_header(wire, "x-hiroute-token").is_none());
        let offset = wire
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .unwrap()
            + 4;
        let actual: Value = serde_json::from_slice(&wire[offset..]).unwrap();
        for field in ["store", "prompt_cache_key"] {
            assert_eq!(actual.get(field), body.get(field));
        }
        assert_eq!(actual["model"], "runtime-native-model-1");
        assert_eq!(actual["max_completion_tokens"], 128);
        assert_eq!(actual["tools"][0]["function"]["name"], "lookup");
        assert_eq!(
            actual["tools"][0]["function"]["parameters"],
            body["tools"][0]["parameters"]
        );
    }
}
