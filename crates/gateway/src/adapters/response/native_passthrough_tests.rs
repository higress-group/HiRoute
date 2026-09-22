use hiroute_domain::ModelRequestRouteV2;
use hiroute_gateway_core::runtime::body::BudgetTree;
use serde_json::{Value, json};

use super::native_passthrough::{
    NativeProjectedUnit, NativeResponseProjector, NativeTerminalOutcome,
};
use crate::ports::ToolContinuationScopeV1;
use crate::server::core_runtime::adapters::ToolLogicalIdProjection;
use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};
use crate::server::request_plan::IngressProtocol;

fn profile(protocol: IngressProtocol) -> CandidateProtocolProfile {
    CandidateProtocolProfile::exact_portable_path(
        protocol,
        protocol,
        "physical",
        fixed_reasoning("fixed"),
    )
}

fn projection() -> ToolLogicalIdProjection {
    ToolLogicalIdProjection::for_test(
        [7_u8; 32],
        ToolContinuationScopeV1 {
            authority_id: "authority".into(),
            authority_epoch: 1,
            grant_id: "grant".into(),
            grant_generation: 1,
            served_model_id: "alias".into(),
            route: ModelRequestRouteV2::Plan {
                revision: 2,
                semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(b"plan"),
            },
        },
    )
}

fn projector(protocol: IngressProtocol, streaming: bool) -> NativeResponseProjector {
    NativeResponseProjector::new_for_observation(
        &profile(protocol),
        streaming,
        "alias".into(),
        None,
        projection(),
    )
    .unwrap()
}

fn sse(event: Option<&str>, value: &Value) -> Vec<u8> {
    let mut wire = Vec::new();
    if let Some(event) = event {
        wire.extend_from_slice(format!("event: {event}\n").as_bytes());
    }
    wire.extend_from_slice(format!("data: {value}\n\n").as_bytes());
    wire
}

fn feed_fragmented(
    projector: &mut NativeResponseProjector,
    wire: &[u8],
    fragment: usize,
) -> Vec<NativeProjectedUnit> {
    let mut output = Vec::new();
    for chunk in wire.chunks(fragment) {
        output.extend(projector.feed(chunk, false).unwrap());
    }
    output
}

fn unit_json(unit: &NativeProjectedUnit) -> Value {
    let wire = std::str::from_utf8(&unit.bytes).unwrap();
    let data = wire
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join("\n");
    serde_json::from_str(&data).unwrap()
}

#[test]
fn responses_native_done_tail_preserves_wire_and_certifies_one_terminal_at_eof() {
    let created = sse(
        Some("response.created"),
        &json!({"type":"response.created","response":{"id":"resp","model":"physical","status":"in_progress"}}),
    );
    let answer = sse(
        Some("response.output_item.done"),
        &json!({"type":"response.output_item.done","output_index":0,"item":{"type":"message","id":"msg","role":"assistant","status":"completed","content":[{"type":"output_text","text":"OK"}]}}),
    );
    let completed = sse(
        Some("response.completed"),
        &json!({"type":"response.completed","response":{"id":"resp","model":"physical","status":"completed","output":[{"type":"message","id":"msg","role":"assistant","status":"completed","content":[{"type":"output_text","text":"OK"}]}],"usage":{"input_tokens":5,"output_tokens":2,"total_tokens":7}}}),
    );
    let done = b"data: [DONE]\n\n";
    let wire = [created, answer, completed, done.to_vec()].concat();

    for fragment in [1, 7, wire.len()] {
        let mut projector = projector(IngressProtocol::Responses, true);
        let mut units = feed_fragmented(&mut projector, &wire, fragment);
        units.extend(projector.feed(&[], true).unwrap());
        assert_eq!(units.len(), 5);
        assert_eq!(unit_json(&units[2])["response"]["model"], "alias");
        assert_eq!(units[3].bytes, done);
        assert!(units[..4].iter().all(|unit| unit.terminal.is_none()));
        assert!(units[4].bytes.is_empty());
        assert_eq!(units[4].terminal, Some(NativeTerminalOutcome::Complete));
        assert_eq!(projector.usage().input_tokens, Some(5));
    }

    let mut duplicate = projector(IngressProtocol::Responses, true);
    assert!(
        duplicate
            .feed(&[wire, done.to_vec()].concat(), true)
            .is_err()
    );
    let mut missing_terminal = projector(IngressProtocol::Responses, true);
    assert!(missing_terminal.feed(done, true).is_err());
}

