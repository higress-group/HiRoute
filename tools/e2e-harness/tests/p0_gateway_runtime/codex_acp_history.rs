//! Sanitized Codex 0.166 / codex-acp 2.5 wire shapes captured with the real client.
//! Loopback providers are controlled; installed-client acceptance is separate.
use super::native_history::{classifier_body, reply};
use super::*;
use serde_json::{Value, json};

const TOOL: &[u8] = br#"{"id":"tool","model":"runtime-native","status":"completed","output":[{"type":"reasoning","id":"rs","summary":[{"type":"summary_text","text":"Check the synthetic command."}],"content":null,"encrypted_content":null,"status":"completed"},{"type":"function_call","id":"fc","call_id":"call","name":"exec_command","arguments":"{\"cmd\":\"echo SYNTHETIC\"}","status":"completed"}],"usage":{"input_tokens":1,"output_tokens":1}}"#;

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
        std::str::from_utf8(&response.body)
            .unwrap()
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter(|data| *data != "[DONE]")
            .map(|data| serde_json::from_str::<Value>(data).expect("invalid native SSE JSON"))
            .find(|event| event["type"] == "response.completed")
            .expect("missing completed response")["response"]
            .clone()
    } else {
        serde_json::from_slice(&response.body).unwrap()
    };
    assert_eq!(response["status"], "completed");
    response["output"].as_array().unwrap().clone()
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
