//! Sanitized Codex 0.166 / codex-acp 2.5 wire shapes captured with the real client.
//! Loopback providers are controlled; installed-client acceptance is separate.
use super::native_history::{classifier_body, reply};
use super::*;
use serde_json::{Value, json};

const TOOL: &[u8] = br#"{"id":"tool","model":"runtime-native","status":"completed","output":[{"type":"reasoning","id":"rs","summary":[{"type":"summary_text","text":"Check the synthetic command."}],"content":null,"encrypted_content":null,"status":"completed"},{"type":"function_call","id":"fc","call_id":"call","name":"exec_command","arguments":"{\"cmd\":\"echo SYNTHETIC\"}","status":"completed"}],"usage":{"input_tokens":1,"output_tokens":1}}"#;

#[test]
fn codex_hosted_search_and_parallel_tool_results_preserve_the_executing_stage() {
    use super::native_encrypted_continuation::{message, streamed};

    let regular = NativeProvider::start(vec![
        streamed("initial", vec![shell_call("first")]),
        streamed("search", search_and_calls()),
        streamed("after-search", vec![shell_call("last")]),
        streamed(
            "final",
            vec![message("answer", "final_answer", "Complete.")],
        ),
    ]);
    let primary = NativeProvider::start(vec![]);
    let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE)]);
    let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
        &[&regular, &primary, &classifier],
        1,
    );
    let turn = "hosted-search-turn";
    let mut input = vec![json!({"role":"user","content":"Inspect the fixture."})];
    for expected_calls in [1, 2, 1] {
        let output = send(&fixture, turn, &input, true);
        let results = tool_results(&output);
        assert_eq!(results.len(), expected_calls);
        input.extend(output);
        input.extend(results);
    }
    let output = send(&fixture, turn, &input, true);
    assert_eq!(output[0]["content"][0]["text"], "Complete.");
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (4, 0, 1)
    );

    let facts = wait_execution_facts(&fixture, 4);
    let timeline = super::observed_history::timeline(&facts);
    assert_eq!(timeline.turns.len(), 1);
    assert_eq!(timeline.turns[0].requests.len(), 4);
    assert_eq!(timeline.turns[0].status, "completed");
    assert!(
        timeline.turns[0].evidence_partial,
        "the optional recording gap remains visible"
    );
    let decisions = facts
        .iter()
        .filter(|event| event["fact"]["kind"] == "route_decision")
        .map(|event| {
            event["fact"]["complexity"]["decision_source"]
                .as_str()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        decisions,
        ["external_classifier", "inherited", "inherited", "inherited"]
    );
}