#[test]
fn responses_stream_preserves_unknown_wire_and_rewrites_only_owned_paths() {
    let unknown = b"event: response.vendor_extension\nid: opaque\ndata: {\"type\":\"response.vendor_extension\",\"model\":\"nested-untouched\",\"x\":7}\n\n";
    let created = json!({
        "type":"response.created",
        "vendor":{"model":"nested-untouched"},
        "response":{"id":"resp","model":"physical","status":"in_progress","vendor":9}
    });
    let tool = json!({
        "type":"response.output_item.added",
        "output_index":0,
        "item":{"type":"function_call","id":"item-native","call_id":"call-native","name":"weather","arguments":"","status":"in_progress","vendor":true}
    });
    let terminal = json!({
        "type":"response.completed",
        "response":{
            "id":"resp","model":"physical","status":"completed",
            "output":[
                {"type":"function_call","id":"item-native","call_id":"call-native","name":"weather","arguments":"{}","status":"completed","vendor":true},
                {"type":"reasoning","id":"reasoning-native","encrypted_content":"opaque-state","vendor":true}
            ],
            "usage":{"input_tokens":5,"output_tokens":2,"vendor":1},
            "vendor":{"status":"future"}
        }
    });
    let mut wire = unknown.to_vec();
    wire.extend(sse(Some("response.created"), &created));
    wire.extend(sse(Some("response.output_item.added"), &tool));
    wire.extend(sse(Some("response.completed"), &terminal));

    for fragment in [1, 7, wire.len()] {
        let mut projector = projector(IngressProtocol::Responses, true);
        let mut units = feed_fragmented(&mut projector, &wire, fragment);
        units.extend(projector.feed(&[], true).unwrap());
        assert_eq!(units[0].bytes, unknown);
        let created = unit_json(&units[1]);
        assert_eq!(created["response"]["model"], "alias");
        assert_eq!(created["vendor"]["model"], "nested-untouched");
        let tool = unit_json(&units[2]);
        assert_eq!(tool["item"]["id"], "item-native");
        assert!(
            tool["item"]["call_id"]
                .as_str()
                .unwrap()
                .starts_with("hiroute_tool_v1_")
        );
        let terminal = unit_json(&units[3]);
        assert_eq!(terminal["response"]["model"], "alias");
        assert_eq!(terminal["response"]["output"][0]["id"], "item-native");
        assert_eq!(
            terminal["response"]["output"][0]["call_id"],
            tool["item"]["call_id"]
        );
        assert_eq!(
            terminal["response"]["output"][1]["encrypted_content"],
            "opaque-state"
        );
        assert_eq!(
            units.last().unwrap().terminal,
            Some(NativeTerminalOutcome::Complete)
        );
        assert_eq!(projector.usage().input_tokens, Some(5));
    }
}

#[test]
fn responses_provider_state_can_advance_before_terminal() {
    let added = json!({
        "type":"response.output_item.added","output_index":0,
        "item":{"type":"reasoning","id":"reasoning","encrypted_content":"initial"}
    });
    let done = json!({
        "type":"response.output_item.done","output_index":0,
        "item":{"type":"reasoning","id":"reasoning","status":"completed","encrypted_content":"final"}
    });
    let terminal = json!({
        "type":"response.completed",
        "response":{
            "id":"response","model":"physical","status":"completed",
            "output":[{"type":"reasoning","id":"reasoning","status":"completed","encrypted_content":"final"}]
        }
    });
    let wire = [
        sse(Some("response.output_item.added"), &added),
        sse(Some("response.output_item.done"), &done),
        sse(Some("response.completed"), &terminal),
    ]
    .concat();

    let mut projector = projector(IngressProtocol::Responses, true);
    let mut units = feed_fragmented(&mut projector, &wire, 1);
    units.extend(projector.feed(&[], true).unwrap());
    assert_eq!(units.len(), 4);
    assert_eq!(unit_json(&units[0])["item"]["encrypted_content"], "initial");
    assert_eq!(unit_json(&units[1])["item"]["encrypted_content"], "final");
    assert_eq!(
        unit_json(&units[2])["response"]["output"][0]["encrypted_content"],
        "final"
    );
    assert_eq!(units[2].terminal, None);
    assert_eq!(units[3].terminal, Some(NativeTerminalOutcome::Complete));
}

