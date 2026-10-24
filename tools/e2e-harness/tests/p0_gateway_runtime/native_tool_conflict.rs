//! A contradictory native tool stream stays deliverable and partial; later
//! independent, fully captured work must still be routable and assessable.
use super::native_encrypted_continuation::{message, streamed};
use super::native_history::{classifier_body, headers, reply};
use super::*;
use serde_json::{Value, json};

const SESSION: &str = "pi-tool-delta-conflict";
const A_USER: &str = "A: read both public fixture files";
const B_USER: &str = "B: independently read the next fixture";
const C_USER: &str = "C: design an independent follow-up";
const ASSESSED_COMPLEX: &[u8] =
    br#"{"group":"primary","assessment":{"score":0.75,"partial":false}}"#;

fn tool(id: &str, path: &str) -> Value {
    json!({"type":"function_call","id":format!("fc-{id}"),"call_id":id,
        "name":"read","status":"completed","arguments":json!({"path":path}).to_string()})
}

fn parallel_tools(conflicting_delta: bool, conflicting_metadata: bool) -> (ProviderReply, Vec<u8>) {
    let output = vec![
        message("a-comment", "commentary", "Reading both fixtures."),
        tool("a-first", "fixture/first.txt"),
        tool("a-second", "fixture/path-b.txt"),
    ];
    let ProviderReply::StreamDrip { chunks, .. } = streamed("a-conflict", output) else {
        unreachable!()
    };
    let mut events = String::from_utf8(chunks.concat())
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str::<Value>(data).unwrap())
        .collect::<Vec<_>>();
    // Both calls are open together in both controls. Only the optional second
    // delta changes; arguments.done, item.done and response.completed use B.
    let second_start = events
        .iter()
        .position(|e| e["type"] == "response.output_item.added" && e["output_index"] == 2)
        .unwrap();
    let second = events.remove(second_start);
    let first_delta = events
        .iter()
        .position(|e| {
            e["type"] == "response.function_call_arguments.delta" && e["output_index"] == 1
        })
        .unwrap();
    events.insert(first_delta, second);
    let delta = events
        .iter_mut()
        .find(|e| e["type"] == "response.function_call_arguments.delta" && e["output_index"] == 2)
        .unwrap();
    assert_eq!(delta["item_id"], "fc-a-second");
    assert_eq!(
        delta["delta"],
        json!({"path":"fixture/path-b.txt"}).to_string()
    );
    if conflicting_delta {
        delta["delta"] = json!(json!({"path":"fixture/path-a.txt"}).to_string());
    }
    if conflicting_metadata {
        let added = events
            .iter_mut()
            .find(|e| e["type"] == "response.output_item.added" && e["output_index"] == 2)
            .unwrap();
        // The same call ID changes provider-owned name and namespace at done.
        // No field is repaired: the client still receives both native versions.
        added["item"]["name"] = json!("previous_read");
        added["item"]["namespace"] = json!("previous_tools");
    }
    let encode = |events: &[Value]| {
        events
            .iter()
            .map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap()))
            .collect::<String>()
            .into_bytes()
    };
    let wire = encode(&events);
    let reply = ProviderReply::StreamDrip {
        status: 200,
        chunks: wire.chunks(113).map(<[u8]>::to_vec).collect(),
        interval: Duration::ZERO,
    };
    // Native delivery deliberately exposes the requested public model alias.
    // Every other byte, including contradictory arguments, must stay intact.
    for event in &mut events {
        if let Some(model) = event.pointer_mut("/response/model") {
            assert_eq!(*model, "runtime-native");
            *model = json!(MODEL);
        }
    }
    (reply, encode(&events))
}

