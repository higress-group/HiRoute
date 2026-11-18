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

#[cfg(all(unix, debug_assertions))]
#[test]
fn codex_goal_context_handoff_preserves_the_turn_while_response_capture_is_paused() {
    use super::native_encrypted_continuation::{message, streamed};
    use super::native_history::{headers, reply};

    for goal_before_result in [false, true] {
        let call = json!({"type":"function_call","id":"fc-goal","call_id":"call-goal",
            "name":"create_goal","arguments":"{}","status":"completed"});
        let regular = NativeProvider::start(vec![
            streamed("goal-call", vec![call]),
            streamed(
                "goal-final",
                vec![message("final", "final_answer", "Complete.")],
            ),
        ]);
        let primary = NativeProvider::start(vec![streamed(
            "different-route",
            vec![message(
                "different-final",
                "final_answer",
                "Different route.",
            )],
        )]);
        let classifier =
            NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE), reply(CLASSIFIER_COMPLEX)]);
        let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
            &[&regular, &primary, &classifier],
            1,
        );
        let pause = fixture
            .observation_root
            .as_ref()
            .unwrap()
            .join("response-capture.pause");
        std::fs::write(&pause, b"").unwrap();
        let native_headers = headers(
            "codex",
            "goal-handoff-session",
            "main",
            "goal-turn",
            "before",
        );
        let native_headers = native_headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect::<Vec<_>>();
        let send = |input: &[Value]| {
            let payload = serde_json::to_vec(&json!({"model":MODEL,"stream":true,"store":false,
                "input":input,"tools":[{"type":"function","name":"create_goal","parameters":{"type":"object"}}]})).unwrap();
            let body = read_chunked_eos(open_request(
                fixture.address,
                "POST",
                "/v1/responses",
                &native_headers,
                &payload,
            ));
            super::codex_acp_history::completed_stream(&body)
        };
        let mut input = vec![json!({"role":"user","content":"Complete the synthetic goal."})];
        let first = send(&input);
        input.extend(first["output"].as_array().unwrap().clone());
        let goal = json!({"type":"function_call_output","id":"goal-context",
            "name":"goal_objective","namespace":"agent","output":"Complete the synthetic goal."});
        let result = json!({"type":"function_call_output","call_id":"call-goal","output":"active"});
        input.extend(if goal_before_result {
            [goal, result]
        } else {
            [result, goal]
        });
        let final_response = send(&input);
        std::fs::remove_file(pause).unwrap();
        assert_eq!(
            (regular.calls(), primary.calls(), classifier.calls()),
            (2, 0, 1)
        );
        assert_eq!(
            final_response["output"][0]["content"][0]["text"],
            "Complete."
        );
        assert_eq!(classifier_body(&regular, 1)["input"], json!(input));

        let facts = wait_execution_facts(&fixture, 2);
        let timeline = super::observed_history::timeline(&facts);
        assert_eq!(timeline.turns.len(), 1);
        assert_eq!(timeline.turns[0].requests.len(), 2);
        let admissions = facts
            .iter()
            .filter(|event| event["fact"]["kind"] == "agent_turn_admitted")
            .collect::<Vec<_>>();
        assert_eq!(admissions.len(), 2);
        assert_eq!(
            admissions[0]["fact"]["agent_turn_id"],
            admissions[1]["fact"]["agent_turn_id"]
        );
        let sources = facts
            .iter()
            .filter(|event| event["fact"]["kind"] == "route_decision")
            .map(|event| {
                event["fact"]["complexity"]["decision_source"]
                    .as_str()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(sources, ["external_classifier", "inherited"]);
    }
}