#[test]
fn responses_hosted_search_identity_is_trusted_in_native_stream_and_snapshot() {
    let created = json!({
        "type":"response.created",
        "response":{"id":"search-response","model":"physical","status":"in_progress"}
    });
    let added = json!({
        "type":"response.output_item.added","output_index":0,
        "item":{"type":"web_search_call","id":"native-search","status":"in_progress"}
    });
    let progress = json!({
        "type":"response.web_search_call.searching","output_index":0,
        "item_id":"native-search"
    });
    let done = json!({
        "type":"response.output_item.done","output_index":0,
        "item":{"type":"web_search_call","id":"native-search","status":"completed","action":{"type":"search","query":"weather"}}
    });
    let terminal = json!({
        "type":"response.completed",
        "response":{
            "id":"search-response","model":"physical","status":"completed",
            "output":[{"type":"web_search_call","id":"native-search","status":"completed","action":{"type":"search","query":"weather"}}]
        }
    });
    let wire = [
        sse(Some("response.created"), &created),
        sse(Some("response.output_item.added"), &added),
        sse(Some("response.web_search_call.searching"), &progress),
        sse(Some("response.output_item.done"), &done),
        sse(Some("response.completed"), &terminal),
    ]
    .concat();
    let mut stream = projector(IngressProtocol::Responses, true);
    let mut units = feed_fragmented(&mut stream, &wire, 3);
    units.extend(stream.feed(&[], true).unwrap());
    let logical = unit_json(&units[1])["item"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(logical.starts_with("hiroute_tool_v1_"));
    assert_eq!(unit_json(&units[2])["item_id"], logical);
    assert_eq!(unit_json(&units[3])["item"]["id"], logical);
    assert_eq!(unit_json(&units[4])["response"]["output"][0]["id"], logical);
    assert_eq!(units[4].terminal, None);
    assert_eq!(units[5].terminal, Some(NativeTerminalOutcome::Complete));

    let mut document = projector(IngressProtocol::Responses, false);
    let body = serde_json::to_vec(&terminal["response"]).unwrap();
    let projected = document.feed(&body, true).unwrap();
    let projected: Value = serde_json::from_slice(&projected[0].bytes).unwrap();
    assert_eq!(projected["output"][0]["id"], logical);
}

#[test]
fn messages_stream_emits_content_before_stop_and_normalizes_cache_usage() {
    let events = [
        (
            "message_start",
            json!({"type":"message_start","message":{"id":"msg","type":"message","role":"assistant","model":"physical","content":[],"stop_reason":null,"usage":{"input_tokens":300,"cache_read_input_tokens":600,"cache_creation_input_tokens":100}}}),
        ),
        (
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":"","vendor":1}}),
        ),
        (
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"你好","vendor":true}}),
        ),
        (
            "message_delta",
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null,"vendor":"kept"},"usage":{"output_tokens":9}}),
        ),
        ("message_stop", json!({"type":"message_stop","vendor":7})),
    ];
    let wire = events
        .iter()
        .flat_map(|(event, value)| sse(Some(event), value))
        .collect::<Vec<_>>();
    let mut projector = projector(IngressProtocol::Messages, true);
    let units = feed_fragmented(&mut projector, &wire, 2);

    assert_eq!(units.len(), events.len());
    assert!(
        !units[1].semantic,
        "empty block_start is only a control shell"
    );
    assert!(units[2].semantic);
    assert_eq!(unit_json(&units[0])["message"]["model"], "alias");
    assert_eq!(unit_json(&units[2])["delta"]["vendor"], true);
    assert_eq!(
        units.last().unwrap().terminal,
        Some(NativeTerminalOutcome::Complete)
    );
    assert_eq!(projector.usage().input_tokens, Some(1000));
    assert_eq!(projector.usage().cache_read_tokens, Some(600));
    assert_eq!(projector.usage().cache_write_tokens, Some(100));
    assert_eq!(projector.usage().output_tokens, Some(9));
}

#[test]
fn chat_stream_keeps_usage_tail_and_only_done_is_terminal() {
    let first = json!({
        "id":"chat","object":"chat.completion.chunk","created":1,"model":"physical",
        "choices":[{"index":0,"delta":{"role":"assistant","content":"answer","vendor":1},"finish_reason":"stop"}],
        "vendor":{"model":"nested-untouched"}
    });
    let usage = json!({
        "id":"chat","object":"chat.completion.chunk","created":1,"model":"physical",
        "choices":[],"usage":{"prompt_tokens":4,"completion_tokens":2,"prompt_tokens_details":{"cached_tokens":3},"vendor":9}
    });
    let mut wire = sse(None, &first);
    wire.extend(sse(None, &usage));
    wire.extend_from_slice(b"data: [DONE]\n\n");
    let mut projector = projector(IngressProtocol::ChatCompletions, true);
    let units = feed_fragmented(&mut projector, &wire, 3);

    assert_eq!(units.len(), 3);
    assert_eq!(unit_json(&units[0])["model"], "alias");
    assert_eq!(unit_json(&units[0])["vendor"]["model"], "nested-untouched");
    assert_eq!(unit_json(&units[1])["usage"]["vendor"], 9);
    assert_eq!(units[0].terminal, None);
    assert_eq!(units[1].terminal, None);
    assert_eq!(units[2].terminal, Some(NativeTerminalOutcome::Complete));
    assert_eq!(projector.usage().input_tokens, Some(4));
    assert_eq!(projector.usage().cache_read_tokens, Some(3));
}

#[test]
fn native_nonstream_preserves_unknown_members_and_marks_incomplete() {
    let mut projector = projector(IngressProtocol::Messages, false);
    let body = serde_json::to_vec(&json!({
        "id":"msg","type":"message","role":"assistant","model":"physical",
        "content":[{"type":"text","text":"partial","vendor":{"future":true}}],
        "stop_reason":"max_tokens","stop_sequence":null,
        "usage":{"input_tokens":0,"output_tokens":1},
        "vendor_status":"future"
    }))
    .unwrap();
    let split = body.len() / 2;
    assert!(projector.feed(&body[..split], false).unwrap().is_empty());
    let units = projector.feed(&body[split..], true).unwrap();
    assert_eq!(units.len(), 1);
    let projected: Value = serde_json::from_slice(&units[0].bytes).unwrap();
    assert_eq!(projected["model"], "alias");
    assert_eq!(projected["content"][0]["vendor"]["future"], true);
    assert_eq!(projected["vendor_status"], "future");
    assert_eq!(units[0].terminal, Some(NativeTerminalOutcome::Incomplete));
    assert_eq!(projector.usage().input_tokens, Some(0));
}

