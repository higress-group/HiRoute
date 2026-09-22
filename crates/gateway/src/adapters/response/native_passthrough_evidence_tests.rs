use super::*;

#[test]
fn responses_item_identity_cannot_alias_across_type_and_id_boundaries() {
    let added = sse(
        Some("response.output_item.added"),
        &json!({
            "type":"response.output_item.added", "output_index":0,
            "item":{"type":"a\u{0000}b", "id":"c"}
        }),
    );
    let terminal = sse(
        Some("response.completed"),
        &json!({
            "type":"response.completed", "response":{
                "id":"response", "model":"physical", "status":"completed",
                "output":[{"type":"a", "id":"b\u{0000}c"}]
            }
        }),
    );
    let mut stream = projector(IngressProtocol::Responses, true);
    let units = stream.feed(&[added, terminal].concat(), true).unwrap();
    assert_eq!(
        unit_json(&units[1])["response"]["output"][0]["id"],
        "b\u{0000}c"
    );
    assert_eq!(
        units.last().unwrap().terminal,
        Some(NativeTerminalOutcome::Unknown)
    );
}

#[test]
fn responses_deltas_are_compared_to_the_final_snapshot_without_buffering_text() {
    for (final_text, outcome) in [
        ("first-second", NativeTerminalOutcome::Complete),
        ("replacement", NativeTerminalOutcome::Unknown),
    ] {
        let mut projector = projector(IngressProtocol::Responses, true);
        let mut events = Vec::new();
        for delta in ["first-", "second"] {
            events.extend(sse(
                Some("response.output_text.delta"),
                &json!({
                    "type":"response.output_text.delta", "item_id":"item",
                    "output_index":0,"content_index":0,"delta":delta
                }),
            ));
        }
        events.extend(sse(Some("response.completed"), &json!({
            "type":"response.completed","response":{
                "id":"response","model":"physical","status":"completed",
                "output":[{"type":"message","id":"item","content":[{"type":"output_text","text":final_text}]}]
            }
        })));
        let units = projector.feed(&events, true).unwrap();
        assert!(units[0].semantic);
        assert_eq!(unit_json(&units[0])["delta"], "first-");
        assert_eq!(units.last().unwrap().terminal, Some(outcome));
    }
}

#[test]
fn responses_delta_after_done_keeps_wire_but_withholds_complete() {
    let mut stream = projector(IngressProtocol::Responses, true);
    let events = [
        sse(
            Some("response.output_text.done"),
            &json!({
                "type":"response.output_text.done", "item_id":"item",
                "output_index":0,"content_index":0,"text":"A"
            }),
        ),
        sse(
            Some("response.output_text.delta"),
            &json!({
                "type":"response.output_text.delta", "item_id":"item",
                "output_index":0,"content_index":0,"delta":"B"
            }),
        ),
        sse(
            Some("response.completed"),
            &json!({
                "type":"response.completed", "response":{
                    "id":"response","model":"physical","status":"completed",
                    "output":[{"type":"message","id":"item","content":[{"type":"output_text","text":"AB"}]}]
                }
            }),
        ),
    ]
    .concat();
    let units = stream.feed(&events, true).unwrap();
    assert_eq!(unit_json(&units[1])["delta"], "B");
    assert_eq!(
        units.last().unwrap().terminal,
        Some(NativeTerminalOutcome::Unknown)
    );
}

#[test]
fn populated_added_item_must_remain_a_prefix_of_the_final_snapshot() {
    for (final_text, outcome) in [
        ("AB", NativeTerminalOutcome::Complete),
        ("B", NativeTerminalOutcome::Unknown),
    ] {
        let mut stream = projector(IngressProtocol::Responses, true);
        let events = [
            sse(
                Some("response.output_item.added"),
                &json!({
                    "type":"response.output_item.added","output_index":0,
                    "item":{"type":"message","id":"item","content":[{"type":"output_text","text":"A"}]}
                }),
            ),
            sse(
                Some("response.completed"),
                &json!({
                    "type":"response.completed","response":{
                        "id":"response","model":"physical","status":"completed",
                        "output":[{"type":"message","id":"item","content":[{"type":"output_text","text":final_text}]}]
                    }
                }),
            ),
        ]
        .concat();
        let units = stream.feed(&events, true).unwrap();
        assert_eq!(unit_json(&units[0])["item"]["content"][0]["text"], "A");
        assert_eq!(units.last().unwrap().terminal, Some(outcome));
    }
}

#[test]
fn populated_added_prefix_and_later_delta_can_certify_the_same_final_text() {
    let events = [
        sse(
            Some("response.output_item.added"),
            &json!({
                "type":"response.output_item.added","output_index":0,
                "item":{"type":"message","id":"item","content":[{"type":"output_text","text":"A"}]}
            }),
        ),
        sse(
            Some("response.output_text.delta"),
            &json!({
                "type":"response.output_text.delta","item_id":"item",
                "output_index":0,"content_index":0,"delta":"B"
            }),
        ),
        sse(
            Some("response.output_text.done"),
            &json!({
                "type":"response.output_text.done","item_id":"item",
                "output_index":0,"content_index":0,"text":"AB"
            }),
        ),
        sse(
            Some("response.completed"),
            &json!({
                "type":"response.completed","response":{
                    "id":"response","model":"physical","status":"completed",
                    "output":[{"type":"message","id":"item","content":[{"type":"output_text","text":"AB"}]}]
                }
            }),
        ),
    ]
    .concat();
    let mut stream = projector(IngressProtocol::Responses, true);
    let units = stream.feed(&events, true).unwrap();
    assert_eq!(units[3].terminal, None);
    assert_eq!(units[4].terminal, Some(NativeTerminalOutcome::Complete));
}

#[test]
fn responses_delta_item_type_mismatch_keeps_wire_without_complete() {
    let events = [
        sse(
            Some("response.output_text.delta"),
            &json!({
                "type":"response.output_text.delta","item_id":"item",
                "output_index":0,"content_index":0,"delta":"A"
            }),
        ),
        sse(
            Some("response.completed"),
            &json!({
                "type":"response.completed","response":{
                    "id":"response","model":"physical","status":"completed",
                    "output":[{"type":"reasoning","id":"item","content":[{"type":"output_text","text":"A"}]}]
                }
            }),
        ),
    ]
    .concat();
    let mut stream = projector(IngressProtocol::Responses, true);
    let units = stream.feed(&events, true).unwrap();
    assert_eq!(
        unit_json(&units[1])["response"]["output"][0]["type"],
        "reasoning"
    );
    assert_eq!(
        units.last().unwrap().terminal,
        Some(NativeTerminalOutcome::Unknown)
    );
}
