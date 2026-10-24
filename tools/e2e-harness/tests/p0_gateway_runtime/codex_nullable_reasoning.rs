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
