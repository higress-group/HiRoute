use super::*;
use crate::server::core_runtime::adapters::decode_ingress_request;
use crate::server::core_runtime::model_ir::ResponsesReasoningHistoryV1;
use crate::server::request_plan::IngressProtocol;
use serde_json::json;

fn request(role: &str) -> ModelRequestIRV1 {
    decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"route","stream":false,"input":[
            {"role":role,"content":"first instructions"},
            {"role":"user","content":"actual task"},
            {"role":"assistant","content":"accepted output"},
            {"type":"function_call","call_id":"call","name":"lookup","arguments":"{}"},
            {"type":"function_call_output","call_id":"call","output":"tool receipt"}
        ]}),
    )
    .unwrap()
}

fn measure(request: &ModelRequestIRV1, prefix: usize) -> HistoryDigests {
    visible_history(request, &[37; 32])
        .unwrap()
        .measure(Some(prefix))
        .unwrap()
}

#[test]
fn only_same_position_system_or_developer_text_is_omitted_from_semantic_digest() {
    for role in ["system", "developer"] {
        let original = request(role);
        let before = measure(&original, original.messages.len());
        let mut changed = original.clone();
        changed.messages[0].content[0] = ContentPart::Text {
            text: "new native instructions and skills".into(),
        };
        let after = measure(&changed, original.messages.len());
        assert_ne!(before.complete_digest, after.complete_digest);
        assert_eq!(
            before.semantic_complete_digest,
            after.semantic_complete_digest
        );
        assert_eq!(
            Some(before.semantic_complete_digest),
            after.semantic_prefix_digest
        );
        assert_ne!(Some(before.complete_digest), after.prefix_digest);
    }
}

#[test]
fn semantic_digest_keeps_roles_content_tools_metadata_and_message_positions_strict() {
    let original = request("system");
    let baseline = measure(&original, original.messages.len());
    let mut variants = Vec::new();
    for index in [1, 2] {
        let mut value = original.clone();
        value.messages[index].content[0] = ContentPart::Text {
            text: "rewritten".into(),
        };
        variants.push(value);
    }
    let mut role = original.clone();
    role.messages[0].role = MessageRole::Developer;
    variants.push(role);
    let mut name = original.clone();
    name.messages[0].name = Some("changed".into());
    variants.push(name);
    let mut part_count = original.clone();
    part_count.messages[0].content.push(ContentPart::Text {
        text: "extra block".into(),
    });
    variants.push(part_count);
    let mut arguments = original.clone();
    let ContentPart::ToolCall {
        arguments: value, ..
    } = &mut arguments.messages[3].content[0]
    else {
        panic!()
    };
    *value = json!({"changed":true});
    variants.push(arguments);
    let mut output = original.clone();
    let ContentPart::ToolResult { output: value, .. } = &mut output.messages[4].content[0] else {
        panic!()
    };
    *value = ToolOutput::Text("changed receipt".into());
    variants.push(output);
    let mut metadata = original.clone();
    metadata.responses_reasoning_history.insert(
        0,
        ResponsesReasoningHistoryV1 {
            native_fields: serde_json::Map::from_iter([(
                "summary".into(),
                json!("native sidecar"),
            )]),
            encrypted_content: ResponsesReasoningEncryptedContentV1::Absent,
        },
    );
    variants.push(metadata);
    let mut shifted = original.clone();
    shifted.messages.insert(0, original.messages[0].clone());
    variants.push(shifted);
    for changed in variants {
        let measured = measure(&changed, original.messages.len());
        assert_ne!(
            Some(baseline.semantic_complete_digest),
            measured.semantic_prefix_digest
        );
    }

    // Non-text instruction content is also strict, including a tool part in an
    // instruction envelope; the exception is never a role-wide content skip.
    let mut with_tool = original.clone();
    with_tool.messages[0]
        .content
        .push(original.messages[3].content[0].clone());
    let before = measure(&with_tool, with_tool.messages.len());
    let ContentPart::ToolCall { name, .. } = &mut with_tool.messages[0].content[1] else {
        panic!()
    };
    *name = "different_tool".into();
    assert_ne!(
        before.semantic_complete_digest,
        measure(&with_tool, with_tool.messages.len()).semantic_complete_digest
    );
}