#[cfg(all(unix, debug_assertions))]
#[test]
fn codex_129th_request_can_continue_while_response_capture_is_paused() {
    use super::native_encrypted_continuation::{message, streamed};

    let mut replies = (0..127)
        .map(|index| streamed("tool", vec![shell_call(&format!("call-{index}"))]))
        .collect::<Vec<_>>();
    replies.push(streamed(
        "parallel",
        vec![shell_call("parallel-left"), shell_call("parallel-right")],
    ));
    replies.push(streamed(
        "final",
        vec![message("answer", "final_answer", "Complete.")],
    ));
    let regular = NativeProvider::start(replies);
    let primary = NativeProvider::start(vec![]);
    let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE)]);
    let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
        &[&regular, &primary, &classifier],
        1,
    );
    let pause = fixture
        .observation_root
        .as_ref()
        .unwrap()
        .join("response-capture.pause");
    let mut input = vec![json!({"role":"user","content":"Inspect the long fixture."})];
    for index in 0..128 {
        if index == 127 {
            std::fs::write(&pause, b"").unwrap();
        }
        let (body, complete) = codex_stream(&fixture, "long-turn", &input, index == 127);
        assert!(complete, "tool response {index} did not finish");
        let output = completed_stream(&body)["output"]
            .as_array()
            .unwrap()
            .clone();
        let results = tool_results(&output);
        assert_eq!(results.len(), if index == 127 { 2 } else { 1 });
        input.extend(output);
        input.extend(results);
    }
    let (body, complete) = codex_stream(&fixture, "long-turn", &input, true);
    std::fs::remove_file(pause).unwrap();
    assert!(complete);
    assert_eq!(
        completed_stream(&body)["output"][0]["content"][0]["text"],
        "Complete."
    );
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (129, 0, 1)
    );

    let facts = wait_execution_facts(&fixture, 129);
    let timeline = super::observed_history::timeline(&facts);
    assert_eq!(timeline.turns.len(), 1);
    assert_eq!(timeline.turns[0].requests.len(), 129);

    let mut admissions = facts
        .iter()
        .filter(|row| row["fact"]["kind"] == "agent_turn_admitted")
        .collect::<Vec<_>>();
    admissions.sort_by_key(|row| row["fact"]["revision"].as_u64().unwrap());
    assert_eq!(admissions.len(), 129);
    let first = admissions[0]["correlation"]["request_id"].as_str().unwrap();
    let predecessor = admissions[127]["correlation"]["request_id"]
        .as_str()
        .unwrap();
    let last = admissions[128]["correlation"]["request_id"]
        .as_str()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let records = loop {
        let records = wait_content_for_request(&fixture, last);
        if records.iter().any(|row| {
            row["correlation"]["request_id"] == predecessor
                && row["direction"] == "response_delivered"
                && row["phase"] == "finish"
        }) {
            break records;
        }
        assert!(
            Instant::now() < deadline,
            "predecessor recording did not finish after handoff"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let anchor = |id| {
        records
            .iter()
            .find(|row| {
                row["correlation"]["request_id"] == id
                    && row["direction"] == "request_input"
                    && row["phase"] == "begin"
            })
            .unwrap()["capture"]["input_anchor_id"]
            .as_str()
            .unwrap()
    };
    assert_eq!(anchor(first), anchor(last));
    assert!(
        records
            .iter()
            .any(|row| row["correlation"]["request_id"] == first
                && row["direction"] == "request_input"
                && row["phase"] == "append"
                && row["message_instance_id"] == anchor(last))
    );
}

#[cfg(all(unix, debug_assertions))]
#[test]
fn codex_failed_terminal_write_does_not_publish_a_prepared_tool_batch() {
    use super::native_encrypted_continuation::{message, streamed};

    let call = shell_call("failed-write");
    let regular = NativeProvider::start(vec![
        streamed("tool", vec![call.clone()]),
        streamed(
            "final",
            vec![message("answer", "final_answer", "New branch.")],
        ),
    ]);
    let primary = NativeProvider::start(vec![]);
    let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE); 2]);
    let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
        &[&regular, &primary, &classifier],
        1,
    );
    let fail = fixture
        .observation_root
        .as_ref()
        .unwrap()
        .join("response-final-write.fail");
    std::fs::write(&fail, b"").unwrap();
    let mut input = vec![json!({"role":"user","content":"Inspect the fixture."})];
    let (_, complete) = codex_stream(&fixture, "failed-write-turn", &input, false);
    assert!(!fail.exists(), "the terminal write fault was not reached");
    assert!(
        !complete,
        "the failed write must leave an incomplete entity"
    );
    let results = tool_results(std::slice::from_ref(&call));
    input.push(call);
    input.extend(results);
    let (body, complete) = codex_stream(&fixture, "failed-write-turn", &input, false);
    assert!(complete);
    assert_eq!(
        completed_stream(&body)["output"][0]["content"][0]["text"],
        "New branch."
    );
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (2, 0, 2)
    );
}

#[cfg(all(unix, debug_assertions))]
fn codex_stream(
    fixture: &RuntimeFixture,
    turn: &str,
    input: &[Value],
    return_at_eos: bool,
) -> (Vec<u8>, bool) {
    let headers =
        super::native_history::headers("codex", "tool-handoff-session", "main", turn, "before");
    let headers = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let payload = serde_json::to_vec(
        &json!({"model":MODEL,"stream":true,"store":false,"input":input,
        "tools":[{"type":"function","name":"exec_command","parameters":{"type":"object"}}]}),
    )
    .unwrap();
    if !return_at_eos {
        let response = request(fixture.address, "POST", "/v1/responses", &headers, &payload);
        assert_eq!(response.status, 200);
        return (response.body, response.http_entity_complete);
    }
    let body = read_chunked_eos(open_request(
        fixture.address,
        "POST",
        "/v1/responses",
        &headers,
        &payload,
    ));
    (body, true)
}

