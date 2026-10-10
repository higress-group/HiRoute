use super::native_passthrough::{NativeResponseProjector, NativeTerminalOutcome};
use crate::server::{
    core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning},
    request_plan::IngressProtocol,
};
use hiroute_gateway_core::runtime::body::BudgetTree;
use serde_json::{Value, json};

fn wire(value: Value) -> Vec<u8> {
    format!("data: {value}\n\n").into_bytes()
}

#[test]
fn long_plain_reasoning_commits_on_first_delta_and_keeps_terminal_usage() {
    let budget = BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024)
        .unwrap()
        .stream(8 * 1024 * 1024)
        .unwrap();
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let mut projector =
        NativeResponseProjector::new_for_attempt(&profile, true, "alias".into(), None, budget)
            .unwrap();
    let reasoning = |text: &str| {
        json!({"type":"reasoning","id":"r","summary":[],
        "content":[{"type":"reasoning_text","text":text}]})
    };
    projector.feed(&wire(json!({"type":"response.created","response":{"id":"resp","model":"physical","status":"in_progress"}})), false).unwrap();
    projector
        .feed(
            &wire(
                json!({"type":"response.output_item.added","output_index":0,"item":reasoning("")}),
            ),
            false,
        )
        .unwrap();
    let chunk = "think ".repeat(512);
    let mut text = String::new();
    for _ in 0..800 {
        text.push_str(&chunk);
        let event = wire(
            json!({"type":"response.reasoning_text.delta","item_id":"r","output_index":0,"content_index":0,"delta":chunk}),
        );
        let units = projector.feed(&event, false).unwrap();
        assert!(
            units.iter().any(|unit| unit.semantic),
            "plain reasoning is output, not an indefinitely buffered opaque prefix"
        );
        assert_eq!(
            units
                .iter()
                .flat_map(|u| u.bytes.iter().copied())
                .collect::<Vec<_>>(),
            event
        );
    }
    assert!(text.len() > 2 * 1024 * 1024);
    let terminal = wire(json!({"type":"response.completed","response":{
        "id":"resp","model":"physical","status":"completed","output":[reasoning(&text)],
        "usage":{"input_tokens":100,"output_tokens":800,"total_tokens":900}}}));
    for chunk in terminal.chunks(16 * 1024) {
        projector.feed(chunk, false).unwrap();
    }
    let last = projector.feed(&[], true).unwrap();
    assert_eq!(
        last.last().unwrap().terminal,
        Some(NativeTerminalOutcome::Complete)
    );
    assert_eq!(projector.usage().output_tokens, Some(800));
}

#[test]
fn failed_response_preserves_reported_usage() {
    let budget = BudgetTree::new(1024 * 1024, 1024 * 1024)
        .unwrap()
        .stream(1024 * 1024)
        .unwrap();
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let mut projector =
        NativeResponseProjector::new_for_attempt(&profile, true, "alias".into(), None, budget)
            .unwrap();
    let units = projector
        .feed(
            &wire(json!({"type":"response.failed","response":{
        "id":"resp","status":"failed","model":"physical","output":[],
        "error":{"code":"server_error","message":"private body"},
        "usage":{"input_tokens":100,"output_tokens":2}}})),
            true,
        )
        .unwrap();
    assert!(units.iter().any(|unit| unit.failure.is_some()));
    assert_eq!(projector.usage().input_tokens, Some(100));
    assert_eq!(projector.usage().output_tokens, Some(2));
}

fn messages_projector(streaming: bool, omitted: bool) -> NativeResponseProjector {
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Messages,
        IngressProtocol::Messages,
        "physical",
        fixed_reasoning("default"),
    );
    NativeResponseProjector::new_for_observation(
        &profile,
        streaming,
        "alias".into(),
        None,
        crate::server::core_runtime::adapters::ToolIdProjection::new(IngressProtocol::Messages),
    )
    .unwrap()
    .with_omitted_thinking(omitted)
}

#[test]
fn thinking_display_native_json_preserves_state_tools_and_usage() {
    let original = json!({"id":"msg","type":"message","role":"assistant","model":"physical",
        "content":[{"type":"thinking","thinking":"private thought","signature":"opaque-signature"},
            {"type":"redacted_thinking","data":"opaque-state"},
            {"type":"text","text":"answer"},
            {"type":"tool_use","id":"tool_1","name":"probe","input":{"value":1}}],
        "stop_reason":"tool_use","usage":{"input_tokens":7,"output_tokens":19}});
    for omitted in [false, true] {
        let mut projector = messages_projector(false, omitted);
        let units = projector
            .feed(&serde_json::to_vec(&original).unwrap(), true)
            .unwrap();
        let body: Value = serde_json::from_slice(&units[0].bytes).unwrap();
        assert_eq!(
            body["content"][0]["thinking"],
            if omitted { "" } else { "private thought" }
        );
        assert_eq!(body["content"][0]["signature"], "opaque-signature");
        assert_eq!(body["content"][1], original["content"][1]);
        assert_eq!(body["content"][2], original["content"][2]);
        assert_eq!(body["content"][3]["name"], "probe");
        assert_eq!(body["content"][3]["input"], json!({"value":1}));
        assert!(!body["content"][3]["id"].as_str().unwrap().is_empty());
        assert_eq!(body["usage"], original["usage"]);
        assert_eq!(projector.usage().output_tokens, Some(19));
        assert_eq!(units[0].terminal, Some(NativeTerminalOutcome::Complete));
    }
}

