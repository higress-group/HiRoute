use super::*;
use hiroute_gateway_core::runtime::body::BudgetTree;
use serde_json::json;

fn projector(streaming: bool, bytes: usize) -> NativeResponseProjector {
    let profile =
        crate::server::core_runtime::profiles::CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "physical",
            crate::server::core_runtime::profiles::fixed_reasoning("fixed"),
        );
    let budget = BudgetTree::new(bytes, bytes)
        .unwrap()
        .stream(bytes)
        .unwrap();
    NativeResponseProjector::new_for_attempt(&profile, streaming, "alias".into(), None, budget)
        .unwrap()
}

#[test]
fn native_large_sse_event_preserves_bytes_across_transport_fragmentation() {
    let content = "中文-content-".repeat(32 * 1024);
    let event = json!({"type":"provider.extension","payload":content});
    let wire = format!("event: provider.extension\ndata: {event}\n\n").into_bytes();
    assert!(wire.len() > 256 * 1024);
    for chunk_size in [1021, wire.len()] {
        let mut projector = projector(true, 16 * 1024 * 1024);
        let mut actual = Vec::new();
        for chunk in wire.chunks(chunk_size) {
            for unit in projector.feed(chunk, false).unwrap() {
                actual.extend_from_slice(&unit.bytes);
            }
        }
        assert_eq!(actual, wire);
    }
}

#[test]
fn native_json_response_above_sixteen_mib_preserves_content() {
    let content = "x".repeat(17 * 1024 * 1024);
    let body = json!({"id":"response", "model":"physical", "status":"completed",
        "output":[{"type":"message", "id":"message", "status":"completed", "role":"assistant",
            "content":[{"type":"output_text","text":content}]}]});
    let wire = serde_json::to_vec(&body).unwrap();
    let mut projector = projector(false, 128 * 1024 * 1024);
    for chunk in wire.chunks(64 * 1024) {
        assert!(projector.feed(chunk, false).unwrap().is_empty());
    }
    let units = projector.feed(&[], true).unwrap();
    let mut expected = body;
    expected["model"] = json!("alias");
    assert_eq!(
        serde_json::from_slice::<Value>(&units[0].bytes).unwrap(),
        expected
    );
    assert_eq!(units[0].terminal, Some(NativeTerminalOutcome::Complete));
}

#[test]
fn json_buffer_still_refuses_unavailable_memory_before_allocation() {
    let mut projector = projector(false, 1024);
    assert!(projector.feed(&vec![b'x'; 2048], false).is_err());
}

#[test]
fn canonical_decode_and_incremental_render_preserve_large_content_and_ids() {
    use crate::server::core_runtime::profiles::{
        CandidateProtocolProfile, ClientProtocolProfile, fixed_reasoning,
    };
    let candidate = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        "physical",
        fixed_reasoning("fixed"),
    );
    let id = "r".repeat(1024 * 1024 + 1);
    let content = "answer".repeat(3 * 1024 * 1024);
    let wire = serde_json::to_vec(&json!({"id":id,"model":"physical",
        "choices":[{"index":0,"message":{"role":"assistant","content":content},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":4,"completion_tokens":2,"total_tokens":6}})).unwrap();
    let mut decoder = NativeResponseDecoder::new(&candidate, 200, false).unwrap();
    decoder.feed(&wire, true).unwrap();
    let decoded = decoder.finish().unwrap();
    let mut renderer = client_stream::IncrementalClientSseRenderer::new(
        ClientProtocolProfile::for_candidate(&candidate).unwrap(),
        "alias",
    )
    .unwrap();
    let mut rendered = Vec::new();
    for event in &decoded.events {
        rendered.extend(renderer.push(event).unwrap());
    }
    let final_response = &rendered.last().unwrap().data["response"];
    assert_eq!(final_response["id"], id);
    let delta = rendered
        .iter()
        .find(|event| event.event.as_deref() == Some("response.output_text.delta"))
        .unwrap();
    assert_eq!(delta.data["delta"], content);
}

#[test]
fn messages_terminal_classification_accepts_more_than_eight_content_blocks() {
    assert_messages_blocks(false);
}

#[test]
fn recovered_terminal_descriptor_does_not_restore_retired_quotas() {
    assert_messages_blocks(true);
}

fn assert_messages_blocks(recovered: bool) {
    use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};
    let mut candidate = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Messages,
        "physical",
        fixed_reasoning("fixed"),
    );
    if recovered {
        candidate.capability.response.stream_refusal = serde_json::from_value(json!({
            "kind":"terminal_classified", "max_buffered_bytes":1, "max_buffered_blocks":1
        }))
        .unwrap();
    }
    let mut decoder = NativeResponseDecoder::new(&candidate, 200, true).unwrap();
    let mut events = vec![
        json!({"type":"message_start","message":{"id":"m","type":"message",
        "role":"assistant","model":"physical","content":[],"stop_reason":null,"stop_sequence":null,
        "usage":{"input_tokens":1,"output_tokens":0}}}),
    ];
    for index in 0..9 {
        events.push(json!({"type":"content_block_start","index":index,"content_block":{"type":"text","text":format!("part-{index}")}}));
        events.push(json!({"type":"content_block_stop","index":index}));
    }
    events.push(json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":9}}));
    events.push(json!({"type":"message_stop"}));
    for event in events {
        let wire = format!(
            "event: {}\ndata: {event}\n\n",
            event["type"].as_str().unwrap()
        );
        let mut status = decoder.feed(wire.as_bytes(), false).unwrap();
        decoder.take_events();
        while status == ResponseDecodeStatus::NeedDrain {
            status = decoder.resume().unwrap();
            decoder.take_events();
        }
    }
    decoder.feed(&[], true).unwrap();
    let result = decoder.finish().unwrap();
    assert_eq!(result.response.blocks.len(), 9);
}

#[test]
fn native_messages_signatures_use_budget_without_byte_or_block_quotas() {
    use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};
    let candidate = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Messages,
        IngressProtocol::Messages,
        "physical",
        fixed_reasoning("fixed"),
    );
    let budget = BudgetTree::new(16 * 1024 * 1024, 16 * 1024 * 1024)
        .unwrap()
        .stream(16 * 1024 * 1024)
        .unwrap();
    let mut projector = NativeResponseProjector::new_for_attempt(
        &candidate,
        true,
        "alias".into(),
        None,
        budget.clone(),
    )
    .unwrap();
    let mut events = Vec::new();
    for index in 0..129 {
        events.push(json!({"type":"content_block_start","index":index,
            "content_block":{"type":"thinking","thinking":""}}));
    }
    let fragment = "s".repeat(160 * 1024);
    for _ in 0..2 {
        events.push(json!({"type":"content_block_delta","index":0,
            "delta":{"type":"signature_delta","signature":fragment}}));
    }
    for index in 0..129 {
        events.push(json!({"type":"content_block_stop","index":index}));
    }
    for event in events {
        let wire = format!(
            "event: {}\ndata: {event}\n\n",
            event["type"].as_str().unwrap()
        );
        let units = projector.feed(wire.as_bytes(), false).unwrap();
        assert_eq!(
            units
                .into_iter()
                .flat_map(|unit| unit.bytes)
                .collect::<Vec<_>>(),
            wire.as_bytes()
        );
    }
    assert!(budget.snapshot().unwrap().live > 0);
    drop(projector);
    assert_eq!(budget.snapshot().unwrap().live, 0);
}
