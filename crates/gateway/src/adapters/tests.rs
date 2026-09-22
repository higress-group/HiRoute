use serde_json::json;

use super::*;
use crate::content_ref::{
    JsonValueExt, compact_ingress_document, externalize_model_request, model_content_refs,
};
use crate::ports::ToolContinuationScopeV1;
use crate::replay::{ReplayConfig, ReplayManager};
use crate::server::core_runtime::model_ir::{
    CanonicalTool, ContentPart, ImageSource, MessageRole, ModelIrError, RequestedReasoningControl,
    ToolChoice, ToolIdMapEntryV1, ToolKindV1, ToolResultStatusV1,
};
use crate::server::core_runtime::profiles::{
    CandidateProtocolProfile, ClientProtocolProfile, CriticalFact, Fidelity,
    NativeProviderStateEmission, StateAffinity, StreamingRefusalSemantics, fixed_reasoning,
};
use crate::server::request_plan::IngressProtocol;

#[path = "tests/native_responses.rs"]
mod native_responses;

fn exact_state_profile(
    ingress: IngressProtocol,
    upstream: IngressProtocol,
) -> CandidateProtocolProfile {
    let mut profile = CandidateProtocolProfile::exact_portable_path(
        ingress,
        upstream,
        "physical",
        fixed_reasoning("fixed"),
    );
    profile.capability.native_provider_state = NativeProviderStateEmission::ExactOwnerAffine;
    profile.capability.request.provider_state = Fidelity::Exact;
    profile.capability.request.state_affinity = StateAffinity::ExactOwner;
    profile.capability.response.provider_state = Fidelity::Exact;
    profile.capability.response.state_affinity = StateAffinity::ExactOwner;
    profile
}

use hiroute_gateway_core::runtime::body::BudgetTree;

#[test]
fn protocol_ingress_rejects_every_unmodeled_field() {
    let error = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"alias","input":"hello","temperature":0.5}),
    )
    .unwrap_err();
    assert!(matches!(error, ModelIrError::UnsupportedField(_)));
}

#[test]
fn messages_ingress_accepts_only_the_known_claude_transport_hints() {
    let request = decode_ingress_request(
        IngressProtocol::Messages,
        &json!({
            "model": "alias",
            "max_tokens": 1024,
            "metadata": {"user_id": "local-user"},
            "context_management": {
                "edits": [{"type": "clear_thinking_20251015", "keep": "all"}]
            },
            "system": [
                {"type": "text", "text": "system", "cache_control": {"type": "ephemeral"}}
            ],
            "messages": [
                {
                    "role": "user",
                    "content": [{
                        "type": "text",
                        "text": "hello",
                        "cache_control": {"type": "ephemeral", "ttl": "1h"}
                    }]
                },
                {"role": "system", "content": "client reminder"}
            ]
        }),
    )
    .unwrap();
    assert_eq!(request.messages.len(), 1);
    assert_eq!(request.instructions.len(), 2);

    let error = decode_ingress_request(
        IngressProtocol::Messages,
        &json!({
            "model": "alias",
            "max_tokens": 1024,
            "metadata": {"tenant": "must-not-be-dropped"},
            "messages": [{"role": "user", "content": "hello"}]
        }),
    )
    .unwrap_err();
    assert!(matches!(error, ModelIrError::UnsupportedField(_)));
}

#[test]
fn messages_ingress_accepts_claude_tool_result_cache_control() {
    let request = decode_ingress_request(
        IngressProtocol::Messages,
        &json!({
            "model": "alias",
            "max_tokens": 1024,
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "hiroute_tool_v1_fixture",
                    "content": "tool output",
                    "cache_control": {"type": "ephemeral"}
                }]
            }]
        }),
    )
    .unwrap();
    assert!(matches!(
        &request.messages[0].content[0],
        ContentPart::ToolResult {
            logical_id,
            status: ToolResultStatusV1::Completed,
            ..
        }
            if logical_id == "hiroute_tool_v1_fixture"
    ));

    let error = decode_ingress_request(
        IngressProtocol::Messages,
        &json!({
            "model": "alias",
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "hiroute_tool_v1_fixture",
                    "content": "tool output",
                    "cache_control": {"type": "persistent"}
                }]
            }]
        }),
    )
    .unwrap_err();
    assert!(matches!(error, ModelIrError::UnsupportedValue(_)));
}

#[test]
fn messages_ingress_accepts_claude_failed_tool_result() {
    let request = decode_ingress_request(
        IngressProtocol::Messages,
        &json!({
            "model": "alias",
            "max_tokens": 1024,
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "hiroute_tool_v1_fixture",
                    "content": "File does not exist.",
                    "is_error": true
                }]
            }]
        }),
    )
    .unwrap();
    assert!(matches!(
        &request.messages[0].content[0],
        ContentPart::ToolResult {
            logical_id,
            output: crate::server::core_runtime::model_ir::ToolOutput::Text(output),
            status: ToolResultStatusV1::Failed,
            ..
        } if logical_id == "hiroute_tool_v1_fixture" && output == "File does not exist."
    ));

    for upstream in [IngressProtocol::Messages, IngressProtocol::Responses] {
        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Messages,
            upstream,
            "physical",
            fixed_reasoning("fixed"),
        );
        let mut continued = request.clone();
        continued.tool_id_map = vec![ToolIdMapEntryV1 {
            logical_id: "hiroute_tool_v1_fixture".into(),
            native_id: "native-tool-id".into(),
            kind: ToolKindV1::Function,
            name: "Read".into(),
            namespace: None,
            owner: profile.exact_provider_path().unwrap(),
        }];
        let projected = project_candidate_request(&continued, &profile).unwrap();
        if upstream == IngressProtocol::Messages {
            assert_eq!(
                projected.body["messages"][0]["content"][0]["is_error"],
                true
            );
        } else {
            assert_eq!(projected.body["input"][0]["type"], "function_call_output");
            assert_eq!(projected.body["input"][0]["output"], "File does not exist.");
            assert!(projected.body["input"][0].get("is_error").is_none());
        }
    }
}

