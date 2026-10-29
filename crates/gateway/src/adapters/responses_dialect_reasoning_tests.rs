//! Synthetic values in the Responses history shape emitted by Codex 0.147.0.
//! This exercises the Gateway boundary, not an actual client or model run.
use super::*;
use crate::server::core_runtime::model_ir::{
    ContentPart, ModelRequestIRV1, ResponsesReasoningEncryptedContentV1 as Encrypted,
};

pub(super) fn producer_request(encrypted: Option<Value>) -> Value {
    let mut body = fixture();
    body["reasoning"] = json!({"effort":"high","summary":"auto"});
    let mut reasoning = json!({"type":"reasoning","id":"rs_synthetic_reasoning_0000000000000000000",
        "summary":[{"type":"summary_text","text":"Inspect the synthetic fixture before answering."}],
        "content":null});
    if let Some(value) = encrypted {
        reasoning["encrypted_content"] = value;
    }
    body["input"] = json!([
        {"role":"user","content":"A_READ_SYNTHETIC_FIXTURE"},
        reasoning,
        {"type":"function_call","id":"fc_synthetic_call_00000000000000000000000",
         "call_id":"call_synthetic_fixture","name":"exec_command",
         "arguments":"{\"cmd\":\"printf SYNTHETIC_RECEIPT\"}"},
        {"type":"function_call_output","id":"fo_synthetic_result_000000000000000000000",
         "call_id":"call_synthetic_fixture","output":"exit_code=0; SYNTHETIC_RECEIPT"},
        {"role":"user","content":"B_INDEPENDENT_SYNTHETIC_TASK"}
    ]);
    body
}

#[test]
fn codex_nullable_reasoning_and_plain_tool_receipt_project_without_changing_history() {
    for (wire, state) in [
        (None, Encrypted::Absent),
        (Some(Value::Null), Encrypted::Null),
        (Some(json!("")), Encrypted::Empty),
    ] {
        let body = producer_request(wire.clone());
        let request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
        assert!(!request.native_only);
        assert_eq!(
            request.responses_reasoning_history[&1].encrypted_content,
            state
        );
        assert!(request.messages[1].content.is_empty());
        assert!(request.responses_item_statuses.is_empty());
        let original = request.clone();
        for standard in [false, true] {
            let output = project_candidate_request(&request, &profile(standard))
                .expect("absent/null/empty encrypted_content is not opaque provider state");
            assert_eq!(output.body["input"], body["input"]);
            assert_eq!(
                output.body["input"][1].get("encrypted_content"),
                wire.as_ref()
            );
            assert!(
                output.body["input"][2]
                    .get("encrypted_function_args")
                    .is_none()
            );
            assert!(output.body["input"][3].get("status").is_none());
            assert_eq!(request, original);
        }
        // The typed history also owns projection after the request-local native tree
        // is absent. Serde must preserve each state instead of creating ciphertext.
        let restored: ModelRequestIRV1 =
            serde_json::from_value(serde_json::to_value(&request).unwrap()).unwrap();
        assert!(restored.native_body.is_none());
        for standard in [false, true] {
            let output = project_candidate_request(&restored, &profile(standard)).unwrap();
            assert_eq!(output.body["input"][1], body["input"][1]);
            assert_eq!(output.body["input"][2], body["input"][2]);
            assert_eq!(output.body["input"][3], body["input"][3]);
        }
    }
}

