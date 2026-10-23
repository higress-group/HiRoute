//! Transport comments cannot change an independently established model outcome.
use super::native_passthrough::{NativeResponseProjector, NativeTerminalOutcome};
use crate::server::core_runtime::adapters::ToolIdProjection;
use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};
use crate::server::request_plan::IngressProtocol;
use serde_json::{Value, json};

fn projector(protocol: IngressProtocol) -> NativeResponseProjector {
    NativeResponseProjector::new_for_observation(
        &CandidateProtocolProfile::exact_portable_path(
            protocol,
            protocol,
            "physical",
            fixed_reasoning("fixed"),
        ),
        true,
        "alias".into(),
        None,
        ToolIdProjection::new(protocol),
    )
    .unwrap()
}

fn frame(value: Value) -> Vec<u8> {
    format!("data: {value}\n\n").into_bytes()
}

fn responses_terminal(event: &str, status: &str) -> Vec<u8> {
    frame(
        json!({"type":event,"response":{"id":"resp","model":"physical",
        "status":status,"output":[],"usage":{"input_tokens":1,"output_tokens":0}}}),
    )
}

fn complete_wire(protocol: IngressProtocol) -> Vec<u8> {
    match protocol {
        IngressProtocol::Responses => responses_terminal("response.completed", "completed"),
        IngressProtocol::ChatCompletions => [
            frame(json!({"id":"resp","model":"physical","choices":[{"index":0,
                "delta":{"role":"assistant","content":"OK"},"finish_reason":"stop"}]})),
            b"data: [DONE]\n\n".to_vec(),
        ]
        .concat(),
        IngressProtocol::Messages => [
            frame(
                json!({"type":"message_start","message":{"id":"resp","model":"physical",
                "role":"assistant","content":[],"usage":{"input_tokens":1,"output_tokens":0}}}),
            ),
            frame(
                json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},
                "usage":{"output_tokens":0}}),
            ),
            frame(json!({"type":"message_stop"})),
        ]
        .concat(),
    }
}

#[test]
fn native_transport_comments_preserve_terminal_across_protocols_and_chunks() {
    for protocol in [
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        IngressProtocol::Messages,
    ] {
        for comment in [
            b": keepalive\n\n".as_slice(),
            b": first\r\n: second\r\n\r\n",
        ] {
            let wire = [complete_wire(protocol), comment.to_vec()].concat();
            for fragment in [1, 7, wire.len()] {
                let mut decoder = projector(protocol);
                let mut units = Vec::new();
                for chunk in wire.chunks(fragment) {
                    units.extend(decoder.feed(chunk, false).unwrap());
                }
                units.extend(decoder.feed(&[], true).unwrap());
                assert_eq!(
                    units
                        .iter()
                        .filter_map(|unit| unit.terminal)
                        .collect::<Vec<_>>(),
                    vec![NativeTerminalOutcome::Complete],
                    "{protocol:?}, chunk {fragment}"
                );
                let preserved = units.iter().find(|unit| unit.bytes == comment).unwrap();
                assert!(!preserved.semantic);
                assert_eq!(preserved.source_bytes, comment.len());
            }
        }
    }
}

#[test]
fn responses_transport_comments_preserve_failed_incomplete_and_done_tail() {
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
        let comment = b": keepalive\n\n";
        let wire = [
            responses_terminal(event, status),
            comment.to_vec(),
            b"data: [DONE]\n\n".to_vec(),
            comment.to_vec(),
        ]
        .concat();
        for fragment in [1, 7, wire.len()] {
            let mut decoder = projector(IngressProtocol::Responses);
            let mut units = Vec::new();
            for chunk in wire.chunks(fragment) {
                units.extend(decoder.feed(chunk, false).unwrap());
            }
            assert!(units.iter().all(|unit| unit.terminal.is_none()));
            units.extend(decoder.feed(&[], true).unwrap());
            assert_eq!(
                units
                    .iter()
                    .filter_map(|unit| unit.terminal)
                    .collect::<Vec<_>>(),
                vec![expected]
            );
            assert_eq!(units.iter().filter(|unit| unit.bytes == comment).count(), 2);
        }
    }
}

