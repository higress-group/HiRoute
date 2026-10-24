//! Register as a child of native_encrypted_continuation to share its fixtures.
//! Cross-protocol clients replay only the semantics actually delivered to them.
use super::*;
use base64::Engine as _;

const OMITTED_STATE: &str = "synthetic-cross-protocol-ciphertext-never-delivered";
// This fixture uses the decision extension, while the parent module exercises
// System One. Keep its response in the selected extension's wire contract.
const ASSESSED_PRIMARY: &[u8] =
    br#"{"group":"primary","assessment":{"score":0.75,"partial":false}}"#;

#[test]
fn real_listener_responses_opaque_omission_keeps_chat_tool_stage_complete() {
    cross_protocol_continuation(IngressProtocol::ChatCompletions);
}

#[test]
fn real_listener_responses_opaque_omission_keeps_messages_tool_stage_complete() {
    cross_protocol_continuation(IngressProtocol::Messages);
}

fn enable_cross_protocol_state(fixture: &RuntimeFixture, ingress: IngressProtocol) {
    let mut publication = fixture.publication();
    for candidate in &mut publication.aliases[0].candidates {
        for profile in &mut candidate.protocol_profiles {
            let mut native: CandidateProtocolProfile =
                serde_json::from_value(serde_json::to_value(&*profile).unwrap()).unwrap();
            if native.ingress_protocol == ingress {
                assert_eq!(
                    native.capability.upstream_protocol,
                    IngressProtocol::Responses
                );
                native.capability.native_provider_state = NativeProviderStateEmission::Native;
                native.capability.response.provider_state = Fidelity::Exact;
                *profile = serde_json::from_value(serde_json::to_value(native).unwrap()).unwrap();
            }
        }
        candidate.protocol_profile_digest =
            hiroute_domain::CanonicalDigest::of(&candidate.protocol_profiles).unwrap();
    }
    publication.publication_revision += 1;
    fixture.publish(&mut publication);
}

fn cross_send(fixture: &RuntimeFixture, ingress: IngressProtocol, messages: &[Value]) -> Value {
    let parameters = json!({"type":"object","properties":{
        "file_path":{"type":"string"},"old_string":{"type":"string"},
        "new_string":{"type":"string"},"justification":{"type":"string"}}});
    let (path, token, tools) = match ingress {
        IngressProtocol::ChatCompletions => (
            "/v1/chat/completions",
            CHAT_TOKEN,
            json!([{"type":"function","function":{"name":"edit","parameters":parameters}}]),
        ),
        IngressProtocol::Messages => (
            "/v1/messages",
            MESSAGES_TOKEN,
            json!([{"name":"edit","input_schema":parameters}]),
        ),
        _ => unreachable!(),
    };
    let response = request(
        fixture.address,
        "POST",
        path,
        &[
            ("X-HiRoute-Token", token),
            ("session-id", "cross-protocol-opaque"),
            ("anthropic-version", "2023-06-01"),
        ],
        &serde_json::to_vec(&json!({"model":MODEL,"stream":true,"max_tokens":1024,
        "messages":messages,"tools":tools}))
        .unwrap(),
    );
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let wire = std::str::from_utf8(&response.body).unwrap();
    assert!(!wire.contains(OMITTED_STATE) && !wire.contains("encrypted_content"));
    let events = wire
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|data| *data != "[DONE]")
        .map(|data| serde_json::from_str::<Value>(data).unwrap())
        .collect::<Vec<_>>();
    assert!(!events.is_empty());
    assert!(
        !events
            .iter()
            .any(|event| event["type"] == "error" || !event["error"].is_null())
    );
    match ingress {
        IngressProtocol::ChatCompletions => chat_assistant(&events),
        IngressProtocol::Messages => messages_assistant(&events),
        _ => unreachable!(),
    }
}

