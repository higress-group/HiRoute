use super::*;

fn classifier_body(classifier: &NativeProvider, index: usize) -> serde_json::Value {
    let requests = classifier.requests();
    let raw = &requests[index];
    let start = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    serde_json::from_slice(&raw[start..]).unwrap()
}

#[test]
fn real_listener_tool_json_key_order_keeps_complete_stage_and_assessment() {
    assert_tool_json_history(false);
}

#[test]
fn real_listener_changed_tool_json_value_cannot_inherit_the_accepted_stage() {
    assert_tool_json_history(true);
}

fn assert_tool_json_history(changed_value: bool) {
    use serde_json::{Value, json};
    // Both the provider and client use deliberate, different nonlexical key
    // orders. A sorted client spelling alone would miss the preserve_order bug.
    const TOOL: &[u8] = br#"{"id":"json-tool","model":"runtime-native","status":"completed","output":[{"type":"function_call","id":"fc-native","call_id":"native-call","namespace":"tools","name":"lookup","arguments":"{\"z\":3,\"list\":[1,2],\"a\":{\"y\":2,\"x\":1}}","status":"completed"}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#;
    const ASSESSED_COMPLEX: &[u8] =
        br#"{"group":"primary","assessment":{"score":0.75,"partial":false}}"#;
    let reply = |body| ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body,
    };
    let mut regular_replies = vec![reply(TOOL)];
    if !changed_value {
        regular_replies.push(reply(RESPONSES_OK));
    }
    let regular = NativeProvider::start(regular_replies);
    let primary = NativeProvider::start(vec![reply(RESPONSES_OK)]);
    let classifier = NativeProvider::start(vec![
        reply(CLASSIFIER_SIMPLE),
        reply(if changed_value {
            CLASSIFIER_COMPLEX
        } else {
            ASSESSED_COMPLEX
        }),
    ]);
    let fixture = RuntimeFixture::launch_rest_classified_with_observation(
        &[&regular, &primary, &classifier],
        1,
        ObservationFaults::healthy(),
    );
    let send = |input: &[Value]| {
        let response = request(
            fixture.address,
            "POST",
            "/v1/responses",
            &[
                ("X-HiRoute-Token", TOKEN),
                ("session-id", "tool-json-history"),
            ],
            &serde_json::to_vec(&json!({"model":MODEL,"input":input,"stream":false})).unwrap(),
        );
        assert_eq!(
            response.status,
            200,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        serde_json::from_slice::<Value>(&response.body).unwrap()
    };
    let mut input = vec![json!({"role":"user","content":"actual A: look up the record"})];
    let first = send(&input);
    let mut tool = first["output"][0].clone();
    let native_arguments = tool["arguments"].as_str().unwrap();
    assert_eq!(
        native_arguments,
        r#"{"z":3,"list":[1,2],"a":{"y":2,"x":1}}"#
    );
    let accepted_arguments: Value = serde_json::from_str(native_arguments).unwrap();
    let replayed_arguments = if changed_value {
        r#" { "list" : [1,2], "a" : { "x":1,"y":2 }, "z":4 } "#
    } else {
        r#" { "list" : [1,2], "a" : { "x":1,"y":2 }, "z":3 } "#
    };
    tool["arguments"] = json!(replayed_arguments);
    let call_id = tool["call_id"].clone();
    input.extend([
        tool,
        json!({"type":"function_call_output","call_id":call_id,"output":"private actual tool receipt"}),
    ]);
    let second = send(&input);
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        if changed_value { (1, 1, 2) } else { (2, 0, 1) },
        "only the semantically identical tool result may inherit A's actual Regular execution"
    );
    let forwarded = classifier_body(
        if changed_value { &primary } else { &regular },
        usize::from(!changed_value),
    );
    let forwarded_tool = forwarded["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call")
        .unwrap();
    assert_eq!(forwarded_tool["call_id"], call_id);
    let forwarded_arguments: Value =
        serde_json::from_str(forwarded_tool["arguments"].as_str().unwrap()).unwrap();
    assert_eq!(
        forwarded_arguments,
        serde_json::from_str::<Value>(replayed_arguments).unwrap()
    );
    assert_eq!(forwarded_arguments == accepted_arguments, !changed_value);
    assert!(!forwarded.to_string().contains("__hiroute_content_ref"));
    if !changed_value {
        input.extend(second["output"].as_array().unwrap().iter().cloned());
        input.push(json!({"role":"user","content":"actual B: design the next step"}));
        send(&input);
        assert_eq!(
            (regular.calls(), primary.calls(), classifier.calls()),
            (2, 1, 2)
        );
        let assessed = classifier_body(&classifier, 1);
        assert_eq!(assessed["history_partial"], false, "{assessed}");
        assert_eq!(assessed["preceding_stage"]["from"], 0, "{assessed}");
        assert_eq!(
            assessed["visible_conversation"].as_array().unwrap().len(),
            1
        );
        assert_eq!(assessed["visible_conversation"][0]["status"], "completed");
        assert_eq!(
            assessed["visible_conversation"][0]["steps"],
            json!([
                [{"kind":"tool_activity","tool":"tools.lookup","status":"unknown"}],
                [{"kind":"text","text":"ok"}]
            ])
        );
        assert!(!assessed.to_string().contains("private actual tool receipt"));
    }
    let facts = wait_execution_facts(&fixture, if changed_value { 2 } else { 3 });
    let decisions = facts
        .iter()
        .filter(|event| event["fact"]["kind"] == "route_decision")
        .collect::<Vec<_>>();
    assert_eq!(decisions.len(), if changed_value { 2 } else { 3 });
    assert_eq!(
        decisions[1]["fact"]["complexity"]["decision_source"],
        if changed_value {
            "external_classifier"
        } else {
            "inherited"
        }
    );
    assert!(
        decisions
            .iter()
            .all(|event| event["fact"]["complexity"]["fallback_used"] == false)
    );
    let members = json!([
        decisions[0]["correlation"]["request_id"],
        decisions[1]["correlation"]["request_id"]
    ]);
    let stage = facts.iter().find(|event| {
        event["fact"]["kind"] == "agent_turn_finished"
            && event["fact"]["history_membership"]["request_ids"] == members
    });
    if changed_value {
        assert!(
            stage.is_none(),
            "changed arguments cannot join both executions into A: {facts:?}"
        );
        return;
    }
    let stage = &stage.expect("both actual Regular requests finish one stage")["fact"];
    assert_eq!(stage["status"], "completed");
    assert_eq!(stage["history_partial"], false);
    assert_eq!(stage["history_membership"]["capture_partial"], false);
    assert_eq!(stage["history_membership"]["clipped"], false);
    let assessment = facts
        .iter()
        .find(|event| {
            event["fact"]["kind"] == "branch_assessment_recorded"
                && event["correlation"]["request_id"] == decisions[2]["correlation"]["request_id"]
        })
        .expect("the real B request assesses A's complete tool stage");
    assert_eq!(
        assessment["fact"]["history_membership"],
        stage["history_membership"]
    );
    assert_eq!(assessment["fact"]["partial"], false);
    assert_eq!(assessment["fact"]["score"], 0.75);
    let attempts = facts
        .iter()
        .filter(|event| event["fact"]["kind"] == "attempt_finished")
        .collect::<Vec<_>>();
    assert_eq!(attempts.len(), 3);
    assert!(
        attempts
            .iter()
            .all(|event| event["fact"]["provider_model_event"] == "response_complete")
    );
}

