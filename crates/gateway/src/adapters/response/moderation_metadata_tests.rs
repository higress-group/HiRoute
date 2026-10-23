//! Known moderation notifications do not add model output or terminate a response.
use super::*;
use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};
use serde_json::json;

fn profile() -> CandidateProtocolProfile {
    CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    )
}

fn metadata() -> Value {
    json!({"type":"response.metadata","response_id":"response-1","sequence_number":7,
    "metadata":{"openai_chatgpt_moderation_metadata":{
        "prompt":{"opaque":"notification"},"generation":{},"tool_call":{},"tool_response":{}
    }}})
}

fn frames() -> Vec<Value> {
    vec![
        json!({"type":"response.created","response":{"id":"response-1","model":"physical"}}),
        json!({"type":"response.output_item.added","output_index":0,"item":{
            "type":"message","id":"message-1","role":"assistant","status":"in_progress","content":[]}}),
        json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"item_id":"message-1","delta":"answer"}),
        json!({"type":"response.output_text.done","output_index":0,"content_index":0,"item_id":"message-1","text":"answer"}),
        json!({"type":"response.completed","response":{"id":"response-1","model":"physical","status":"completed",
            "usage":{"input_tokens":3,"output_tokens":2}}}),
    ]
}

fn wire(frames: &[Value]) -> Vec<u8> {
    frames
        .iter()
        .flat_map(|value| {
            format!(
                "event: {}\ndata: {value}\n\n",
                value["type"].as_str().unwrap()
            )
            .into_bytes()
        })
        .collect()
}

fn decode(
    frames: &[Value],
    fragment: usize,
    projection: super::super::continuation::ToolIdProjection,
) -> Result<DecodedNativeResponse, ProtocolAdapterError> {
    let mut decoder =
        NativeResponseDecoder::new_for_observation(&profile(), 200, true, projection, None)?;
    let mut events = Vec::new();
    for chunk in wire(frames).chunks(fragment) {
        let mut status = decoder.feed(chunk, false)?;
        loop {
            events.extend(decoder.take_events());
            if status != ResponseDecodeStatus::NeedDrain {
                break;
            }
            status = decoder.resume()?;
        }
    }
    decoder.feed(&[], true)?;
    let mut result = decoder.finish()?;
    events.append(&mut result.events);
    result.events = events;
    // Framing work necessarily differs, but model semantics must not.
    result.sse_complexity = None;
    Ok(result)
}

#[test]
fn moderation_metadata_preserves_output_before_during_and_after_content() {
    let projection = test_tool_projection();
    let expected = decode(&frames(), 17, projection.clone()).unwrap();
    for position in [1, 3, 5] {
        let mut input = frames();
        input.insert(position, metadata());
        for fragment in [1, 17, usize::MAX] {
            assert_eq!(
                decode(&input, fragment, projection.clone()).unwrap(),
                expected
            );
        }
    }
}

#[test]
fn moderation_metadata_preserves_native_notification_bytes() {
    let bytes = wire(&[metadata()]);
    for fragment in [1, 17, usize::MAX] {
        let mut projector = NativeResponseProjector::new_for_observation(
            &profile(),
            true,
            "alias".into(),
            None,
            test_tool_projection(),
        )
        .unwrap();
        projector.feed(&wire(&frames()[..1]), false).unwrap();
        let mut units = Vec::new();
        for chunk in bytes.chunks(fragment) {
            units.extend(projector.feed(chunk, false).unwrap());
        }
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].bytes.as_slice(), bytes.as_slice());
        assert_eq!(units[0].terminal, None);
        assert!(!units[0].semantic);
    }
}

#[test]
fn moderation_metadata_preserves_native_terminal_at_eof() {
    for (event, status, expected) in [
        (
            "response.completed",
            "completed",
            NativeTerminalOutcome::Complete,
        ),
        (
            "response.incomplete",
            "incomplete",
            NativeTerminalOutcome::Incomplete,
        ),
        ("response.failed", "failed", NativeTerminalOutcome::Failed),
    ] {
        let terminal = json!({"type":event,"response":{"id":"response-1","model":"physical",
            "status":status,"output":[],"usage":{"input_tokens":3,"output_tokens":0}}});
        for invalid in [false, true] {
            let mut notification = metadata();
            if invalid {
                notification["safety_buffering"] = json!(true);
            }
            let notification_bytes = wire(&[notification.clone()]);
            let bytes = wire(&[frames()[0].clone(), terminal.clone(), notification]);
            for fragment in [1, 17, usize::MAX] {
                let mut projector = NativeResponseProjector::new_for_observation(
                    &profile(),
                    true,
                    "alias".into(),
                    None,
                    test_tool_projection(),
                )
                .unwrap();
                let mut units = Vec::new();
                for chunk in bytes.chunks(fragment) {
                    units.extend(projector.feed(chunk, false).unwrap());
                }
                units.extend(projector.feed(&[], true).unwrap());
                assert_eq!(
                    units.last().unwrap().terminal,
                    Some(if invalid {
                        NativeTerminalOutcome::Unknown
                    } else {
                        expected
                    })
                );
                assert!(units.iter().any(|unit| unit.bytes == notification_bytes));
                assert_eq!(
                    units.iter().filter(|unit| unit.terminal.is_some()).count(),
                    1
                );
            }
        }
    }
}