#[test]
fn responses_nonstream_empty_output_does_not_cross_the_semantic_commit_gate() {
    let mut projector = projector(IngressProtocol::Responses, false);
    let body = serde_json::to_vec(&json!({
        "id":"response","model":"physical","status":"completed","output":[],
        "usage":{"input_tokens":1,"output_tokens":0,"total_tokens":1},
        "vendor":{"future":true}
    }))
    .unwrap();

    let units = projector.feed(&body, true).unwrap();

    assert_eq!(units.len(), 1);
    assert!(!units[0].semantic);
    assert_eq!(units[0].terminal, Some(NativeTerminalOutcome::Complete));
    let projected: Value = serde_json::from_slice(&units[0].bytes).unwrap();
    assert_eq!(projected["model"], "alias");
    assert_eq!(projected["vendor"]["future"], true);
}

#[test]
fn native_control_only_stream_has_no_semantic_output_even_with_a_valid_terminal() {
    let created = sse(
        Some("response.created"),
        &json!({
            "type":"response.created", "response":{"id":"empty","model":"physical"}
        }),
    );
    let terminal = sse(
        Some("response.completed"),
        &json!({
            "type":"response.completed",
            "response":{"id":"empty","model":"physical","status":"completed","output":[]}
        }),
    );
    let mut wire = b": heartbeat\n\n".to_vec();
    wire.extend(created);
    wire.extend(terminal);
    let mut projector = projector(IngressProtocol::Responses, true);
    let units = projector.feed(&wire, true).unwrap();
    assert_eq!(units.len(), 4);
    assert!(units.iter().all(|unit| !unit.semantic));
    assert_eq!(units[3].terminal, Some(NativeTerminalOutcome::Complete));
}

#[test]
fn opaque_extension_and_control_tail_keep_wire_without_certifying_completion() {
    let opaque = b"event: response.vendor_extension\ndata: opaque\n\n";
    let terminal = sse(
        Some("response.completed"),
        &json!({
            "type":"response.completed", "response":{
                "id":"response","model":"physical","status":"completed",
                "output":[{"type":"message","id":"item","content":[{"type":"output_text","text":"answer"}]}]
            }
        }),
    );
    let mut stream = projector(IngressProtocol::Responses, true);
    let mut wire = opaque.to_vec();
    wire.extend(terminal);
    wire.extend_from_slice(b": trailing heartbeat\n\n");
    let units = stream.feed(&wire, false).unwrap();
    assert_eq!(units[0].bytes, opaque);
    assert_eq!(units[2].bytes, b": trailing heartbeat\n\n");
    assert_eq!(units[1].terminal, None);
    assert_eq!(units[2].terminal, None);
    assert_eq!(
        stream.feed(&[], true).unwrap()[0].terminal,
        Some(NativeTerminalOutcome::Unknown)
    );

    let mut late_content = projector(IngressProtocol::Responses, true);
    let wire = [
        sse(
            Some("response.completed"),
            &json!({
                "type":"response.completed", "response":{
                    "id":"response","model":"physical","status":"completed",
                    "output":[{"type":"message","id":"item","content":[{"text":"answer"}]}]
                }
            }),
        ),
        sse(
            Some("response.output_text.delta"),
            &json!({
                "type":"response.output_text.delta", "item_id":"item","output_index":0,
                "content_index":0,"delta":"late"
            }),
        ),
    ]
    .concat();
    let units = late_content.feed(&wire, false).unwrap();
    assert_eq!(units[0].terminal, None);
    assert_eq!(unit_json(&units[1])["delta"], "late");
    assert_eq!(units[1].terminal, None);
    assert_eq!(
        late_content.feed(&[], true).unwrap()[0].terminal,
        Some(NativeTerminalOutcome::Unknown)
    );
}

#[test]
fn terminal_waits_for_a_split_legal_sse_tail_before_closing_the_wire() {
    let terminal = sse(
        Some("response.completed"),
        &json!({
            "type":"response.completed", "response":{
                "id":"response","model":"physical","status":"completed",
                "output":[{"type":"message","id":"item","content":[{"text":"answer"}]}]
            }
        }),
    );
    let tail = b"event: response.vendor_extension\ndata: opaque\n\n";
    let split = tail.len() / 2;
    let mut stream = projector(IngressProtocol::Responses, true);
    let mut first = terminal;
    first.extend_from_slice(&tail[..split]);
    let before_tail = stream.feed(&first, false).unwrap();
    assert_eq!(before_tail.len(), 1);
    assert_eq!(before_tail[0].terminal, None);

    let after_tail = stream.feed(&tail[split..], false).unwrap();
    assert_eq!(after_tail.len(), 1);
    assert_eq!(after_tail[0].bytes, tail);
    assert_eq!(after_tail[0].terminal, None);
    assert_eq!(
        stream.feed(&[], true).unwrap()[0].terminal,
        Some(NativeTerminalOutcome::Unknown)
    );
}

