//! Controlled providers force the native phase that previously made Regular permanently
//! ineligible. Controlled providers and frozen installed-producer output are distinct
//! evidence from a live DSH UI/paid-provider acceptance run.
use super::*;
use hiroute_gateway::server::request_plan::IngressProtocol;

const CHAT_OK: &[u8] = br#"{"id":"regular-ok","model":"regular-chat","choices":[{"index":0,"message":{"role":"assistant","content":"regular answer"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;
const PRIMARY_TOOLS: &[u8] = br#"{"id":"primary-tools","model":"primary-responses","status":"completed","output":[{"type":"message","id":"msg-commentary","role":"assistant","phase":"commentary","status":"completed","content":[{"type":"output_text","text":"I will check the record."}]},{"type":"function_call","id":"fc_lookup","call_id":"lookup-call","name":"lookup","arguments":"{\"key\":\"alpha\"}","status":"completed"},{"type":"function_call","id":"fc_check","call_id":"check-call","name":"check","arguments":"{\"key\":\"beta\"}","status":"completed"}],"usage":{"input_tokens":5,"output_tokens":4,"total_tokens":9}}"#;
const PRIMARY_FINAL: &[u8] = br#"{"id":"primary-final","model":"primary-responses","status":"completed","output":[{"type":"message","id":"msg-final","role":"assistant","phase":"final_answer","status":"completed","content":[{"type":"output_text","text":"Both records are valid."}]}],"usage":{"input_tokens":9,"output_tokens":3,"total_tokens":12}}"#;
const SIMPLE: &[u8] = br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}}}}"#;
const COMPLEX_WITH_ASSESSMENT: &[u8] = br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.1,"1":0.9}},"q1":{"type":"score","score":1.8},"q2":{"type":"choice","probabilities":{"applicable":0.9,"not_established":0.1}}}}"#;
const SIMPLE_WITH_ASSESSMENT: &[u8] = br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}},"q1":{"type":"score","score":1.8},"q2":{"type":"choice","probabilities":{"applicable":0.9,"not_established":0.1}}}}"#;

#[test]
fn completed_responses_commentary_returns_to_regular_chat_with_tools_and_assessment() {
    assert_return_to_chat(false, false);
}

#[test]
fn installed_dsh_commentary_replay_returns_to_chat_after_restart_over_sse() {
    assert_return_to_chat(true, true);
}

