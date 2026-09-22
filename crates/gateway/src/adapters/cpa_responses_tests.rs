//! Independently authored Rust cases informed by CPA's native fidelity/signature tests.
//! Provenance, deliberate differences and missing route-level coverage:
//! docs/cpa-responses-test-migration.md.
use super::*;
use crate::server::core_runtime::model_ir::{ContentPart, ToolResultStatusV1};
use crate::server::core_runtime::profiles::{
    CandidateProtocolProfile, ClientProtocolProfile, Fidelity, NativeProviderStateEmission,
    StateAffinity, fixed_reasoning,
};
use crate::server::request_plan::IngressProtocol;
use serde_json::{Value, json};

fn profile() -> CandidateProtocolProfile {
    let mut profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "native-luna",
        fixed_reasoning("fixed"),
    );
    profile.capability.native_provider_state = NativeProviderStateEmission::ExactOwnerAffine;
    profile.capability.request.provider_state = Fidelity::Exact;
    profile.capability.request.state_affinity = StateAffinity::ExactOwner;
    profile.capability.response.provider_state = Fidelity::Exact;
    profile.capability.response.state_affinity = StateAffinity::ExactOwner;
    profile
}

fn native_events(terminal_tier: &str) -> Vec<Value> {
    let reasoning = json!({"type":"reasoning","id":"rs_fixture","summary":[],
        "encrypted_content":"synthetic-final-state"});
    let message = json!({"type":"message","id":"msg_fixture","role":"assistant",
        "status":"completed","content":[{"type":"output_text","text":"translated","annotations":[]}]});
    vec![
        json!({"type":"response.created","response":{"id":"resp_fixture","model":"native-luna",
            "status":"in_progress","metadata":{},"service_tier":"auto"}}),
        json!({"type":"response.output_item.added","output_index":0,
            "item":{"type":"reasoning","id":"rs_fixture","summary":[],"encrypted_content":"synthetic-initial-state"}}),
        json!({"type":"response.output_item.done","output_index":0,"item":reasoning}),
        json!({"type":"response.output_item.added","output_index":1,
            "item":{"type":"message","id":"msg_fixture","role":"assistant","status":"in_progress","content":[]}}),
        json!({"type":"response.output_text.delta","output_index":1,"item_id":"msg_fixture",
            "content_index":0,"delta":"translated"}),
        json!({"type":"response.output_text.done","output_index":1,"item_id":"msg_fixture",
            "content_index":0,"text":"translated"}),
        json!({"type":"response.output_item.done","output_index":1,"item":message}),
        json!({"type":"response.completed","response":{"id":"resp_fixture","model":"native-luna",
            "status":"completed","metadata":{},"service_tier":terminal_tier,
            "output":[reasoning,message],"future_additive_field":{"ok":true},
            "usage":{"input_tokens":4,"output_tokens":2,"total_tokens":6}}}),
    ]
}

fn round_trip_stream(terminal_tier: &str, chunk_size: usize) -> Vec<RenderedSseEvent> {
    let profile = profile();
    let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
    let mut renderer = IncrementalClientSseRenderer::new(
        ClientProtocolProfile::for_candidate(&profile).unwrap(),
        "hiroute-fixture",
    )
    .unwrap();
    let mut rendered = Vec::new();
    for event in native_events(terminal_tier) {
        let bytes = format!("data: {event}\n\n");
        for chunk in bytes.as_bytes().chunks(chunk_size) {
            let mut status = decoder.feed(chunk, false).unwrap();
            loop {
                for decoded in decoder.take_events() {
                    rendered.extend(renderer.push(&decoded).unwrap());
                }
                if status != ResponseDecodeStatus::NeedDrain {
                    break;
                }
                status = decoder.resume().unwrap();
            }
        }
    }
    assert_eq!(
        decoder.feed(&[], true).unwrap(),
        ResponseDecodeStatus::Terminal
    );
    for decoded in decoder.take_events() {
        rendered.extend(renderer.push(&decoded).unwrap());
    }
    decoder.finish().unwrap();
    let terminal = rendered
        .iter()
        .find(|event| event.data["type"] == "response.completed")
        .unwrap();
    assert_eq!(terminal.data["response"]["service_tier"], terminal_tier);
    assert_eq!(
        terminal.data["response"]["future_additive_field"],
        json!({"ok":true})
    );
    rendered
}