#[test]
fn protocols_without_explicit_tool_result_status_remain_unknown() {
    let chat = decode_ingress_request(
        IngressProtocol::ChatCompletions,
        &json!({
            "model":"alias",
            "messages":[{
                "role":"tool",
                "tool_call_id":"call-1",
                "content":"tool failed and was cancelled"
            }]
        }),
    )
    .unwrap();
    assert!(matches!(
        &chat.messages[0].content[0],
        ContentPart::ToolResult {
            status: ToolResultStatusV1::Unknown,
            ..
        }
    ));

    let custom = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({
            "model":"alias",
            "input":[{
                "type":"custom_tool_call_output",
                "call_id":"call-2",
                "output":"completed successfully"
            }]
        }),
    )
    .unwrap();
    assert!(matches!(
        &custom.messages[0].content[0],
        ContentPart::ToolResult {
            status: ToolResultStatusV1::Unknown,
            ..
        }
    ));
}

#[test]
fn messages_ingress_rejects_context_management_that_can_change_semantics() {
    for context_management in [
        json!({
            "edits": [{
                "type": "clear_thinking_20251015",
                "keep": {"type": "thinking_turns", "value": 2}
            }]
        }),
        json!({
            "edits": [{"type": "clear_tool_uses_20250919", "keep": "all"}]
        }),
        json!({
            "edits": [
                {"type": "clear_thinking_20251015", "keep": "all"},
                {"type": "clear_thinking_20251015", "keep": "all"}
            ]
        }),
        json!({
            "edits": [{
                "type": "clear_thinking_20251015",
                "keep": "all",
                "trigger": {"type": "input_tokens", "value": 1000}
            }]
        }),
    ] {
        let error = decode_ingress_request(
            IngressProtocol::Messages,
            &json!({
                "model": "alias",
                "max_tokens": 1024,
                "context_management": context_management,
                "messages": [{"role": "user", "content": "hello"}]
            }),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ModelIrError::InvalidField(_)
                | ModelIrError::UnsupportedField(_)
                | ModelIrError::UnsupportedValue(_)
        ));
    }
}

#[test]
fn protocol_chat_preserves_mid_conversation_instruction_position() {
    let request = decode_ingress_request(
        IngressProtocol::ChatCompletions,
        &json!({
            "model":"alias",
            "messages":[
                {"role":"system","content":"initial"},
                {"role":"user","content":"question"},
                {"role":"developer","content":"mid-turn"}
            ]
        }),
    )
    .unwrap();
    assert_eq!(request.instructions.len(), 1);
    assert_eq!(request.messages.len(), 2);
    assert_eq!(request.messages[1].role, MessageRole::Developer);
    assert!(request.requirements().mid_conversation_instructions);
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::ChatCompletions,
        IngressProtocol::Messages,
        "physical",
        fixed_reasoning("fixed"),
    );
    assert_eq!(
        project_candidate_request(&request, &profile)
            .unwrap_err()
            .code(),
        "PROTOCOL_CAPABILITY_UNSUPPORTED"
    );
}

#[test]
fn protocol_unknown_connector_fact_rejects_projection() {
    let request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"alias","input":"hello"}),
    )
    .unwrap();
    let mut profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    profile.connector.errors = CriticalFact::Unknown;
    assert_eq!(
        project_candidate_request(&request, &profile)
            .unwrap_err()
            .code(),
        "PROTOCOL_CAPABILITY_UNSUPPORTED"
    );
}

#[test]
fn protocol_eof_never_synthesizes_a_terminal_event() {
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
    let error = decoder
        .feed(
            b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"r\",\"model\":\"m\"}}\n\n",
            true,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        ProtocolAdapterError::ModelIr(ModelIrError::MissingTerminalEvent)
    ));
}

#[test]
fn responses_decoder_accepts_current_codex_stream_metadata() {
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
    let status = decoder
        .feed(
            br#"event: response.created
data: {"type":"response.created","sequence_number":0,"response":{"id":"r","model":"physical"}}

event: response.output_item.added
data: {"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":{"type":"reasoning","id":"reasoning","summary":[],"content":[],"encrypted_content":"opaque"}}

event: response.output_item.done
data: {"type":"response.output_item.done","sequence_number":2,"output_index":0,"item":{"type":"reasoning","id":"reasoning","summary":[],"content":[],"encrypted_content":"opaque"}}

event: response.output_item.added
data: {"type":"response.output_item.added","sequence_number":3,"output_index":1,"item":{"type":"message","id":"m","status":"in_progress","role":"assistant","content":[],"phase":"final_answer"}}

event: response.output_text.delta
data: {"type":"response.output_text.delta","sequence_number":4,"item_id":"m","output_index":1,"content_index":0,"delta":"ok","logprobs":[],"obfuscation":"noise"}

event: response.output_text.done
data: {"type":"response.output_text.done","sequence_number":5,"item_id":"m","output_index":1,"content_index":0,"text":"ok","logprobs":[],"attribution":null}

event: response.output_item.done
data: {"type":"response.output_item.done","sequence_number":6,"output_index":1,"item":{"type":"message","id":"m","status":"completed","role":"assistant","content":[],"phase":"final_answer"}}

event: response.completed
data: {"type":"response.completed","sequence_number":7,"response":{"id":"r","model":"physical","status":"completed"}}

"#,
            true,
        )
        .unwrap();

    assert_eq!(status, ResponseDecodeStatus::Terminal);
    decoder.finish().unwrap();
}

