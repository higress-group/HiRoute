//! Stop loses an execution edge, not an unchanged request's new input boundary.
use super::native_encrypted_continuation::{message, streamed};
use super::native_history::{classifier_body, reply};
use super::*;
use hiroute_domain::RoutingTurnBoundaryV1;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Read;

const SESSION: &str = "interrupted-input-boundary";
const A_USER: &str = "A: inspect the first fixture";
const A_TEXT: &str = "I found the fixture and will read its details.";
const B_USER: &str = "B: stop that task and independently read the next fixture";
const C_USER: &str = "C: assess the completed independent task";

fn encode(events: &[Value]) -> Vec<u8> {
    events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect::<String>()
        .into_bytes()
}

fn events(wire: &[u8]) -> Vec<Value> {
    std::str::from_utf8(wire)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str(data).unwrap())
        .collect()
}

fn tool(id: &str) -> Value {
    json!({"type":"function_call","id":format!("fc-{id}"),"call_id":id,
        "name":"read","arguments":json!({"path":"fixture/next.txt"}).to_string(),
        "status":"completed"})
}

fn interrupted_reply() -> (ProviderReply, Vec<u8>) {
    let ProviderReply::StreamDrip { chunks, .. } = streamed(
        "a-interrupted",
        vec![
            message("a-text", "commentary", A_TEXT),
            tool("a-unfinished"),
        ],
    ) else {
        unreachable!()
    };
    let mut prefix = events(&chunks.concat());
    let last = prefix
        .iter()
        .position(|event| event["type"] == "response.function_call_arguments.delta")
        .unwrap();
    prefix.truncate(last + 1);
    prefix[last]["delta"] = json!("{\"path\":\"fixture/");
    let upstream = encode(&prefix);
    // The requested public alias is the only intentional native wire rewrite.
    for event in &mut prefix {
        if let Some(model) = event.pointer_mut("/response/model") {
            assert_eq!(*model, "runtime-native");
            *model = json!(MODEL);
        }
    }
    (
        ProviderReply::StreamDrip {
            status: 200,
            chunks: vec![upstream, b": pending tool arguments\n\n".to_vec()],
            interval: Duration::from_secs(2),
        },
        encode(&prefix),
    )
}

fn input_headers(input: &[Value], turn: &str) -> Vec<(String, String)> {
    let hashes = input
        .iter()
        .filter(|item| item["role"] == "user")
        .map(|item| {
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&[item["content"].as_str().unwrap()]).unwrap())
            )
        })
        .collect::<Vec<_>>();
    let last = hashes.len() - 1;
    vec![
        ("X-HiRoute-Token".into(), TOKEN.into()),
        ("x-hiroute-agent".into(), "dsh".into()),
        ("x-deepseek-harness-session-id".into(), SESSION.into()),
        ("x-hiroute-routing-context".into(), json!({
            "version":2,"agent":"dsh","session_id":SESSION,"purpose":"main","turn_id":turn,
            "input":{"kind":"native_messages","event_id":turn,"coverage":"window",
                "messages":[{"index":last,"sha256":hashes[last]}],
                "scope_digest":format!("{:x}",Sha256::digest(serde_json::to_vec(&hashes).unwrap()))}
        }).to_string()),
    ]
}

fn body(input: &[Value]) -> Vec<u8> {
    serde_json::to_vec(&json!({"model":MODEL,"stream":true,"input":input,
        "tools":[{"type":"function","name":"read","parameters":{"type":"object",
            "properties":{"path":{"type":"string"}}}}]}))
    .unwrap()
}

