//! Cache-write evidence must survive both forwarding and protocol conversion.
use super::client_stream::IncrementalClientSseRenderer;
use super::native_passthrough::{NativeResponseProjector, NativeTerminalOutcome};
use super::{ClientResponseRenderer, NativeResponseDecoder, RenderedClientResponse};
use crate::server::core_runtime::adapters::ToolIdProjection;
use crate::server::core_runtime::profiles::{
    CandidateProtocolProfile, ClientProtocolProfile, fixed_reasoning,
};
use crate::server::request_plan::IngressProtocol;
use serde_json::{Value, json};

fn document(protocol: IngressProtocol, write: Option<u64>) -> Value {
    let mut details = json!({"cached_tokens":3});
    if let Some(write) = write {
        details["cache_write_tokens"] = json!(write);
    }
    match protocol {
        IngressProtocol::Responses => json!({
            "id":"response-cache", "model":"physical", "status":"completed",
            "output":[{"id":"message-cache","type":"message","role":"assistant",
                "status":"completed","content":[{"type":"output_text","text":"answer"}]}],
            "usage":{"input_tokens":32,"output_tokens":9,"total_tokens":41,
                "input_tokens_details":details}
        }),
        IngressProtocol::ChatCompletions => json!({
            "id":"chat-cache","object":"chat.completion","created":1,"model":"physical",
            "choices":[{"index":0,"message":{"role":"assistant","content":"answer"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":32,"completion_tokens":9,"total_tokens":41,
                "prompt_tokens_details":details}
        }),
        _ => unreachable!(),
    }
}

fn assert_write(usage: &Value, protocol: IngressProtocol, write: Option<u64>) {
    let details = match protocol {
        IngressProtocol::Responses => &usage["input_tokens_details"],
        IngressProtocol::ChatCompletions => &usage["prompt_tokens_details"],
        _ => unreachable!(),
    };
    assert_eq!(details["cached_tokens"], 3);
    if write.is_none() {
        assert!(details.get("cache_write_tokens").is_none());
    }
    assert_eq!(details["cache_write_tokens"].as_u64(), write);
}

#[test]
fn cache_usage_native_forwarding_preserves_explicit_zero_positive_and_absence() {
    for protocol in [IngressProtocol::Responses, IngressProtocol::ChatCompletions] {
        for write in [None, Some(0), Some(7)] {
            for streaming in [false, true] {
                let profile = CandidateProtocolProfile::exact_portable_path(
                    protocol,
                    protocol,
                    "physical",
                    fixed_reasoning("fixed"),
                );
                let mut projector = NativeResponseProjector::new_for_observation(
                    &profile,
                    streaming,
                    "alias".into(),
                    None,
                    ToolIdProjection::new(protocol),
                )
                .unwrap();
                let body = document(protocol, write);
                let wire = if !streaming {
                    serde_json::to_vec(&body).unwrap()
                } else if protocol == IngressProtocol::Responses {
                    format!("event: response.created\ndata: {}\n\nevent: response.completed\ndata: {}\n\n",
                        json!({"type":"response.created","response":{"id":"response-cache","model":"physical","status":"in_progress","output":[]}}),
                        json!({"type":"response.completed","response":body})).into_bytes()
                } else {
                    format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                        json!({"id":"chat-cache","object":"chat.completion.chunk","model":"physical","choices":[{"index":0,"delta":{"role":"assistant","content":"answer"},"finish_reason":"stop"}]}),
                        json!({"id":"chat-cache","object":"chat.completion.chunk","model":"physical","choices":[],"usage":body["usage"]})).into_bytes()
                };
                let units = projector.feed(&wire, true).unwrap();
                assert_eq!(
                    units.last().unwrap().terminal,
                    Some(NativeTerminalOutcome::Complete)
                );
                assert_eq!(
                    projector.usage().cache_write_tokens,
                    write,
                    "{protocol:?}, streaming={streaming}"
                );
                assert_eq!(projector.usage().input_tokens, Some(32));
                assert_eq!(projector.usage().cache_read_tokens, Some(3));
            }
        }
    }
}

#[test]
fn cache_usage_conversion_preserves_buckets_in_all_client_renderers() {
    for (upstream, downstream) in [
        (IngressProtocol::Responses, IngressProtocol::ChatCompletions),
        (IngressProtocol::ChatCompletions, IngressProtocol::Responses),
    ] {
        for write in [None, Some(0), Some(7)] {
            let candidate = CandidateProtocolProfile::exact_portable_path(
                downstream,
                upstream,
                "physical",
                fixed_reasoning("fixed"),
            );
            let profile = ClientProtocolProfile::for_candidate(&candidate).unwrap();
            let mut decoder = NativeResponseDecoder::new(&candidate, 200, false).unwrap();
            decoder
                .feed(
                    &serde_json::to_vec(&document(upstream, write)).unwrap(),
                    true,
                )
                .unwrap();
            let mut live = IncrementalClientSseRenderer::new(profile.clone(), "alias").unwrap();
            let mut live_events = Vec::new();
            for event in decoder.take_events() {
                live_events.extend(live.push(&event).unwrap());
            }
            let decoded = decoder.finish().unwrap();
            assert_eq!(decoded.response.usage.cache_write_tokens, write);
            let RenderedClientResponse::Json { body, .. } =
                ClientResponseRenderer::render_nonstream_with_profile(
                    &profile,
                    "alias",
                    &decoded.response,
                )
                .unwrap()
            else {
                panic!("JSON response")
            };
            assert_write(&body["usage"], downstream, write);
            let RenderedClientResponse::Sse { events, .. } =
                ClientResponseRenderer::render_stream_with_profile(
                    &profile,
                    "alias",
                    &decoded.response,
                )
                .unwrap()
            else {
                panic!("SSE response")
            };
            for events in [&live_events, &events] {
                let usage = events
                    .iter()
                    .rev()
                    .find_map(|event| {
                        event
                            .data
                            .get("usage")
                            .or_else(|| event.data.get("response")?.get("usage"))
                            .filter(|usage| usage.is_object())
                    })
                    .unwrap();
                assert_write(usage, downstream, write);
            }
        }
    }
}