#[test]
fn responses_bridge_preserves_final_opaque_state_in_messages_lifecycle() {
    let profile = exact_state_profile(IngressProtocol::Messages, IngressProtocol::Responses);
    let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
    assert_eq!(
        decoder
            .feed(
                br#"event: response.created
data: {"type":"response.created","sequence_number":0,"response":{"id":"r","model":"physical","status":"in_progress","metadata":{},"service_tier":"auto"}}

event: response.in_progress
data: {"type":"response.in_progress","sequence_number":1,"response":{"id":"r","model":"physical","status":"in_progress","metadata":{},"service_tier":"auto"}}

event: response.output_item.added
data: {"type":"response.output_item.added","sequence_number":2,"output_index":0,"item":{"type":"reasoning","id":"reasoning","summary":[],"content":[],"encrypted_content":"initial-opaque-state"}}

event: response.output_item.done
data: {"type":"response.output_item.done","sequence_number":3,"output_index":0,"item":{"type":"reasoning","id":"reasoning","summary":[],"content":[],"encrypted_content":"final-opaque-state"}}

event: response.output_item.added
data: {"type":"response.output_item.added","sequence_number":4,"output_index":1,"item":{"type":"message","id":"message","status":"in_progress","role":"assistant","content":[],"phase":"final_answer"}}

event: response.content_part.added
data: {"type":"response.content_part.added","sequence_number":5,"item_id":"message","output_index":1,"content_index":0,"part":{"type":"output_text","annotations":[],"logprobs":[],"text":""}}

event: response.output_text.delta
data: {"type":"response.output_text.delta","sequence_number":6,"item_id":"message","output_index":1,"content_index":0,"delta":"HIROUTE-SUBSCRIPTION-E2E-OK","logprobs":[],"obfuscation":"noise"}

event: response.output_text.done
data: {"type":"response.output_text.done","sequence_number":7,"item_id":"message","output_index":1,"content_index":0,"text":"HIROUTE-SUBSCRIPTION-E2E-OK","logprobs":[]}

event: response.content_part.done
data: {"type":"response.content_part.done","sequence_number":8,"item_id":"message","output_index":1,"content_index":0,"part":{"type":"output_text","annotations":[],"logprobs":[],"text":"HIROUTE-SUBSCRIPTION-E2E-OK"}}

event: response.output_item.done
data: {"type":"response.output_item.done","sequence_number":9,"output_index":1,"item":{"type":"message","id":"message","status":"completed","role":"assistant","content":[{"type":"output_text","annotations":[],"logprobs":[],"text":"HIROUTE-SUBSCRIPTION-E2E-OK"}],"phase":"final_answer"}}

event: response.completed
data: {"type":"response.completed","sequence_number":10,"response":{"id":"r","model":"physical","status":"completed","metadata":{},"service_tier":"auto","output":[{"type":"reasoning","id":"reasoning","summary":[],"content":[],"encrypted_content":"final-opaque-state"},{"type":"message","id":"message","status":"completed","role":"assistant","content":[{"type":"output_text","annotations":[],"logprobs":[],"text":"HIROUTE-SUBSCRIPTION-E2E-OK"}],"phase":"final_answer"}],"usage":{"input_tokens":20,"input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0},"output_tokens":140,"output_tokens_details":{"reasoning_tokens":122},"total_tokens":160,"attribution":{}}}}

"#,
                true,
            )
            .unwrap(),
        ResponseDecodeStatus::Terminal,
    );

    let mut renderer = IncrementalClientSseRenderer::new(
        ClientProtocolProfile::for_candidate(&profile).unwrap(),
        "served-alias",
    )
    .unwrap();
    let mut rendered = Vec::new();
    for event in decoder.take_events() {
        rendered.extend(renderer.push(&event).unwrap());
    }
    let lifecycle = rendered
        .iter()
        .filter_map(|event| event.event.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(
        lifecycle,
        [
            "message_start",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop",
        ]
    );
    assert_eq!(rendered[1].data["content_block"]["type"], "thinking");
    assert_eq!(rendered[2].data["delta"]["text"], serde_json::Value::Null);
    assert_eq!(rendered[2].data["delta"]["signature"], "final-opaque-state");
    assert_eq!(rendered[4].data["content_block"]["type"], "text");
    assert_eq!(
        rendered[5].data["delta"]["text"],
        "HIROUTE-SUBSCRIPTION-E2E-OK"
    );
    decoder.finish().unwrap();
}

#[test]
fn responses_bridge_rejects_non_answer_message_phase() {
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Messages,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let mut decoder = NativeResponseDecoder::new(&profile, 200, false).unwrap();
    decoder
        .feed(
            br#"{"id":"r","model":"physical","status":"completed","output":[{"type":"message","id":"m","status":"completed","role":"assistant","phase":"analysis","content":[{"type":"output_text","text":"must not become an ordinary answer","annotations":[]}]}]}"#,
            true,
        )
        .unwrap();
    let decoded = decoder.finish().unwrap();
    let client = ClientProtocolProfile::for_candidate(&profile).unwrap();

    assert!(matches!(
        ClientResponseRenderer::render_nonstream_with_profile(
            &client,
            "served-alias",
            &decoded.response,
        ),
        Err(ProtocolAdapterError::ClientUnrepresentable(_))
    ));
}

#[test]
fn responses_bridge_uses_added_state_only_when_done_omits_it() {
    let profile = exact_state_profile(IngressProtocol::Messages, IngressProtocol::Responses);
    let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
    decoder
        .feed(
            br#"event: response.created
data: {"type":"response.created","response":{"id":"r","model":"physical"}}

event: response.output_item.added
data: {"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"reasoning","summary":[],"encrypted_content":"fallback-state"}}

event: response.output_item.done
data: {"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","id":"reasoning","status":"completed","summary":[]}}

event: response.completed
data: {"type":"response.completed","response":{"id":"r","model":"physical","status":"completed","output":[{"type":"reasoning","id":"reasoning","status":"completed","summary":[]}]}}

"#,
            true,
        )
        .unwrap();
    let decoded = decoder.finish().unwrap();
    assert_eq!(decoded.response.provider_state.len(), 1);
    assert_eq!(
        decoded.response.provider_state[0].value,
        json!("fallback-state")
    );
}

#[test]
fn responses_bridge_rejects_completed_state_that_replaces_done_state() {
    let profile = exact_state_profile(IngressProtocol::Messages, IngressProtocol::Responses);
    let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
    let error = decoder
        .feed(
            br#"event: response.created
data: {"type":"response.created","response":{"id":"r","model":"physical"}}

event: response.output_item.added
data: {"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"reasoning","summary":[],"encrypted_content":"early"}}

event: response.output_item.done
data: {"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","id":"reasoning","status":"completed","summary":[],"encrypted_content":"done"}}

event: response.completed
data: {"type":"response.completed","response":{"id":"r","model":"physical","status":"completed","output":[{"type":"reasoning","id":"reasoning","status":"completed","summary":[],"encrypted_content":"conflict"}]}}

"#,
            true,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        ProtocolAdapterError::ModelIr(ModelIrError::InvalidResponseLifecycle(_))
    ));
}

#[test]
fn messages_thinking_signature_replays_as_responses_reasoning_item_in_order() {
    let profile = exact_state_profile(IngressProtocol::Messages, IngressProtocol::Responses);
    let owner = profile.exact_provider_path().unwrap();
    let request = decode_ingress_request_with_bindings(
        IngressProtocol::Messages,
        &json!({
            "model":"alias",
            "max_tokens":128,
            "messages":[{
                "role":"assistant",
                "content":[
                    {"type":"text","text":"before"},
                    {"type":"thinking","thinking":"not replayed","signature":"opaque-replay"},
                    {"type":"text","text":"after"}
                ]
            }]
        }),
        &IngressRequestBindings {
            provider_state_owner: Some(owner),
            tool_id_map: Vec::new(),
        },
    )
    .unwrap();
    let projected = project_candidate_request(&request, &profile).unwrap();
    assert_eq!(
        projected.body["input"],
        json!([
            {"type":"message","role":"assistant","content":[{"type":"output_text","text":"before"}]},
            {"type":"reasoning","summary":[],"content":null,"encrypted_content":"opaque-replay"},
            {"type":"message","role":"assistant","content":[{"type":"output_text","text":"after"}]}
        ])
    );
}

#[test]
fn responses_completed_output_must_match_accepted_stream_content() {
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
    let error = decoder
        .feed(
            br#"event: response.created
data: {"type":"response.created","sequence_number":0,"response":{"id":"r","model":"physical"}}

event: response.output_item.added
data: {"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":{"type":"message","id":"m","status":"in_progress","role":"assistant","content":[],"phase":"final_answer"}}

event: response.output_text.delta
data: {"type":"response.output_text.delta","sequence_number":2,"item_id":"m","output_index":0,"content_index":0,"delta":"accepted"}

event: response.output_text.done
data: {"type":"response.output_text.done","sequence_number":3,"item_id":"m","output_index":0,"content_index":0,"text":"accepted"}

event: response.output_item.done
data: {"type":"response.output_item.done","sequence_number":4,"output_index":0,"item":{"type":"message","id":"m","status":"completed","role":"assistant","content":[{"type":"output_text","text":"accepted","annotations":[]}],"phase":"final_answer"}}

event: response.completed
data: {"type":"response.completed","sequence_number":5,"response":{"id":"r","model":"physical","status":"completed","output":[{"type":"message","id":"m","status":"completed","role":"assistant","content":[{"type":"output_text","text":"conflict","annotations":[]}],"phase":"final_answer"}]}}

"#,
            true,
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ProtocolAdapterError::ModelIr(ModelIrError::InvalidResponseLifecycle(_))
    ));
}

#[test]
fn responses_decoder_tolerates_additive_event_metadata() {
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
    assert_eq!(
        decoder
        .feed(
            br#"event: response.created
data: {"type":"response.created","sequence_number":0,"response":{"id":"r","model":"physical"}}

event: response.output_text.done
data: {"type":"response.output_text.done","sequence_number":1,"item_id":"m","output_index":0,"content_index":0,"text":"ok","logprobs":[],"attribution":{"source":"provider"}}

event: response.completed
data: {"type":"response.completed","sequence_number":2,"response":{"id":"r","model":"physical","status":"completed"}}

"#,
            true,
        )
            .unwrap(),
        ResponseDecodeStatus::Terminal,
    );
    decoder.finish().unwrap();
}

#[test]
fn responses_decoder_still_rejects_unknown_event_types() {
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
    let error = decoder
        .feed(
            br#"event: response.future_semantic_delta
data: {"type":"response.future_semantic_delta","sequence_number":0,"delta":"must not be dropped"}

"#,
            false,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        ProtocolAdapterError::ModelIr(ModelIrError::UnsupportedValue(_))
    ));
}

#[test]
fn responses_decoder_still_rejects_unknown_output_item_types() {
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
    let error = decoder
        .feed(
            br#"event: response.created
data: {"type":"response.created","sequence_number":0,"response":{"id":"r","model":"physical"}}

event: response.output_item.added
data: {"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":{"type":"future_semantic_block","id":"x"}}

"#,
            false,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        ProtocolAdapterError::ModelIr(ModelIrError::UnsupportedValue(_))
    ));
}

