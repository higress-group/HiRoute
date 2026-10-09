use super::*;
use serde_json::{Value, json};

const CHAT_OK: &[u8] = br#"{"id":"chat-options","model":"runtime-native-model-1","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;

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
            Some("Bearer provider-secret-1-1".into())
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