#[test]
fn moderation_metadata_cannot_hide_unknown_or_control_semantics() {
    let mut invalid = vec![
        json!({"type":"response.metadata","response_id":"response-1","sequence_number":7,"metadata":{"safety_buffering":{}}}),
        json!({"type":"response.future_event","response_id":"response-1","sequence_number":7,"metadata":{}}),
    ];
    for (field, value) in [
        ("response_id", json!("different-response")),
        ("response_id", json!(null)),
        ("sequence_number", json!(-1)),
        ("sequence_number", json!("7")),
        ("metadata", json!([])),
        ("safety_buffering", json!(true)),
        ("delta", json!("hidden output")),
    ] {
        let mut item = metadata();
        item[field] = value;
        invalid.push(item);
    }
    for (field, value) in [
        ("headers", json!({})),
        ("verification", json!({})),
        ("type", json!("safety_buffering")),
        ("openai_chatgpt_moderation_metadata", json!(false)),
    ] {
        let mut item = metadata();
        item["metadata"][field] = value;
        invalid.push(item);
    }
    for value in [json!(false), json!("unknown"), json!([])] {
        let mut item = metadata();
        item["metadata"]["openai_chatgpt_moderation_metadata"]["prompt"] = value;
        invalid.push(item);
    }
    let mut unknown_part = metadata();
    unknown_part["metadata"]["openai_chatgpt_moderation_metadata"]["future_control"] = json!({});
    invalid.push(unknown_part);
    let mut missing_sequence = metadata();
    missing_sequence
        .as_object_mut()
        .unwrap()
        .remove("sequence_number");
    invalid.push(missing_sequence);
    for item in invalid {
        let mut input = frames();
        input.insert(3, item);
        assert!(decode(&input, 17, test_tool_projection()).is_err());
    }
    let mut before_start = frames();
    before_start.insert(0, metadata());
    assert!(decode(&before_start, 17, test_tool_projection()).is_err());
    let mut decoder = NativeResponseDecoder::new(&profile(), 200, true).unwrap();
    decoder.feed(&wire(&frames()[..1]), false).unwrap();
    assert!(
        decoder
            .feed(
                format!("event: response.completed\ndata: {}\n\n", metadata()).as_bytes(),
                false
            )
            .is_err()
    );
}

#[test]
fn moderation_metadata_does_not_supply_a_missing_terminal() {
    let mut input = frames();
    input.pop();
    input.push(metadata());
    assert!(decode(&input, 17, test_tool_projection()).is_err());
}

#[test]
fn moderation_metadata_preserves_tool_completion_and_usage() {
    let mut input = vec![
        frames()[0].clone(),
        json!({"type":"response.output_item.added","output_index":0,"item":{
            "type":"function_call","id":"tool-1","call_id":"call-1","name":"probe","arguments":""}}),
        json!({"type":"response.function_call_arguments.delta","output_index":0,"item_id":"tool-1","delta":"{}"}),
        json!({"type":"response.function_call_arguments.done","output_index":0,"item_id":"tool-1","arguments":"{}"}),
        frames().pop().unwrap(),
    ];
    let projection = test_tool_projection();
    let expected = decode(&input, 17, projection.clone()).unwrap();
    input.insert(3, metadata());
    assert_eq!(decode(&input, 1, projection).unwrap(), expected);
}

#[test]
fn moderation_metadata_does_not_replace_refusal_or_error() {
    for terminal in [
        json!({"type":"response.incomplete","response":{"id":"response-1","model":"physical","status":"incomplete",
            "incomplete_details":{"reason":"content_filter"},"usage":{"input_tokens":3,"output_tokens":2}}}),
        json!({"type":"response.failed","response":{"error":{"code":"server_error","message":"failure"}}}),
    ] {
        let mut input = vec![frames()[0].clone(), terminal];
        let projection = test_tool_projection();
        let expected = decode(&input, 17, projection.clone()).unwrap();
        input.insert(1, metadata());
        assert_eq!(decode(&input, 17, projection).unwrap(), expected);
    }
}
