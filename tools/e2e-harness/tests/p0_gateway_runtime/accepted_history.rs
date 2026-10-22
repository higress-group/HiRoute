use super::*;

fn classifier_body(classifier: &NativeProvider, index: usize) -> serde_json::Value {
    let requests = classifier.requests();
    let raw = &requests[index];
    let start = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    serde_json::from_slice(&raw[start..]).unwrap()
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