#[test]
fn responses_nonstream_decoder_tolerates_additive_output_metadata() {
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let mut decoder = NativeResponseDecoder::new(&profile, 200, false).unwrap();
    assert_eq!(
        decoder
            .feed(
                br#"{"id":"r","object":"response","created_at":1,"status":"completed","error":null,"incomplete_details":null,"instructions":null,"max_output_tokens":64,"model":"physical","output":[{"type":"message","id":"m","status":"completed","role":"assistant","content":[{"type":"output_text","text":"ok","annotations":[],"logprobs":null,"attribution":{"source":"provider"}}]}],"parallel_tool_calls":false,"previous_response_id":null,"reasoning":null,"store":false,"temperature":null,"text":null,"tool_choice":"auto","tools":[],"top_p":null,"truncation":"disabled","usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2,"input_tokens_details":{"cached_tokens":0},"output_tokens_details":{"reasoning_tokens":0}},"user":null,"metadata":{},"service_tier":"auto"}"#,
                true,
            )
            .unwrap(),
        ResponseDecodeStatus::Terminal,
    );
    decoder.finish().unwrap();
}

#[test]
fn responses_nonstream_decoder_rejects_malformed_envelope_metadata() {
    for body in [
        json!({
            "id":"r", "model":"physical", "status":"completed", "output":[],
            "metadata":123, "service_tier":"auto"
        }),
        json!({
            "id":"r", "model":"physical", "status":"completed", "output":[],
            "metadata":{}, "service_tier":true
        }),
    ] {
        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "physical",
            fixed_reasoning("fixed"),
        );
        let mut decoder = NativeResponseDecoder::new(&profile, 200, false).unwrap();
        let error = decoder
            .feed(&serde_json::to_vec(&body).unwrap(), true)
            .unwrap_err();
        assert!(matches!(
            error,
            ProtocolAdapterError::ModelIr(ModelIrError::InvalidField(_))
        ));
    }
}

