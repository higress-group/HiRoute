use super::*;
use crate::server::core_runtime::profiles::{
    CandidateProtocolProfile, ClientProtocolProfile, fixed_reasoning,
};
use serde_json::{Value, json};

fn candidate() -> CandidateProtocolProfile {
    CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Messages,
        IngressProtocol::ChatCompletions,
        "physical",
        fixed_reasoning("default"),
    )
}

fn response() -> Value {
    json!({"id":"r","model":"physical","service_tier":"default",
        "choices":[{"index":0,"message":{"role":"assistant","content":"answer",
            "tool_calls":[{"id":"call_1","type":"function","function":{"name":"probe","arguments":"{\"value\":1}"}}]},"finish_reason":"tool_calls"}],
        "usage":{"prompt_tokens":7,"completion_tokens":19,"total_tokens":26}})
}

#[test]
fn chat_service_tier_json_keeps_answer_tools_and_usage() {
    for tier in [
        Value::Null,
        json!("default"),
        json!("priority"),
        json!("provider-future-tier"),
    ] {
        let mut document = response();
        document["service_tier"] = tier;
        let profile = candidate();
        let mut decoder = NativeResponseDecoder::new(&profile, 200, false).unwrap();
        decoder
            .feed(&serde_json::to_vec(&document).unwrap(), true)
            .unwrap();
        let decoded = decoder.finish().unwrap();
        let RenderedClientResponse::Json { body, .. } =
            ClientResponseRenderer::render_nonstream_with_profile(
                &ClientProtocolProfile::for_candidate(&profile).unwrap(),
                "alias",
                &decoded.response,
            )
            .unwrap()
        else {
            panic!("expected JSON")
        };
        assert_eq!(body["content"][0]["text"], "answer");
        let tool = body["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["type"] == "tool_use")
            .unwrap();
        assert_eq!(tool["name"], "probe");
        assert_eq!(tool["input"], json!({"value":1}));
        assert_eq!(body["stop_reason"], "tool_use");
        assert_eq!(body["usage"], json!({"input_tokens":7,"output_tokens":19}));
        assert!(body.get("service_tier").is_none());
    }
}

fn stream(tier: Value) -> Vec<u8> {
    let chunks = [
        json!({"id":"r","model":"physical","choices":[{"index":0,"delta":{"role":"assistant","content":"answer"},"finish_reason":null}]}),
        json!({"id":"r","model":"physical","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"probe","arguments":"{\"value\":"}}]},"finish_reason":null}]}),
        json!({"id":"r","model":"physical","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1}"}}]},"finish_reason":"tool_calls"}]}),
        // The observed failure is metadata arriving after semantic output is committed.
        json!({"id":"r","model":"physical","choices":[],"service_tier":tier,"usage":{"prompt_tokens":7,"completion_tokens":19,"total_tokens":26}}),
    ];
    let mut wire: String = chunks.iter().map(|v| format!("data: {v}\n\n")).collect();
    wire.push_str("data: [DONE]\n\n");
    wire.into_bytes()
}

#[test]
fn chat_service_tier_stream_tail_completes_tools_and_usage() {
    for size in [1, 17, usize::MAX] {
        let profile = candidate();
        let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
        let mut renderer = IncrementalClientSseRenderer::new(
            ClientProtocolProfile::for_candidate(&profile).unwrap(),
            "alias",
        )
        .unwrap();
        let mut output = Vec::new();
        for chunk in stream(json!("default")).chunks(size) {
            let mut status = decoder.feed(chunk, false).unwrap();
            loop {
                for event in decoder.take_events() {
                    output.extend(renderer.push(&event).unwrap());
                }
                if status != ResponseDecodeStatus::NeedDrain {
                    break;
                }
                status = decoder.resume().unwrap();
            }
        }
        assert_eq!(
            decoder.feed(&[], true).unwrap(),
            ResponseDecodeStatus::Terminal
        );
        for event in decoder.take_events() {
            output.extend(renderer.push(&event).unwrap());
        }
        decoder.finish().unwrap();
        assert_eq!(
            output
                .iter()
                .filter(|v| v.data["type"] == "message_stop")
                .count(),
            1
        );
        let text: String = output
            .iter()
            .filter_map(|v| v.data["delta"]["text"].as_str())
            .collect();
        assert_eq!(text, "answer");
        assert!(
            output
                .iter()
                .any(|v| v.data["content_block"]["name"] == "probe")
        );
        let arguments: String = output
            .iter()
            .filter_map(|v| v.data["delta"]["partial_json"].as_str())
            .collect();
        assert_eq!(
            serde_json::from_str::<Value>(&arguments).unwrap(),
            json!({"value":1})
        );
        assert_eq!(renderer.usage().input_tokens, Some(7));
        assert_eq!(renderer.usage().output_tokens, Some(19));
    }
}

#[test]
fn chat_service_tier_invalid_types_still_fail() {
    for tier in [json!(true), json!(7), json!({}), json!([])] {
        let mut document = response();
        document["service_tier"] = tier.clone();
        let profile = candidate();
        let mut decoder = NativeResponseDecoder::new(&profile, 200, false).unwrap();
        assert!(
            decoder
                .feed(&serde_json::to_vec(&document).unwrap(), true)
                .is_err()
        );
        let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
        let mut result = decoder.feed(&stream(tier), true);
        while matches!(result, Ok(ResponseDecodeStatus::NeedDrain)) {
            decoder.take_events();
            result = decoder.resume();
        }
        assert!(result.is_err());
    }
}