fn send(fixture: &RuntimeFixture, input: &[Value], turn: &str) -> (Vec<Value>, Vec<u8>) {
    let h = headers("pi", SESSION, "main", turn, "after");
    let response = request(
        fixture.address,
        "POST",
        "/v1/responses",
        &h.iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect::<Vec<_>>(),
        &serde_json::to_vec(&json!({"model":MODEL,"stream":true,"input":input,
            "parallel_tool_calls":true,"tools":[{"type":"function","name":"read",
            "parameters":{"type":"object","properties":{"path":{"type":"string"}}}}]}))
        .unwrap(),
    );
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let events = std::str::from_utf8(&response.body)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|data| *data != "[DONE]")
        .map(|data| serde_json::from_str::<Value>(data).unwrap())
        .collect::<Vec<_>>();
    assert!(
        !events.iter().any(|e| matches!(
            e["type"].as_str(),
            Some("error" | "response.failed" | "response.incomplete")
        )),
        "{events:?}"
    );
    let completed = events
        .iter()
        .filter(|e| e["type"] == "response.completed")
        .collect::<Vec<_>>();
    assert_eq!(completed.len(), 1, "{events:?}");
    (
        completed[0]["response"]["output"]
            .as_array()
            .unwrap()
            .clone(),
        response.body,
    )
}

fn replay_with_results(input: &mut Vec<Value>, delivered: Vec<Value>, expected_calls: usize) {
    let results = delivered
        .iter()
        .filter(|item| item["type"] == "function_call")
        .map(|item| {
            json!({"type":"function_call_output","call_id":item["call_id"],
            "output":"Controlled fixture read completed."})
        })
        .collect::<Vec<_>>();
    assert_eq!(results.len(), expected_calls);
    input.extend(delivered);
    input.extend(results);
}

#[test]
fn real_listener_conflicting_parallel_tool_final_stays_partial_and_next_stage_recovers() {
    assert_conflicting_parallel_tool_recovers(false);
}

#[test]
fn real_listener_conflicting_native_tool_metadata_forwards_and_next_stage_recovers() {
    assert_conflicting_parallel_tool_recovers(true);
}