#[test]
fn native_sse_keeps_unrecognized_data_prefixed_fields_as_opaque_metadata() {
    let extension = b"event: response.vendor_extension\ndata-meta: opaque\ndata: {\"type\":\"response.vendor_extension\"}\n\n";
    let terminal = sse(
        Some("response.completed"),
        &json!({
            "type":"response.completed", "response":{
                "id":"response","model":"physical","status":"completed",
                "output":[{"type":"message","id":"item","content":[{"text":"answer"}]}]
            }
        }),
    );
    let mut stream = projector(IngressProtocol::Responses, true);
    let units = stream
        .feed(&[extension.as_slice(), &terminal].concat(), true)
        .unwrap();
    assert_eq!(units[0].bytes, extension);
    assert_eq!(units[1].terminal, None);
    assert_eq!(units[2].terminal, Some(NativeTerminalOutcome::Complete));
}

#[test]
fn native_inconsistent_descriptions_pass_wire_but_never_certify_complete() {
    let mut responses = projector(IngressProtocol::Responses, true);
    let added = sse(
        Some("response.output_item.done"),
        &json!({
            "type":"response.output_item.done", "output_index":0,
            "item":{"type":"message","id":"output","status":"completed","content":[{"type":"output_text","text":"seen"}]}
        }),
    );
    let terminal = sse(
        Some("response.completed"),
        &json!({
            "type":"response.completed", "response":{
                "id":"response","model":"physical","status":"completed","output":[]
            }
        }),
    );
    let units = responses.feed(&[added, terminal].concat(), true).unwrap();
    assert_eq!(unit_json(&units[0])["item"]["id"], "output");
    assert_eq!(
        units.last().unwrap().terminal,
        Some(NativeTerminalOutcome::Unknown)
    );

    let mut added_identity = projector(IngressProtocol::Responses, true);
    let events = [
        sse(
            Some("response.output_item.added"),
            &json!({
                "type":"response.output_item.added","output_index":0,
                "item":{"type":"message","id":"first","content":[]}
            }),
        ),
        sse(
            Some("response.completed"),
            &json!({
                "type":"response.completed","response":{
                    "id":"response","model":"physical","status":"completed",
                    "output":[{"type":"message","id":"second","content":[{"text":"kept"}]}]
                }
            }),
        ),
    ]
    .concat();
    let units = added_identity.feed(&events, true).unwrap();
    assert_eq!(
        units.last().unwrap().terminal,
        Some(NativeTerminalOutcome::Unknown)
    );

    let mut mismatched_status = projector(IngressProtocol::Responses, true);
    let event = sse(
        Some("response.completed"),
        &json!({
            "type":"response.completed", "response":{
                "id":"response","model":"physical","status":"incomplete",
                "output":[{"type":"message","content":[{"text":"partial"}]}]
            }
        }),
    );
    let units = mismatched_status.feed(&event, true).unwrap();
    assert_eq!(
        units.last().unwrap().terminal,
        Some(NativeTerminalOutcome::Unknown)
    );
    assert_eq!(unit_json(&units[0])["response"]["status"], "incomplete");

    let mut mismatched_event_field = projector(IngressProtocol::Responses, true);
    let event = sse(
        Some("response.in_progress"),
        &json!({
            "type":"response.completed", "response":{
                "id":"response","model":"physical","status":"completed",
                "output":[{"type":"message","content":[{"text":"kept"}]}]
            }
        }),
    );
    let units = mismatched_event_field.feed(&event, true).unwrap();
    assert_eq!(
        units.last().unwrap().terminal,
        Some(NativeTerminalOutcome::Unknown)
    );
    assert_eq!(
        unit_json(&units[0])["response"]["output"][0]["content"][0]["text"],
        "kept"
    );

    let mut early_progress = projector(IngressProtocol::Responses, true);
    let events = [
        sse(
            Some("response.web_search_call.searching"),
            &json!({
                "type":"response.web_search_call.searching", "output_index":0,"item_id":"native"
            }),
        ),
        sse(
            Some("response.completed"),
            &json!({
                "type":"response.completed","response":{
                    "id":"response","model":"physical","status":"completed","output":[]
                }
            }),
        ),
    ]
    .concat();
    let units = early_progress.feed(&events, true).unwrap();
    assert_eq!(unit_json(&units[0])["item_id"], "native");
    assert_eq!(
        units.last().unwrap().terminal,
        Some(NativeTerminalOutcome::Unknown)
    );

    let mut messages = projector(IngressProtocol::Messages, true);
    let conflicting = [
        sse(
            Some("message_delta"),
            &json!({"type":"message_delta","delta":{"stop_reason":"max_tokens"}}),
        ),
        sse(
            Some("message_delta"),
            &json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}),
        ),
        sse(Some("message_stop"), &json!({"type":"message_stop"})),
    ]
    .concat();
    let units = messages.feed(&conflicting, true).unwrap();
    assert_eq!(units[2].terminal, Some(NativeTerminalOutcome::Unknown));

    let mut same_class_messages = projector(IngressProtocol::Messages, true);
    let events = [
        sse(
            Some("message_delta"),
            &json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}),
        ),
        sse(
            Some("message_delta"),
            &json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}}),
        ),
        sse(Some("message_stop"), &json!({"type":"message_stop"})),
    ]
    .concat();
    let units = same_class_messages.feed(&events, true).unwrap();
    assert_eq!(units[2].terminal, Some(NativeTerminalOutcome::Unknown));

    let mut chat = projector(IngressProtocol::ChatCompletions, true);
    let chunk = |reason| {
        sse(
            None,
            &json!({
                "id":"chat", "model":"physical", "choices":[{"index":0,"delta":{},"finish_reason":reason}]
            }),
        )
    };
    let mut wire = [chunk("length"), chunk("stop")].concat();
    wire.extend_from_slice(b"data: [DONE]\n\n");
    let units = chat.feed(&wire, true).unwrap();
    assert_eq!(units[2].terminal, Some(NativeTerminalOutcome::Unknown));

    let mut same_class_chat = projector(IngressProtocol::ChatCompletions, true);
    let mut wire = [chunk("stop"), chunk("tool_calls")].concat();
    wire.extend_from_slice(b"data: [DONE]\n\n");
    let units = same_class_chat.feed(&wire, true).unwrap();
    assert_eq!(units[2].terminal, Some(NativeTerminalOutcome::Unknown));
}

