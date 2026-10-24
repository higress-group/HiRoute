//! Real listener/attempt/capture wiring. Providers and tool receipts are controlled;
//! this does not claim a native client executed the tool or a model understood peers.
use super::native_history::{classifier_body, headers, reply};
use super::*;
use hiroute_domain::UpstreamProtocol;
use hiroute_gateway::server::publication::GatewayPublicationSnapshotV3;
use serde_json::{Value, json};

#[path = "codex_nullable_reasoning.rs"]
mod nullable_reasoning;

const ASSESSED_PRIMARY: &[u8] =
    br#"{"group":"primary","assessment":{"score":0.75,"partial":false}}"#;
const TOOL: &[u8] = br#"{"id":"tool","model":"runtime-native","status":"completed","output":[{"type":"function_call","id":"fc","call_id":"call","namespace":"collaboration","name":"spawn_agent","arguments":"{\"message\":\"read fixture\"}","status":"completed"}],"usage":{"input_tokens":1,"output_tokens":1}}"#;
const TOOL_DECLARED: &[u8] = br#"{"id":"tool","model":"runtime-native","status":"completed","output":[{"type":"function_call","id":"fc","call_id":"call","namespace":"collaboration","name":"spawn_agent","arguments":"{\"message\":\"read fixture\"}","status":"completed","encrypted_function_args":[]}],"usage":{"input_tokens":1,"output_tokens":1}}"#;

fn dialect(fixture: &RuntimeFixture, primary_standard: bool) {
    let mut publication = fixture.publication();
    configure_dialect(&mut publication, primary_standard);
    publication.publication_revision += 1;
    fixture.publish(&mut publication);
}

fn configure_dialect(publication: &mut GatewayPublicationSnapshotV3, primary_standard: bool) {
    for (index, candidate) in publication.aliases[0].candidates.iter_mut().enumerate() {
        if index == 1 && !primary_standard {
            continue;
        }
        for profile in &mut candidate.protocol_profiles {
            if profile.ingress_protocol == UpstreamProtocol::Responses {
                profile.adapter_revision =
                    hiroute_domain::STANDARD_RESPONSES_JSON_ADAPTER_V1.into();
            }
        }
        candidate.protocol_profile_digest =
            hiroute_domain::CanonicalDigest::of(&candidate.protocol_profiles).unwrap();
    }
}