#[test]
fn thinking_display_fragmented_sse_retains_signature_tool_lifecycle_and_usage() {
    let events = vec![
        json!({"type":"message_start","message":{"id":"msg","type":"message","role":"assistant","model":"physical","content":[],"usage":{"input_tokens":7,"output_tokens":0}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"private start"}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"private delta"}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"opaque-signature"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"content_block_start","index":1,"content_block":{"type":"redacted_thinking","data":"opaque-state"}}),
        json!({"type":"content_block_stop","index":1}),
        json!({"type":"content_block_start","index":2,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":2,"delta":{"type":"text_delta","text":"answer"}}),
        json!({"type":"content_block_stop","index":2}),
        json!({"type":"content_block_start","index":3,"content_block":{"type":"tool_use","id":"tool_1","name":"probe","input":{}}}),
        json!({"type":"content_block_delta","index":3,"delta":{"type":"input_json_delta","partial_json":"{\"value\":1}"}}),
        json!({"type":"content_block_stop","index":3}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":19}}),
        json!({"type":"message_stop"}),
    ];
    let input: Vec<u8> = events.iter().flat_map(|v| wire(v.clone())).collect();
    for omitted in [false, true] {
        let mut projector = messages_projector(true, omitted);
        let mut units = Vec::new();
        for chunk in input.chunks(7) {
            units.extend(projector.feed(chunk, false).unwrap());
        }
        units.extend(projector.feed(&[], true).unwrap());
        let output: Vec<u8> = units.iter().flat_map(|u| u.bytes.clone()).collect();
        let text = String::from_utf8(output).unwrap();
        assert_eq!(text.contains("private start"), !omitted);
        assert_eq!(text.contains("private delta"), !omitted);
        assert!(text.contains("opaque-signature") && text.contains("opaque-state"));
        assert!(text.contains("answer") && text.contains("probe"));
        let projected: Vec<Value> = text
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert_eq!(projected.len(), events.len());
        assert_eq!(projected[3], events[3]);
        assert_eq!(projected[11], events[11]);
        assert_eq!(projected[13], events[13]);
        assert_eq!(projector.usage().input_tokens, Some(7));
        assert_eq!(projector.usage().output_tokens, Some(19));
        assert_eq!(
            units.last().unwrap().terminal,
            Some(NativeTerminalOutcome::Complete)
        );
    }
}

#[test]
fn thinking_display_cross_protocol_messages_omit_text_but_keep_tools_and_usage() {
    use super::{
        ClientResponseRenderer, IncrementalClientSseRenderer, NativeResponseDecoder,
        RenderedClientResponse,
    };
    use crate::server::core_runtime::profiles::ClientProtocolProfile;
    for (upstream, document) in [
        (
            IngressProtocol::ChatCompletions,
            json!({"id":"r","model":"physical",
            "choices":[{"index":0,"message":{"role":"assistant","content":"answer","reasoning_content":"private thought",
                "tool_calls":[{"id":"tool_1","type":"function","function":{"name":"probe","arguments":"{\"value\":1}"}}]},"finish_reason":"tool_calls"}],
            "usage":{"prompt_tokens":7,"completion_tokens":19,"total_tokens":26,"completion_tokens_details":{"reasoning_tokens":10}}}),
        ),
        (
            IngressProtocol::Responses,
            json!({"id":"r","model":"physical","status":"completed",
            "output":[{"type":"reasoning","id":"reason","summary":[{"type":"summary_text","text":"private thought"}],"encrypted_content":"opaque-state"},
                {"type":"message","id":"msg","role":"assistant","status":"completed","content":[{"type":"output_text","text":"answer","annotations":[]}]},
                {"type":"function_call","id":"fc","call_id":"tool_1","name":"probe","arguments":"{\"value\":1}","status":"completed"}],
            "usage":{"input_tokens":7,"output_tokens":19,"total_tokens":26,"output_tokens_details":{"reasoning_tokens":10}}}),
        ),
    ] {
        let candidate = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Messages,
            upstream,
            "physical",
            fixed_reasoning("default"),
        );
        let profile = ClientProtocolProfile::for_candidate(&candidate).unwrap();
        let mut decoder = NativeResponseDecoder::new(&candidate, 200, false).unwrap();
        decoder
            .feed(&serde_json::to_vec(&document).unwrap(), true)
            .unwrap();
        let decoded = decoder.finish().unwrap();
        let RenderedClientResponse::Json { body, .. } =
            ClientResponseRenderer::render_nonstream_with_profile(
                &profile,
                "alias",
                &decoded.response,
            )
            .unwrap()
        else {
            panic!("JSON expected")
        };
        assert!(!body.to_string().contains("private thought"));
        assert!(body.to_string().contains("answer") && body.to_string().contains("probe"));
        assert!(!body.to_string().contains("signature"));
        assert_eq!(body["usage"]["output_tokens"], 19);
        assert_eq!(body["usage"]["input_tokens"], 7);
        let mut renderer = IncrementalClientSseRenderer::new(profile, "alias").unwrap();
        let mut output = Vec::new();
        for event in &decoded.events {
            output.extend(renderer.push(event).unwrap());
        }
        let text: String = output.iter().map(|event| event.data.to_string()).collect();
        assert!(!text.contains("private thought") && !text.contains("signature"));
        assert!(text.contains("answer") && text.contains("probe") && text.contains("message_stop"));
        assert_eq!(renderer.usage().output_tokens, Some(19));
        assert_eq!(renderer.usage().reasoning_tokens, Some(10));
    }
}