#[path = "native_passthrough_evidence_tests.rs"]
mod evidence_tests;

#[test]
fn content_after_a_finish_description_withholds_complete_without_rejecting_wire() {
    let mut messages = projector(IngressProtocol::Messages, true);
    let events = [
        sse(
            Some("message_delta"),
            &json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}),
        ),
        sse(
            Some("content_block_delta"),
            &json!({"type":"content_block_delta","index":0,"delta":{"text":"late"}}),
        ),
        sse(Some("message_stop"), &json!({"type":"message_stop"})),
    ]
    .concat();
    let units = messages.feed(&events, true).unwrap();
    assert_eq!(unit_json(&units[1])["delta"]["text"], "late");
    assert_eq!(units[2].terminal, Some(NativeTerminalOutcome::Unknown));

    let mut chat = projector(IngressProtocol::ChatCompletions, true);
    let first = |finish: Value, text: &str| {
        sse(
            None,
            &json!({
                "id":"chat","model":"physical","choices":[{
                    "index":0,"delta":{"content":text},"finish_reason":finish
                }]
            }),
        )
    };
    let mut events = [first(json!("stop"), "early"), first(Value::Null, "late")].concat();
    events.extend_from_slice(b"data: [DONE]\n\n");
    let units = chat.feed(&events, true).unwrap();
    assert_eq!(
        unit_json(&units[1])["choices"][0]["delta"]["content"],
        "late"
    );
    assert_eq!(units[2].terminal, Some(NativeTerminalOutcome::Unknown));
}

#[test]
fn extra_chat_choice_keeps_wire_without_trusted_tool_or_success() {
    let mut chat = projector(IngressProtocol::ChatCompletions, true);
    let tool = json!({
        "index":17,"delta":{"content":"extra","tool_calls":[{
            "index":0,"id":"opaque-native","function":{"name":"weather","arguments":"{}"}
        }]},"finish_reason":"stop"
    });
    let mut wire = sse(
        None,
        &json!({
            "id":"chat","model":"physical","choices":[
                {"index":0,"delta":{"content":"primary"},"finish_reason":"stop"},tool
            ]
        }),
    );
    wire.extend_from_slice(b"data: [DONE]\n\n");
    let units = chat.feed(&wire, true).unwrap();
    assert_eq!(
        unit_json(&units[0])["choices"][1]["delta"]["tool_calls"][0]["id"],
        "opaque-native"
    );
    assert_eq!(units[1].terminal, Some(NativeTerminalOutcome::Unknown));
}

#[test]
fn native_execution_projection_uses_the_request_stream_budget() {
    let tree = BudgetTree::new(512, 512).unwrap();
    let budget = tree.stream(512).unwrap();
    let mut projector = NativeResponseProjector::new_for_attempt(
        &profile(IngressProtocol::Responses),
        true,
        "alias".into(),
        None,
        budget,
    )
    .unwrap();
    let event = sse(
        Some("response.output_text.delta"),
        &json!({
            "type":"response.output_text.delta", "delta":"x".repeat(1000)
        }),
    );
    assert!(projector.feed(&event, false).is_err());
}