fn cancel_after_partial_tool(fixture: &RuntimeFixture, input: &[Value], expected: &[u8]) -> Value {
    let headers = input_headers(input, "a");
    let mut client = open_request(
        fixture.address,
        "POST",
        "/v1/responses",
        &headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect::<Vec<_>>(),
        &body(input),
    );
    client
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut received = Vec::new();
    let mut buffer = [0u8; 4096];
    let delivered = loop {
        let count = client.read(&mut buffer).unwrap();
        assert!(
            count > 0,
            "provider ended before the client could stop its unfinished tool"
        );
        received.extend_from_slice(&buffer[..count]);
        if let Some(split) = received.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            assert_eq!(
                wire_header(&received, "transfer-encoding").as_deref(),
                Some("chunked")
            );
            let (delivered, complete) =
                hiroute_e2e::gateway_fixture::decode_chunked_response_body(&received[split + 4..])
                    .unwrap();
            assert!(
                !complete,
                "unfinished tool response must not complete its HTTP entity"
            );
            if delivered.len() >= expected.len() {
                assert_eq!(
                    delivered, expected,
                    "all accepted bytes except the public alias stay exact"
                );
                break events(&delivered);
            }
        }
    };
    assert!(
        std::str::from_utf8(&received)
            .unwrap()
            .starts_with("HTTP/1.1 200")
    );
    assert!(
        delivered
            .iter()
            .any(|e| e["type"] == "response.output_text.done")
    );
    assert_eq!(
        delivered.last().unwrap()["type"],
        "response.function_call_arguments.delta"
    );
    assert!(!delivered.iter().any(|e| matches!(
        e["type"].as_str(),
        Some("response.function_call_arguments.done" | "response.completed")
    )));
    assert!(delivered.iter().any(
        |e| e["type"] == "response.output_item.added" && e["item"]["call_id"] == "a-unfinished"
    ));
    assert!(
        !delivered
            .iter()
            .any(|e| e["type"] == "response.output_item.done"
                && e["item"]["type"] == "function_call")
    );
    client.shutdown(std::net::Shutdown::Both).unwrap();
    drop(client);
    // Replay only the actual completed text. The started but unfinished call is
    // deliberately absent from the next request, as after a native client Stop.
    let text = delivered
        .iter()
        .find(|e| e["type"] == "response.output_item.done" && e["item"]["type"] == "message")
        .unwrap();
    assert_eq!(text["item"]["content"][0]["text"], A_TEXT);
    text["item"].clone()
}

fn send(fixture: &RuntimeFixture, input: &[Value], turn: &str) -> Vec<Value> {
    let headers = input_headers(input, turn);
    let response = request(
        fixture.address,
        "POST",
        "/v1/responses",
        &headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect::<Vec<_>>(),
        &body(input),
    );
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert!(
        response.http_entity_complete,
        "successful later stage must finish its HTTP entity"
    );
    let events = events(&response.body);
    assert!(!events.iter().any(|e| matches!(
        e["type"].as_str(),
        Some("error" | "response.failed" | "response.incomplete")
    )));
    let completed = events
        .iter()
        .filter(|e| e["type"] == "response.completed")
        .collect::<Vec<_>>();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0]["response"]["model"], MODEL);
    completed[0]["response"]["output"]
        .as_array()
        .unwrap()
        .clone()
}

fn fact_envelopes<'a>(facts: &'a [Value], kind: &'a str) -> impl Iterator<Item = &'a Value> {
    facts
        .iter()
        .filter(move |entry| entry["fact"]["kind"] == kind)
}

#[track_caller]
fn assert_regular_routes<'a>(facts: &'a [Value], sources: &[&str]) -> Vec<&'a Value> {
    assert_single_planner_run_per_request(facts, sources.len());
    let routes = fact_envelopes(facts, "route_decision").collect::<Vec<_>>();
    assert_eq!(routes.len(), sources.len());
    for (index, (route, source)) in routes.iter().zip(sources).enumerate() {
        let decision = &route["fact"]["complexity"];
        assert_eq!(decision["execution_group"], "regular", "route {index}");
        assert_eq!(decision["fallback_used"], false, "route {index}");
        assert_eq!(decision["decision_source"], *source, "route {index}");
    }
    routes
}

