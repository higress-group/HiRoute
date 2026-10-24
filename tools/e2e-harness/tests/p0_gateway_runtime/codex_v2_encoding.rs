//! Real listener/attempt/capture wiring. Providers and tool receipts are controlled;
//! this does not claim a native client executed the tool or a model understood peers.
use super::native_history::{classifier_body, headers, reply};
use super::*;
use hiroute_gateway::server::request_plan::IngressProtocol;
use serde_json::{Value, json};

const ASSESSED_PRIMARY: &[u8] =
    br#"{"group":"primary","assessment":{"score":0.75,"partial":false}}"#;
const TOOL: &[u8] = br#"{"id":"tool","model":"runtime-native","status":"completed","output":[{"type":"function_call","id":"fc","call_id":"call","namespace":"collaboration","name":"spawn_agent","arguments":"{\"message\":\"read fixture\"}","status":"completed"}],"usage":{"input_tokens":1,"output_tokens":1}}"#;
const TOOL_DECLARED: &[u8] = br#"{"id":"tool","model":"runtime-native","status":"completed","output":[{"type":"function_call","id":"fc","call_id":"call","namespace":"collaboration","name":"spawn_agent","arguments":"{\"message\":\"read fixture\"}","status":"completed","encrypted_function_args":[]}],"usage":{"input_tokens":1,"output_tokens":1}}"#;

fn dialect(fixture: &RuntimeFixture, primary_standard: bool) {
    let mut publication = fixture.publication();
    for (index, candidate) in publication.aliases[0].candidates.iter_mut().enumerate() {
        if index == 1 && !primary_standard {
            continue;
        }
        for profile in &mut candidate.protocol_profiles {
            if profile.ingress_protocol == IngressProtocol::Responses {
                profile.adapter_revision =
                    hiroute_domain::STANDARD_RESPONSES_JSON_ADAPTER_V1.into();
            }
        }
        candidate.protocol_profile_digest =
            hiroute_domain::CanonicalDigest::of(&candidate.protocol_profiles).unwrap();
    }
    publication.publication_revision += 1;
    fixture.publish(&mut publication);
}