fn assert_conflicting_parallel_tool_recovers(conflicting_metadata: bool) {
    let (conflicting, raw_conflicting) = parallel_tools(true, conflicting_metadata);
    let regular = NativeProvider::start(vec![
        streamed("b-tool", vec![tool("b-read", "fixture/next.txt")]),
        streamed(
            "b-final",
            vec![message("b-answer", "final_answer", "B_COMPLETE")],
        ),
    ]);
    let primary = NativeProvider::start(vec![
        conflicting,
        streamed(
            "a-final",
            vec![message(
                "a-answer",
                "final_answer",
                "A_CONTINUATION_COMPLETE",
            )],
        ),
        streamed(
            "c-final",
            vec![message("c-answer", "final_answer", "C_COMPLETE")],
        ),
    ]);
    let classifier = NativeProvider::start(vec![
        reply(CLASSIFIER_COMPLEX),
        // Even a classifier claiming a complete assessment cannot repair the
        // contradictory provider lifecycle of the actual old execution.
        reply(ASSESSED_COMPLEX),
        reply(CLASSIFIER_SIMPLE),
        reply(ASSESSED_COMPLEX),
    ]);
    let fixture = RuntimeFixture::launch_rest_classified_with_observation(
        &[&regular, &primary, &classifier],
        1,
        ObservationFaults::healthy(),
    );
    let mut input = vec![json!({"role":"user","content":A_USER})];
    let (delivered, raw) = send(&fixture, &input, "a");
    assert_eq!(
        String::from_utf8_lossy(&raw),
        String::from_utf8_lossy(&raw_conflicting),
        "optional capture must not rewrite native delivery beyond the public model alias"
    );
    assert_eq!(
        delivered[2]["arguments"],
        json!({"path":"fixture/path-b.txt"}).to_string()
    );
    let first_facts = wait_execution_facts(&fixture, 1);
    let first_attempt = first_facts
        .iter()
        .find(|e| e["fact"]["kind"] == "attempt_finished")
        .unwrap();
    assert_eq!(first_attempt["fact"]["outcome"], "accepted");
    assert_eq!(
        first_attempt["fact"]["provider_model_event"],
        "response_unknown"
    );
    let first_id = first_attempt["correlation"]["request_id"].clone();
    replay_with_results(&mut input, delivered, 2);
    let (a_final, _) = send(&fixture, &input, "a");
    assert_eq!(a_final[0]["content"][0]["text"], "A_CONTINUATION_COMPLETE");
    let replayed = classifier_body(&primary, 1);
    assert_eq!(
        replayed["input"],
        json!(input),
        "the client must replay the actual final tools and both results"
    );
    input.extend(a_final);
    input.push(json!({"role":"user","content":B_USER}));
    let (b_tool, _) = send(&fixture, &input, "b");
    replay_with_results(&mut input, b_tool, 1);
    let (b_final, _) = send(&fixture, &input, "b");
    assert_eq!(b_final[0]["content"][0]["text"], "B_COMPLETE");
    input.extend(b_final);
    input.push(json!({"role":"user","content":C_USER}));
    let (c_final, _) = send(&fixture, &input, "c");
    assert_eq!(c_final[0]["content"][0]["text"], "C_COMPLETE");
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (2, 3, 4)
    );

    let facts = wait_execution_facts(&fixture, 5);
    assert_single_planner_run_per_request(&facts, 5);
    let routes = facts
        .iter()
        .filter(|e| e["fact"]["kind"] == "route_decision")
        .collect::<Vec<_>>();
    let expected_groups = ["primary", "primary", "regular", "regular", "primary"];
    for (index, route) in routes.iter().enumerate() {
        assert_eq!(
            route["fact"]["complexity"]["execution_group"],
            expected_groups[index]
        );
        assert_eq!(route["fact"]["complexity"]["fallback_used"], false);
        assert_eq!(
            route["fact"]["complexity"]["decision_source"],
            if index == 3 {
                "inherited"
            } else {
                "external_classifier"
            }
        );
    }
    let stages = facts
        .iter()
        .filter(|e| e["fact"]["kind"] == "agent_turn_finished")
        .map(|e| &e["fact"])
        .collect::<Vec<_>>();
    let old = stages
        .iter()
        .filter(|stage| {
            stage["history_membership"]["request_ids"]
                .as_array()
                .unwrap()
                .contains(&first_id)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        old.len(),
        1,
        "the contradictory execution must remain an actual retained stage"
    );
    assert_eq!(old[0]["status"], "unknown");
    assert_eq!(old[0]["history_partial"], true);
    assert_eq!(old[0]["history_membership"]["capture_partial"], true);
    assert_eq!(
        old[0]["history_membership"]["request_ids"],
        json!([first_id])
    );

    let b_members = json!([
        routes[2]["correlation"]["request_id"],
        routes[3]["correlation"]["request_id"]
    ]);
    let b = stages
        .iter()
        .find(|stage| stage["history_membership"]["request_ids"] == b_members)
        .unwrap();
    assert_eq!(b["status"], "completed");
    assert_eq!(b["history_partial"], false);
    assert_eq!(b["history_membership"]["capture_partial"], false);
    assert_eq!(b["history_membership"]["clipped"], false);
    assert_eq!(b["branch_execution"]["group"], "regular");
    let scores = facts
        .iter()
        .filter(|e| e["fact"]["kind"] == "branch_assessment_recorded")
        .collect::<Vec<_>>();
    assert_eq!(scores.len(), 2);
    assert_eq!(
        scores[0]["correlation"]["request_id"],
        routes[1]["correlation"]["request_id"]
    );
    assert_eq!(scores[0]["fact"]["partial"], true);
    let mut uncertain_target = old[0]["history_membership"].clone();
    uncertain_target["relation"] = json!("contextual");
    uncertain_target["association_unverified"] = json!(true);
    assert_eq!(
        scores[0]["fact"]["history_membership"], uncertain_target,
        "the later request adds association uncertainty without changing frozen members"
    );
    assert_eq!(
        scores[1]["correlation"]["request_id"],
        routes[4]["correlation"]["request_id"]
    );
    let score = &scores[1]["fact"];
    assert_eq!(score["partial"], false);
    assert_eq!(score["history_membership"], b["history_membership"]);
    for key in [
        "segment_id",
        "plan_id",
        "plan_revision",
        "model_configuration_id",
        "profile_digest",
    ] {
        assert!(!b[key].is_null());
        assert_eq!(
            score[key], b[key],
            "assessment must freeze B's actual {key}"
        );
    }
    let assessed = classifier_body(&classifier, 3);
    let from = assessed["preceding_stage"]["from"].as_u64().unwrap() as usize;
    let target = &assessed["visible_conversation"].as_array().unwrap()[from..];
    assert_eq!(target.len(), 1);
    assert_eq!(target[0]["status"], "completed");
    assert!(target[0]["user"].to_string().contains(B_USER));
    assert_eq!(
        target[0]["steps"],
        json!([
            [{"kind":"tool_activity","tool":"read","status":"unknown"}],
            [{"kind":"text","text":"B_COMPLETE"}]
        ])
    );
    assert!(!serde_json::to_string(target).unwrap().contains(A_USER));
    let page = super::observed_history::timeline(&facts);
    assert!(page.turns.iter().any(|turn| turn.status == "unknown"));
    let observed_b = page
        .turns
        .iter()
        .find(|turn| turn.requests.len() == 2)
        .unwrap();
    assert_eq!(observed_b.status, "completed");
    assert_eq!(observed_b.assessment.as_ref().unwrap().score, 0.75);
}