#[track_caller]
fn assert_complete_assessment(score: &Value, stage: &Value) {
    assert_eq!(score["partial"], false);
    assert_eq!(score["history_membership"], stage["history_membership"]);
    for key in [
        "segment_id",
        "plan_id",
        "plan_revision",
        "model_configuration_id",
        "profile_digest",
    ] {
        assert!(!stage[key].is_null(), "assessed stage is missing {key}");
        assert_eq!(
            score[key], stage[key],
            "score must assess the actual stage's {key}"
        );
    }
}

#[test]
fn real_listener_stop_then_appended_input_records_new_message_and_complete_later_stage() {
    run(false);
}

#[test]
fn real_listener_stop_then_changed_old_prefix_keeps_unknown_boundary() {
    run(true);
}

fn run(rewrite_old_prefix: bool) {
    let (interrupted, expected) = interrupted_reply();
    // A, B and C deliberately execute on the same group/model. Recovery must
    // follow the real input boundary, never a convenient model switch.
    let regular = NativeProvider::start(vec![
        interrupted,
        streamed("b-tool", vec![tool("b-read")]),
        streamed(
            "b-final",
            vec![message("b-answer", "final_answer", "B_COMPLETE")],
        ),
        streamed(
            "c-final",
            vec![message("c-answer", "final_answer", "C_COMPLETE")],
        ),
    ]);
    let primary = NativeProvider::start(vec![]);
    let classifier = NativeProvider::start(vec![
        reply(CLASSIFIER_SIMPLE),
        reply(CLASSIFIER_SIMPLE_WITH_ASSESSMENT),
        reply(CLASSIFIER_SIMPLE_WITH_ASSESSMENT),
    ]);
    let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
        &[&regular, &primary, &classifier],
        1,
    );
    // Historical client context is not reconstructed execution. These 81 input
    // items become an exact prefix of B's 83 items in the positive control.
    let mut input = (0..40)
        .flat_map(|n| {
            [
                json!({"role":"user","content":format!("old fixture question {n}")}),
                json!({"role":"assistant","content":format!("old fixture context {n}")}),
            ]
        })
        .collect::<Vec<_>>();
    input.push(json!({"role":"user","content":A_USER}));
    assert_eq!(input.len(), 81);
    let completed_text = cancel_after_partial_tool(&fixture, &input, &expected);
    let cancelled_facts = wait_execution_facts(&fixture, 1);
    let a = fact_envelopes(&cancelled_facts, "agent_turn_finished")
        .next()
        .unwrap()["fact"]
        .clone();
    assert_eq!(a["status"], "interrupted");
    assert_eq!(a["history_partial"], true);
    assert_eq!(a["history_membership"]["capture_partial"], true);
    assert_eq!(
        a["history_membership"]["request_ids"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let a_admission = &fact_envelopes(&cancelled_facts, "agent_turn_admitted")
        .next()
        .unwrap()["fact"];
    if rewrite_old_prefix {
        input[0]["content"] = json!("changed old fixture question");
    }
    input.extend([completed_text, json!({"role":"user","content":B_USER})]);
    assert_eq!(input.len(), 83);
    let b_tool = send(&fixture, &input, "b");
    let upstream_a = classifier_body(&regular, 0);
    let upstream_b = classifier_body(&regular, 1);
    let old = upstream_a["input"].as_array().unwrap();
    let new = upstream_b["input"].as_array().unwrap();
    assert_eq!(new.len(), 83);
    assert_eq!(old.as_slice() == &new[..81], !rewrite_old_prefix);
    assert_eq!(new[81]["content"][0]["text"], A_TEXT);
    assert_eq!(new[82]["content"], B_USER);
    assert!(!new.iter().any(|item| item["call_id"] == "a-unfinished"));
    let after_b = wait_execution_facts(&fixture, 2);
    let admissions = fact_envelopes(&after_b, "agent_turn_admitted")
        .map(|e| &e["fact"])
        .collect::<Vec<_>>();
    assert_eq!(admissions.len(), 2);
    assert_ne!(admissions[1]["branch_id"], a_admission["branch_id"]);
    let cause = if rewrite_old_prefix {
        "history_rewrite"
    } else {
        "user_message"
    };
    assert_eq!(admissions[1]["opening_cause"], cause);
    assert_eq!(b_tool.len(), 1);
    assert_eq!(b_tool[0]["type"], "function_call");
    let receipt = json!({"type":"function_call_output","call_id":b_tool[0]["call_id"],
        "output":"The controlled fixture read completed."});
    input.extend(b_tool);
    input.push(receipt);
    let b_final = send(&fixture, &input, "b");
    assert_eq!(b_final[0]["content"][0]["text"], "B_COMPLETE");
    input.extend(b_final);
    input.push(json!({"role":"user","content":C_USER}));
    let c = send(&fixture, &input, "c");
    assert_eq!(c[0]["content"][0]["text"], "C_COMPLETE");
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (4, 0, 3)
    );
    assert_finished(&fixture, &classifier, &a, cause);
}