fn original_body(v2: bool) -> Value {
    let mut body: Value = if v2 {
        serde_json::from_str(include_str!(
            "../../../../crates/gateway/src/adapters/fixtures/codex-v2-request.json"
        ))
        .unwrap()
    } else {
        json!({"tools":[{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":"spawn_agent","parameters":{"type":"object","properties":{"message":{"type":"string"}}}}]}]})
    };
    // These are business data, not schema encoding declarations.
    let schema = &mut body["tools"][0];
    let schema = if v2 {
        &mut schema["parameters"]
    } else {
        &mut schema["tools"][0]["parameters"]
    };
    schema["properties"]["encrypted"] = json!({"type":"boolean", "default":true});
    schema["default"] = json!({"encrypted":true, "parameters":{"encrypted":true}});
    schema["examples"] = json!([{"encrypted":true}]);
    body
}

fn expected_tools(v2: bool, standard: bool) -> Value {
    let mut body = original_body(v2);
    if v2 && standard {
        for i in [0, 3, 4] {
            body["tools"][5]["tools"][i]["parameters"]["properties"]["message"]
                .as_object_mut()
                .unwrap()
                .remove("encrypted");
        }
    }
    body["tools"].clone()
}

fn send(
    fixture: &RuntimeFixture,
    v2: bool,
    turn: &str,
    input: &[Value],
    streaming: bool,
) -> Vec<Value> {
    let mut body = original_body(v2);
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
    if v2 {
        // Sanitized fixture IDs must describe this actual synthetic session.
        // Competing body/header identities correctly remain request-scoped.
        body["client_metadata"]["thread_id"] = json!("encoding-session");
        body["client_metadata"]["session_id"] = json!("encoding-session");
        body["client_metadata"]["turn_id"] = json!(turn);
        body["client_metadata"]["x-codex-window-id"] = json!("encoding-session:0");
        body["client_metadata"]["x-codex-turn-metadata"] = json!(
            h.iter()
                .find(|(name, _)| name == "x-codex-turn-metadata")
                .unwrap()
                .1
        );
    }
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
    let mut response: Value = serde_json::from_slice(body).unwrap();
    if streaming {
        if !tool {
            // The compact buffered fixture omits item identity. A streamed
            // lifecycle requires the same ID in added/delta/done/terminal events.
            response["output"][0]["id"] = json!("message-final");
            response["output"][0]["status"] = json!("completed");
        }
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
            complete_tool_stage(v2, declared, streaming, false);
        }
    }
}

fn complete_tool_stage(v2: bool, declared: bool, streaming: bool, nullable: bool) {
    let regular = NativeProvider::start(vec![
        if nullable {
            nullable_reasoning::tool_reply(streaming)
        } else {
            provider_reply(streaming, true, declared)
        },
        provider_reply(streaming, false, declared),
    ]);
    let primary = NativeProvider::start(vec![provider_reply(streaming, false, declared)]);
    let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE), reply(ASSESSED_PRIMARY)]);
    let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
        &[&regular, &primary, &classifier],
        1,
    );
    dialect(&fixture, true);
    let mut input = vec![json!({"role":"user","content":"A_READ_FIXTURE"})];
    let mut output = send(&fixture, v2, "a", &input, streaming);
    let tool_index = usize::from(nullable);
    assert_eq!(
        output[tool_index].get("encrypted_function_args").cloned(),
        if v2 { Some(json!([])) } else { None }
    );
    let call = output[tool_index]["call_id"].clone();
    if !v2 || nullable {
        output[tool_index]
            .as_object_mut()
            .unwrap()
            .remove("encrypted_function_args");
    }
    if nullable {
        // Codex 0.147's request DTO preserves a null reasoning carrier
        // and one summary_text, but does not serialize output status.
        for item in &mut output {
            item.as_object_mut().unwrap().remove("status");
        }
        nullable_reasoning::assert_replayed_shape(&output);
    }
    input.extend(output);
    input.push(
        json!({"type":"function_call_output","call_id":call,"output":"CONTROLLED_TOOL_RECEIPT"}),
    );
    let original_input = input.clone();
    input.extend(send(&fixture, v2, "a", &input, streaming));
    assert_eq!(
        classifier.calls(),
        1,
        "v2={v2}, declared={declared}, streaming={streaming}: tool continuation opened a new decision"
    );
    input.push(json!({"role":"user","content":"B_INDEPENDENT_TASK"}));
    send(&fixture, v2, "b", &input, streaming);
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (2, 1, 2)
    );
    let upstream = classifier_body(&regular, 0);
    assert_eq!(upstream["tools"], expected_tools(v2, true));
    if nullable {
        // This producer replay omits encoding metadata; the ordinary cases below
        // separately assert the existing consumption of an explicit [] declaration.
        assert_eq!(classifier_body(&regular, 1)["input"], json!(original_input));
        assert_eq!(classifier_body(&primary, 0)["input"], json!(input));
    }
    assert_eq!(
        classifier_body(&regular, 1)["input"][tool_index + 2],
        json!({"type":"function_call_output","call_id":"call","output":"CONTROLLED_TOOL_RECEIPT"})
    );
    assert_eq!(
        classifier_body(&regular, 1)["input"][tool_index + 1]["call_id"],
        "call"
    );
    assert!(
        classifier_body(&regular, 1)["input"][tool_index + 1]
            .get("encrypted_function_args")
            .is_none()
    );
    let judged = classifier_body(&classifier, 1);
    assert_eq!(
        judged["preceding_stage"]["complete"], true,
        "v2={v2}, declared={declared}, streaming={streaming}: {judged}"
    );
    let from = judged["preceding_stage"]["from"].as_u64().unwrap() as usize;
    let assessed = json!(&judged["visible_conversation"].as_array().unwrap()[from..]);
    // Decision API deliberately excludes raw tool-result bodies. Its status
    // follows the native result's declared status, absent in this fixture.
    let mut assessed_user = vec![json!({"kind":"text","text":"A_READ_FIXTURE"})];
    if v2 {
        assessed_user
            .push(json!({"kind":"unavailable","source_kind":"unverified_request_user_origin"}));
    }
    let tool_name = if nullable {
        "exec_command"
    } else {
        "collaboration.spawn_agent"
    };
    assert_eq!(
        assessed,
        json!([{
            "user":assessed_user,
            "status":"completed",
            "steps":[[{"kind":"tool_activity","tool":tool_name,"status":"unknown"}],
                     [{"kind":"text","text":"ok"}]]
        }])
    );
    let facts = wait_execution_facts(&fixture, 3);
    let routes = facts
        .iter()
        .filter(|e| e["fact"]["kind"] == "route_decision")
        .collect::<Vec<_>>();
    assert_eq!(routes.len(), 3);
    let decisions = routes
        .iter()
        .map(|event| {
            serde_json::from_value::<hiroute_domain::BranchDecisionV1>(
                event["fact"]["complexity"].clone(),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    use hiroute_domain::{ComplexityDecisionSourceV1, ExecutionGroupV1};
    assert_eq!(
        decisions
            .iter()
            .map(|d| d.decision_source)
            .collect::<Vec<_>>(),
        [
            ComplexityDecisionSourceV1::ExternalClassifier,
            ComplexityDecisionSourceV1::Inherited,
            ComplexityDecisionSourceV1::ExternalClassifier,
        ]
    );
    assert_eq!(
        decisions
            .iter()
            .map(|d| d.execution_group)
            .collect::<Vec<_>>(),
        [
            ExecutionGroupV1::Regular,
            ExecutionGroupV1::Regular,
            ExecutionGroupV1::Primary
        ]
    );
    assert!(decisions.iter().all(|d| !d.fallback_used));
    let members = routes[..2]
        .iter()
        .map(|e| e["correlation"]["request_id"].clone())
        .collect::<Vec<_>>();
    let score = facts
        .iter()
        .find(|e| e["fact"]["kind"] == "branch_assessment_recorded")
        .unwrap();
    assert_eq!(score["fact"]["score"], 0.75);
    assert_eq!(score["fact"]["partial"], false);
    assert_eq!(
        score["fact"]["history_membership"]["request_ids"],
        json!(members)
    );
    assert_eq!(
        score["fact"]["history_membership"]["capture_partial"],
        false
    );
    assert_eq!(score["fact"]["history_membership"]["source_unverified"], v2);
    if nullable {
        assert_eq!(
            score["fact"]["history_membership"]["association_unverified"],
            true
        );
        for member in &members {
            let attempt = facts
                .iter()
                .find(|event| {
                    event["fact"]["kind"] == "attempt_finished"
                        && &event["correlation"]["request_id"] == member
                })
                .unwrap();
            assert_eq!(attempt["fact"]["provider_model_event"], "response_complete");
            assert!(attempt["attempt_id"].as_str().is_some());
            let started = facts
                .iter()
                .find(|event| {
                    event["fact"]["kind"] == "attempt_started"
                        && event["correlation"]["request_id"] == *member
                        && event["attempt_id"] == attempt["attempt_id"]
                })
                .unwrap();
            for field in ["model_configuration_id", "profile_digest"] {
                assert!(!started["fact"][field].is_null());
                assert_eq!(score["fact"][field], started["fact"][field]);
            }
            assert!(facts.iter().any(|event| {
                event["fact"]["kind"] == "request_finished"
                    && &event["correlation"]["request_id"] == member
                    && event["fact"]["outcome"] == "accepted"
            }));
        }
    }
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

#[test]
fn real_listener_codex_v2_missing_stream_item_identity_cannot_claim_complete_history() {
    let malformed: Value = serde_json::from_slice(RESPONSES_OK).unwrap();
    let regular = NativeProvider::start(vec![
        provider_reply(true, true, false),
        super::native_encrypted_continuation::streamed(
            "missing-item-id",
            malformed["output"].as_array().unwrap().clone(),
        ),
    ]);
    let primary = NativeProvider::start(vec![provider_reply(true, false, false)]);
    let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE), reply(ASSESSED_PRIMARY)]);
    let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
        &[&regular, &primary, &classifier],
        1,
    );
    dialect(&fixture, true);
    let mut input = vec![json!({"role":"user","content":"A_READ_FIXTURE"})];
    input.extend(send(&fixture, true, "a", &input, true));
    input.push(
        json!({"type":"function_call_output","call_id":"call","output":"CONTROLLED_TOOL_RECEIPT"}),
    );
    input.extend(send(&fixture, true, "a", &input, true));
    input.push(json!({"role":"user","content":"B_INDEPENDENT_TASK"}));
    send(&fixture, true, "b", &input, true);
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (2, 1, 2)
    );
    let judged = classifier_body(&classifier, 1);
    assert_eq!(judged["history_partial"], true);
    assert_eq!(judged["preceding_stage"]["complete"], false);
    let facts = wait_execution_facts(&fixture, 3);
    let score = facts
        .iter()
        .find(|e| e["fact"]["kind"] == "branch_assessment_recorded")
        .unwrap();
    assert_eq!(score["fact"]["partial"], true);
    assert_eq!(score["fact"]["history_membership"]["capture_partial"], true);
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
    let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
        &[&regular, &primary, &classifier],
        2,
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
    assert_eq!(
        classifier_body(&regular, 0)["tools"],
        expected_tools(true, true)
    );
    assert_eq!(
        classifier_body(&primary, 0)["tools"],
        expected_tools(true, false)
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
