use super::*;

fn classifier_body(classifier: &NativeProvider, index: usize) -> serde_json::Value {
    let requests = classifier.requests();
    let raw = &requests[index];
    let start = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    serde_json::from_slice(&raw[start..]).unwrap()
}

#[test]
fn real_listener_moderation_metadata_keeps_complete_history_and_assessment() {
    use serde_json::json;
    const ASSESSED_COMPLEX: &[u8] = br#"{"decision":{"kind":"ordinal","probabilities":{"simple":0.1,"complex":0.9}},"assessment":{"score":0.75,"partial":false}}"#;
    let notification = json!({"type":"response.metadata","response_id":"stream-ok","sequence_number":7,
        "metadata":{"openai_chatgpt_moderation_metadata":{
            "prompt":{"marker":"notification-only"},"generation":{},"tool_call":{},"tool_response":{}}}});
    let notification_wire = format!("event: response.metadata\ndata: {notification}\n\n");
    let wire = std::str::from_utf8(RESPONSES_STREAM_OK).unwrap().replace(
        "event: response.completed",
        &format!("{notification_wire}event: response.completed"),
    );
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
            .windows(notification_wire.len())
            .any(|part| part == notification_wire.as_bytes())
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
    assert_eq!(decision["assessment_target"]["from"], 0, "{decision}");
    assert!(!decision.to_string().contains("notification-only"));
    let facts = wait_execution_facts(&fixture, 2);
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