fn assert_one_complete_answer(rendered: &[RenderedSseEvent]) -> &Value {
    let completed: Vec<_> = rendered
        .iter()
        .filter(|event| event.data["type"] == "response.completed")
        .collect();
    assert_eq!(
        completed.len(),
        1,
        "exactly one successful terminal is required"
    );
    let response = &completed[0].data["response"];
    assert_eq!(response["status"], "completed");
    assert_eq!(response["model"], "hiroute-fixture");
    assert_eq!(response["usage"]["input_tokens"], 4);
    assert_eq!(response["usage"]["output_tokens"], 2);
    let deltas: String = rendered
        .iter()
        .filter(|event| event.data["type"] == "response.output_text.delta")
        .map(|event| event.data["delta"].as_str().unwrap())
        .collect();
    assert_eq!(
        deltas, "translated",
        "terminal output must not repeat streamed text"
    );
    let output = response["output"].as_array().unwrap();
    assert_eq!(output.len(), 2);
    assert_eq!(output[0]["encrypted_content"], "synthetic-final-state");
    assert_eq!(output[1]["content"][0]["text"], "translated");
    response
}

#[test]
fn cpa_native_terminal_and_final_state_survive_chunk_boundaries() {
    for chunk_size in [1, 17, usize::MAX] {
        assert_one_complete_answer(&round_trip_stream("auto", chunk_size));
    }
}

#[test]
fn cpa_real_auto_to_default_terminal_completes_once() {
    for chunk_size in [1, 17, usize::MAX] {
        assert_one_complete_answer(&round_trip_stream("default", chunk_size));
    }
}

#[test]
fn cpa_native_output_replays_final_state_once_to_its_owner() {
    let rendered = round_trip_stream("auto", 17);
    let response = assert_one_complete_answer(&rendered);
    let mut input = response["output"].as_array().unwrap().clone();
    input.push(
        json!({"type":"message","role":"user","content":[{"type":"input_text","text":"continue"}]}),
    );
    assert_replay_to_owner(input);
}

#[test]
fn cpa_native_client_reasoning_is_preserved_once_for_its_owner() {
    // Codex's observed request shape omits reasoning.status. Keep this independent
    // of the output-to-input consistency regression above.
    assert_replay_to_owner(vec![
        json!({"type":"reasoning","id":"rs_fixture","summary":[],"encrypted_content":"synthetic-final-state"}),
        json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"translated"}]}),
        json!({"type":"message","role":"user","content":[{"type":"input_text","text":"continue"}]}),
    ]);
}

fn assert_replay_to_owner(input: Vec<Value>) {
    let profile = profile();
    let request = decode_ingress_request_with_bindings(
        IngressProtocol::Responses,
        &json!({"model":"hiroute-fixture","input":input}),
        &IngressRequestBindings {
            provider_state_owner: Some(profile.exact_provider_path().unwrap()),
            tool_id_map: Vec::new(),
        },
    )
    .unwrap();
    let projected = project_candidate_request(&request, &profile).unwrap();
    for (index, original) in input.iter().enumerate() {
        assert_eq!(
            projected.body["input"][index].get("status"),
            original.get("status")
        );
    }
    let input = projected.body["input"].as_array().unwrap();
    assert_eq!(input.len(), 3);
    assert_eq!(input[0]["type"], "reasoning");
    assert_eq!(input[0]["encrypted_content"], "synthetic-final-state");
    assert_eq!(input[1]["content"][0]["text"], "translated");
    assert_eq!(input[2]["content"][0]["text"], "continue");

    for different_owner in ["model", "account"] {
        let mut other = profile.clone();
        if different_owner == "model" {
            other.capability.native_model = "native-terra".into();
        } else {
            other.connector.entitlement_id = "different-account".into();
        }
        assert_eq!(
            project_candidate_request(&request, &other).unwrap_err(),
            ProtocolAdapterError::ModelIr(ModelIrError::ProviderStateNotPortable),
        );
    }
}

#[test]
fn cpa_native_eof_after_text_does_not_fabricate_completion() {
    let mut decoder = NativeResponseDecoder::new(&profile(), 200, true).unwrap();
    let mut events = native_events("auto");
    events.pop();
    for event in events {
        decoder
            .feed(format!("data: {event}\n\n").as_bytes(), false)
            .unwrap();
        let _ = decoder.take_events();
    }
    assert_eq!(
        decoder.feed(&[], true).unwrap_err(),
        ProtocolAdapterError::ModelIr(ModelIrError::MissingTerminalEvent),
    );
}

#[test]
fn cpa_native_request_without_reasoning_does_not_gain_state() {
    let request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"hiroute-fixture","input":"hello"}),
    )
    .unwrap();
    let projected = project_candidate_request(&request, &profile()).unwrap();
    let input = projected.body["input"].as_array().unwrap();
    assert_eq!(input.len(), 1);
    assert_eq!(input[0]["role"], "user");
    assert!(
        input
            .iter()
            .all(|item| item.get("encrypted_content").is_none())
    );
}