fn assert_return_to_chat(restart: bool, stream: bool) {
    let replay: Value = serde_json::from_str(include_str!(
        "../../fixtures/dsh-commentary/replayed-input.json"
    ))
    .unwrap();
    assert_eq!(replay["network_calls"], 0);
    assert_eq!(replay["replay_degraded"], 0);
    let replay_items = replay["input"].as_array().unwrap();
    let regular = NativeProvider::start(vec![
        provider_reply(CHAT_OK, stream, true);
        if restart { 4 } else { 2 }
    ]);
    let primary = NativeProvider::start(vec![
        provider_reply(PRIMARY_TOOLS, stream, false),
        provider_reply(PRIMARY_FINAL, stream, false),
    ]);
    let mut decisions = vec![
        reply(SIMPLE),
        reply(COMPLEX_WITH_ASSESSMENT),
        reply(SIMPLE_WITH_ASSESSMENT),
    ];
    if restart {
        decisions.push(reply(SIMPLE));
        decisions.push(reply(SIMPLE_WITH_ASSESSMENT));
    }
    let decider = NativeProvider::start(decisions);
    let mut fixture = RuntimeFixture::launch_builtin_branches_with_diagnostics(
        &[&regular, &primary, &decider],
        true,
    );
    let mut publication = chat_candidate_publication(&fixture, &regular);
    enable_c_in_publication(&mut publication);
    fixture.publish(&mut publication);
    let send = |fixture: &RuntimeFixture, input: &[Value], prior_stage_known: bool| {
        let response = request(
            fixture.address,
            "POST",
            "/v1/responses",
            &[("X-HiRoute-Token", TOKEN), ("session-id", "phase-return")],
            &serde_json::to_vec(&json!({"model":MODEL,"stream":stream,"input":input,
            "tools":[
                {"type":"function","name":"lookup","parameters":{"type":"object"}},
                {"type":"function","name":"check","parameters":{"type":"object"}}
            ]}))
            .unwrap(),
        );
        let final_user = input.last().and_then(|item| item.get("content"));
        if prior_stage_known
            && final_user.is_some_and(|content| {
                content.as_str() == Some("Now answer a simple question.")
                    || content.pointer("/0/text").and_then(Value::as_str)
                        == Some("Now answer a simple question.")
            })
        {
            let facts = wait_execution_facts(fixture, 4);
            let decision = facts
                .iter()
                .rfind(|fact| fact["fact"]["kind"] == "route_decision")
                .unwrap();
            let complexity = &decision["fact"]["complexity"];
            assert_eq!(
                complexity["execution_group"], "regular",
                "the target failure must not be Primary protection"
            );
            assert_eq!(complexity["selection_reason"], "simple_task");
            assert_eq!(complexity["simple_probability"], 0.9);
            assert_eq!(complexity["routing_observation"]["score"], 0.9);
            assert!(complexity["competence_trigger"].is_null());
            assert_ne!(
                complexity["routing_observation"]["protection_action"],
                "applied"
            );
            if response.status != 200 {
                assert!(
                    facts.iter().any(|fact| fact["correlation"]["request_id"]
                        == decision["correlation"]["request_id"]
                        && fact["fact"]["kind"] == "candidate_decision"
                        && fact["fact"]["upstream_protocol"] == "chat_completions"
                        && fact["fact"]["exclusion_reason"] == "PROTOCOL_PATH_UNAVAILABLE"),
                    "the old candidate must fail at Chat history projection"
                );
            }
        }
        assert_eq!(
            response.status,
            200,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        if stream {
            String::from_utf8(response.body)
                .unwrap()
                .lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .find(|event| event["type"] == "response.completed")
                .expect("full Responses stream terminal")["response"]
                .clone()
        } else {
            serde_json::from_slice::<Value>(&response.body).unwrap()
        }
    };
    // Exceeds the production Replay memory threshold, forcing external strings.
    let first_user = "starting context ".repeat(8192);
    let mut input = vec![json!({"role":"user","content":first_user})];
    let first = send(&fixture, &input, true);
    assert_eq!(
        (regular.calls(), primary.calls(), decider.calls()),
        (1, 0, 1)
    );
    input.extend(first["output"].as_array().unwrap().iter().cloned());
    input.push(replay_items[0].clone());
    let tools = send(&fixture, &input, true);
    assert_eq!(
        (regular.calls(), primary.calls(), decider.calls()),
        (1, 1, 2)
    );
    assert_eq!(tools["output"].as_array().unwrap().len(), 3);
    assert_eq!(tools["output"][0]["phase"], "commentary");
    assert_eq!(tools["output"][0]["id"], "msg-commentary");
    for (actual, restored) in tools["output"]
        .as_array()
        .unwrap()
        .iter()
        .zip(&replay_items[1..4])
    {
        for field in ["type", "id", "call_id", "name", "arguments", "phase"] {
            assert_eq!(
                actual.get(field),
                restored.get(field),
                "actual producer keeps {field}"
            );
        }
    }
    input.extend(replay_items[1..6].iter().cloned());
    let final_answer = send(&fixture, &input, true);
    assert_eq!(
        (regular.calls(), primary.calls(), decider.calls()),
        (1, 2, 2)
    );
    let primary_request: Value = serde_json::from_slice(http_body(&primary.requests()[1])).unwrap();
    assert_eq!(
        primary_request["input"],
        json!(input),
        "native replay keeps phases and identities"
    );
    assert_eq!(final_answer["output"][0]["phase"], replay_items[6]["phase"]);
    assert_eq!(final_answer["output"][0]["id"], replay_items[6]["id"]);
    assert_eq!(
        final_answer["output"][0]["content"][0]["text"],
        replay_items[6]["content"][0]["text"]
    );
    input.extend(replay_items[6..].iter().cloned());
    let original = input.clone();
    let last = send(&fixture, &input, true);
    assert_eq!(last["output"][0]["content"][0]["text"], "regular answer");
    assert_eq!(
        (regular.calls(), primary.calls(), decider.calls()),
        (2, 2, 3),
        "Regular must execute again; Primary success cannot substitute for this assertion"
    );
    assert_eq!(input, original);
    let chat: Value = serde_json::from_slice(http_body(&regular.requests()[1])).unwrap();
    let messages = chat["messages"].as_array().unwrap();
    assert_eq!(messages[0]["content"], first_user);
    assert_eq!(
        messages[3],
        json!({"role":"assistant","content":"I will check the record."})
    );
    assert_eq!(
        messages[4],
        json!({"role":"assistant","tool_calls":[
            {"id":"lookup-call","type":"function","function":{"name":"lookup","arguments":"{\"key\":\"alpha\"}"}},
            {"id":"check-call","type":"function","function":{"name":"check","arguments":"{\"key\":\"beta\"}"}}
        ]})
    );
    assert_eq!(
        messages[5],
        json!({"role":"tool","tool_call_id":"lookup-call","content":"alpha=valid"})
    );
    assert_eq!(
        messages[6],
        json!({"role":"tool","tool_call_id":"check-call","content":"beta=valid"})
    );
    assert_eq!(
        messages[7],
        json!({"role":"assistant","content":"Both records are valid."})
    );
    assert_eq!(
        messages[8],
        json!({"role":"user","content":"Now answer a simple question."})
    );
    let facts = wait_execution_facts(&fixture, 4);
    let decisions = facts
        .iter()
        .filter(|f| f["fact"]["kind"] == "route_decision")
        .collect::<Vec<_>>();
    assert_eq!(decisions.len(), 4);
    assert!(
        decisions
            .iter()
            .all(|f| f["fact"]["complexity"]["fallback_used"] == false)
    );
    let tools_membership = json!([
        decisions[1]["correlation"]["request_id"],
        decisions[2]["correlation"]["request_id"]
    ]);
    let assessment = facts
        .iter()
        .find(|f| {
            f["fact"]["kind"] == "branch_assessment_recorded"
                && f["correlation"]["request_id"] == decisions[3]["correlation"]["request_id"]
        })
        .expect("the new Regular request assesses the Primary tool stage");
    assert_eq!(
        assessment["fact"]["history_membership"]["request_ids"],
        tools_membership
    );
    assert_eq!(assessment["fact"]["partial"], false);
    let deadline = Instant::now() + Duration::from_secs(5);
    let original_projection_token = loop {
        let log = std::fs::read_to_string(
            fixture
                .diagnostics_root
                .as_ref()
                .unwrap()
                .join("daemon/current.jsonl"),
        )
        .unwrap();
        let projections: Vec<Value> = log
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter_map(|record| record["event"].get("request_projection").cloned())
            .collect();
        if !projections.is_empty() {
            assert_eq!(
                projections.len(),
                1,
                "preview and unselected candidates must not report phase loss"
            );
            let projection = &projections[0];
            assert_eq!(projection["outcome"], "declared_loss");
            assert_eq!(projection["stage"], "attempt_materialization");
            assert_eq!(projection["reason"], "message_phase_omitted");
            assert_eq!(projection["first_message_index"], 3);
            assert_eq!(projection["affected_items"], 1);
            assert!(!projection["binding_token"].is_null());
            assert!(!log.contains("I will check the record."));
            assert!(!log.contains("message_phase_omitted\",\"fields_removed"));
            break projection["request_token"].clone();
        }
        assert!(
            Instant::now() < deadline,
            "actual projection diagnostic missing"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    if restart {
        fixture.restart_with_publication(&publication);
        let resumed = send(&fixture, &input, false);
        assert_eq!(resumed["output"][0]["content"][0]["text"], "regular answer");
        assert_eq!(
            (regular.calls(), primary.calls(), decider.calls()),
            (3, 2, 4)
        );
        let repeated: Value = serde_json::from_slice(http_body(&regular.requests()[2])).unwrap();
        assert_eq!(
            repeated, chat,
            "restarted projection uses the same supplied history and tool identities"
        );
        let facts = wait_execution_facts(&fixture, 5);
        let decision = facts
            .iter()
            .rfind(|event| event["fact"]["kind"] == "route_decision")
            .unwrap();
        assert_eq!(decision["fact"]["complexity"]["execution_group"], "regular");
        assert_eq!(decision["fact"]["complexity"]["fallback_used"], false);
        assert_eq!(
            decision["fact"]["complexity"]["decision_source"],
            "external_classifier"
        );
        // The standalone process has no persisted accepted-stage store. Visible client
        // replay remains portable, without fabricating a recovered assessment target.
        assert!(decision["fact"]["complexity"]["routing_observation"]["score"].is_null());
        assert!(!facts.iter().any(
            |event| event["fact"]["kind"] == "branch_assessment_recorded"
                && event["correlation"]["request_id"] == decision["correlation"]["request_id"]
        ));
        let resumed_request_id = decision["correlation"]["request_id"].clone();
        input.extend(resumed["output"].as_array().unwrap().iter().cloned());
        input.push(
            json!({"role":"user","content":"After restart, assess that answer and greet again."}),
        );
        let after = send(&fixture, &input, false);
        assert_eq!(after["output"][0]["content"][0]["text"], "regular answer");
        assert_eq!(
            (regular.calls(), primary.calls(), decider.calls()),
            (4, 2, 5)
        );
        let facts = wait_execution_facts(&fixture, 6);
        let after_decision = facts
            .iter()
            .rfind(|event| event["fact"]["kind"] == "route_decision")
            .unwrap();
        let after_id = &after_decision["correlation"]["request_id"];
        assert_eq!(
            after_decision["fact"]["complexity"]["execution_group"],
            "regular"
        );
        assert_eq!(after_decision["fact"]["complexity"]["fallback_used"], false);
        assert_eq!(
            after_decision["fact"]["complexity"]["routing_observation"]["score"],
            0.9
        );
        assert_eq!(
            after_decision["fact"]["complexity"]["routing_observation"]["target_complete"],
            true
        );
        let resumed_attempt = facts
            .iter()
            .find(|event| {
                event["fact"]["kind"] == "attempt_started"
                    && event["correlation"]["request_id"] == resumed_request_id
            })
            .unwrap();
        assert!(
            facts
                .iter()
                .any(|event| event["fact"]["kind"] == "request_finished"
                    && event["correlation"]["request_id"] == resumed_request_id
                    && event["fact"]["outcome"] == "accepted")
        );
        let assessment = facts
            .iter()
            .find(|event| {
                event["fact"]["kind"] == "branch_assessment_recorded"
                    && &event["correlation"]["request_id"] == after_id
            })
            .expect("the next new user assesses the actual accepted post-restart request");
        assert_eq!(
            assessment["fact"]["history_membership"]["request_ids"],
            json!([resumed_request_id])
        );
        assert_eq!(
            assessment["fact"]["history_membership"]["capture_partial"],
            false
        );
        assert_eq!(assessment["fact"]["partial"], false);
        assert_eq!(assessment["fact"]["score"], 0.9);
        for field in ["profile_digest", "model_configuration_id"] {
            assert_eq!(
                assessment["fact"][field], resumed_attempt["fact"][field],
                "assessment keeps actual post-restart {field}"
            );
        }
        assert_eq!(
            resumed_attempt["fact"]["upstream_protocol"],
            "chat_completions"
        );
        assert_eq!(
            resumed_attempt["fact"]["request_model"],
            publication.aliases[0].candidates[0]
                .upstream_model_id
                .as_str()
        );
        assert_eq!(
            resumed_attempt["fact"]["stable_binding_id"],
            publication.aliases[0].candidates[0]
                .stable_target_key
                .as_str()
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let log = std::fs::read_to_string(
                fixture
                    .diagnostics_root
                    .as_ref()
                    .unwrap()
                    .join("daemon/current.jsonl"),
            )
            .unwrap();
            if projection_records(&log).iter().any(|event| {
                event["request_token"] != original_projection_token
                    && event["stage"] == "attempt_materialization"
                    && event["reason"] == "message_phase_omitted"
            }) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "restarted materialization diagnostic missing"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn provider_reply(document: &'static [u8], stream: bool, chat: bool) -> ProviderReply {
    if !stream {
        return reply(document);
    }
    let document: Value = serde_json::from_slice(document).unwrap();
    let mut events = Vec::new();
    if chat {
        let choice = &document["choices"][0];
        events.push(json!({"id":document["id"],"model":document["model"],"choices":[{"index":0,"delta":choice["message"],"finish_reason":null}]}));
        events.push(json!({"id":document["id"],"model":document["model"],"choices":[{"index":0,"delta":{},"finish_reason":choice["finish_reason"]}]}));
        events.push(json!({"id":document["id"],"model":document["model"],"choices":[],"usage":document["usage"]}));
    } else {
        events.push(json!({"type":"response.created","response":{"id":document["id"],"model":document["model"]}}));
        for (index, item) in document["output"].as_array().unwrap().iter().enumerate() {
            let mut added = item.clone();
            added["status"] = json!("in_progress");
            if item["type"] == "message" {
                added["content"] = json!([]);
            } else {
                added["arguments"] = json!("");
            }
            events.push(
                json!({"type":"response.output_item.added","output_index":index,"item":added}),
            );
            if item["type"] == "message" {
                events.push(json!({"type":"response.output_text.delta","output_index":index,"content_index":0,"item_id":item["id"],"delta":item["content"][0]["text"]}));
            } else {
                events.push(json!({"type":"response.function_call_arguments.delta","output_index":index,"item_id":item["id"],"delta":item["arguments"]}));
            }
            events
                .push(json!({"type":"response.output_item.done","output_index":index,"item":item}));
        }
        events.push(json!({"type":"response.completed","response":document}));
    }
    let mut chunks: Vec<Vec<u8>> = events
        .iter()
        .map(|event| format!("data: {event}\n\n").into_bytes())
        .collect();
    if chat {
        chunks.push(b"data: [DONE]\n\n".to_vec());
    }
    ProviderReply::StreamDrip {
        status: 200,
        chunks,
        interval: Duration::ZERO,
    }
}

fn http_body(request: &[u8]) -> &[u8] {
    let offset = request
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .unwrap()
        + 4;
    &request[offset..]
}

#[test]
fn unsupported_phase_control_and_unfinished_history_exclude_chat_with_safe_reasons() {
    let regular = NativeProvider::start(vec![]);
    let primary = NativeProvider::start(vec![reply(RESPONSES_OK); 3]);
    let decider = NativeProvider::start(vec![reply(SIMPLE); 3]);
    let fixture = RuntimeFixture::launch_builtin_branches_with_diagnostics(
        &[&regular, &primary, &decider],
        true,
    );
    let mut publication = chat_candidate_publication(&fixture, &regular);
    publication.publication_revision += 1;
    fixture.publish(&mut publication);
    for (index, (field, value)) in [
        ("phase", json!("future_phase")),
        ("status", json!("in_progress")),
        (
            "unknown_control",
            json!({"resume":"private-control-marker"}),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut assistant = json!({"type":"message","role":"assistant","phase":"commentary","status":"completed","content":[{"type":"output_text","text":"private-history-marker"}]});
        assistant[field] = value;
        let response = request(fixture.address, "POST", "/v1/responses",
            &[("X-HiRoute-Token",TOKEN),("session-id",&format!("negative-phase-{index}"))],
            &serde_json::to_vec(&json!({"model":MODEL,"stream":false,"input":[
                {"role":"user","content":"prior user"},assistant,{"role":"user","content":"next user"}
            ]})).unwrap());
        assert_eq!(
            response.status,
            200,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
    }
    assert_eq!(
        (regular.calls(), primary.calls(), decider.calls()),
        (0, 3, 3)
    );
    let facts = wait_execution_facts(&fixture, 3);
    let exclusions = facts
        .iter()
        .filter(|event| {
            event["fact"]["kind"] == "candidate_decision"
                && event["fact"]["upstream_protocol"] == "chat_completions"
        })
        .collect::<Vec<_>>();
    assert_eq!(exclusions.len(), 3);
    assert!(
        exclusions
            .iter()
            .all(|event| event["fact"]["exclusion_reason"] == "PROTOCOL_PATH_UNAVAILABLE")
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let log = std::fs::read_to_string(
            fixture
                .diagnostics_root
                .as_ref()
                .unwrap()
                .join("daemon/current.jsonl"),
        )
        .unwrap();
        let projections = projection_records(&log);
        if projections.len() >= 3 {
            assert_eq!(projections.len(), 3);
            assert_eq!(
                projections
                    .iter()
                    .map(|event| event["reason"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                [
                    "native_semantics_unsupported",
                    "item_lifecycle_unsupported",
                    "native_semantics_unsupported"
                ]
            );
            assert!(
                projections
                    .iter()
                    .all(|event| event["outcome"] == "unsupported"
                        && event["stage"] == "candidate_preflight")
            );
            assert_eq!(projections[1]["first_message_index"], 1);
            assert_eq!(projections[1]["affected_items"], 1);
            for event in [&projections[0], &projections[2]] {
                assert!(event["first_message_index"].is_null());
                assert!(event["affected_items"].is_null());
            }
            assert!(
                !log.contains("private-history-marker") && !log.contains("private-control-marker")
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "closed request projection causes missing"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn concurrent_frozen_dsh_replays_keep_independent_chat_projections() {
    let regular = NativeProvider::start(vec![reply(CHAT_OK); 2]);
    let primary = NativeProvider::start(vec![]);
    let decider = NativeProvider::start(vec![reply(SIMPLE); 2]);
    let fixture = RuntimeFixture::launch_builtin_branches(&[&regular, &primary, &decider], true);
    let mut publication = chat_candidate_publication(&fixture, &regular);
    publication.publication_revision += 1;
    fixture.publish(&mut publication);
    let producer: Value = serde_json::from_str(include_str!(
        "../../fixtures/dsh-commentary/replayed-input.json"
    ))
    .unwrap();
    let input = producer["input"].clone();
    let body = serde_json::to_vec(&json!({"model":MODEL,"stream":false,"input":input,
    "tools":[
        {"type":"function","name":"lookup","parameters":{"type":"object"}},
        {"type":"function","name":"check","parameters":{"type":"object"}}
    ]}))
    .unwrap();
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    let response = request(
                        fixture.address,
                        "POST",
                        "/v1/responses",
                        &[
                            ("X-HiRoute-Token", TOKEN),
                            ("session-id", "concurrent-phase-replay"),
                        ],
                        &body,
                    );
                    assert_eq!(
                        response.status,
                        200,
                        "{}",
                        String::from_utf8_lossy(&response.body)
                    );
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    });
    assert_eq!((regular.calls(), primary.calls()), (2, 0));
    let requests = regular.requests();
    for native in requests {
        let projected: Value = serde_json::from_slice(http_body(&native)).unwrap();
        assert_eq!(
            projected["messages"][1]["content"],
            "I will check the record."
        );
        assert_eq!(
            projected["messages"][2]["tool_calls"][0]["id"],
            "lookup-call"
        );
        assert_eq!(projected["messages"][3]["tool_call_id"], "lookup-call");
        assert_eq!(projected["messages"][4]["tool_call_id"], "check-call");
    }
    assert_eq!(input, producer["input"]);
    let facts = wait_execution_facts(&fixture, 2);
    let decisions = facts
        .iter()
        .filter(|event| event["fact"]["kind"] == "route_decision")
        .collect::<Vec<_>>();
    assert_eq!(decisions.len(), 2);
    assert!(
        decisions.iter().all(
            |event| event["fact"]["complexity"]["execution_group"] == "regular"
                && event["fact"]["complexity"]["fallback_used"] == false
        )
    );
}

fn projection_records(log: &str) -> Vec<Value> {
    log.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|record| record["event"].get("request_projection").cloned())
        .collect()
}

fn chat_candidate_publication(
    fixture: &RuntimeFixture,
    regular: &NativeProvider,
) -> hiroute_gateway::server::publication::GatewayPublicationSnapshotV3 {
    let mut publication = fixture.publication();
    let candidate = &mut publication.aliases[0].candidates[0];
    *candidate = hiroute_e2e::gateway_fixture::sealed_native_candidate(
        candidate.local_id,
        &candidate.stable_target_key,
        &candidate.credential_refs,
        regular.authority(),
        &candidate.upstream_model_id,
        &[
            (IngressProtocol::Responses, IngressProtocol::ChatCompletions),
            (
                IngressProtocol::ChatCompletions,
                IngressProtocol::ChatCompletions,
            ),
            (IngressProtocol::Messages, IngressProtocol::ChatCompletions),
        ],
    );
    publication
}