fn chat_assistant(events: &[Value]) -> Value {
    let mut text = String::new();
    let mut calls = BTreeMap::<u64, Value>::new();
    let mut finishes = Vec::new();
    for choice in events
        .iter()
        .flat_map(|event| event["choices"].as_array().unwrap())
    {
        assert_eq!(choice["index"], 0);
        let delta = &choice["delta"];
        if let Some(value) = delta["content"].as_str() {
            text.push_str(value);
        }
        assert!(delta["reasoning_content"].is_null() || delta["reasoning_content"] == "");
        if let Some(chunks) = delta["tool_calls"].as_array() {
            for chunk in chunks {
                let call = calls
                    .entry(chunk["index"].as_u64().unwrap())
                    .or_insert_with(
                        || json!({"type":"function","id":"","function":{"name":"","arguments":""}}),
                    );
                for key in ["id", "type"] {
                    if let Some(value) = chunk.get(key) {
                        call[key] = value.clone();
                    }
                }
                for key in ["name", "arguments"] {
                    if let Some(value) = chunk["function"][key].as_str() {
                        let joined = format!("{}{value}", call["function"][key].as_str().unwrap());
                        call["function"][key] = json!(joined);
                    }
                }
            }
        }
        if let Some(reason) = choice["finish_reason"].as_str() {
            finishes.push(reason);
        }
    }
    assert_eq!(
        finishes,
        [if calls.is_empty() {
            "stop"
        } else {
            "tool_calls"
        }]
    );
    let mut assistant = json!({"role":"assistant","content":text});
    if !calls.is_empty() {
        assistant["tool_calls"] = json!(calls.into_values().collect::<Vec<_>>());
    }
    assistant
}

fn messages_assistant(events: &[Value]) -> Value {
    let mut blocks = BTreeMap::<u64, Value>::new();
    let mut arguments = BTreeMap::<u64, String>::new();
    let mut stops = Vec::new();
    for event in events {
        match event["type"].as_str().unwrap() {
            "content_block_start" => {
                let index = event["index"].as_u64().unwrap();
                let block = event["content_block"].clone();
                assert!(
                    matches!(block["type"].as_str(), Some("text" | "tool_use")),
                    "Responses encrypted reasoning must not be invented as Messages thinking: {block}"
                );
                assert!(blocks.insert(index, block).is_none());
            }
            "content_block_delta" => {
                let index = event["index"].as_u64().unwrap();
                let delta = &event["delta"];
                match delta["type"].as_str().unwrap() {
                    "text_delta" => {
                        let block = blocks.get_mut(&index).unwrap();
                        block["text"] = json!(format!(
                            "{}{}",
                            block["text"].as_str().unwrap(),
                            delta["text"].as_str().unwrap()
                        ));
                    }
                    "input_json_delta" => arguments
                        .entry(index)
                        .or_default()
                        .push_str(delta["partial_json"].as_str().unwrap()),
                    other => panic!("unexpected delivered delta {other}"),
                }
            }
            "message_delta" => {
                if let Some(reason) = event["delta"]["stop_reason"].as_str() {
                    stops.push(reason);
                }
            }
            "message_start" | "content_block_stop" | "message_stop" | "ping" => {}
            other => panic!("unexpected Messages event {other}"),
        }
    }
    for (index, json) in arguments {
        blocks.get_mut(&index).unwrap()["input"] = serde_json::from_str(&json).unwrap();
    }
    let has_tool = blocks.values().any(|block| block["type"] == "tool_use");
    assert_eq!(stops, [if has_tool { "tool_use" } else { "end_turn" }]);
    assert!(events.iter().any(|event| event["type"] == "message_stop"));
    json!({"role":"assistant","content":blocks.into_values().collect::<Vec<_>>()})
}