#[test]
fn opaque_reasoning_stays_native_including_whitespace_and_legacy_ir_without_sidecar() {
    for encrypted in ["ciphertext", " \n\t"] {
        let body = producer_request(Some(json!(encrypted)));
        let request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
        assert_eq!(
            request.responses_reasoning_history[&1].encrypted_content,
            Encrypted::Opaque
        );
        assert!(matches!(
            request.messages[1].content[0],
            ContentPart::ProviderState { .. }
        ));
        let original = request.clone();
        assert_eq!(
            project_candidate_request(&request, &profile(false))
                .unwrap()
                .body["input"],
            body["input"]
        );
        assert_eq!(
            project_candidate_request(&request, &profile(true)).unwrap_err(),
            ProtocolAdapterError::ModelIr(ModelIrError::ProviderStateNotPortable)
        );
        assert_eq!(request, original);
        let mut legacy: ModelRequestIRV1 =
            serde_json::from_value(serde_json::to_value(&request).unwrap()).unwrap();
        legacy.responses_reasoning_history.clear();
        assert_eq!(
            project_candidate_request(&legacy, &profile(true)).unwrap_err(),
            ProtocolAdapterError::ModelIr(ModelIrError::ProviderStateNotPortable)
        );
        assert_eq!(
            project_candidate_request(&legacy, &profile(false))
                .unwrap()
                .body["input"][1]["encrypted_content"],
            encrypted
        );
    }
}

#[test]
fn malformed_encrypted_reasoning_remains_an_ingress_error() {
    for malformed in [json!([]), json!({}), json!(true), json!(7), json!(1.5)] {
        assert_eq!(
            decode_ingress_request(
                IngressProtocol::Responses,
                &producer_request(Some(malformed))
            )
            .unwrap_err(),
            ModelIrError::InvalidField("encrypted_content")
        );
    }
}

#[test]
fn standard_reasoning_projection_preserves_unknown_native_fields_without_promoting_them() {
    let mut body = producer_request(Some(Value::Null));
    body["input"][1]["future_control"] = json!({"mode":"synthetic"});
    let request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
    assert!(request.native_only);
    for standard in [false, true] {
        assert_eq!(
            project_candidate_request(&request, &profile(standard))
                .unwrap()
                .body["input"],
            body["input"]
        );
    }
    for upstream in [IngressProtocol::ChatCompletions, IngressProtocol::Messages] {
        let p = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            upstream,
            "physical",
            fixed_reasoning("fixed"),
        );
        assert!(project_candidate_request(&request, &p).is_err());
    }
}

pub(super) fn prefix_request(retained: Option<Value>) -> Value {
    let mut body = producer_request(Some(json!("old ciphertext")));
    body["input"]
        .as_array_mut()
        .unwrap()
        .insert(0, json!({"type":"additional_tools","tools":[]}));
    let mut current = body["input"][2].clone();
    current["id"] = json!("rs_current_synthetic");
    current.as_object_mut().unwrap().remove("encrypted_content");
    if let Some(value) = retained {
        current["encrypted_content"] = value;
    }
    body["input"].as_array_mut().unwrap().insert(3, current);
    body
}

#[test]
fn standard_reasoning_gate_uses_retained_canonical_positions_before_prefix_cleanup() {
    for retained in [
        None,
        Some(Value::Null),
        Some(json!("")),
        Some(json!("new ciphertext")),
    ] {
        let body = prefix_request(retained.clone());
        let request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
        assert_eq!(
            request.messages.len(),
            6,
            "additional_tools is not a message"
        );
        assert_eq!(
            request
                .responses_reasoning_history
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            [1, 2]
        );
        let original = request.clone();
        for (end, rejected) in [
            (1, true),
            (2, retained.as_ref().is_some_and(|v| v == "new ciphertext")),
            (3, false),
        ] {
            let projected = project_candidate_request_template_with_cleanup(
                &request,
                &profile(true),
                Some(end),
            );
            if rejected {
                assert_eq!(
                    projected.unwrap_err(),
                    ProtocolAdapterError::ModelIr(ModelIrError::ProviderStateNotPortable)
                );
            } else {
                let value: Value = serde_json::from_slice(&projected.unwrap().bytes).unwrap();
                let mut expected = body["input"].as_array().unwrap().clone();
                if end == 3 {
                    expected.remove(3);
                }
                expected.remove(2);
                assert_eq!(value["input"], json!(expected));
            }
            assert_eq!(request, original);
        }
    }
}