#[test]
fn codex_hosted_search_does_not_inherit_on_conflicting_or_incomplete_tool_input() {
    use super::native_encrypted_continuation::{message, streamed};

    for case in [
        "arguments",
        "wrong_id",
        "missing_result",
        "duplicate_result",
        "new_user",
        "rewritten_prefix",
    ] {
        let regular = NativeProvider::start(vec![
            streamed("search", search_and_calls()),
            streamed(
                "final",
                vec![message("answer", "final_answer", "New branch.")],
            ),
        ]);
        let primary = NativeProvider::start(vec![]);
        let classifier =
            NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE), reply(CLASSIFIER_SIMPLE)]);
        let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
            &[&regular, &primary, &classifier],
            1,
        );
        let mut input = vec![json!({"role":"user","content":"Inspect the fixture."})];
        let mut output = send(&fixture, "same-native-id", &input, true);
        let mut results = tool_results(&output);
        match case {
            "arguments" => output[2]["arguments"] = json!("{\"cmd\":\"echo different\"}"),
            "wrong_id" => results[1]["call_id"] = json!("not-delivered"),
            "missing_result" => {
                results.pop();
            }
            "duplicate_result" => results.push(results[0].clone()),
            "new_user" | "rewritten_prefix" => {}
            _ => unreachable!(),
        }
        if case == "rewritten_prefix" {
            input[0]["content"] = json!("Inspect a different fixture.");
        }
        input.extend(output);
        input.extend(results);
        if case == "new_user" {
            input.push(json!({"role":"user","content":"Start another task."}));
        }
        send(&fixture, "same-native-id", &input, true);
        let facts = wait_execution_facts(&fixture, 2);
        assert_eq!(classifier.calls(), 2, "{case}");
        assert_eq!(
            super::observed_history::timeline(&facts).turns.len(),
            2,
            "{case}"
        );
    }
}

#[test]
fn codex_repeated_tool_input_does_not_move_to_a_sibling_batch() {
    use super::native_encrypted_continuation::{message, streamed};

    let hosted = search_and_calls().remove(0);
    let call = shell_call("shared");
    let regular = NativeProvider::start(vec![
        streamed("exact", vec![call.clone()]),
        streamed("sibling", vec![hosted, call]),
        streamed(
            "finish",
            vec![message("answer", "final_answer", "Complete.")],
        ),
        streamed(
            "duplicate",
            vec![message("repeat", "final_answer", "Repeated.")],
        ),
    ]);
    let primary = NativeProvider::start(vec![]);
    let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE); 3]);
    let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
        &[&regular, &primary, &classifier],
        1,
    );
    let turn = "shared-native-turn";
    let mut input = vec![json!({"role":"user","content":"Inspect the fixture."})];
    let exact = send(&fixture, turn, &input, true);
    assert_eq!(tool_results(&send(&fixture, turn, &input, true)).len(), 1);
    let results = tool_results(&exact);
    input.extend(exact);
    input.extend(results);
    send(&fixture, turn, &input, true);
    assert_eq!(
        classifier.calls(),
        2,
        "the first continuation has an exact predecessor"
    );
    send(&fixture, turn, &input, true);
    assert_eq!(
        classifier.calls(),
        3,
        "a duplicate must not claim the sibling's batch"
    );
}

fn shell_call(call: &str) -> Value {
    json!({"type":"function_call","id":format!("fc-{call}"),"call_id":call,
        "name":"exec_command","status":"completed","arguments":"{\"cmd\":\"printf fixture\"}"})
}

fn search_and_calls() -> Vec<Value> {
    vec![
        json!({"type":"web_search_call","id":"ws-open","status":"completed",
            "action":{"type":"open_page","url":"https://example.com/"}}),
        json!({"type":"function_call","id":"fc-list","call_id":"list","namespace":"mcp",
            "name":"list_artifacts","status":"completed","arguments":"{}"}),
        shell_call("parallel"),
    ]
}