#[test]
fn legal_unknown_terminal_values_are_not_promoted_to_success() {
    let mut responses = projector(IngressProtocol::Responses, false);
    let responses_body = serde_json::to_vec(&json!({
        "id":"response","model":"physical","status":"future_terminal","output":[]
    }))
    .unwrap();
    assert_eq!(
        responses.feed(&responses_body, true).unwrap()[0].terminal,
        Some(NativeTerminalOutcome::Unknown)
    );

    let mut messages = projector(IngressProtocol::Messages, false);
    let messages_body = serde_json::to_vec(&json!({
        "id":"message","type":"message","role":"assistant","model":"physical",
        "content":[],"stop_reason":"future_stop","usage":{"input_tokens":1,"output_tokens":0}
    }))
    .unwrap();
    assert_eq!(
        messages.feed(&messages_body, true).unwrap()[0].terminal,
        Some(NativeTerminalOutcome::Unknown)
    );

    let mut chat = projector(IngressProtocol::ChatCompletions, true);
    let mut chat_wire = sse(
        None,
        &json!({
            "id":"chat","object":"chat.completion.chunk","created":1,"model":"physical",
            "choices":[{"index":0,"delta":{},"finish_reason":"future_finish"}]
        }),
    );
    chat_wire.extend_from_slice(b"data: [DONE]\n\n");
    let units = chat.feed(&chat_wire, false).unwrap();
    assert_eq!(
        units.last().unwrap().terminal,
        Some(NativeTerminalOutcome::Unknown)
    );
    chat.feed(&[], true).unwrap();
}

#[test]
fn rewritten_multiline_sse_preserves_control_fields_and_fragmented_utf8() {
    let wire = concat!(
        ": heartbeat\n",
        "id: event-1\n",
        "retry: 1200\n",
        "vendor-control: keep\n",
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg\",\"type\":\"message\",\n",
        "data: \"role\":\"assistant\",\"model\":\"physical\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":1},\"provider\":\"你好\"}}\n\n",
    )
    .as_bytes();
    let mut projector = projector(IngressProtocol::Messages, true);
    let units = feed_fragmented(&mut projector, wire, 1);

    assert_eq!(units.len(), 1);
    let projected_wire = std::str::from_utf8(&units[0].bytes).unwrap();
    assert!(projected_wire.contains(": heartbeat\n"));
    assert!(projected_wire.contains("id: event-1\n"));
    assert!(projected_wire.contains("retry: 1200\n"));
    assert!(projected_wire.contains("vendor-control: keep\n"));
    assert!(projected_wire.contains("event: message_start\n"));
    let value = unit_json(&units[0]);
    assert_eq!(value["message"]["model"], "alias");
    assert_eq!(value["message"]["provider"], "你好");
}

#[test]
fn native_stream_rejects_identity_changes_and_transport_eof_without_terminal() {
    let mut responses = projector(IngressProtocol::Responses, true);
    let created = sse(
        Some("response.created"),
        &json!({
            "type":"response.created",
            "response":{"id":"response","model":"physical","status":"in_progress"}
        }),
    );
    responses.feed(&created, false).unwrap();
    let changed = sse(
        Some("response.in_progress"),
        &json!({
            "type":"response.in_progress",
            "response":{"id":"response","model":"changed","status":"in_progress"}
        }),
    );
    assert!(responses.feed(&changed, false).is_err());

    let mut chat = projector(IngressProtocol::ChatCompletions, true);
    let unfinished = sse(
        None,
        &json!({
            "id":"chat","object":"chat.completion.chunk","created":1,"model":"physical",
            "choices":[{"index":0,"delta":{"content":"partial"},"finish_reason":"stop"}]
        }),
    );
    assert!(chat.feed(&unfinished, true).is_err());
}

#[test]
fn native_messages_usage_overflow_keeps_cache_dimensions_and_unknown_total() {
    let mut projector = projector(IngressProtocol::Messages, false);
    let body = serde_json::to_vec(&json!({
        "id":"message","type":"message","role":"assistant","model":"physical",
        "content":[],"stop_reason":"end_turn",
        "usage":{
            "input_tokens":u64::MAX,
            "output_tokens":0,
            "cache_read_input_tokens":1,
            "cache_creation_input_tokens":2
        }
    }))
    .unwrap();
    let units = projector.feed(&body, true).unwrap();

    assert_eq!(units[0].terminal, Some(NativeTerminalOutcome::Complete));
    assert_eq!(projector.usage().input_tokens, None);
    assert_eq!(projector.usage().output_tokens, Some(0));
    assert_eq!(projector.usage().cache_read_tokens, Some(1));
    assert_eq!(projector.usage().cache_write_tokens, Some(2));
}