#[test]
fn native_transport_partial_comment_delays_without_reclassifying_terminal() {
    let comment = b": keepalive\n\n";
    for protocol in [
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        IngressProtocol::Messages,
    ] {
        let mut decoder = projector(protocol);
        let prefix = [complete_wire(protocol), comment[..5].to_vec()].concat();
        let mut units = decoder.feed(&prefix, false).unwrap();
        assert!(units.iter().all(|unit| unit.terminal.is_none()));
        units.extend(decoder.feed(&comment[5..], false).unwrap());
        units.extend(decoder.feed(&[], true).unwrap());
        assert_eq!(
            units
                .iter()
                .filter_map(|unit| unit.terminal)
                .collect::<Vec<_>>(),
            vec![NativeTerminalOutcome::Complete],
            "{protocol:?}"
        );
    }
}

#[test]
fn native_transport_eof_comment_preserves_terminal_for_all_protocols() {
    for protocol in [
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        IngressProtocol::Messages,
    ] {
        let terminal = complete_wire(protocol);
        for comment in [
            b": keepalive".as_slice(),
            b": keepalive\n",
            b": keepalive\r",
            b": first\r\n: second\r\n",
        ] {
            let wire = [terminal.as_slice(), comment].concat();
            // Also force the terminal and the first comment byte into one feed.
            // That path must delay Chat/Messages terminal delivery until EOF.
            for first_end in [0, terminal.len() + 1] {
                for fragment in [1, 7, wire.len()] {
                    let mut decoder = projector(protocol);
                    let mut units = Vec::new();
                    if first_end != 0 {
                        units.extend(decoder.feed(&wire[..first_end], false).unwrap());
                        assert!(units.iter().all(|unit| unit.terminal.is_none()));
                    }
                    for chunk in wire[first_end..].chunks(fragment) {
                        units.extend(decoder.feed(chunk, false).unwrap());
                    }
                    if first_end != 0 {
                        assert!(units.iter().all(|unit| unit.terminal.is_none()));
                    }
                    units.extend(decoder.feed(&[], true).unwrap());
                    assert_eq!(
                        units
                            .iter()
                            .filter_map(|unit| unit.terminal)
                            .collect::<Vec<_>>(),
                        vec![NativeTerminalOutcome::Complete],
                        "{protocol:?}, comment {comment:?}, first {first_end}, chunk {fragment}"
                    );
                    assert_eq!(
                        units.iter().map(|unit| unit.source_bytes).sum::<usize>(),
                        wire.len()
                    );
                    let tail = units.iter().find(|unit| unit.bytes == comment).unwrap();
                    assert!(!tail.semantic);
                    assert_eq!(tail.source_bytes, comment.len());
                    assert!(
                        units
                            .iter()
                            .flat_map(|unit| unit.bytes.iter().copied())
                            .collect::<Vec<_>>()
                            .ends_with(comment)
                    );
                }
            }
        }
    }
}

#[test]
fn responses_eof_comment_keeps_failed_incomplete_and_done_evidence() {
    for (event, status, expected) in [
        ("response.failed", "failed", NativeTerminalOutcome::Failed),
        (
            "response.incomplete",
            "incomplete",
            NativeTerminalOutcome::Incomplete,
        ),
    ] {
        for comment in [
            b": keepalive".as_slice(),
            b": keepalive\n",
            b": keepalive\r",
        ] {
            for with_done in [false, true] {
                let mut wire = responses_terminal(event, status);
                if with_done {
                    wire.extend_from_slice(b"data: [DONE]\n\n");
                }
                wire.extend_from_slice(comment);
                for fragment in [1, wire.len()] {
                    let mut decoder = projector(IngressProtocol::Responses);
                    let mut units = Vec::new();
                    for chunk in wire.chunks(fragment) {
                        units.extend(decoder.feed(chunk, false).unwrap());
                    }
                    assert!(units.iter().all(|unit| unit.terminal.is_none()));
                    units.extend(decoder.feed(&[], true).unwrap());
                    assert_eq!(
                        units
                            .iter()
                            .filter_map(|unit| unit.terminal)
                            .collect::<Vec<_>>(),
                        vec![expected],
                        "{event}, comment {comment:?}, done {with_done}, chunk {fragment}"
                    );
                    let tail = units.iter().find(|unit| unit.bytes == comment).unwrap();
                    assert!(!tail.semantic);
                    assert_eq!(tail.source_bytes, comment.len());
                }
            }
        }
    }
}