#[test]
fn responses_reasoning_usage_projects_to_messages_billable_total() {
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Messages,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
    assert_eq!(
        decoder
            .feed(
                br#"event: response.created
data: {"type":"response.created","sequence_number":0,"response":{"id":"r","model":"physical"}}

event: response.output_item.added
data: {"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":{"type":"message","id":"m","status":"in_progress","role":"assistant","content":[]}}

event: response.output_text.delta
data: {"type":"response.output_text.delta","sequence_number":2,"item_id":"m","output_index":0,"content_index":0,"delta":"ok"}

event: response.completed
data: {"type":"response.completed","sequence_number":3,"response":{"id":"r","model":"physical","status":"completed","usage":{"input_tokens":10,"output_tokens":7,"total_tokens":17,"output_tokens_details":{"reasoning_tokens":5}}}}

"#,
                true,
            )
            .unwrap(),
        ResponseDecodeStatus::Terminal,
    );

    let mut renderer = IncrementalClientSseRenderer::new(
        ClientProtocolProfile::for_candidate(&profile).unwrap(),
        "served-alias",
    )
    .unwrap();
    let mut wire = Vec::new();
    for event in decoder.take_events() {
        for rendered in renderer.push(&event).unwrap() {
            wire.extend_from_slice(&rendered.wire_bytes().unwrap());
        }
    }
    decoder.finish().unwrap();

    let wire = String::from_utf8(wire).unwrap();
    assert!(wire.contains("\"usage\":{\"input_tokens\":10,\"output_tokens\":7}"));
    assert!(wire.contains("event: message_stop"));
}

#[test]
fn messages_terminal_delta_completes_without_waiting_for_message_stop() {
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Messages,
        IngressProtocol::Messages,
        "physical",
        fixed_reasoning("fixed"),
    );
    let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
    let status = decoder
        .feed(
            br#"event: message_start
data: {"type":"message_start","message":{"id":"m","type":"message","role":"assistant","model":"physical","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":0}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ok"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"input_tokens":1,"output_tokens":1}}

"#,
            false,
        )
        .unwrap();
    assert_eq!(status, ResponseDecodeStatus::Terminal);
    assert!(decoder.take_events().iter().any(|event| {
        matches!(
            event.event,
            crate::server::core_runtime::model_ir::ModelEvent::ResponseCompleted { .. }
        )
    }));
    assert_eq!(
        decoder.feed(&[], true).unwrap(),
        ResponseDecodeStatus::Terminal
    );
    decoder.finish().unwrap();
}

#[test]
fn observation_decoder_projects_request_bound_tool_id_without_ambient_authority() {
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let owner = profile.exact_provider_path().unwrap();
    let projection = ToolLogicalIdProjection::for_test(
        [7_u8; 32],
        ToolContinuationScopeV1 {
            authority_id: "authority".into(),
            authority_epoch: 1,
            grant_id: "grant".into(),
            grant_generation: 1,
            served_model_id: "agent".into(),
            route: hiroute_domain::ModelRequestRouteV2::Plan {
                revision: 2,
                semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(b"plan"),
            },
        },
    );
    let expected = projection
        .project(
            "capture-response",
            0,
            "provider-native-secret",
            ToolKindV1::Function,
            None,
            "weather",
            &owner,
        )
        .unwrap();
    let decoded = std::thread::spawn(move || {
        let mut decoder =
            NativeResponseDecoder::new_for_observation(&profile, 200, false, projection, None)
                .unwrap();
        decoder
            .feed(
                br#"{"id":"capture-response","model":"physical","status":"completed","output":[{"type":"function_call","id":"fc-native","call_id":"provider-native-secret","name":"weather","arguments":"{}","status":"completed"}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#,
                true,
            )
            .unwrap();
        decoder.finish().unwrap()
    })
    .join()
    .unwrap();
    let (logical_id, native_id) = decoded
        .events
        .iter()
        .find_map(|event| match &event.event {
            crate::server::core_runtime::model_ir::ModelEvent::ToolCallStarted {
                logical_id,
                native_id,
                ..
            } => Some((logical_id, native_id)),
            _ => None,
        })
        .expect("Tool start event");
    assert_eq!(logical_id, &expected);
    assert_eq!(native_id, "provider-native-secret");
    assert!(logical_id.starts_with("hiroute_tool_v1_"));
}

fn assert_nonstream_only_profile(upstream: IngressProtocol) {
    let nonstream = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"alias","input":"hello","stream":false}),
    )
    .unwrap();
    let streaming = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"alias","input":"hello","stream":true}),
    )
    .unwrap();
    let mut profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        upstream,
        "physical",
        fixed_reasoning("fixed"),
    );
    profile.capability.native_streaming = CriticalFact::Exact(false);
    profile.capability.response.stream_refusal = StreamingRefusalSemantics::Unsupported;

    project_candidate_request(&nonstream, &profile).unwrap();
    assert!(NativeResponseDecoder::new(&profile, 200, false).is_ok());
    assert_eq!(
        project_candidate_request(&streaming, &profile)
            .unwrap_err()
            .code(),
        "PROTOCOL_CAPABILITY_UNSUPPORTED"
    );
    assert_eq!(
        NativeResponseDecoder::new(&profile, 200, true)
            .err()
            .unwrap()
            .code(),
        "PROTOCOL_CAPABILITY_UNSUPPORTED"
    );

    let mut no_refusal = profile;
    no_refusal.capability.response.refusal = Fidelity::Unsupported;
    assert_eq!(
        project_candidate_request(&nonstream, &no_refusal)
            .unwrap_err()
            .code(),
        "PROTOCOL_CAPABILITY_UNSUPPORTED"
    );
    assert_eq!(
        NativeResponseDecoder::new(&no_refusal, 200, false)
            .err()
            .unwrap()
            .code(),
        "PROTOCOL_CAPABILITY_UNSUPPORTED"
    );
}

#[test]
fn protocol_responses_nonstream_profile_does_not_require_stream_refusal() {
    assert_nonstream_only_profile(IngressProtocol::Responses);
}

#[test]
fn protocol_chat_nonstream_profile_does_not_require_stream_refusal() {
    assert_nonstream_only_profile(IngressProtocol::ChatCompletions);
}

#[test]
fn protocol_messages_nonstream_profile_does_not_require_stream_refusal() {
    assert_nonstream_only_profile(IngressProtocol::Messages);
}

