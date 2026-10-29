//! Native instruction text may change without rewriting captured user history.
use super::*;
use serde_json::{Value, json};

fn classifier_body(provider: &NativeProvider, index: usize) -> Value {
    let requests = provider.requests();
    let raw = &requests[index];
    let offset = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    serde_json::from_slice(&raw[offset..]).unwrap()
}

#[test]
fn real_listener_instruction_text_changes_keep_captured_native_history() {
    for role in ["system", "developer"] {
        let model = NativeProvider::start(vec![
            ProviderReply::Complete {
                status: 200,
                error_kind: None,
                body: RESPONSES_OK,
            };
            2
        ]);
        let classifier = NativeProvider::start(vec![
            ProviderReply::Complete {
                status: 200,
                error_kind: None,
                body: CLASSIFIER_SIMPLE,
            };
            2
        ]);
        let unused_primary = NativeProvider::start(vec![]);
        let fixture = RuntimeFixture::launch_rest_classified_with_diagnostics(
            &[&model, &unused_primary, &classifier],
            1,
        );
        for (turn, input) in [
            (
                "1",
                json!([
                    {"role":role,"content":"native skills location A"},
                    {"role":"user","content":"FIRST_NATIVE_TASK"}
                ]),
            ),
            (
                "2",
                json!([
                    {"role":role,"content":"native skills location B"},
                    {"role":"user","content":"FIRST_NATIVE_TASK"},
                    {"role":"assistant","content":"ok"},
                    {"role":"user","content":"NEXT_NATIVE_TASK"}
                ]),
            ),
        ] {
            let metadata = json!({"version":1,"agent":"opencode","session_id":"dynamic-system","purpose":"main","turn_id":turn,"continuation_requires_existing_turn":false}).to_string();
            let response = request(
                fixture.address,
                "POST",
                "/v1/responses",
                &[
                    ("X-HiRoute-Token", TOKEN),
                    ("x-hiroute-agent", "opencode"),
                    ("x-opencode-session-id", "dynamic-system"),
                    ("x-hiroute-routing-context", metadata.as_str()),
                ],
                &serde_json::to_vec(&json!({"model":MODEL,"input":input,"stream":false})).unwrap(),
            );
            assert_eq!(
                response.status,
                200,
                "{}",
                String::from_utf8_lossy(&response.body)
            );
        }
        assert_eq!(classifier.calls(), 2);
        assert_eq!(model.calls(), 2);
        let decision = classifier_body(&classifier, 1);
        assert_eq!(decision["history_partial"], true);
        assert!(!decision["preceding_stage"].is_null());
        let visible = decision["visible_conversation"].to_string();
        assert!(visible.contains("FIRST_NATIVE_TASK"), "{decision}");
        assert!(visible.contains("ok"), "{decision}");
        assert!(!visible.contains("native skills location"), "{decision}");
        let executed = classifier_body(&model, 1);
        assert!(
            executed["input"][0]["content"]
                .to_string()
                .contains("native skills location B")
        );
        assert_eq!(executed["input"][0]["role"], role);
    }
}