fn assert_finished(fixture: &RuntimeFixture, classifier: &NativeProvider, a: &Value, cause: &str) {
    let facts = wait_execution_facts(fixture, 4);
    let routes = assert_regular_routes(
        &facts,
        &[
            "external_classifier",
            "external_classifier",
            "inherited",
            "external_classifier",
        ],
    );
    let stages = fact_envelopes(&facts, "agent_turn_finished")
        .map(|e| &e["fact"])
        .collect::<Vec<_>>();
    assert_eq!(stages.len(), 3);
    assert_eq!(
        stages[0], a,
        "later admission cannot rewrite the cancelled stage"
    );
    let b = stages[1];
    assert_eq!(b["opening_cause"], cause);
    assert_eq!(b["status"], "completed");
    assert_eq!(b["history_partial"], false);
    assert_eq!(b["history_membership"]["capture_partial"], false);
    assert_eq!(b["history_membership"]["clipped"], false);
    assert_eq!(
        b["history_membership"]["request_ids"],
        json!([
            routes[1]["correlation"]["request_id"],
            routes[2]["correlation"]["request_id"]
        ])
    );
    assert_ne!(b["segment_id"], a["segment_id"]);
    for key in [
        "model_configuration_id",
        "profile_digest",
        "branch_execution",
    ] {
        assert!(!b[key].is_null());
        assert_eq!(
            b[key], a[key],
            "same execution configuration must recover independently"
        );
    }
    let scores = fact_envelopes(&facts, "branch_assessment_recorded").collect::<Vec<_>>();
    assert_eq!(scores.len(), 2);
    assert_eq!(
        scores[0]["correlation"]["request_id"],
        routes[1]["correlation"]["request_id"]
    );
    assert_eq!(scores[0]["fact"]["partial"], true);
    assert_eq!(
        scores[0]["fact"]["history_membership"]["request_ids"],
        a["history_membership"]["request_ids"]
    );
    assert_eq!(
        scores[0]["fact"]["model_configuration_id"],
        a["model_configuration_id"]
    );
    assert_eq!(
        scores[1]["correlation"]["request_id"],
        routes[3]["correlation"]["request_id"]
    );
    assert_complete_assessment(&scores[1]["fact"], b);
    let assessed = classifier_body(classifier, 2);
    let from = assessed["preceding_stage"]["from"].as_u64().unwrap() as usize;
    let target = &assessed["visible_conversation"].as_array().unwrap()[from..];
    assert_eq!(target.len(), 1);
    assert_eq!(target[0]["status"], "completed");
    assert!(target[0]["user"].to_string().contains(B_USER));
    assert!(target[0]["steps"].to_string().contains("B_COMPLETE"));
    assert!(!serde_json::to_string(target).unwrap().contains(A_USER));
    // Actual envelopes pass the production adapter, are persisted, then read
    // after reopening the observation store; no synthetic timeline facts.
    let page = super::observed_history::timeline(&facts);
    assert_eq!(page.turns.len(), 3);
    assert_eq!(page.turns[0].status, "interrupted");
    assert!(page.turns[0].assessment.as_ref().unwrap().partial);
    assert_eq!(page.turns[1].status, "completed");
    assert_eq!(page.turns[1].requests.len(), 2);
    assert!(!page.turns[1].assessment.as_ref().unwrap().partial);
    assert_eq!(
        page.turns[1].boundary,
        if cause == "user_message" {
            RoutingTurnBoundaryV1::NewUserMessage
        } else {
            RoutingTurnBoundaryV1::Unknown
        }
    );
}