#[test]
fn replay_sequential_encoder_matches_native_bytes_for_two_attempts() {
    let text = "quotes: \" slash: \\ newline:\n unicode: 路由 ".repeat(512);
    let mut request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"alias","input":text,"stream":true}),
    )
    .expect("decode request");
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let expected = project_candidate_request(&request, &profile)
        .expect("inline projection")
        .bytes;

    let root = std::env::temp_dir().join(format!(
        "hiroute-adapter-replay-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let manager = ReplayManager::open(ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 128,
        record_bytes: 31,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .expect("replay manager");
    let tree = BudgetTree::new(4 * 1024 * 1024, 4 * 1024 * 1024).expect("budget tree");
    let budget = tree.stream(4 * 1024 * 1024).expect("stream budget");
    let store = manager.begin_request(budget.clone()).expect("replay store");
    externalize_model_request(&mut request, &store, 8).expect("externalize");
    let template = project_candidate_request_template(&request, &profile).expect("template");
    assert_eq!(template.wire_len, expected.len());
    assert!(template.bytes.len() < expected.len());
    store
        .prevalidate(&model_content_refs(&request))
        .expect("prevalidate replay backing");

    for _ in 0..2 {
        let mut reader = sequential_attempt_body(template.clone(), store.clone(), &budget, 37)
            .expect("attempt reader");
        let mut actual = Vec::new();
        while let Some(chunk) = reader.next_chunk().expect("sequential chunk") {
            assert!(chunk.bytes().len() <= 37);
            actual.extend_from_slice(chunk.bytes());
        }
        reader.release();
        assert_eq!(actual, expected);
    }

    drop(store);
    drop(manager);
    assert_eq!(budget.snapshot().expect("snapshot").live, 0);
    fs_err_remove_dir(&root);
}

#[test]
fn replay_sequential_encoder_streams_large_json_in_raw_and_string_positions() {
    let mut canonical = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({
            "model":"alias",
            "input":[
                {"type":"function_call","call_id":"logical","name":"weather","arguments":
                    serde_json::to_string(&json!({"query":"Paris".repeat(4096)})).unwrap()},
                {"type":"function_call_output","call_id":"logical","output":{
                    "nested":{"a":"forecast".repeat(4096),"z":2},"temperature":21
                }}
            ]
        }),
    )
    .expect("decode JSON tool roundtrip");
    canonical.tools.push(CanonicalTool {
        kind: ToolKindV1::Function,
        name: "weather".into(),
        description: Some("large tool description ".repeat(1024)),
        input_schema: Some(json!({
            "type":"object",
            "properties":{"query":{"description":"schema detail ".repeat(2048),"type":"string"}}
        })),
        strict: None,
        format: None,
    });
    canonical.tool_choice = ToolChoice::Auto;
    canonical.requested_reasoning = RequestedReasoningControl::overridden(json!({
        "requested_native_control": "retained but profile-authoritative".repeat(1024)
    }));

    let root = std::env::temp_dir().join(format!(
        "hiroute-adapter-json-replay-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let manager = ReplayManager::open(ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 128,
        record_bytes: 31,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .expect("replay manager");
    let tree = BudgetTree::new(16 * 1024 * 1024, 16 * 1024 * 1024).expect("budget tree");
    let budget = tree.stream(16 * 1024 * 1024).expect("stream budget");
    let store = manager.begin_request(budget.clone()).expect("replay store");

    let mut prepared = Vec::new();
    let mut references = Vec::new();
    for target in [
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        IngressProtocol::Messages,
    ] {
        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            target,
            format!("physical-{target:?}"),
            fixed_reasoning("fixed"),
        );
        let mut request = canonical.clone();
        request.tool_id_map = vec![ToolIdMapEntryV1 {
            logical_id: "logical".into(),
            native_id: format!("native-{target:?}"),
            kind: ToolKindV1::Function,
            name: "weather".into(),
            namespace: None,
            owner: profile.exact_provider_path().expect("provider path"),
        }];
        let expected = project_candidate_request(&request, &profile)
            .expect("inline JSON projection")
            .bytes;
        externalize_model_request(&mut request, &store, 32).expect("externalize JSON fields");
        assert!(project_candidate_request(&request, &profile).is_err());
        let template = project_candidate_request_template(&request, &profile).expect("template");
        assert_eq!(template.wire_len, expected.len());
        assert!(template.bytes.len() < expected.len());
        references.extend(model_content_refs(&request));
        prepared.push((target, template, expected));
    }
    store
        .prevalidate(&references)
        .expect("prevalidate JSON replay backing");
    for (target, template, expected) in prepared {
        let mut reader =
            sequential_attempt_body(template, store.clone(), &budget, 43).expect("attempt reader");
        let mut actual = Vec::new();
        while let Some(chunk) = reader.next_chunk().expect("sequential JSON chunk") {
            assert!(chunk.bytes().len() <= 43);
            actual.extend_from_slice(chunk.bytes());
        }
        assert_eq!(actual, expected, "target {target:?}");
    }

    drop(store);
    drop(manager);
    assert_eq!(budget.snapshot().expect("snapshot").live, 0);
    fs_err_remove_dir(&root);
}

#[test]
fn replay_sequential_encoder_preserves_externalized_custom_input_wrappers() {
    let custom_input = "quotes: \" slash: \\ newline:\n unicode: 路由 ".repeat(512);
    let canonical = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({
            "model":"alias",
            "input":[
                {"type":"custom_tool_call","call_id":"logical","name":"shell","input":custom_input},
                {"type":"custom_tool_call_output","call_id":"logical","output":"done"}
            ],
            "tools":[{"type":"custom","name":"shell"}]
        }),
    )
    .expect("decode custom tool roundtrip");

    let root = std::env::temp_dir().join(format!(
        "hiroute-adapter-custom-replay-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let manager = ReplayManager::open(ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 128,
        record_bytes: 31,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .expect("replay manager");
    let tree = BudgetTree::new(16 * 1024 * 1024, 16 * 1024 * 1024).expect("budget tree");
    let budget = tree.stream(16 * 1024 * 1024).expect("stream budget");
    let store = manager.begin_request(budget.clone()).expect("replay store");

    let mut prepared = Vec::new();
    let mut references = Vec::new();
    for target in [IngressProtocol::Responses, IngressProtocol::ChatCompletions] {
        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            target,
            format!("physical-{target:?}"),
            fixed_reasoning("fixed"),
        );
        let mut request = canonical.clone();
        request.tool_id_map = vec![ToolIdMapEntryV1 {
            logical_id: "logical".into(),
            native_id: format!("native-{target:?}"),
            kind: ToolKindV1::Custom,
            name: "shell".into(),
            namespace: None,
            owner: profile.exact_provider_path().expect("provider path"),
        }];
        let expected = project_candidate_request(&request, &profile)
            .expect("inline custom projection")
            .bytes;
        externalize_model_request(&mut request, &store, 16).expect("externalize custom input");
        assert!(request.messages[0].content.iter().any(|part| {
            matches!(
                part,
                ContentPart::ToolCall { arguments, .. }
                    if arguments.content_ref().is_some()
            )
        }));
        let template =
            project_candidate_request_template(&request, &profile).expect("custom input template");
        assert_eq!(template.wire_len, expected.len());
        references.extend(model_content_refs(&request));
        prepared.push((target, template, expected));
    }
    store
        .prevalidate(&references)
        .expect("prevalidate custom input replay backing");

    for (target, template, expected) in prepared {
        let mut reader =
            sequential_attempt_body(template, store.clone(), &budget, 43).expect("attempt reader");
        let mut actual = Vec::new();
        while let Some(chunk) = reader.next_chunk().expect("custom input chunk") {
            actual.extend_from_slice(chunk.bytes());
        }
        assert_eq!(actual, expected, "target {target:?}");
        let body: serde_json::Value = serde_json::from_slice(&actual).expect("provider JSON");
        match target {
            IngressProtocol::Responses => assert_eq!(body["input"][0]["input"], custom_input),
            IngressProtocol::ChatCompletions => {
                let arguments = body["messages"][0]["tool_calls"][0]["function"]["arguments"]
                    .as_str()
                    .expect("Chat arguments");
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(arguments).unwrap(),
                    json!({"input":custom_input})
                );
            }
            IngressProtocol::Messages => unreachable!(),
        }
    }

    drop(store);
    drop(manager);
    assert_eq!(budget.snapshot().expect("snapshot").live, 0);
    fs_err_remove_dir(&root);
}

