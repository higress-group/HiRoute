//! Safe producer-shaped history through the actual listener, attempts and capture.
//! The receipt is controlled; no native client, shell tool or paid model runs here.
use super::*;

const NULLABLE_TOOL: &[u8] = br#"{"id":"nullable-tool","model":"runtime-native","status":"completed","output":[{"type":"reasoning","id":"rs_synthetic","summary":[{"type":"summary_text","text":"Inspect the synthetic fixture before answering."}],"content":null,"encrypted_content":null,"status":"completed"},{"type":"function_call","id":"fc","call_id":"call","name":"exec_command","arguments":"{\"cmd\":\"printf SYNTHETIC_RECEIPT\"}","status":"completed"}],"usage":{"input_tokens":1,"output_tokens":1}}"#;

#[test]
fn real_listener_nullable_reasoning_tool_receipt_and_next_input_keep_complete_assessment() {
    for streaming in [false, true] {
        complete_tool_stage(true, false, streaming, true);
    }
}

#[test]
fn real_listener_opaque_reasoning_excludes_only_standard_and_executes_native_owner() {
    opaque_boundary(false);
}

#[test]
fn real_listener_opaque_reasoning_standard_only_reports_no_eligible_candidate() {
    opaque_boundary(true);
}

fn opaque_boundary(standard_only: bool) {
    let standard = NativeProvider::start(vec![]);
    let native = NativeProvider::start(vec![reply(RESPONSES_OK)]);
    let classifier = NativeProvider::start(vec![reply(CLASSIFIER_SIMPLE)]);
    let fixture = RuntimeFixture::launch_rest_classified_with_native_control(
        &[&standard, &native, &classifier],
        2,
    );
    dialect(&fixture, standard_only);
    super::super::native_encrypted_continuation::enable_native_provider_state(&fixture);
    let body = json!({"model":MODEL,"stream":false,
        "instructions":"synthetic instruction ".repeat(4096),
        "input":[{"role":"user","content":"A_SYNTHETIC_HISTORY"},
            {"type":"reasoning","id":"rs_opaque_synthetic",
             "summary":[{"type":"summary_text","text":"Synthetic summary."}],
             "content":null,"encrypted_content":"synthetic opaque state"},
            {"role":"user","content":"B_INDEPENDENT_SYNTHETIC_INPUT"}]});
    let response = request(
        fixture.address,
        "POST",
        "/v1/responses",
        &[
            ("X-HiRoute-Token", TOKEN),
            ("session-id", "opaque-boundary"),
        ],
        &serde_json::to_vec(&body).unwrap(),
    );
    assert_eq!(
        response.status,
        if standard_only { 502 } else { 200 },
        "valid opaque state excludes a candidate, not the entire planner: {}",
        String::from_utf8_lossy(&response.body)
    );
    let received: Value = serde_json::from_slice(&response.body).unwrap();
    if standard_only {
        assert_eq!(received["error"]["code"], "NO_ELIGIBLE_CANDIDATE");
        assert_eq!(native.calls(), 0);
    } else {
        assert_eq!(received["status"], "completed");
        assert_eq!(received["output"][0]["content"][0]["text"], "ok");
        assert_eq!(native.calls(), 1);
        assert_eq!(classifier_body(&native, 0)["input"], body["input"]);
    }
    assert_eq!(
        standard.calls(),
        0,
        "ciphertext never reaches standard Responses"
    );
    assert_eq!(classifier.calls(), 1);
    let facts = wait_execution_facts(&fixture, 1);
    let excluded = facts
        .iter()
        .filter(|event| {
            event["fact"]["kind"] == "candidate_decision"
                && event["fact"]["exclusion_reason"] == "OPAQUE_STATE_UNPORTABLE"
        })
        .count();
    assert_eq!(excluded, if standard_only { 2 } else { 1 });
    let attempts = facts
        .iter()
        .filter(|event| event["fact"]["kind"] == "attempt_finished")
        .collect::<Vec<_>>();
    if standard_only {
        assert!(attempts.is_empty());
    } else {
        assert_eq!(attempts.len(), 1);
        assert_eq!(
            attempts[0]["fact"]["provider_model_event"],
            "response_complete"
        );
    }
}

pub(super) fn tool_reply(streaming: bool) -> ProviderReply {
    if streaming {
        let output = serde_json::from_slice::<Value>(NULLABLE_TOOL).unwrap()["output"]
            .as_array()
            .unwrap()
            .clone();
        super::super::native_encrypted_continuation::streamed("nullable-tool", output)
    } else {
        ProviderReply::Complete {
            status: 200,
            error_kind: None,
            body: NULLABLE_TOOL,
        }
    }
}

pub(super) fn assert_replayed_shape(output: &[Value]) {
    assert_eq!(output.len(), 2);
    assert_eq!(
        output[0],
        json!({"type":"reasoning","id":"rs_synthetic",
        "summary":[{"type":"summary_text","text":"Inspect the synthetic fixture before answering."}],
        "content":null,"encrypted_content":null})
    );
    assert_eq!(output[1]["name"], "exec_command");
    assert_eq!(output[1]["call_id"], "call");
    assert_eq!(
        output[1]["arguments"],
        "{\"cmd\":\"printf SYNTHETIC_RECEIPT\"}"
    );
    assert!(output[1].get("encrypted_function_args").is_none());
    assert!(output[1].get("status").is_none());
}