fn tool_results(output: &[Value]) -> Vec<Value> {
    output.iter().filter(|item| item["type"] == "function_call")
        .map(|item| json!({"type":"function_call_output","call_id":item["call_id"],"output":"fixture"}))
        .collect()
}

#[test]
fn codex_commentary_between_reasoning_and_tool_keeps_one_stage() {
    let mut output: Vec<Value> = serde_json::from_slice::<Value>(TOOL).unwrap()["output"]
        .as_array()
        .unwrap()
        .clone();
    output.insert(1, json!({"type":"message","id":"commentary","role":"assistant","status":"completed","phase":"commentary","content":[{"type":"output_text","text":"Inspecting the synthetic input.","annotations":[],"logprobs":[]}]}));
    let regular = NativeProvider::start(vec![
        super::native_encrypted_continuation::streamed("commentary-response", output),
        reply(RESPONSES_OK),
        reply(RESPONSES_OK),
    ]);
    let primary = NativeProvider::start(vec![]);
    let classifier = NativeProvider::start(vec![
        reply(CLASSIFIER_SIMPLE),
        reply(br#"{"group":"regular","assessment":{"score":0.75,"partial":false}}"#),
    ]);
    let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
        &[&regular, &primary, &classifier],
        1,
    );
    let mut input = vec![json!({"role":"user","content":"Inspect the synthetic input."})];
    let mut output = send(&fixture, "commentary-turn", &input, true);
    for item in &mut output {
        item.as_object_mut()
            .unwrap()
            .remove("encrypted_function_args");
        if item["type"] == "reasoning" {
            item.as_object_mut().unwrap().remove("status");
        }
    }
    input.extend(output);
    input.push(json!({"type":"function_call_output","call_id":"call","output":"SYNTHETIC"}));
    input.extend(send(&fixture, "commentary-turn", &input, false));
    assert_eq!(classifier.calls(), 1);
    input.push(json!({"role":"user","content":"Assess the complete preceding task."}));
    send(&fixture, "commentary-next-turn", &input, false);
    assert_eq!(classifier.calls(), 2);
    let judged = classifier_body(&classifier, 1);
    assert_eq!(judged["preceding_stage"]["complete"], true, "{judged}");
    let facts = wait_execution_facts(&fixture, 3);
    let score = facts
        .iter()
        .find(|e| e["fact"]["kind"] == "branch_assessment_recorded")
        .unwrap();
    assert_eq!(score["fact"]["partial"], false);
    assert_eq!(
        score["fact"]["history_membership"]["request_ids"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

fn send(fixture: &RuntimeFixture, turn: &str, input: &[Value], streaming: bool) -> Vec<Value> {
    let metadata = json!({"installation_id":"fixture-installation", "session_id":"acp-session",
        "thread_id":"acp-session", "turn_id":turn, "window_id":"acp-session:0",
        "window_number":0, "context_window_id":"fixture-context", "request_kind":"turn",
        "history_initialization":"new", "root_turn_id":turn, "agent_name":"/root",
        "sandbox_mode":"workspace-write", "auto_review_enabled":true});
    let encoded = metadata.to_string();
    let body = json!({"model":MODEL, "stream":streaming, "store":false,
        "tools":[{"type":"function", "name":"exec_command", "parameters":{"type":"object","properties":{"cmd":{"type":"string"}}}}],
        "input":input,
        "client_metadata":{"x-codex-turn-metadata":encoded, "turn_id":turn,
            "session_id":"acp-session", "thread_id":"acp-session",
            "x-codex-window-id":"acp-session:0", "x-codex-installation-id":"fixture-installation"}});
    let response = request(
        fixture.address,
        "POST",
        "/v1/responses",
        &[
            ("X-HiRoute-Token", TOKEN),
            (
                "user-agent",
                "acp-fixture/0.166.0 (linux; x86_64) unknown (acp-fixture; 1)",
            ),
            ("session-id", "acp-session"),
            ("thread-id", "acp-session"),
            ("x-codex-window-id", "acp-session:0"),
            ("x-codex-turn-metadata", &encoded),
        ],
        &serde_json::to_vec(&body).unwrap(),
    );
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let response: Value = if streaming {
        completed_stream(&response.body)
    } else {
        serde_json::from_slice(&response.body).unwrap()
    };
    assert_eq!(response["status"], "completed");
    response["output"].as_array().unwrap().clone()
}

fn completed_stream(body: &[u8]) -> Value {
    std::str::from_utf8(body)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|data| *data != "[DONE]")
        .map(|data| serde_json::from_str::<Value>(data).expect("invalid native SSE JSON"))
        .find(|event| event["type"] == "response.completed")
        .expect("missing completed response")["response"]
        .clone()
}

#[test]
fn codex_acp_tool_steps_reuse_one_stage_and_next_input_assesses_the_whole_stage() {
    let second = Box::leak(
        String::from_utf8(TOOL.to_vec())
            .unwrap()
            .replace("\"call\"", "\"call-2\"")
            .into_bytes()
            .into_boxed_slice(),
    );
    let regular = NativeProvider::start(vec![reply(TOOL), reply(second), reply(RESPONSES_OK)]);
    let primary = NativeProvider::start(vec![reply(RESPONSES_OK)]);
    let classifier = NativeProvider::start(vec![
        reply(CLASSIFIER_SIMPLE),
        reply(br#"{"group":"primary","assessment":{"score":0.75,"partial":false}}"#),
    ]);
    let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
        &[&regular, &primary, &classifier],
        1,
    );
    let mut input = vec![
        json!({"type":"message","id":null,"role":"developer","content":[{"type":"input_text","text":"Synthetic tool task."}]}),
        json!({"type":"message","id":null,"role":"user","content":[{"type":"input_text","text":"Run two harmless commands."}]}),
    ];
    for index in 0..2 {
        let mut output = send(&fixture, "turn-a", &input, false);
        assert_eq!(
            classifier.calls(),
            1,
            "tool step {index} opened a new decision"
        );
        // The real 0.166 producer omits reasoning status and nullable function
        // encoding, retains the completed tool status and assigns receipt IDs.
        for item in &mut output {
            if item["type"] == "reasoning" {
                item.as_object_mut().unwrap().remove("status");
            }
            item.as_object_mut()
                .unwrap()
                .remove("encrypted_function_args");
        }
        let call = output
            .iter()
            .find(|item| item["type"] == "function_call")
            .unwrap()["call_id"]
            .clone();
        input.extend(output);
        input.push(json!({"type":"function_call_output","id":format!("receipt-{index}"),"call_id":call,"output":"SYNTHETIC"}));
    }
    input.extend(send(&fixture, "turn-a", &input, false));
    assert_eq!(classifier.calls(), 1);
    input.push(json!({"type":"message","id":null,"role":"user","content":[{"type":"input_text","text":"Assess the preceding task."}]}));
    send(&fixture, "turn-b", &input, false);
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (3, 1, 2)
    );
    let judged = classifier_body(&classifier, 1);
    assert_eq!(judged["preceding_stage"]["complete"], true, "{judged}");
    let from = judged["preceding_stage"]["from"].as_u64().unwrap() as usize;
    let stages = &judged["visible_conversation"].as_array().unwrap()[from..];
    assert_eq!(stages.len(), 1, "{judged}");
    assert_eq!(stages[0]["steps"].as_array().unwrap().len(), 3, "{judged}");
    let facts = wait_execution_facts(&fixture, 4);
    let routes = facts
        .iter()
        .filter(|e| e["fact"]["kind"] == "route_decision")
        .collect::<Vec<_>>();
    assert_eq!(routes.len(), 4);
    let score = facts
        .iter()
        .find(|e| e["fact"]["kind"] == "branch_assessment_recorded")
        .unwrap();
    assert_eq!(score["fact"]["partial"], false);
    assert_eq!(
        score["fact"]["history_membership"]["request_ids"],
        json!(
            routes[..3]
                .iter()
                .map(|e| e["correlation"]["request_id"].clone())
                .collect::<Vec<_>>()
        )
    );
}
