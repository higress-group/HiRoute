use super::native_history::classifier_body;
use super::*;
use serde_json::{Value, json};

fn body() -> Value {
    json!({"model":MODEL,"stream":false,"input":[
        {"role":"user","content":"Complete the synthetic goal"},
        {"type":"function_call","call_id":"call-goal","name":"create_goal","arguments":"{}"},
        {"type":"function_call_output","call_id":"call-goal","output":"{\"status\":\"active\"}"},
        {"type":"function_call_output","id":"fco-goal","name":"goal_objective","namespace":"agent","output":"synthetic goal context"}
    ]})
}

#[test]
fn codex_goal_context_reaches_native_upstream_and_preserves_provider_outcomes() {
    for status in [200, 400] {
        let provider = NativeProvider::start(vec![ProviderReply::Complete {
            status,
            error_kind: None,
            body: if status == 200 {
                RESPONSES_OK
            } else {
                PERMANENT_ERROR
            },
        }]);
        let fixture = RuntimeFixture::launch(&[&provider], 1);
        let body = body();
        let response = fixture.request_body(&serde_json::to_vec(&body).unwrap());
        assert_eq!(
            provider.calls(),
            1,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        assert_eq!(
            response.status,
            status,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        let wire = classifier_body(&provider, 0);
        assert_eq!(wire["input"], body["input"]);
        assert!(wire["input"][3].get("call_id").is_none());
        if status == 400 {
            assert_eq!(
                serde_json::from_slice::<Value>(&response.body).unwrap(),
                json!({"error":{"code":"UPSTREAM_ATTEMPT_FAILED","type":"upstream_error"}})
            );
        }
    }
}

#[test]
fn codex_goal_context_excludes_unrepresentable_protocols_without_weakening_result_ids() {
    for protocol in ["chat_completions", "messages"] {
        let incompatible = NativeProvider::start(vec![]);
        let native = NativeProvider::start(vec![ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
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
        let fixture = RuntimeFixture::launch_with_publication_candidates(
            &[&incompatible, &native],
            1,
            Some(&candidates),
        );
        let invalid = json!({"model":MODEL,"input":[{"type":"function_call_output","output":"ordinary result"}]});
        let rejected = fixture.request_body(&serde_json::to_vec(&invalid).unwrap());
        assert_eq!(rejected.status, 400);
        let error: Value = serde_json::from_slice(&rejected.body).unwrap();
        assert_eq!(error["code"], "PROTOCOL_INVALID_PAYLOAD");
        assert_eq!(error["phase"], "canonical_request");
        assert_eq!((incompatible.calls(), native.calls()), (0, 0));
        let body = body();
        let response = fixture.request_body(&serde_json::to_vec(&body).unwrap());
        assert_eq!(
            response.status,
            200,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        assert_eq!((incompatible.calls(), native.calls()), (0, 1));
        assert_eq!(classifier_body(&native, 0)["input"], body["input"]);
    }
}