#[test]
fn real_listener_consistent_parallel_tools_keep_known_completion_and_one_stage() {
    let (consistent, raw_consistent) = parallel_tools(false, false);
    let regular = NativeProvider::start(vec![
        consistent,
        streamed(
            "a-final",
            vec![message("a-answer", "final_answer", "A_COMPLETE")],
        ),
    ]);
    let primary = NativeProvider::start(vec![]);
    let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE)]);
    let fixture = RuntimeFixture::launch_rest_classified_with_observation(
        &[&regular, &primary, &classifier],
        1,
        ObservationFaults::healthy(),
    );
    let mut input = vec![json!({"role":"user","content":A_USER})];
    let (delivered, raw) = send(&fixture, &input, "a");
    assert_eq!(
        String::from_utf8_lossy(&raw),
        String::from_utf8_lossy(&raw_consistent)
    );
    let before_final = wait_execution_facts(&fixture, 1);
    let first_attempt = before_final
        .iter()
        .find(|e| e["fact"]["kind"] == "attempt_finished")
        .unwrap();
    assert_eq!(
        first_attempt["fact"]["provider_model_event"],
        "response_complete"
    );
    assert!(
        !before_final
            .iter()
            .any(|e| e["fact"]["kind"] == "agent_turn_finished"),
        "a known completed tool response still waits for client continuation"
    );
    replay_with_results(&mut input, delivered, 2);
    let (final_output, _) = send(&fixture, &input, "a");
    assert_eq!(final_output[0]["content"][0]["text"], "A_COMPLETE");
    assert_eq!(classifier_body(&regular, 1)["input"], json!(input));
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (2, 0, 1)
    );
    let facts = wait_execution_facts(&fixture, 2);
    assert_single_planner_run_per_request(&facts, 2);
    let routes = facts
        .iter()
        .filter(|e| e["fact"]["kind"] == "route_decision")
        .collect::<Vec<_>>();
    assert_eq!(
        routes[1]["fact"]["complexity"]["decision_source"],
        "inherited"
    );
    let stages = facts
        .iter()
        .filter(|e| e["fact"]["kind"] == "agent_turn_finished")
        .collect::<Vec<_>>();
    assert_eq!(stages.len(), 1);
    let stage = &stages[0]["fact"];
    assert_eq!(stage["status"], "completed");
    assert_eq!(stage["history_partial"], false);
    assert_eq!(stage["history_membership"]["capture_partial"], false);
    assert_eq!(
        stage["history_membership"]["request_ids"],
        json!([
            routes[0]["correlation"]["request_id"],
            routes[1]["correlation"]["request_id"]
        ])
    );
    let page = super::observed_history::timeline(&facts);
    assert_eq!(page.turns.len(), 1);
    assert_eq!(page.turns[0].status, "completed");
    assert_eq!(page.turns[0].requests.len(), 2);
}
