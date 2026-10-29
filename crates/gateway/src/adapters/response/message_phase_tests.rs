//! Derived semantic regression, not a recording of the original provider wire.
use super::*;

fn message(phase: Option<Value>, text: &str, status: &str) -> Value {
    let mut item = json!({"type":"message","id":"message-1","role":"assistant",
        "status":status,"content":[{"type":"output_text","text":text,"annotations":[]}]});
    if let Some(phase) = phase {
        item["phase"] = phase;
    }
    item
}

fn events(start: Option<Value>, end: Option<Value>) -> Vec<Value> {
    let mut added = message(start, "", "in_progress");
    added["content"] = json!([]);
    let done = message(end, "I will inspect the file.", "completed");
    vec![
        json!({"type":"response.created","response":{"id":"response-1","model":"physical","status":"in_progress","output":[]}}),
        json!({"type":"response.output_item.added","output_index":0,"item":added}),
        json!({"type":"response.content_part.added","output_index":0,"item_id":"message-1","content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),
        json!({"type":"response.output_text.delta","output_index":0,"item_id":"message-1","content_index":0,"delta":"I will inspect the file."}),
        json!({"type":"response.output_item.done","output_index":0,"item":done.clone()}),
        json!({"type":"response.completed","response":{"id":"response-1","model":"physical","status":"completed","output":[done],"usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5}}}),
    ]
}

fn stream(events: &[Value]) -> Vec<u8> {
    events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect::<String>()
        .into_bytes()
}

fn candidate() -> CandidateProtocolProfile {
    CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    )
}

#[test]
fn completed_message_phase_reconciles_provisional_and_late_metadata() {
    for (start, end) in [
        (Some(json!("final_answer")), Some(json!("commentary"))),
        (Some(json!("commentary")), Some(json!("final_answer"))),
        (None, Some(json!("commentary"))),
        (Some(Value::Null), Some(json!("final_answer"))),
        (Some(json!("final_answer")), None),
        (Some(json!("commentary")), Some(Value::Null)),
        (Some(json!("final_answer")), Some(json!("final_answer"))),
        (None, None),
        (None, Some(json!("future-phase"))),
    ] {
        let input = events(start.clone(), end.clone());
        let bytes = stream(&input);
        for chunk_size in [1, bytes.len()] {
            let mut decoder = NativeResponseDecoder::new(&candidate(), 200, true).unwrap();
            for chunk in bytes.chunks(chunk_size) {
                decoder.feed(chunk, false).unwrap();
            }
            decoder.feed(&[], true).unwrap();
            let canonical = decoder.take_events();
            let response = decoder.finish().unwrap().response;
            let expected = end.as_ref().and_then(Value::as_str);
            assert!(
                matches!(&response.blocks[0], ResponseBlock::Text { phase, text, item_id, .. }
                if phase.as_deref() == expected && text == "I will inspect the file."
                    && item_id.as_deref() == Some("message-1"))
            );

            let profile = ClientProtocolProfile::for_candidate(&candidate()).unwrap();
            let mut renderer = IncrementalClientSseRenderer::new(profile, "route").unwrap();
            let rendered: Vec<_> = canonical
                .iter()
                .flat_map(|event| renderer.push(event).unwrap())
                .collect();
            let done = rendered
                .iter()
                .find(|event| event.event.as_deref() == Some("response.output_item.done"))
                .unwrap();
            assert_eq!(
                done.data["item"].get("phase").and_then(Value::as_str),
                expected
            );
            let completed = rendered
                .iter()
                .find(|event| event.event.as_deref() == Some("response.completed"))
                .unwrap();
            assert_eq!(completed.data["response"]["output"][0], done.data["item"]);
        }
    }
}

#[test]
fn phase_reconciliation_does_not_repair_conflicting_identity_content_or_final_metadata() {
    for (target, key, value) in [
        (4, "id", json!("different-message")),
        (4, "role", json!("user")),
        (
            4,
            "content",
            json!([{"type":"output_text","text":"Different body","annotations":[]}]),
        ),
        (4, "phase", json!("")),
        (4, "phase", json!("bad\nphase")),
        (4, "phase", json!({"unexpected":true})),
        (5, "phase", json!("final_answer")),
    ] {
        let mut input = events(Some(json!("final_answer")), Some(json!("commentary")));
        if target == 4 {
            input[target]["item"][key] = value;
        } else {
            input[target]["response"]["output"][0][key] = value;
        }
        let mut decoder = NativeResponseDecoder::new(&candidate(), 200, true).unwrap();
        assert!(
            decoder
                .feed(&stream(&input), true)
                .and_then(|_| decoder.finish().map(drop))
                .is_err(),
            "must reject conflicting {key}"
        );
    }
}

#[test]
fn missing_terminal_phase_does_not_erase_completed_message_metadata() {
    let mut input = events(Some(json!("final_answer")), Some(json!("commentary")));
    input[5]["response"]["output"][0]
        .as_object_mut()
        .unwrap()
        .remove("phase");
    let mut decoder = NativeResponseDecoder::new(&candidate(), 200, true).unwrap();
    decoder.feed(&stream(&input), true).unwrap();
    assert!(
        matches!(&decoder.finish().unwrap().response.blocks[0], ResponseBlock::Text { phase, .. }
        if phase.as_deref() == Some("commentary"))
    );
}

#[test]
fn final_phase_remains_a_cross_protocol_capability_boundary() {
    let input = events(Some(json!("final_answer")), Some(json!("commentary")));
    let mut decoder = NativeResponseDecoder::new(&candidate(), 200, true).unwrap();
    decoder.feed(&stream(&input), true).unwrap();
    let canonical = decoder.take_events();
    for protocol in [IngressProtocol::ChatCompletions, IngressProtocol::Messages] {
        let candidate = CandidateProtocolProfile::exact_portable_path(
            protocol,
            IngressProtocol::Responses,
            "physical",
            fixed_reasoning("fixed"),
        );
        let profile = ClientProtocolProfile::for_candidate(&candidate).unwrap();
        let mut renderer = IncrementalClientSseRenderer::new(profile, "route").unwrap();
        assert!(
            canonical
                .iter()
                .try_for_each(|event| renderer.push(event).map(drop))
                .is_err()
        );
    }
}

#[test]
fn released_text_finished_bytes_without_final_phase_keep_their_meaning() {
    // Shape emitted by stable v0.3.1 / 13856abd873da5b4373d39322493b4cc105c4867.
    for original in [
        r#"{"kind":"text_finished","index":0,"text":"old answer","annotations":[],"status":"completed"}"#,
        r#"{"kind":"refusal_finished","index":0,"text":"old refusal","status":"completed"}"#,
    ] {
        let event: ModelEvent = serde_json::from_str(original).unwrap();
        assert_eq!(serde_json::to_string(&event).unwrap(), original);
    }
}

#[test]
fn completed_snapshot_can_supply_phase_without_an_item_done_event() {
    let mut input = events(Some(json!("final_answer")), Some(json!("commentary")));
    input.remove(4);
    let mut decoder = NativeResponseDecoder::new(&candidate(), 200, true).unwrap();
    decoder.feed(&stream(&input), true).unwrap();
    assert!(
        matches!(&decoder.finish().unwrap().response.blocks[0], ResponseBlock::Text { phase, .. }
        if phase.as_deref() == Some("commentary"))
    );
}

#[test]
fn refusal_completion_uses_final_phase_without_changing_refusal_text() {
    let mut input = events(Some(json!("final_answer")), Some(json!("commentary")));
    input[2]["part"] = json!({"type":"refusal","refusal":""});
    input[3]["type"] = json!("response.refusal.delta");
    for index in [4, 5] {
        let item = if index == 4 {
            &mut input[index]["item"]
        } else {
            &mut input[index]["response"]["output"][0]
        };
        item["content"] = json!([{"type":"refusal","refusal":"I will inspect the file."}]);
    }
    let mut decoder = NativeResponseDecoder::new(&candidate(), 200, true).unwrap();
    decoder.feed(&stream(&input), true).unwrap();
    let canonical = decoder.take_events();
    assert!(
        matches!(&decoder.finish().unwrap().response.blocks[0], ResponseBlock::Refusal { phase, text, .. }
        if phase.as_deref() == Some("commentary") && text == "I will inspect the file.")
    );
    let profile = ClientProtocolProfile::for_candidate(&candidate()).unwrap();
    let mut renderer = IncrementalClientSseRenderer::new(profile, "route").unwrap();
    let rendered: Vec<_> = canonical
        .iter()
        .flat_map(|event| renderer.push(event).unwrap())
        .collect();
    let done = rendered
        .iter()
        .find(|event| event.event.as_deref() == Some("response.output_item.done"))
        .unwrap();
    assert_eq!(done.data["item"]["phase"], "commentary");
    assert_eq!(
        done.data["item"]["content"][0]["refusal"],
        "I will inspect the file."
    );
}

#[test]
fn normal_status_usage_and_late_response_metadata_remain_updateable() {
    let mut input = events(None, None);
    input[0]["response"]["service_tier"] = json!("auto");
    input[0]["response"]["usage"] = Value::Null;
    input[5]["response"]["service_tier"] = json!("priority");
    input[5]["response"]["future_metadata"] = json!({"revision":2});
    let mut decoder = NativeResponseDecoder::new(&candidate(), 200, true).unwrap();
    decoder.feed(&stream(&input), true).unwrap();
    let response = decoder.finish().unwrap().response;
    assert_eq!(response.responses_metadata["service_tier"], "priority");
    assert_eq!(
        response.responses_metadata["future_metadata"],
        json!({"revision":2})
    );
    assert!(matches!(
        &response.blocks[0],
        ResponseBlock::Text {
            status: ResponseItemStatus::Completed,
            ..
        }
    ));
    assert_eq!(
        serde_json::to_value(response.usage).unwrap()["output_tokens"],
        3
    );
}

fn annotated_events() -> Vec<Value> {
    let first = json!({"type":"url_citation","start_index":0,"end_index":1,
        "title":"First","url":"https://example.com/first"});
    let second = json!({"type":"url_citation","start_index":2,"end_index":3,
        "title":"Second","url":"https://example.com/second"});
    let mut input = events(None, None);
    input[4]["item"]["content"][0]["annotations"] = json!([first, second]);
    input[5]["response"]["output"][0] = input[4]["item"].clone();
    input.insert(
        4,
        json!({"type":"response.output_text.annotation.added",
        "output_index":0,"item_id":"message-1","content_index":0,
        "annotation_index":0,"annotation":first}),
    );
    input
}

#[test]
fn final_annotations_extend_an_accepted_prefix_without_rewriting_it() {
    let mut decoder = NativeResponseDecoder::new(&candidate(), 200, true).unwrap();
    decoder.feed(&stream(&annotated_events()), true).unwrap();
    assert!(
        matches!(&decoder.finish().unwrap().response.blocks[0], ResponseBlock::Text { annotations, .. }
        if annotations.len() == 2 && annotations[0].title == "First" && annotations[1].title == "Second")
    );
}

#[test]
fn final_annotations_cannot_remove_or_replace_an_accepted_citation() {
    for removed in [false, true] {
        let mut input = annotated_events();
        if removed {
            input[5]["item"]["content"][0]["annotations"] = json!([]);
        } else {
            input[5]["item"]["content"][0]["annotations"][0]["url"] =
                json!("https://example.com/replaced");
        }
        input[6]["response"]["output"][0] = input[5]["item"].clone();
        let mut decoder = NativeResponseDecoder::new(&candidate(), 200, true).unwrap();
        assert!(decoder.feed(&stream(&input), true).is_err());
    }
}
