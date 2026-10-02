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