#[test]
fn real_listener_native_id_change_without_new_input_keeps_one_complete_tool_stage() {
    native_id_and_input_boundary("b");
}

#[test]
fn real_listener_reused_native_id_with_new_input_reclassifies_and_assesses_complete_tool_stage() {
    native_id_and_input_boundary("a-renamed");
}

fn native_id_and_input_boundary(b_turn: &str) {
    let regular = NativeProvider::start(vec![
        streamed("id-a-tool", vec![tool("id-a-call")]),
        streamed(
            "id-a-final",
            vec![message("id-a-answer", "final_answer", "ID_A_COMPLETE")],
        ),
        streamed(
            "id-b-final",
            vec![message("id-b-answer", "final_answer", "ID_B_COMPLETE")],
        ),
    ]);
    let primary = NativeProvider::start(vec![]);
    let classifier = NativeProvider::start(vec![
        reply(CLASSIFIER_SIMPLE),
        reply(CLASSIFIER_SIMPLE_WITH_ASSESSMENT),
    ]);
    let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
        &[&regular, &primary, &classifier],
        1,
    );
    let mut input = vec![json!({"role":"user","content":A_USER})];
    let first_headers = input_headers(&input, "a-original");
    let delivered = send(&fixture, &input, "a-original");
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0]["type"], "function_call");
    let receipt = json!({"type":"function_call_output","call_id":delivered[0]["call_id"],
        "output":"The controlled fixture read completed."});
    input.extend(delivered);
    input.push(receipt);
    let after_tool = wait_execution_facts(&fixture, 1);
    assert!(!after_tool.iter().any(|e| matches!(
        e["fact"]["kind"].as_str(),
        Some("agent_turn_finished" | "branch_assessment_recorded")
    )));

    // The same validated User window survives unchanged; only advisory native
    // IDs differ. No new User envelope is inserted by the client or fixture.
    let metadata = |headers: &[(String, String)]| -> Value {
        serde_json::from_str(
            &headers
                .iter()
                .find(|(key, _)| key == "x-hiroute-routing-context")
                .unwrap()
                .1,
        )
        .unwrap()
    };
    let first_context = metadata(&first_headers);
    let mut next_context = metadata(&input_headers(&input, "a-renamed"));
    assert_ne!(next_context["turn_id"], first_context["turn_id"]);
    assert_ne!(
        next_context["input"]["event_id"],
        first_context["input"]["event_id"]
    );
    assert_eq!(next_context["input"]["messages"][0]["index"], 0);
    next_context["turn_id"] = first_context["turn_id"].clone();
    next_context["input"]["event_id"] = first_context["input"]["event_id"].clone();
    assert_eq!(
        next_context, first_context,
        "all window hashes, indices and scope remain exact"
    );
    let final_output = send(&fixture, &input, "a-renamed");
    assert_eq!(final_output[0]["content"][0]["text"], "ID_A_COMPLETE");
    assert_eq!(classifier_body(&regular, 1)["input"], json!(input));
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (2, 0, 1),
        "changing a native ID must not reclassify an exact tool continuation"
    );
    let before_b = wait_execution_facts(&fixture, 2);
    let admissions = fact_envelopes(&before_b, "agent_turn_admitted")
        .map(|e| &e["fact"])
        .collect::<Vec<_>>();
    assert_eq!(admissions.len(), 2);
    for key in ["agent_turn_id", "branch_id", "ordinal", "opening_cause"] {
        assert_eq!(
            admissions[0][key], admissions[1][key],
            "ID-only change cannot replace {key}"
        );
    }
    assert!(
        fact_envelopes(&before_b, "branch_assessment_recorded")
            .next()
            .is_none()
    );
    let finished = fact_envelopes(&before_b, "agent_turn_finished")
        .map(|e| &e["fact"])
        .collect::<Vec<_>>();
    assert_eq!(finished.len(), 1);
    let a = finished[0];
    assert_eq!(a["status"], "completed");
    assert_eq!(a["history_partial"], false);
    assert_eq!(a["history_membership"]["capture_partial"], false);
    assert_eq!(
        a["history_membership"]["request_ids"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    input.extend(final_output);
    input.push(json!({"role":"user","content":B_USER}));
    let b_context = metadata(&input_headers(&input, b_turn));
    assert_eq!(b_context["input"]["messages"][0]["index"], 1);
    assert_eq!(
        b_context["input"]["messages"][0]["sha256"],
        json!(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&[B_USER]).unwrap())
        ))
    );
    assert_ne!(
        b_context["input"]["scope_digest"],
        first_context["input"]["scope_digest"]
    );
    if b_turn == "a-renamed" {
        assert_eq!(b_context["turn_id"], "a-renamed");
        assert_eq!(b_context["input"]["event_id"], "a-renamed");
    }
    let b = send(&fixture, &input, b_turn);
    assert_eq!(b[0]["content"][0]["text"], "ID_B_COMPLETE");
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (3, 0, 2)
    );
    let facts = wait_execution_facts(&fixture, 3);
    let b_admission = fact_envelopes(&facts, "agent_turn_admitted")
        .map(|e| &e["fact"])
        .nth(2)
        .unwrap();
    assert_ne!(b_admission["agent_turn_id"], a["agent_turn_id"]);
    assert_eq!(b_admission["opening_cause"], "user_message");
    let routes = assert_regular_routes(
        &facts,
        &["external_classifier", "inherited", "external_classifier"],
    );
    assert_eq!(
        a["history_membership"]["request_ids"],
        json!([
            routes[0]["correlation"]["request_id"],
            routes[1]["correlation"]["request_id"]
        ])
    );
    let scores = fact_envelopes(&facts, "branch_assessment_recorded").collect::<Vec<_>>();
    assert_eq!(scores.len(), 1);
    assert_eq!(
        scores[0]["correlation"]["request_id"],
        routes[2]["correlation"]["request_id"]
    );
    assert_complete_assessment(&scores[0]["fact"], a);
    let assessed = classifier_body(&classifier, 1);
    let from = assessed["preceding_stage"]["from"].as_u64().unwrap() as usize;
    let target = &assessed["visible_conversation"].as_array().unwrap()[from..];
    assert_eq!(target.len(), 1);
    assert_eq!(target[0]["status"], "completed");
    assert!(target[0]["user"].to_string().contains(A_USER));
    assert!(target[0]["steps"].to_string().contains("ID_A_COMPLETE"));
    assert_eq!(target[0]["steps"].as_array().unwrap().len(), 2);
    let page = super::observed_history::timeline(&facts);
    assert_eq!(page.turns.len(), 2);
    assert_eq!(page.turns[0].status, "completed");
    assert_eq!(page.turns[0].requests.len(), 2);
    assert!(!page.turns[0].assessment.as_ref().unwrap().partial);
    assert_eq!(
        page.turns[1].boundary,
        RoutingTurnBoundaryV1::NewUserMessage
    );
}
