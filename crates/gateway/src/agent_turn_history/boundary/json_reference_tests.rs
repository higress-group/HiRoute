//! Semantic JSON references must preserve the accepted tool boundary.
//! Run in both the isolated Gateway graph and the product graph which enables
//! serde_json/preserve_order. Raw argument spellings exercise that distinction.
use super::*;
use crate::agent_turn_history::boundary::{BoundaryCapture, incoming_boundary};
use crate::context_hold::{branch_anchors, fingerprint::FingerprintBudget};
use crate::server::core_runtime::model_ir::{ModelEvent, ResponseItemStatus};

const ORDERED: &str = r#"{"a":{"x":1,"y":2},"list":[1,2],"z":3}"#;
const REORDERED: &str = r#"{"z":3,"list":[1,2],"a":{"y":2,"x":1}}"#;

fn captured_tool() -> BoundaryCapture {
    let mut capture = BoundaryCapture::default();
    capture.accept(&ModelEvent::ToolCallStarted {
        index: 0,
        logical_id: "call".into(),
        native_id: "call".into(),
        tool_kind: ToolKindV1::Function,
        namespace: None,
        name: "read".into(),
        owner: Box::new(super::accepted_output::owner()),
        item_id: None,
    });
    capture.accept(&ModelEvent::ToolCallFinished {
        encrypted_function_args: None,
        index: 0,
        logical_id: "call".into(),
        tool_kind: ToolKindV1::Function,
        namespace: None,
        name: "read".into(),
        arguments: serde_json::from_str(ORDERED).unwrap(),
        status: ResponseItemStatus::Completed,
    });
    capture
}

fn fingerprints(
    protocol: IngressProtocol,
    prefix_bytes: usize,
    arguments: &str,
) -> ([u8; 32], [u8; 32]) {
    let (_directory, replay) = replay();
    let user = if prefix_bytes == 0 {
        "actual task".to_owned()
    } else {
        "x".repeat(prefix_bytes)
    };
    let body = match protocol {
        IngressProtocol::Responses => json!({"model":"route","input":[
            {"role":"user","content":user},
            {"type":"function_call","call_id":"call","name":"read","arguments":arguments},
            {"type":"function_call_output","call_id":"call","output":"receipt"}
        ]}),
        IngressProtocol::ChatCompletions => json!({"model":"route","messages":[
            {"role":"user","content":user},
            {"role":"assistant","content":null,"tool_calls":[
                {"id":"call","type":"function","function":{"name":"read","arguments":arguments}}
            ]},
            {"role":"tool","tool_call_id":"call","content":"receipt"}
        ]}),
        IngressProtocol::Messages => unreachable!(),
    };
    // The production helper scans/parses raw ingress, compacts semantic fields,
    // decodes the protocol, then externalizes Model IR. In the large-prefix
    // case the preceding user consumes exactly the ingress 8 KiB allowance.
    let request =
        super::reasoning_boundary::production_request_for(protocol, &body, &replay, 8 * 1024);
    let ContentPart::ToolCall { arguments, .. } = request.messages[1]
        .content
        .iter()
        .find(|part| matches!(part, ContentPart::ToolCall { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    assert!(
        crate::content_ref::JsonValueExt::content_ref(arguments).is_some(),
        "the assertion must compare production Replay JSON, not only inline Value"
    );
    let boundary = incoming_boundary(&request, 1, &replay, &mut FingerprintBudget::default())
        .expect("small supported tool arguments remain comparable");
    let anchors = branch_anchors(&request, &[41; 32], Some(&replay), &[])
        .expect("the complete supported history remains comparable");
    (boundary, anchors.complete)
}

fn assert_order_equivalence(protocol: IngressProtocol, prefix_bytes: usize) {
    let expected = captured_tool().digest().unwrap();
    let ordered = fingerprints(protocol, prefix_bytes, ORDERED);
    let reordered = fingerprints(protocol, prefix_bytes, REORDERED);
    assert_eq!(ordered.0, expected, "ordered native arguments");
    assert_eq!(reordered.0, expected, "reordered native arguments");
    assert_eq!(ordered.1, reordered.1, "complete history uses JSON meaning");
}

#[test]
fn responses_small_tool_json_reference_matches_accepted_output() {
    assert_order_equivalence(IngressProtocol::Responses, 0);
}

#[test]
fn chat_small_tool_json_reference_matches_accepted_output() {
    assert_order_equivalence(IngressProtocol::ChatCompletions, 0);
}

#[test]
fn responses_tool_json_reference_matches_after_8k_ingress_prefix() {
    assert_order_equivalence(IngressProtocol::Responses, 8 * 1024);
}

#[test]
fn chat_tool_json_reference_matches_after_8k_ingress_prefix() {
    assert_order_equivalence(IngressProtocol::ChatCompletions, 8 * 1024);
}

#[test]
fn tool_json_references_keep_array_order_and_unknown_argument_values_significant() {
    for protocol in [IngressProtocol::Responses, IngressProtocol::ChatCompletions] {
        for prefix_bytes in [0, 8 * 1024] {
            let original = fingerprints(protocol, prefix_bytes, ORDERED);
            for changed in [
                r#"{"a":{"x":1,"y":2},"list":[2,1],"z":3}"#,
                r#"{"a":{"x":1,"y":9},"list":[1,2],"z":3}"#,
                r#"{"a":{"x":1,"y":2},"list":[1,2],"z":"3"}"#,
            ] {
                let changed = fingerprints(protocol, prefix_bytes, changed);
                assert_ne!(original.0, changed.0, "accepted tool output must differ");
                assert_ne!(original.1, changed.1, "full history must differ");
            }
        }
    }
}