#[test]
fn real_listener_moderation_metadata_keeps_complete_history_and_assessment() {
    use serde_json::json;
    let notification = json!({"type":"response.metadata","response_id":"stream-ok","sequence_number":7,
        "metadata":{"openai_chatgpt_moderation_metadata":{
            "prompt":{"marker":"notification-only"},"generation":{},"tool_call":{},"tool_response":{}}}});
    let notification_wire = format!("event: response.metadata\ndata: {notification}\n\n");
    let wire = std::str::from_utf8(RESPONSES_STREAM_OK).unwrap().replace(
        "event: response.completed",
        &format!("{notification_wire}event: response.completed"),
    );
    assert_stream_keeps_complete_history_and_assessment(&wire, &notification_wire);
}

#[test]
fn real_listener_native_completion_ignores_object_order_and_transport_comments() {
    // This is the terminal message with reversed top-level and nested object keys.
    // Keeping the raw spelling exercises production serde_json/preserve_order too.
    let done = concat!(
        "event: response.output_item.done\n",
        r#"data: {"type":"response.output_item.done","output_index":0,"item":{"content":[{"annotations":[],"text":"ok","type":"output_text"}],"role":"assistant","status":"completed","id":"runtime-message","type":"message"}}"#,
        "\n\n"
    );
    let mut wire = std::str::from_utf8(RESPONSES_STREAM_OK).unwrap().replace(
        "event: response.completed",
        &format!("{done}event: response.completed"),
    );
    wire.push_str(": keepalive\n\ndata: [DONE]\n\n: final keepalive\n\n");
    assert_stream_keeps_complete_history_and_assessment(&wire, done);
}

