use super::*;
use crate::server::core_runtime::model_ir::ResponseBlock;
use crate::server::core_runtime::profiles::{
    CandidateProtocolProfile, ClientProtocolProfile, fixed_reasoning,
};
use serde_json::json;

#[test]
fn chat_filtered_json_preserves_accepted_text_in_all_responses_renderers() {
    assert_filtered_text(false);
}

#[test]
fn chat_filtered_stream_preserves_accepted_text_in_all_responses_renderers() {
    assert_filtered_text(true);
}

fn assert_filtered_text(streaming: bool) {
    let candidate = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        "physical",
        fixed_reasoning("fixed"),
    );
    let profile = ClientProtocolProfile::for_candidate(&candidate).unwrap();
    let usage = json!({"prompt_tokens":4,"completion_tokens":2,"total_tokens":6});
    let wire = if streaming {
        let mut wire = Vec::new();
        for (delta, reason, usage) in [
            (
                json!({"role":"assistant","content":"accepted "}),
                Value::Null,
                Value::Null,
            ),
            (json!({"content":"prefix"}), Value::Null, Value::Null),
            (json!({}), json!("content_filter"), usage.clone()),
        ] {
            wire.extend(
                RenderedSseEvent {
                    event: None,
                    data: json!({"id":"filtered-response","model":"physical", "choices":[{
                    "index":0,"delta":delta,"finish_reason":reason
                }],"usage":usage}),
                }
                .wire_bytes()
                .unwrap(),
            );
        }
        wire.extend_from_slice(b"data: [DONE]\n\n");
        wire
    } else {
        serde_json::to_vec(
            &json!({"id":"filtered-response","model":"physical", "choices":[{
            "index":0,"message":{"role":"assistant","content":"accepted prefix"},
            "finish_reason":"content_filter"
        }],"usage":usage}),
        )
        .unwrap()
    };
    for quantum in [1, 4096] {
        let mut decoder = NativeResponseDecoder::new(&candidate, 200, streaming).unwrap();
        let mut renderer =
            client_stream::IncrementalClientSseRenderer::new(profile.clone(), "route").unwrap();
        let mut incremental = Vec::new();
        for chunk in wire.chunks(quantum) {
            decoder.feed(chunk, false).unwrap();
            for event in decoder.take_events() {
                incremental.extend(renderer.push(&event).unwrap());
            }
        }
        decoder.feed(&[], true).unwrap();
        for event in decoder.take_events() {
            incremental.extend(renderer.push(&event).unwrap());
        }
        let response = decoder.finish().unwrap().response;
        assert!(
            matches!(
                response.blocks.as_slice(),
                [ResponseBlock::Text { text, status: ResponseItemStatus::Incomplete, .. }]
                    if text == "accepted prefix"
            ),
            "a filtered terminal cannot replace already accepted text with a refusal"
        );
        assert_eq!(response.finish_reason, Some(FinishReason::Refusal));
        let RenderedClientResponse::Json { body, .. } =
            ClientResponseRenderer::render_nonstream_with_profile(&profile, "route", &response)
                .unwrap()
        else {
            panic!("JSON response")
        };
        let RenderedClientResponse::Sse {
            events: buffered, ..
        } = ClientResponseRenderer::render_stream_with_profile(&profile, "route", &response)
            .unwrap()
        else {
            panic!("SSE response")
        };
        assert_eq!(body["output"][0]["content"][0]["type"], "output_text");
        for events in [&incremental, &buffered] {
            let deltas: String = events
                .iter()
                .filter(|event| event.event.as_deref() == Some("response.output_text.delta"))
                .map(|event| event.data["delta"].as_str().unwrap())
                .collect();
            assert_eq!(deltas, "accepted prefix");
            assert!(!events.iter().any(|event| {
                event
                    .event
                    .as_deref()
                    .is_some_and(|name| name.starts_with("response.refusal."))
            }));
            let done = events
                .iter()
                .find(|event| event.event.as_deref() == Some("response.output_item.done"))
                .unwrap();
            assert_eq!(done.data["item"], body["output"][0]);
            let terminal = events
                .iter()
                .find(|event| event.event.as_deref() == Some("response.incomplete"))
                .unwrap();
            for field in ["output", "status", "incomplete_details", "usage"] {
                assert_eq!(terminal.data["response"][field], body[field]);
            }
            assert_eq!(
                terminal.data["response"]["incomplete_details"]["reason"],
                "content_filter"
            );
        }
    }
}