fn cross_receipt(ingress: IngressProtocol, assistant: &Value) -> Value {
    match ingress {
        IngressProtocol::ChatCompletions => {
            let calls = assistant["tool_calls"].as_array().unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0]["function"]["name"], "edit");
            let args: Value =
                serde_json::from_str(calls[0]["function"]["arguments"].as_str().unwrap()).unwrap();
            assert_eq!(args["file_path"], "fixture.py");
            json!({"role":"tool","tool_call_id":calls[0]["id"],"content":"Edit applied successfully."})
        }
        IngressProtocol::Messages => {
            let calls = assistant["content"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|block| block["type"] == "tool_use")
                .collect::<Vec<_>>();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0]["name"], "edit");
            assert_eq!(calls[0]["input"]["file_path"], "fixture.py");
            json!({"role":"user","content":[{"type":"tool_result","tool_use_id":calls[0]["id"],"content":"Edit applied successfully.","is_error":false}]})
        }
        _ => unreachable!(),
    }
}

fn assert_cross_response_retention(fixture: &RuntimeFixture, decisions: &[&Value]) {
    let path = fixture
        .observation_root
        .as_ref()
        .unwrap()
        .join("conversation-content.jsonl");
    let deadline = Instant::now() + Duration::from_secs(5);
    let rows = loop {
        let rows = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        if decisions.iter().all(|decision| {
            rows.iter().any(|row| {
                row["correlation"]["request_id"] == decision["correlation"]["request_id"]
                    && row["direction"] == "response_delivered"
                    && row["phase"] == "finish"
            })
        }) {
            break rows;
        }
        assert!(
            Instant::now() < deadline,
            "cross-protocol response capture did not finish"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let mut parts = BTreeMap::<String, Vec<(u64, Vec<u8>)>>::new();
    for row in rows
        .iter()
        .filter(|row| row["direction"] == "response_delivered")
    {
        assert_ne!(row["phase"], "abort");
        assert!(row["loss_watermark"].is_null());
        assert_ne!(row["content_kind"], "provider_state");
        if row["phase"] == "append" {
            parts
                .entry(row["content_id"].as_str().unwrap().to_owned())
                .or_default()
                .push((
                    row["chunk_ordinal"].as_u64().unwrap(),
                    base64::engine::general_purpose::STANDARD
                        .decode(row["canonical_bytes_base64"].as_str().unwrap())
                        .unwrap(),
                ));
        }
    }
    assert!(!parts.is_empty());
    for mut chunks in parts.into_values() {
        chunks.sort_by_key(|(ordinal, _)| *ordinal);
        let bytes = chunks
            .into_iter()
            .flat_map(|(_, bytes)| bytes)
            .collect::<Vec<_>>();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(
            !text.contains(OMITTED_STATE)
                && !text.contains("provider_state")
                && !text.contains("encrypted_content")
        );
    }
}

// Responses message phases have no exact Chat/Messages representation. Keep
// this control focused on supported conversion plus omitted provider state;
// native_encrypted_continuation separately preserves the actual DSH phases.
fn portable_message(id: &str, text: &str) -> Value {
    let mut item = message(id, "final_answer", text);
    item.as_object_mut().unwrap().remove("phase");
    item
}

fn cross_protocol_continuation(ingress: IngressProtocol) {
    let regular = NativeProvider::start(vec![
        streamed(
            "cross-a-tool",
            vec![
                json!({"type":"reasoning","id":"cross-rs","summary":[],"status":"completed","encrypted_content":OMITTED_STATE}),
                portable_message("cross-commentary", COMMENTARY),
                edit("cross-edit", "The requested fixture must be updated."),
            ],
        ),
        streamed(
            "cross-a-final",
            vec![portable_message("cross-a-message", "A_COMPLETE")],
        ),
    ]);
    let primary = NativeProvider::start(vec![streamed(
        "cross-b-final",
        vec![portable_message("cross-b-message", "B_COMPLETE")],
    )]);
    let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE), reply(ASSESSED_PRIMARY)]);
    let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
        &[&regular, &primary, &classifier],
        1,
    );
    enable_cross_protocol_state(&fixture, ingress);
    let mut messages = vec![json!({"role":"user","content":A_USER})];
    let first = cross_send(&fixture, ingress, &messages);
    assert!(first.to_string().contains(COMMENTARY));
    assert_not_closed(&fixture, 1);
    let receipt = cross_receipt(ingress, &first);
    messages.extend([first, receipt]);
    let final_output = cross_send(&fixture, ingress, &messages);
    assert!(final_output.to_string().contains("A_COMPLETE"));
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (2, 0, 1),
        "omitted upstream ciphertext must not be required to replay visible delivered output"
    );
    let forwarded = upstream_body(&regular, 1);
    assert!(!forwarded.to_string().contains(OMITTED_STATE));
    assert!(!forwarded.to_string().contains("encrypted_content"));
    assert!(!forwarded.to_string().contains("__hiroute_content_ref"));
    assert!(
        forwarded["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "function_call_output")
    );
    messages.extend([final_output, json!({"role":"user","content":B_USER})]);
    let b = cross_send(&fixture, ingress, &messages);
    assert!(b.to_string().contains("B_COMPLETE"));
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (2, 1, 2)
    );

    let facts = wait_execution_facts(&fixture, 3);
    assert_single_planner_run_per_request(&facts, 3);
    let decisions = facts
        .iter()
        .filter(|event| event["fact"]["kind"] == "route_decision")
        .collect::<Vec<_>>();
    assert_cross_response_retention(&fixture, &decisions);
    for (index, decision) in decisions.iter().enumerate() {
        assert_eq!(decision["fact"]["complexity"]["fallback_used"], false);
        assert_eq!(
            decision["fact"]["complexity"]["decision_source"],
            if index == 1 {
                "inherited"
            } else {
                "external_classifier"
            }
        );
        assert_eq!(
            decision["fact"]["complexity"]["execution_group"],
            if index < 2 { "regular" } else { "primary" }
        );
    }
    let members = json!([
        decisions[0]["correlation"]["request_id"],
        decisions[1]["correlation"]["request_id"]
    ]);
    let stages = facts
        .iter()
        .filter(|event| {
            event["fact"]["kind"] == "agent_turn_finished"
                && event["fact"]["history_membership"]["request_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|id| members.as_array().unwrap().contains(id))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        stages.len(),
        1,
        "A must not have a prematurely sealed prefix"
    );
    let stage = &stages[0]["fact"];
    assert_eq!(stage["history_membership"]["request_ids"], members);
    assert_eq!(stage["history_membership"]["capture_partial"], false);
    assert_eq!(stage["history_membership"]["clipped"], false);
    assert_eq!(stage["history_partial"], false);
    assert_eq!(stage["status"], "completed");
    let scores = facts
        .iter()
        .filter(|event| event["fact"]["kind"] == "branch_assessment_recorded")
        .collect::<Vec<_>>();
    assert_eq!(scores.len(), 1);
    assert_eq!(
        scores[0]["correlation"]["request_id"],
        decisions[2]["correlation"]["request_id"]
    );
    let score = &scores[0]["fact"];
    assert_eq!(score["history_membership"], stage["history_membership"]);
    for key in [
        "segment_id",
        "plan_id",
        "plan_revision",
        "model_configuration_id",
        "profile_digest",
    ] {
        assert!(!stage[key].is_null());
        assert_eq!(score[key], stage[key]);
    }
    assert_eq!(score["partial"], false);
    assert_eq!(score["score"], 0.75);
    let assessed = upstream_body(&classifier, 1);
    assert_eq!(assessed["history_partial"], false);
    assert_eq!(assessed["preceding_stage"]["from"], 0);
    assert_eq!(
        assessed["visible_conversation"].as_array().unwrap().len(),
        1
    );
    assert_eq!(assessed["visible_conversation"][0]["status"], "completed");
    // Messages carries a native success flag; Chat's tool receipt does not.
    let tool_status = if ingress == IngressProtocol::Messages {
        "completed"
    } else {
        "unknown"
    };
    assert_eq!(
        assessed["visible_conversation"][0]["steps"],
        json!([
            [{"kind":"text","text":COMMENTARY},{"kind":"tool_activity","tool":"edit","status":tool_status}],
            [{"kind":"text","text":"A_COMPLETE"}]
        ])
    );
    assert!(!assessed.to_string().contains(OMITTED_STATE));
}