#[test]
fn real_listener_native_eof_comment_keeps_complete_history_and_assessment() {
    for tail in [
        ": final keepalive",
        ": final keepalive\n",
        ": final keepalive\r",
    ] {
        let mut wire = std::str::from_utf8(RESPONSES_STREAM_OK).unwrap().to_owned();
        wire.push_str("data: [DONE]\n\n");
        wire.push_str(tail);
        // The shared listener assertions require the exact tail bytes, a complete
        // model event, and a full assessment of the captured execution next turn.
        assert_stream_keeps_complete_history_and_assessment(&wire, tail);
    }
}

fn assert_stream_keeps_complete_history_and_assessment(wire: &str, preserved: &str) {
    use serde_json::json;
    const ASSESSED_COMPLEX: &[u8] =
        br#"{"group":"primary","assessment":{"score":0.75,"partial":false}}"#;
    let regular = NativeProvider::start(vec![ProviderReply::StreamDrip {
        status: 200,
        chunks: wire.as_bytes().chunks(31).map(<[u8]>::to_vec).collect(),
        interval: Duration::ZERO,
    }]);
    let primary = NativeProvider::start(vec![ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }]);
    let classifier = NativeProvider::start(
        [CLASSIFIER_SIMPLE, ASSESSED_COMPLEX]
            .into_iter()
            .map(|body| ProviderReply::Complete {
                status: 200,
                error_kind: None,
                body,
            })
            .collect(),
    );
    let fixture = RuntimeFixture::launch_rest_classified_with_observation(
        &[&regular, &primary, &classifier],
        1,
        ObservationFaults::healthy(),
    );
    let send = |input, stream| {
        request(
            fixture.address,
            "POST",
            "/v1/responses",
            &[
                ("X-HiRoute-Token", TOKEN),
                ("session-id", "moderation-history"),
            ],
            &serde_json::to_vec(&json!({"model":MODEL,"input":input,"stream":stream})).unwrap(),
        )
    };
    let a = json!({"role":"user","content":"first actual task"});
    let first = send(json!([a.clone()]), true);
    assert_eq!(first.status, 200);
    assert!(
        first
            .body
            .windows(preserved.len())
            .any(|part| part == preserved.as_bytes())
    );
    let second = send(
        json!([a, {"role":"assistant","content":"ok"},
        {"role":"user","content":"second actual task"}]),
        false,
    );
    assert_eq!(second.status, 200);
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (1, 1, 2)
    );
    let decision = classifier_body(&classifier, 1);
    assert_eq!(
        decision["visible_conversation"].as_array().unwrap().len(),
        1,
        "{decision}"
    );
    assert_eq!(
        decision["visible_conversation"][0]["steps"],
        json!([[{"kind":"text","text":"ok"}]])
    );
    assert_eq!(
        decision["visible_conversation"][0]["status"], "completed",
        "{decision}"
    );
    assert_eq!(decision["history_partial"], false, "{decision}");
    assert_eq!(decision["preceding_stage"]["from"], 0, "{decision}");
    assert!(!decision.to_string().contains("notification-only"));
    let facts = wait_execution_facts(&fixture, 2);
    let attempts = facts
        .iter()
        .filter(|row| row["fact"]["kind"] == "attempt_finished")
        .collect::<Vec<_>>();
    assert_eq!(attempts.len(), 2, "{facts:?}");
    for attempt in attempts {
        assert_eq!(attempt["fact"]["provider_model_event"], "response_complete");
    }
    let assessments = facts
        .iter()
        .filter(|row| row["fact"]["kind"] == "branch_assessment_recorded")
        .collect::<Vec<_>>();
    assert_eq!(assessments.len(), 1, "{facts:?}");
    assert_eq!(assessments[0]["fact"]["partial"], false);
    assert_eq!(assessments[0]["fact"]["score"], 0.75);
    assert_eq!(
        assessments[0]["fact"]["history_membership"]["request_ids"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn real_listener_accepted_history_does_not_infer_missing_tool_result_status() {
    let simple = NativeProvider::start(vec![
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_TOOL_CALL,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
    ]);
    let complex = NativeProvider::start(Vec::new());
    let classifier = NativeProvider::start(
        (0..2)
            .map(|_| ProviderReply::Complete {
                status: 200,
                error_kind: None,
                body: CLASSIFIER_SIMPLE,
            })
            .collect(),
    );
    let fixture = RuntimeFixture::launch_rest_classified(&[&simple, &complex, &classifier], 2);
    let send = |input| {
        request(
            fixture.address,
            "POST",
            "/v1/responses",
            &[
                ("X-HiRoute-Token", "runtime-token"),
                ("session-id", "accepted-tools"),
            ],
            &serde_json::to_vec(
                &serde_json::json!({"model":"runtime-model", "input":input, "stream":false}),
            )
            .unwrap(),
        )
    };
    let user = serde_json::json!({"type":"message","role":"user","content":[{"type":"input_text","text":"look up the record"}]});
    let first = send(serde_json::json!([user.clone()]));
    assert_eq!(
        first.status,
        200,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    let output: serde_json::Value = serde_json::from_slice(&first.body).unwrap();
    let id = &output["output"][0]["call_id"];
    let second = send(serde_json::json!([
        user,
        {"type":"function_call","call_id":id,"namespace":"tools","name":"lookup","arguments":"{}"},
        {"type":"function_call_output","call_id":id,"output":"private tool payload"}
    ]));
    assert_eq!(
        second.status,
        200,
        "{}",
        String::from_utf8_lossy(&second.body)
    );
    assert_eq!(
        classifier.calls(),
        1,
        "tool result does not start a new turn"
    );
    assert_eq!(send(serde_json::json!([{"type":"message","role":"user","content":[{"type":"input_text","text":"next task after compaction"}]}])).status, 200);
    let body = classifier_body(&classifier, 1);
    assert_eq!(
        body["visible_conversation"][0]["steps"],
        serde_json::json!([
            [{"kind":"tool_activity","tool":"tools.lookup","status":"unknown"}],
            [{"kind":"text","text":"ok"}]
        ]),
        "{body}"
    );
    assert!(!body.to_string().contains("private tool payload"));
    assert!(!body.to_string().contains("secret"));
}

#[test]
fn real_listener_accepted_history_keeps_interrupted_stream_prefix() {
    const PREFIX: &[u8] = b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"partial\",\"model\":\"runtime-native\"}}\n\nevent: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"item_id\":\"message\",\"output_index\":0,\"content_index\":0,\"delta\":\"accepted prefix\"}\n\n";
    let simple = NativeProvider::start(vec![
        ProviderReply::StreamThenClose {
            body: PREFIX,
            declared_length: PREFIX.len() + 1000,
        },
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: RESPONSES_OK,
        },
    ]);
    let complex = NativeProvider::start(Vec::new());
    let classifier = NativeProvider::start(
        (0..2)
            .map(|_| ProviderReply::Complete {
                status: 200,
                error_kind: None,
                body: CLASSIFIER_SIMPLE,
            })
            .collect(),
    );
    let fixture = RuntimeFixture::launch_rest_classified(&[&simple, &complex, &classifier], 2);
    let send = |input: &str, stream| {
        request(
            fixture.address,
            "POST",
            "/v1/responses",
            &[
                ("X-HiRoute-Token", "runtime-token"),
                ("session-id", "accepted-partial"),
            ],
            &serde_json::to_vec(
                &serde_json::json!({"model":"runtime-model", "input":input, "stream":stream}),
            )
            .unwrap(),
        )
    };
    let first = send("first task", true);
    assert_eq!(first.status, 200);
    assert!(String::from_utf8_lossy(&first.body).contains("accepted prefix"));
    assert_eq!(send("next task", false).status, 200);
    let body = classifier_body(&classifier, 1);
    assert_eq!(
        body["visible_conversation"][0]["steps"],
        serde_json::json!([[{"kind":"text","text":"accepted prefix"}]]),
        "{body}"
    );
    assert_eq!(body["visible_conversation"][0]["status"], "interrupted");
    assert_eq!(body["history_partial"], true);
}

#[test]
fn real_listener_accepted_history_survives_rewrite_and_compaction_without_observation() {
    for stream in [false, true] {
        let simple = NativeProvider::start(
            (0..3)
                .map(|_| {
                    if stream {
                        ProviderReply::StreamComplete {
                            status: 200,
                            body: RESPONSES_STREAM_OK,
                        }
                    } else {
                        ProviderReply::Complete {
                            status: 200,
                            error_kind: None,
                            body: RESPONSES_OK,
                        }
                    }
                })
                .collect(),
        );
        let complex = NativeProvider::start(Vec::new());
        let classifier = NativeProvider::start(
            (0..3)
                .map(|_| ProviderReply::Complete {
                    status: 200,
                    error_kind: None,
                    body: CLASSIFIER_SIMPLE,
                })
                .collect(),
        );
        let fixture = RuntimeFixture::launch_rest_classified_with_diagnostics(
            &[&simple, &complex, &classifier],
            2,
        );
        assert!(fixture.observation_root.is_none());
        let message = |role: &str, value: &str| {
            serde_json::json!({
                "type": "message", "role": role, "content": [{
                    "type": if role == "user" { "input_text" } else { "output_text" }, "text": value,
                }],
            })
        };
        for input in [
            serde_json::json!([message("user", "first")]),
            serde_json::json!([
                message("user", "first"),
                message("assistant", "fabricated"),
                message("user", "second")
            ]),
            serde_json::json!([message("user", "third after compaction")]),
        ] {
            let response = request(fixture.address, "POST", "/v1/responses", &[
                ("X-HiRoute-Token", "runtime-token"), ("session-id", "accepted-history"),
            ], &serde_json::to_vec(&serde_json::json!({ "model": "runtime-model", "input": input, "stream": stream })).unwrap());
            assert_eq!(
                response.status,
                200,
                "stream={stream}, body={}",
                String::from_utf8_lossy(&response.body)
            );
        }
        assert_eq!(
            (simple.calls(), complex.calls(), classifier.calls()),
            (3, 0, 3)
        );
        let requests = classifier.requests();
        for (index, raw) in requests.iter().enumerate().skip(1) {
            let start = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
            let body: serde_json::Value = serde_json::from_slice(&raw[start..]).unwrap();
            let turns = body["visible_conversation"].as_array().unwrap();
            assert_eq!(turns.len(), index);
            assert_eq!(body["history_partial"], true);
            for turn in turns {
                assert_eq!(
                    turn["steps"],
                    serde_json::json!([[{"kind":"text","text":"ok"}]]),
                    "stream={stream}, wire={body}"
                );
            }
        }
    }
}