#[test]
fn native_transport_eof_comments_do_not_create_model_terminal() {
    for protocol in [
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        IngressProtocol::Messages,
    ] {
        for wire in [
            b": keepalive".as_slice(),
            b": keepalive\n",
            b"\r\n: keepalive\r",
        ] {
            for fragment in [1, wire.len()] {
                let mut decoder = projector(protocol);
                for chunk in wire.chunks(fragment) {
                    let units = decoder.feed(chunk, false).unwrap();
                    assert!(units.iter().all(|unit| unit.terminal.is_none()));
                }
                assert_eq!(
                    decoder.feed(&[], true).unwrap_err(),
                    super::ProtocolAdapterError::ModelIr(super::ModelIrError::MissingTerminalEvent),
                    "{protocol:?}, wire {wire:?}, chunk {fragment}"
                );
            }
        }
    }
}

#[test]
fn native_transport_eof_still_rejects_unfinished_control_and_json() {
    for tail in [
        b"data: {\"type\":\"response.completed\"".as_slice(),
        b"data:\n",
        b"event: response.future\n",
        b"id: next\r",
        b"retry: 1000\r\n",
        b"vendor-control: unknown",
        b": comment\ndata: {",
    ] {
        let wire = [complete_wire(IngressProtocol::Responses), tail.to_vec()].concat();
        for fragment in [1, wire.len()] {
            let mut decoder = projector(IngressProtocol::Responses);
            for chunk in wire.chunks(fragment) {
                let units = decoder.feed(chunk, false).unwrap();
                assert!(units.iter().all(|unit| unit.terminal.is_none()));
            }
            assert!(
                decoder.feed(&[], true).is_err(),
                "tail {tail:?}, chunk {fragment}"
            );
        }
    }
}

#[test]
fn native_transport_empty_control_description_still_withholds_completion() {
    for control in [
        b"event: response.future\n\n".as_slice(),
        b"data:\n\n",
        b": comment\nevent: response.future\n\n",
    ] {
        let wire = [complete_wire(IngressProtocol::Responses), control.to_vec()].concat();
        for fragment in [1, wire.len()] {
            let mut decoder = projector(IngressProtocol::Responses);
            let mut units = Vec::new();
            for chunk in wire.chunks(fragment) {
                units.extend(decoder.feed(chunk, false).unwrap());
            }
            units.extend(decoder.feed(&[], true).unwrap());
            assert_eq!(
                units.last().unwrap().terminal,
                Some(NativeTerminalOutcome::Unknown)
            );
        }
    }
}

#[test]
fn native_item_object_order_is_not_completion_evidence_but_values_are() {
    // Parse distinct native spellings, so preserve_order feature unification is
    // exercised rather than accidentally normalized by a shared fixture clone.
    let original: Value = serde_json::from_str(
        r#"{
        "type":"message","id":"item","status":"completed","role":"assistant",
        "content":[{"type":"output_text","text":"OK","annotations":[]}],
        "extension":{"first":1,"second":["a","b"]}}
    "#,
    )
    .unwrap();
    let reordered: Value = serde_json::from_str(
        r#"{
        "extension":{"second":["a","b"],"first":1},
        "content":[{"annotations":[],"text":"OK","type":"output_text"}],
        "role":"assistant","status":"completed","id":"item","type":"message"}
    "#,
    )
    .unwrap();
    assert_eq!(original, reordered);
    for mutation in 0..4 {
        let mut final_item = reordered.clone();
        match mutation {
            0 => {}
            1 => final_item["extension"]["first"] = json!(2),
            2 => final_item["extension"]["second"] = json!(["b", "a"]),
            _ => final_item["content"][0]["text"] = json!("different"),
        }
        let wire = [
            frame(json!({"type":"response.output_item.done","output_index":0,"item":original})),
            frame(json!({"type":"response.completed","response":{
                "id":"resp","model":"physical","status":"completed","output":[final_item]}})),
        ]
        .concat();
        for fragment in [1, wire.len()] {
            let mut decoder = projector(IngressProtocol::Responses);
            let mut units = Vec::new();
            for chunk in wire.chunks(fragment) {
                units.extend(decoder.feed(chunk, false).unwrap());
            }
            units.extend(decoder.feed(&[], true).unwrap());
            assert_eq!(
                units.last().unwrap().terminal,
                Some(if mutation == 0 {
                    NativeTerminalOutcome::Complete
                } else {
                    NativeTerminalOutcome::Unknown
                }),
                "mutation {mutation}, chunk {fragment}"
            );
        }
    }
}