fn send(
    fixture: &RuntimeFixture,
    v2: bool,
    turn: &str,
    input: &[Value],
    streaming: bool,
) -> Vec<Value> {
    let mut body: Value = if v2 {
        serde_json::from_str(include_str!(
            "../../../../crates/gateway/src/adapters/fixtures/codex-v2-request.json"
        ))
        .unwrap()
    } else {
        json!({"tools":[{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":"spawn_agent","parameters":{"type":"object","properties":{"message":{"type":"string"}}}}]}]})
    };
    body["model"] = json!(MODEL);
    body["stream"] = json!(streaming);
    // Also exercise production Replay without changing the tool/schema contract.
    body["instructions"] = json!("stable instruction ".repeat(4096));
    body["input"] = json!(input);
    let h = headers(
        if v2 { "codex" } else { "opencode" },
        "encoding-session",
        "main",
        turn,
        "checkpoint",
    );
    let response = request(
        fixture.address,
        "POST",
        "/v1/responses",
        &h.iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect::<Vec<_>>(),
        &serde_json::to_vec(&body).unwrap(),
    );
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let value: Value = if streaming {
        let events = std::str::from_utf8(&response.body)
            .unwrap()
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .filter(|s| *s != "[DONE]")
            .map(|s| serde_json::from_str::<Value>(s).unwrap())
            .collect::<Vec<_>>();
        assert!(
            !events.iter().any(|e| matches!(
                e["type"].as_str(),
                Some("error" | "response.failed" | "response.incomplete")
            )),
            "{events:?}"
        );
        events
            .iter()
            .find(|e| e["type"] == "response.completed")
            .unwrap()["response"]
            .clone()
    } else {
        serde_json::from_slice(&response.body).unwrap()
    };
    assert_eq!(value["status"], "completed");
    value["output"].as_array().unwrap().clone()
}

fn provider_reply(streaming: bool, tool: bool, declared: bool) -> ProviderReply {
    let body = if tool {
        if declared { TOOL_DECLARED } else { TOOL }
    } else {
        RESPONSES_OK
    };
    let response: Value = serde_json::from_slice(body).unwrap();
    if streaming {
        super::native_encrypted_continuation::streamed(
            if tool { "tool" } else { "final" },
            response["output"].as_array().unwrap().clone(),
        )
    } else {
        reply(body)
    }
}

#[test]
fn real_listener_codex_v2_and_standard_responses_have_exact_complete_tool_stage() {
    for (v2, declared) in [(false, false), (false, true), (true, false), (true, true)] {
        for streaming in [false, true] {
            let regular = NativeProvider::start(vec![
                provider_reply(streaming, true, declared),
                provider_reply(streaming, false, declared),
            ]);
            let primary = NativeProvider::start(vec![provider_reply(streaming, false, declared)]);
            let classifier =
                NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE), reply(ASSESSED_PRIMARY)]);
            let fixture = RuntimeFixture::launch_rest_classified_with_observation(
                &[&regular, &primary, &classifier],
                1,
                ObservationFaults::healthy(),
            );
            dialect(&fixture, true);
            let mut input = vec![json!({"role":"user","content":"A_READ_FIXTURE"})];
            let mut output = send(&fixture, v2, "a", &input, streaming);
            assert_eq!(
                output[0].get("encrypted_function_args"),
                if v2 { Some(&json!([])) } else { None }
            );
            let call = output[0]["call_id"].clone();
            if !v2 {
                output[0]
                    .as_object_mut()
                    .unwrap()
                    .remove("encrypted_function_args");
            }
            input.extend(output);
            input.push(json!({"type":"function_call_output","call_id":call,"output":"CONTROLLED_TOOL_RECEIPT"}));
            input.extend(send(&fixture, v2, "a", &input, streaming));
            assert_eq!(
                classifier.calls(),
                1,
                "tool continuation opened a new decision"
            );
            input.push(json!({"role":"user","content":"B_INDEPENDENT_TASK"}));
            send(&fixture, v2, "b", &input, streaming);
            assert_eq!(
                (regular.calls(), primary.calls(), classifier.calls()),
                (2, 1, 2)
            );
            let upstream = classifier_body(&regular, 0);
            assert!(!upstream["tools"].to_string().contains("\"encrypted\":true"));
            assert_eq!(
                classifier_body(&regular, 1)["input"][2]["output"],
                "CONTROLLED_TOOL_RECEIPT"
            );
            assert!(
                classifier_body(&regular, 1)["input"][1]
                    .get("encrypted_function_args")
                    .is_none()
            );
            let judged = classifier_body(&classifier, 1);
            assert_eq!(judged["preceding_stage"]["complete"], true, "{judged}");
            let from = judged["preceding_stage"]["from"].as_u64().unwrap() as usize;
            let assessed =
                json!(&judged["visible_conversation"].as_array().unwrap()[from..]).to_string();
            assert!(
                assessed.contains("A_READ_FIXTURE")
                    && assessed.contains("CONTROLLED_TOOL_RECEIPT")
                    && assessed.contains("ok"),
                "{assessed}"
            );
            assert!(!assessed.contains("B_INDEPENDENT_TASK"));
            let facts = wait_execution_facts(&fixture, 3);
            let routes = facts
                .iter()
                .filter(|e| e["fact"]["kind"] == "route_decision")
                .collect::<Vec<_>>();
            assert_eq!(routes.len(), 3);
            assert_eq!(routes[1]["fact"]["complexity"]["inherited"], true);
            assert_eq!(routes[2]["fact"]["complexity"]["inherited"], false);
            let members = routes[..2]
                .iter()
                .map(|e| e["correlation"]["request_id"].clone())
                .collect::<Vec<_>>();
            let score = facts
                .iter()
                .find(|e| e["fact"]["kind"] == "branch_assessment_recorded")
                .unwrap();
            assert_eq!(score["fact"]["score"], 0.75);
            assert_eq!(
                score["fact"]["history_membership"]["request_ids"],
                json!(members)
            );
            assert_eq!(
                score["fact"]["history_membership"]["capture_partial"],
                false
            );
            let admissions = facts
                .iter()
                .filter(|e| e["fact"]["kind"] == "agent_turn_admitted")
                .collect::<Vec<_>>();
            assert_eq!(admissions.len(), 3);
            assert_eq!(
                admissions[0]["fact"]["agent_turn_id"],
                admissions[1]["fact"]["agent_turn_id"]
            );
            assert_ne!(
                admissions[1]["fact"]["agent_turn_id"],
                admissions[2]["fact"]["agent_turn_id"]
            );
        }
    }
}

#[test]
fn real_listener_codex_v2_retry_uses_successful_attempt_encoding_contract() {
    let regular = NativeProvider::start(vec![ProviderReply::Complete {
        status: 503,
        error_kind: None,
        body: OVERLOAD_ERROR,
    }]);
    let primary = NativeProvider::start(vec![reply(TOOL)]);
    let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE)]);
    let fixture = RuntimeFixture::launch_rest_classified_with_observation(
        &[&regular, &primary, &classifier],
        2,
        ObservationFaults::healthy(),
    );
    dialect(&fixture, false);
    let output = send(
        &fixture,
        true,
        "a",
        &[json!({"role":"user","content":"A_READ_FIXTURE"})],
        false,
    );
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (1, 1, 1)
    );
    assert!(
        output[0].get("encrypted_function_args").is_none(),
        "native fallback must not inherit a failed standard attempt's plaintext declaration"
    );
    assert!(
        !classifier_body(&regular, 0)["tools"]
            .to_string()
            .contains("\"encrypted\":true")
    );
    assert!(
        classifier_body(&primary, 0)["tools"]
            .to_string()
            .contains("\"encrypted\":true")
    );
    let facts = wait_execution_facts(&fixture, 1);
    assert_eq!(
        facts
            .iter()
            .filter(|e| e["fact"]["kind"] == "attempt_finished")
            .count(),
        2
    );
}