#[test]
fn replay_externalization_keeps_tool_identity_keys_comparable_for_chat_projection() {
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        "physical-chat",
        fixed_reasoning("fixed"),
    );
    let mapping = ToolIdMapEntryV1 {
        logical_id: "logical".into(),
        native_id: "native".into(),
        kind: ToolKindV1::Function,
        name: "lookup".into(),
        namespace: Some("records".into()),
        owner: profile.exact_provider_path().expect("provider path"),
    };
    let document = json!({
        "model":"alias",
        "input":[
            {"type":"message","role":"user","content":[{"type":"input_text","text":"x".repeat(8 * 1024)}]},
            {"type":"function_call","call_id":"logical","namespace":"records","name":"lookup","arguments":"{\"id\":7}"},
            {"type":"function_call_output","call_id":"logical","output":"found"}
        ],
        "tools":[{"type":"namespace","name":"records","tools":[
            {"type":"function","name":"lookup","parameters":{"type":"object"}}
        ]}],
        "tool_choice":{"type":"function","name":"lookup"}
    });
    let inline =
        decode_ingress_request_with_tool_resolver(IngressProtocol::Responses, &document, |_| {
            Ok(vec![mapping.clone()])
        })
        .expect("decode inline namespaced continuation");
    let expected = project_candidate_request(&inline, &profile)
        .expect("inline Chat projection")
        .bytes;

    let root = std::env::temp_dir().join(format!(
        "hiroute-adapter-identity-replay-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let manager = ReplayManager::open(ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 128,
        record_bytes: 31,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .expect("replay manager");
    let tree = BudgetTree::new(4 * 1024 * 1024, 4 * 1024 * 1024).expect("budget tree");
    let budget = tree.stream(4 * 1024 * 1024).expect("stream budget");
    let store = manager.begin_request(budget.clone()).expect("replay store");

    let mut compacted = document;
    compact_ingress_document(IngressProtocol::Responses, &mut compacted, &store)
        .expect("compact ingress payloads");
    let mut request =
        decode_ingress_request_with_tool_resolver(IngressProtocol::Responses, &compacted, |_| {
            Ok(vec![mapping])
        })
        .expect("decode compacted namespaced continuation");
    externalize_model_request(&mut request, &store, 16).expect("externalize request payloads");
    let ContentPart::ToolCall {
        namespace, name, ..
    } = &request.messages[1].content[0]
    else {
        panic!("expected function call")
    };
    assert_eq!(namespace.as_deref(), Some("records"));
    assert_eq!(name, "lookup");
    assert_eq!(request.tool_namespaces[0].name, "records");
    assert_eq!(request.tool_namespaces[0].tools[0].name, "lookup");
    assert!(matches!(
        &request.tool_choice,
        ToolChoice::RequiredNamed { name, .. } if name == "lookup"
    ));

    let template = project_candidate_request_template(&request, &profile)
        .expect("externalized Chat projection");
    assert_eq!(template.wire_len, expected.len());
    store
        .prevalidate(&model_content_refs(&request))
        .expect("prevalidate replay backing");
    let mut reader =
        sequential_attempt_body(template, store.clone(), &budget, 29).expect("attempt reader");
    let mut actual = Vec::new();
    while let Some(chunk) = reader.next_chunk().expect("identity replay chunk") {
        actual.extend_from_slice(chunk.bytes());
    }
    reader.release();
    assert_eq!(actual, expected);
    let body: serde_json::Value = serde_json::from_slice(&actual).expect("provider JSON");
    assert_eq!(body["tools"][0]["function"]["name"], "records__lookup");
    assert_eq!(body["tool_choice"]["function"]["name"], "records__lookup");

    drop(store);
    drop(manager);
    assert_eq!(budget.snapshot().expect("snapshot").live, 0);
    fs_err_remove_dir(&root);
}

#[test]
fn replay_large_responses_and_chat_data_uris_preserve_base64_in_messages() {
    let payload = "QUJD".repeat(3_000);
    assert!(payload.len() > 8 * 1024);
    let cases = [
        (
            IngressProtocol::Responses,
            json!({
                "model":"alias",
                "input":[{
                    "type":"message",
                    "role":"user",
                    "content":[{
                        "type":"input_image",
                        "image_url":format!("data:image/png;base64,{payload}")
                    }]
                }]
            }),
        ),
        (
            IngressProtocol::ChatCompletions,
            json!({
                "model":"alias",
                "messages":[{
                    "role":"user",
                    "content":[{
                        "type":"image_url",
                        "image_url":{"url":format!("data:image/png;base64,{payload}")}
                    }]
                }]
            }),
        ),
    ];

    for (protocol, mut document) in cases {
        let root = std::env::temp_dir().join(format!(
            "hiroute-adapter-image-replay-{protocol:?}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let manager = ReplayManager::open(ReplayConfig {
            root: root.clone(),
            memory_threshold_bytes: 128,
            record_bytes: 31,
            orphan_ttl: std::time::Duration::from_secs(60),
        })
        .expect("replay manager");
        let tree = BudgetTree::new(4 * 1024 * 1024, 4 * 1024 * 1024).expect("budget tree");
        let budget = tree.stream(4 * 1024 * 1024).expect("stream budget");
        let store = manager.begin_request(budget.clone()).expect("replay store");

        compact_ingress_document(protocol, &mut document, &store).expect("compact image payload");
        let mut request =
            decode_ingress_request(protocol, &document).expect("decode compact image");
        let ContentPart::Image {
            source: ImageSource::Base64 { media_type, data },
        } = &request.messages[0].content[0]
        else {
            panic!("image discriminator was not preserved for {protocol:?}");
        };
        assert_eq!(media_type, "image/png");
        assert!(
            crate::content_ref::ContentRef::from_wire_marker(data).is_some(),
            "only the Base64 payload must be externalized"
        );

        externalize_model_request(&mut request, &store, 8 * 1024)
            .expect("externalize compact model IR");
        let profile = CandidateProtocolProfile::exact_portable_path(
            protocol,
            IngressProtocol::Messages,
            "physical",
            fixed_reasoning("fixed"),
        );
        let template =
            project_candidate_request_template(&request, &profile).expect("Messages template");
        let references = model_content_refs(&request);
        store
            .prevalidate(&references)
            .expect("prevalidate image payload");
        let mut reader = sequential_attempt_body(template, store.clone(), &budget, 257)
            .expect("image body reader");
        let mut actual = Vec::new();
        while let Some(chunk) = reader.next_chunk().expect("image body chunk") {
            actual.extend_from_slice(chunk.bytes());
        }
        let projected: serde_json::Value =
            serde_json::from_slice(&actual).expect("Messages provider JSON");
        let source = &projected["messages"][0]["content"][0]["source"];
        assert_eq!(source["type"], "base64", "protocol {protocol:?}");
        assert_eq!(source["media_type"], "image/png");
        assert_eq!(source["data"], payload);
        assert!(source.get("url").is_none());

        drop(reader);
        drop(store);
        drop(manager);
        assert_eq!(budget.snapshot().expect("released image replay").live, 0);
        fs_err_remove_dir(&root);
    }
}

#[test]
fn replay_aggregate_inline_budget_prevents_fragmented_full_wire_copy() {
    let input = (0..32)
        .map(|index| {
            json!({
                "type":"message",
                "role":"user",
                "content":[{
                    "type":"input_text",
                    "text":format!("fragment-{index:02}-{}", "x".repeat(1012))
                }]
            })
        })
        .collect::<Vec<_>>();
    let mut request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"alias","input":input,"stream":false}),
    )
    .expect("decode fragmented request");
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let expected = project_candidate_request(&request, &profile)
        .expect("inline projection")
        .bytes;

    let root = std::env::temp_dir().join(format!(
        "hiroute-adapter-fragmented-replay-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let manager = ReplayManager::open(ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 4 * 1024,
        record_bytes: 1024,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .expect("replay manager");
    let tree = BudgetTree::new(4 * 1024 * 1024, 4 * 1024 * 1024).expect("budget tree");
    let budget = tree.stream(4 * 1024 * 1024).expect("stream budget");
    let store = manager.begin_request(budget.clone()).expect("replay store");

    externalize_model_request(&mut request, &store, 4 * 1024)
        .expect("externalize aggregate overflow");
    let template = project_candidate_request_template(&request, &profile).expect("template");
    assert_eq!(template.wire_len, expected.len());
    assert!(
        template.bytes.len() < expected.len() / 2,
        "aggregate-small fields must not rebuild the large wire body"
    );

    store
        .prevalidate(&model_content_refs(&request))
        .expect("prevalidate fragmented replay");
    let mut reader =
        sequential_attempt_body(template, store.clone(), &budget, 257).expect("attempt reader");
    let mut actual = Vec::new();
    while let Some(chunk) = reader.next_chunk().expect("sequential chunk") {
        actual.extend_from_slice(chunk.bytes());
    }
    assert_eq!(actual, expected);

    drop(reader);
    drop(store);
    drop(manager);
    assert_eq!(budget.snapshot().expect("snapshot").live, 0);
    fs_err_remove_dir(&root);
}

#[test]
fn replay_ten_thousand_small_fields_use_one_stream_and_linear_template_scan() {
    let input = (0..10_000)
        .map(|index| {
            json!({
                "type":"message",
                "role":"user",
                "content":[{"type":"input_text","text":format!("part-{index}")}]
            })
        })
        .collect::<Vec<_>>();
    let mut request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"alias","input":input,"stream":false}),
    )
    .expect("decode fragmented request");
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let root = std::env::temp_dir().join(format!(
        "hiroute-adapter-ten-thousand-replay-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let manager = ReplayManager::open(ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 1024,
        record_bytes: 1024,
        orphan_ttl: std::time::Duration::from_secs(60),
    })
    .expect("replay manager");
    let tree = BudgetTree::new(16 * 1024 * 1024, 16 * 1024 * 1024).expect("budget tree");
    let budget = tree.stream(16 * 1024 * 1024).expect("stream budget");
    let store = manager.begin_request(budget.clone()).expect("replay store");
    let started = std::time::Instant::now();

    externalize_model_request(&mut request, &store, 8 * 1024)
        .expect("externalize ten thousand fields");
    assert_eq!(store.snapshot().live_streams, 1);
    let references = model_content_refs(&request);
    assert!(references.len() > 1_000);
    let template = project_candidate_request_template(&request, &profile).expect("linear template");
    assert_eq!(template.replacements.len(), references.len());
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    assert!(budget.snapshot().expect("budget snapshot").live < 8 * 1024 * 1024);
    store
        .prevalidate(&references)
        .expect("prevalidate aggregate backing");

    drop(template);
    drop(store);
    drop(manager);
    assert_eq!(budget.snapshot().expect("released budget").live, 0);
    fs_err_remove_dir(&root);
}

fn fs_err_remove_dir(path: &std::path::Path) {
    match std::fs::remove_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("remove replay test root: {error}"),
    }
}
