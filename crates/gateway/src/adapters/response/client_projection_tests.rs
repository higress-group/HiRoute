use super::*;
use crate::server::core_runtime::adapters::{
    ClientResponseRenderer, DecodedNativeResponse, NativeResponseDecoder, RenderedClientResponse,
    ResponseDecodeStatus,
};
use crate::server::core_runtime::model_ir::{
    FinishReason, ModelStreamEventV1, ModelUsage, OpaqueProviderState, ResponseBlock,
    ResponseItemStatus,
};
use crate::server::core_runtime::profiles::{
    CandidateProtocolProfile, Fidelity, NativeProviderStateEmission, fixed_reasoning,
};
use serde_json::{Value, json};

const CIPHER: &str = "opaque-state-projection-control";
const SUMMARY: &str = "private reasoning projection control";

fn candidate(ingress: IngressProtocol) -> CandidateProtocolProfile {
    let mut candidate = CandidateProtocolProfile::exact_portable_path(
        ingress,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    candidate.capability.native_provider_state = NativeProviderStateEmission::Native;
    candidate.capability.request.provider_state = Fidelity::Exact;
    candidate.capability.response.provider_state = Fidelity::Exact;
    candidate
}

fn response(summary: &str) -> Value {
    json!({
        "id":"projection-response", "model":"physical", "status":"completed",
        "output":[
            {"type":"reasoning", "id":"reason", "status":"completed",
                "summary":if summary.is_empty() { vec![] } else {
                    vec![json!({"type":"summary_text", "text":summary})]
                }, "encrypted_content":CIPHER},
            {"type":"message", "id":"answer", "role":"assistant", "status":"completed",
                "content":[{"type":"output_text", "text":"visible answer", "annotations":[]}]},
            {"type":"function_call", "id":"tool", "call_id":"call-probe", "name":"probe",
                "arguments":"{\"value\":1}", "status":"completed"}
        ],
        "usage":{"input_tokens":7, "output_tokens":19, "total_tokens":26}
    })
}

fn response_stream(response: &Value) -> Vec<u8> {
    let mut frames = vec![json!({"type":"response.created", "response":{
        "id":response["id"], "model":response["model"], "status":"in_progress", "output":[]
    }})];
    for (index, item) in response["output"].as_array().unwrap().iter().enumerate() {
        let mut start = item.clone();
        start["status"] = json!("in_progress");
        match item["type"].as_str().unwrap() {
            "reasoning" => {
                start.as_object_mut().unwrap().remove("encrypted_content");
                start["summary"] = json!([]);
            }
            "message" => start["content"] = json!([]),
            "function_call" => start["arguments"] = json!(""),
            _ => unreachable!("fixture output kind"),
        }
        frames
            .push(json!({"type":"response.output_item.added", "output_index":index, "item":start}));
        match item["type"].as_str().unwrap() {
            "message" => {
                let text = &item["content"][0]["text"];
                frames.extend([
                    json!({"type":"response.output_text.delta", "output_index":index,
                        "item_id":item["id"], "content_index":0, "delta":text}),
                    json!({"type":"response.output_text.done", "output_index":index,
                        "item_id":item["id"], "content_index":0, "text":text}),
                ]);
            }
            "function_call" => {
                frames.extend([
                    json!({"type":"response.function_call_arguments.delta", "output_index":index,
                        "item_id":item["id"], "delta":item["arguments"]}),
                    json!({"type":"response.function_call_arguments.done", "output_index":index,
                        "item_id":item["id"], "arguments":item["arguments"]}),
                ]);
            }
            "reasoning" => {}
            _ => unreachable!("fixture output kind"),
        }
        frames.push(json!({"type":"response.output_item.done", "output_index":index, "item":item}));
    }
    frames.push(json!({"type":"response.completed", "response":response}));
    frames
        .into_iter()
        .flat_map(|frame| format!("data: {frame}\n\n").into_bytes())
        .collect()
}

fn decode(
    candidate: &CandidateProtocolProfile,
    response: &Value,
    streaming: bool,
) -> DecodedNativeResponse {
    let wire = if streaming {
        response_stream(response)
    } else {
        serde_json::to_vec(response).unwrap()
    };
    let mut decoder = NativeResponseDecoder::new(candidate, 200, streaming).unwrap();
    assert_ne!(
        decoder.feed(&wire, true).unwrap(),
        ResponseDecodeStatus::NeedDrain,
        "the bounded fixture must be consumed completely"
    );
    decoder.finish().unwrap()
}

#[test]
fn responses_opaque_projection_matches_stream_and_buffered_client_delivery() {
    for ingress in [
        IngressProtocol::ChatCompletions,
        IngressProtocol::Messages,
        IngressProtocol::Responses,
    ] {
        for streaming in [false, true] {
            for summary in ["", SUMMARY] {
                let candidate = candidate(ingress);
                let profile = ClientProtocolProfile::for_candidate(&candidate).unwrap();
                let decoded = decode(&candidate, &response(summary), streaming);
                let state = decoded
                    .events
                    .iter()
                    .find(|event| matches!(&event.event, ModelEvent::ProviderState { .. }))
                    .expect("the real decoder must emit opaque state, not consume it as Never");
                assert_eq!(
                    IncrementalClientSseRenderer::retains_event(&profile, &state.event),
                    ingress == IngressProtocol::Responses
                );
                let mut renderer =
                    IncrementalClientSseRenderer::new(profile.clone(), "route").unwrap();
                let mut wire = Vec::new();
                let mut retained_tool = false;
                let mut retained_text = false;
                let mut saw_reasoning = false;
                for event in &decoded.events {
                    let retained =
                        IncrementalClientSseRenderer::retains_event(&profile, &event.event);
                    if matches!(
                        &event.event,
                        ModelEvent::ContentBlockStarted {
                            block_kind: ResponseBlockKind::Reasoning,
                            ..
                        } | ModelEvent::ReasoningDelta { .. }
                            | ModelEvent::ReasoningFinished { .. }
                    ) {
                        saw_reasoning = true;
                        assert_eq!(retained, ingress != IngressProtocol::Messages);
                    }
                    retained_tool |=
                        retained && matches!(&event.event, ModelEvent::ToolCallFinished { .. });
                    retained_text |=
                        retained && matches!(&event.event, ModelEvent::TextFinished { .. });
                    let frames = renderer.push(event).unwrap();
                    if !retained {
                        assert!(
                            frames.is_empty(),
                            "omitted events must not emit client wire"
                        );
                    }
                    for frame in frames {
                        wire.extend(frame.wire_bytes().unwrap());
                    }
                }
                assert!(saw_reasoning && retained_tool && retained_text);
                assert_eq!(renderer.usage().output_tokens, Some(19));
                let text = String::from_utf8(wire).unwrap();
                assert_eq!(text.contains(CIPHER), ingress == IngressProtocol::Responses);
                assert_eq!(
                    text.contains(SUMMARY),
                    !summary.is_empty() && ingress != IngressProtocol::Messages
                );
                assert!(text.contains("visible answer") && text.contains("probe"));

                // This is capture/tracker's actual buffered entry, without a
                // ClientProtocolProfile argument. It must have the same content
                // projection as the event predicate for successful conversions.
                let RenderedClientResponse::Json { body, .. } =
                    ClientResponseRenderer::render_nonstream(ingress, "route", &decoded.response)
                        .unwrap()
                else {
                    panic!("buffered JSON expected")
                };
                let text = body.to_string();
                assert_eq!(text.contains(CIPHER), ingress == IngressProtocol::Responses);
                assert_eq!(
                    text.contains(SUMMARY),
                    !summary.is_empty() && ingress != IngressProtocol::Messages
                );
                assert!(text.contains("visible answer") && text.contains("probe"));
            }
        }
    }
}

#[test]
fn retained_reasoning_completion_can_defer_wire_until_the_native_signature() {
    let profile = ClientProtocolProfile::exact_portable(IngressProtocol::Messages);
    let mut renderer = IncrementalClientSseRenderer::new(profile.clone(), "route").unwrap();
    for (sequence, event) in [
        ModelEvent::ResponseStarted {
            response_id: "message".into(),
            provider_model: "physical".into(),
        },
        ModelEvent::ContentBlockStarted {
            index: 0,
            block_kind: ResponseBlockKind::Reasoning,
            item_id: None,
            phase: None,
        },
        ModelEvent::ReasoningDelta {
            index: 0,
            text: "thought".into(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        renderer
            .push(&ModelStreamEventV1::new(sequence as u64, event))
            .unwrap();
    }
    let finished = ModelEvent::ReasoningFinished {
        index: 0,
        text: "thought".into(),
        status: ResponseItemStatus::Completed,
    };
    assert!(IncrementalClientSseRenderer::retains_event(
        &profile, &finished
    ));
    assert!(
        renderer
            .push(&ModelStreamEventV1::new(3, finished))
            .unwrap()
            .is_empty(),
        "retained completion waits for the native signature"
    );
    let signature = ModelEvent::ProviderState {
        state: Box::new(OpaqueProviderState {
            block_index: Some(0),
            kind: "thinking_signature".into(),
            value: json!("native-signature"),
        }),
    };
    assert!(IncrementalClientSseRenderer::retains_event(
        &profile, &signature
    ));
    let frames = renderer
        .push(&ModelStreamEventV1::new(4, signature))
        .unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].data["delta"]["type"], "signature_delta");
    assert_eq!(frames[0].data["delta"]["signature"], "native-signature");
    assert_eq!(frames[1].data["type"], "content_block_stop");
    renderer
        .push(&ModelStreamEventV1::new(
            5,
            ModelEvent::UsageUpdated {
                usage: ModelUsage {
                    input_tokens: Some(1),
                    output_tokens: Some(1),
                    ..ModelUsage::default()
                },
            },
        ))
        .unwrap();
    renderer
        .push(&ModelStreamEventV1::new(
            6,
            ModelEvent::FinishReason {
                reason: FinishReason::Stop,
            },
        ))
        .unwrap();
    assert!(
        renderer
            .push(&ModelStreamEventV1::new(
                7,
                ModelEvent::ResponseCompleted {
                    output: vec![ResponseBlock::Reasoning {
                        index: 0,
                        item_id: None,
                        text: "thought".into(),
                        status: ResponseItemStatus::Completed,
                    }],
                },
            ))
            .unwrap()
            .iter()
            .any(|frame| frame.data["type"] == "message_stop")
    );
}

#[test]
fn failed_responses_json_cannot_silently_discard_preceding_content() {
    let document = json!({
        "id":"failed-response", "model":"physical", "status":"failed",
        "error":{"code":"server_error", "message":"upstream failed"},
        "output":[
            {"type":"message", "id":"answer", "role":"assistant", "status":"completed",
                "content":[{"type":"output_text", "text":"visible answer", "annotations":[]}]},
            {"type":"function_call", "id":"tool", "call_id":"call-probe", "name":"probe",
                "arguments":"{\"value\":1}", "status":"completed"}
        ]
    });
    for ingress in [IngressProtocol::ChatCompletions, IngressProtocol::Messages] {
        let candidate = candidate(ingress);
        let decoded = decode(&candidate, &document, false);
        assert!(decoded.response.error.is_some());
        assert!(decoded.events.iter().any(|event| matches!(
            &event.event,
            ModelEvent::TextFinished { text, .. } if text == "visible answer"
        )));
        assert!(decoded.events.iter().any(|event| matches!(
            &event.event,
            ModelEvent::ToolCallFinished { name, .. } if name == "probe"
        )));
        assert!(matches!(
            &decoded.events.last().unwrap().event,
            ModelEvent::ResponseFailed { .. }
        ));
        let error = ClientResponseRenderer::render_nonstream(ingress, "route", &decoded.response)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("batch renderer cannot discard semantics emitted before an error")
        );
    }
}