#[test]
fn native_completion_metadata_survives_json_and_accumulated_stream() {
    let profile = profile();
    let client = ClientProtocolProfile::for_candidate(&profile).unwrap();
    let mut response = native_events("default").pop().unwrap()["response"].clone();
    response["metadata"] = json!({"tag":"test"});
    response["created_at"] = json!(12345);
    let mut decoder = NativeResponseDecoder::new(&profile, 200, false).unwrap();
    decoder
        .feed(&serde_json::to_vec(&response).unwrap(), true)
        .unwrap();
    let decoded = decoder.finish().unwrap();
    for stream in [false, true] {
        let rendered = if stream {
            ClientResponseRenderer::render_stream_with_profile(&client, "alias", &decoded.response)
        } else {
            ClientResponseRenderer::render_nonstream_with_profile(
                &client,
                "alias",
                &decoded.response,
            )
        }
        .unwrap();
        let actual = match rendered {
            RenderedClientResponse::Json { body, .. } => body,
            RenderedClientResponse::Sse { events, .. } => {
                events.last().unwrap().data["response"].clone()
            }
        };
        for field in [
            "service_tier",
            "metadata",
            "created_at",
            "future_additive_field",
        ] {
            assert_eq!(actual[field], response[field], "{field}");
        }
        assert_eq!(actual["model"], "alias");
        assert_eq!(actual["status"], "completed");
    }
}

#[test]
fn native_input_status_preserves_function_output_lifecycle_only() {
    let profile = profile();
    for item in [
        json!({"type":"message","role":"assistant","content":"answer"}),
        json!({"type":"reasoning","summary":[],"encrypted_content":"state"}),
        json!({"type":"function_call","call_id":"call","name":"tool","arguments":"{}"}),
    ] {
        for status in [
            json!("completed"),
            json!("incomplete"),
            json!("in_progress"),
            json!("unknown"),
            json!(null),
            json!(1),
        ] {
            let mut item = item.clone();
            item["status"] = status.clone();
            let decoded = decode_ingress_request_with_bindings(
                IngressProtocol::Responses,
                &json!({"model":"alias","input":[item]}),
                &IngressRequestBindings {
                    provider_state_owner: Some(profile.exact_provider_path().unwrap()),
                    tool_id_map: Vec::new(),
                },
            );
            if status == "completed" {
                assert_eq!(
                    decoded
                        .unwrap()
                        .responses_item_statuses
                        .get(&0)
                        .map(String::as_str),
                    Some("completed")
                );
            } else {
                assert!(decoded.is_err(), "accepted {status}");
            }
        }
    }

    for (status, expected) in [
        (Some("completed"), ToolResultStatusV1::Completed),
        (Some("incomplete"), ToolResultStatusV1::Failed),
        (Some("in_progress"), ToolResultStatusV1::Unknown),
        (None, ToolResultStatusV1::Unknown),
    ] {
        let mut item = json!({
            "type":"function_call_output",
            "call_id":"call",
            "output":"text says failed but is not parsed"
        });
        if let Some(status) = status {
            item["status"] = json!(status);
        }
        let decoded = decode_ingress_request_with_bindings(
            IngressProtocol::Responses,
            &json!({"model":"alias","input":[item]}),
            &IngressRequestBindings {
                provider_state_owner: Some(profile.exact_provider_path().unwrap()),
                tool_id_map: Vec::new(),
            },
        )
        .unwrap();
        assert!(matches!(
            &decoded.messages[0].content[0],
            ContentPart::ToolResult { status, .. } if *status == expected
        ));
        assert_eq!(
            decoded.responses_item_statuses.get(&0).map(String::as_str),
            status
        );
    }

    for status in [json!("failed"), json!("unknown"), json!(null), json!(1)] {
        let decoded = decode_ingress_request_with_bindings(
            IngressProtocol::Responses,
            &json!({"model":"alias","input":[{
                "type":"function_call_output",
                "call_id":"call",
                "output":"answer",
                "status":status
            }]}),
            &IngressRequestBindings {
                provider_state_owner: Some(profile.exact_provider_path().unwrap()),
                tool_id_map: Vec::new(),
            },
        );
        assert!(decoded.is_err(), "accepted {status}");
    }
}

#[test]
fn reasoning_ir_requires_explicit_encrypted_content_shape() {
    use crate::server::core_runtime::model_ir::ResponsesReasoningHistoryV1;
    let mut history = json!({"native_fields": {"summary": [], "content": [{"type":"reasoning_text","text":"plain"}]}});
    assert!(serde_json::from_value::<ResponsesReasoningHistoryV1>(history.clone()).is_err());
    for shape in ["opaque", "absent", "null", "empty"] {
        history["encrypted_content"] = json!(shape);
        let decoded: ResponsesReasoningHistoryV1 = serde_json::from_value(history.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), history);
    }
}