#[test]
fn messages_late_usage_overflow_cannot_leave_an_earlier_total_trusted() {
    let mut projector = projector(IngressProtocol::Messages, true);
    let events = [
        sse(
            Some("message_start"),
            &json!({
                "type":"message_start","message":{
                    "id":"message","model":"physical","usage":{"input_tokens":10}
                }
            }),
        ),
        sse(
            Some("message_delta"),
            &json!({
                "type":"message_delta","delta":{"stop_reason":"end_turn"},
                "usage":{"input_tokens":u64::MAX,"cache_read_input_tokens":1}
            }),
        ),
        sse(Some("message_stop"), &json!({"type":"message_stop"})),
    ]
    .concat();
    projector.feed(&events, true).unwrap();
    assert_eq!(projector.usage().input_tokens, None);
    assert_eq!(projector.usage().cache_read_tokens, Some(1));
}

#[test]
fn reasoning_and_tool_argument_deltas_emit_before_terminal() {
    let mut responses = projector(IngressProtocol::Responses, true);
    let responses_wire = [
        sse(
            Some("response.created"),
            &json!({
                "type":"response.created",
                "response":{"id":"response","model":"physical","status":"in_progress"}
            }),
        ),
        sse(
            Some("response.reasoning_summary_text.delta"),
            &json!({
                "type":"response.reasoning_summary_text.delta","item_id":"reasoning",
                "output_index":0,"summary_index":0,"delta":"plan"
            }),
        ),
        sse(
            Some("response.output_item.added"),
            &json!({
                "type":"response.output_item.added","output_index":1,
                "item":{"type":"function_call","id":"item","call_id":"native-call","name":"weather","arguments":"","status":"in_progress"}
            }),
        ),
        sse(
            Some("response.function_call_arguments.delta"),
            &json!({
                "type":"response.function_call_arguments.delta","item_id":"item",
                "output_index":1,"delta":"{\"city\":"
            }),
        ),
    ]
    .concat();
    let responses_units = feed_fragmented(&mut responses, &responses_wire, 3);
    assert_eq!(responses_units.len(), 4);
    assert!(responses_units[1].semantic);
    assert!(responses_units[3].semantic);
    assert!(responses_units.iter().all(|unit| unit.terminal.is_none()));

    let mut messages = projector(IngressProtocol::Messages, true);
    let messages_wire = [
        sse(
            Some("message_start"),
            &json!({
                "type":"message_start",
                "message":{"id":"message","type":"message","role":"assistant","model":"physical","content":[],"stop_reason":null,"usage":{"input_tokens":1}}
            }),
        ),
        sse(
            Some("content_block_start"),
            &json!({
                "type":"content_block_start","index":0,
                "content_block":{"type":"thinking","thinking":"","signature":"opaque-signature"}
            }),
        ),
        sse(
            Some("content_block_delta"),
            &json!({
                "type":"content_block_delta","index":0,
                "delta":{"type":"thinking_delta","thinking":"plan"}
            }),
        ),
        sse(
            Some("content_block_start"),
            &json!({
                "type":"content_block_start","index":1,
                "content_block":{"type":"tool_use","id":"native-tool","name":"weather","input":{}}
            }),
        ),
        sse(
            Some("content_block_delta"),
            &json!({
                "type":"content_block_delta","index":1,
                "delta":{"type":"input_json_delta","partial_json":"{\"city\":"}
            }),
        ),
    ]
    .concat();
    let messages_units = feed_fragmented(&mut messages, &messages_wire, 2);
    assert_eq!(messages_units.len(), 5);
    assert_eq!(
        unit_json(&messages_units[1])["content_block"]["signature"],
        "opaque-signature"
    );
    assert_eq!(unit_json(&messages_units[2])["delta"]["thinking"], "plan");
    assert!(
        unit_json(&messages_units[3])["content_block"]["id"]
            .as_str()
            .unwrap()
            .starts_with("hiroute_tool_v1_")
    );
    assert_eq!(
        unit_json(&messages_units[4])["delta"]["partial_json"],
        "{\"city\":"
    );
    assert!(messages_units.iter().all(|unit| unit.terminal.is_none()));

    let mut chat = projector(IngressProtocol::ChatCompletions, true);
    let chat_wire = sse(
        None,
        &json!({
            "id":"chat","object":"chat.completion.chunk","created":1,"model":"physical",
            "choices":[{
                "index":0,"finish_reason":null,
                "delta":{
                    "reasoning_content":"plan",
                    "tool_calls":[{
                        "index":0,"id":"native-chat-tool","type":"function",
                        "function":{"name":"weather","arguments":"{\"city\":"}
                    }]
                }
            }]
        }),
    );
    let chat_units = feed_fragmented(&mut chat, &chat_wire, 5);
    assert_eq!(chat_units.len(), 1);
    let chat_value = unit_json(&chat_units[0]);
    assert_eq!(
        chat_value["choices"][0]["delta"]["reasoning_content"],
        "plan"
    );
    assert!(
        chat_value["choices"][0]["delta"]["tool_calls"][0]["id"]
            .as_str()
            .unwrap()
            .starts_with("hiroute_tool_v1_")
    );
    assert!(chat_units[0].semantic);
    assert_eq!(chat_units[0].terminal, None);
}
