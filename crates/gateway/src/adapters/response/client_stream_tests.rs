use super::super::{
    ClientResponseRenderer, NativeResponseDecoder, RenderedClientResponse, ResponseDecodeStatus,
};
use super::*;
use crate::server::core_runtime::model_ir::ResponseItemStatus;
use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};

#[test]
fn responses_decoder_accepts_one_done_marker_only_after_terminal() {
    let candidate = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let terminal = format!(
        "event: response.completed\ndata: {}\n\n",
        json!({"type":"response.completed","response":{
            "id":"response","model":"physical","status":"completed","output":[]
        }})
    );
    let done = b"data: [DONE]\n\n";
    let mut decoder = NativeResponseDecoder::new(&candidate, 200, true).unwrap();
    decoder.feed(terminal.as_bytes(), false).unwrap();
    decoder.feed(done, true).unwrap();
    assert!(decoder.finish().is_ok());

    let mut before_terminal = NativeResponseDecoder::new(&candidate, 200, true).unwrap();
    assert!(before_terminal.feed(done, true).is_err());
    let mut duplicate = NativeResponseDecoder::new(&candidate, 200, true).unwrap();
    let wire = [terminal.as_bytes(), done, done].concat();
    assert!(duplicate.feed(&wire, true).is_err());
}

#[test]
fn only_the_default_responses_answer_phase_is_portable_to_messages() {
    assert!(client_can_represent_message_phase(
        IngressProtocol::Messages,
        Some("final_answer")
    ));
    assert!(!client_can_represent_message_phase(
        IngressProtocol::Messages,
        Some("analysis")
    ));
    assert!(!client_can_represent_message_phase(
        IngressProtocol::ChatCompletions,
        Some("commentary")
    ));
}

#[test]
fn every_native_upstream_renders_correlated_messages_for_responses_clients() {
    for (upstream, document) in [
        (
            IngressProtocol::Responses,
            json!({"id":"r","model":"physical","status":"completed","output":[{"type":"message","id":"native-message","role":"assistant","content":[{"type":"output_text","text":"fixture answer","annotations":[]}]}],"usage":{"input_tokens":4,"output_tokens":2,"total_tokens":6}}),
        ),
        (
            IngressProtocol::ChatCompletions,
            json!({"id":"r","model":"physical","choices":[{"index":0,"message":{"role":"assistant","content":"fixture answer"},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":2,"total_tokens":6}}),
        ),
        (
            IngressProtocol::Messages,
            json!({"id":"r","type":"message","role":"assistant","model":"physical","content":[{"type":"text","text":"fixture answer"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":4,"output_tokens":2}}),
        ),
    ] {
        let expected_item_id = if upstream == IngressProtocol::Responses {
            "native-message"
        } else {
            "msg_0"
        };
        let candidate = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            upstream,
            "physical",
            fixed_reasoning("fixed"),
        );
        let profile = ClientProtocolProfile::for_candidate(&candidate).unwrap();
        let mut decoder = NativeResponseDecoder::new(&candidate, 200, false).unwrap();
        decoder
            .feed(&serde_json::to_vec(&document).unwrap(), true)
            .unwrap();
        let mut renderer = IncrementalClientSseRenderer::new(profile.clone(), "alias").unwrap();
        let mut rendered = Vec::new();
        for event in decoder.take_events() {
            rendered.extend(renderer.push(&event).unwrap());
            assert_eq!(renderer.buffered_semantic_bytes(), 0);
        }
        assert_text_lifecycle(&rendered, expected_item_id);
        let decoded = decoder.finish().unwrap();
        let rendered = ClientResponseRenderer::render_stream_with_profile(
            &profile,
            "alias",
            &decoded.response,
        )
        .unwrap();
        let RenderedClientResponse::Sse { events, .. } = rendered else {
            panic!("expected SSE")
        };
        assert_text_lifecycle(&events, expected_item_id);
    }
}

fn assert_text_lifecycle(events: &[RenderedSseEvent], expected_item_id: &str) {
    let lifecycle = events
        .iter()
        .filter_map(|event| event.event.as_deref())
        .filter(|event| {
            matches!(
                *event,
                "response.output_item.added"
                    | "response.content_part.added"
                    | "response.output_text.delta"
                    | "response.output_text.done"
                    | "response.content_part.done"
                    | "response.output_item.done"
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        lifecycle,
        [
            "response.output_item.added",
            "response.content_part.added",
            "response.output_text.delta",
            "response.output_text.done",
            "response.content_part.done",
            "response.output_item.done",
        ]
    );
    let added = events
        .iter()
        .find(|event| event.event.as_deref() == Some("response.output_item.added"))
        .unwrap();
    let delta = events
        .iter()
        .find(|event| event.event.as_deref() == Some("response.output_text.delta"))
        .unwrap();
    assert_eq!(added.data["item"]["id"], delta.data["item_id"]);
    assert_eq!(added.data["item"]["id"], expected_item_id);
    assert_eq!(added.data["item"]["role"], "assistant");
    assert_eq!(added.data["item"]["status"], "in_progress");
    assert_eq!(added.data["item"]["content"], json!([]));
    assert_eq!(delta.data["delta"], "fixture answer");
    let done = events
        .iter()
        .find(|event| event.event.as_deref() == Some("response.output_item.done"))
        .unwrap();
    assert_eq!(done.data["item"]["id"], expected_item_id);
    assert_eq!(done.data["item"]["content"][0]["text"], "fixture answer");
}

#[test]
fn responses_same_protocol_preserves_message_phase_and_completed_output() {
    let events = render_incrementally(
        IngressProtocol::Responses,
        json!({
            "id": "r",
            "model": "physical",
            "status": "completed",
            "output": [{
                "type": "message",
                "id": "native-message",
                "status": "completed",
                "role": "assistant",
                "phase": "final_answer",
                "content": [{
                    "type": "output_text",
                    "text": "fixture answer",
                    "annotations": []
                }]
            }],
            "usage": {"input_tokens": 4, "output_tokens": 2, "total_tokens": 6}
        }),
    );

    let added = events
        .iter()
        .find(|event| event.event.as_deref() == Some("response.output_item.added"))
        .unwrap();
    let done = events
        .iter()
        .find(|event| event.event.as_deref() == Some("response.output_item.done"))
        .unwrap();
    let completed = events
        .iter()
        .find(|event| event.event.as_deref() == Some("response.completed"))
        .unwrap();
    let expected = json!({
        "type": "message",
        "id": "native-message",
        "status": "completed",
        "role": "assistant",
        "phase": "final_answer",
        "content": [{
            "type": "output_text",
            "text": "fixture answer",
            "annotations": []
        }]
    });

    assert_eq!(added.data["item"]["phase"], "final_answer");
    assert_eq!(done.data["item"], expected);
    assert_eq!(completed.data["response"]["output"], json!([expected]));
}

#[test]
fn every_native_upstream_emits_complete_responses_reasoning_lifecycle() {
    for (upstream, document, expected_item_id) in [
        (
            IngressProtocol::Responses,
            json!({"id":"r","model":"physical","status":"completed","output":[{"type":"reasoning","id":"native-reasoning","status":"completed","summary":[{"type":"summary_text","text":"fixture thought"}]}],"usage":{"input_tokens":4,"output_tokens":2,"total_tokens":6}}),
            "native-reasoning",
        ),
        (
            IngressProtocol::ChatCompletions,
            json!({"id":"r","model":"physical","choices":[{"index":0,"message":{"role":"assistant","content":null,"reasoning_content":"fixture thought"},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":2,"total_tokens":6}}),
            "rs_0",
        ),
        (
            IngressProtocol::Messages,
            json!({"id":"r","type":"message","role":"assistant","model":"physical","content":[{"type":"thinking","thinking":"fixture thought","signature":""}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":4,"output_tokens":2}}),
            "rs_0",
        ),
    ] {
        let events = render_incrementally(upstream, document);
        assert_lifecycle(
            &events,
            &[
                "response.output_item.added",
                "response.reasoning_summary_part.added",
                "response.reasoning_summary_text.delta",
                "response.reasoning_summary_text.done",
                "response.reasoning_summary_part.done",
                "response.output_item.done",
            ],
        );
        let done = events
            .iter()
            .find(|event| event.event.as_deref() == Some("response.output_item.done"))
            .unwrap();
        assert_eq!(done.data["item"]["id"], expected_item_id);
        assert_eq!(done.data["item"]["summary"][0]["text"], "fixture thought");
    }
}

#[test]
fn every_native_upstream_emits_complete_responses_refusal_lifecycle() {
    for (upstream, document, expected_item_id, expected_terminal) in [
        (
            IngressProtocol::Responses,
            json!({"id":"r","model":"physical","status":"completed","output":[{"type":"message","id":"native-refusal","status":"completed","role":"assistant","content":[{"type":"refusal","refusal":"cannot comply"}]}],"usage":{"input_tokens":4,"output_tokens":2,"total_tokens":6}}),
            "native-refusal",
            "response.completed",
        ),
        (
            IngressProtocol::ChatCompletions,
            json!({"id":"r","model":"physical","choices":[{"index":0,"message":{"role":"assistant","content":null,"refusal":"cannot comply"},"finish_reason":"content_filter"}],"usage":{"prompt_tokens":4,"completion_tokens":2,"total_tokens":6}}),
            "msg_0",
            "response.incomplete",
        ),
        (
            IngressProtocol::Messages,
            json!({"id":"r","type":"message","role":"assistant","model":"physical","content":[{"type":"text","text":"cannot comply"}],"stop_reason":"refusal","stop_sequence":null,"usage":{"input_tokens":4,"output_tokens":2}}),
            "msg_0",
            "response.incomplete",
        ),
    ] {
        let events = render_incrementally(upstream, document);
        assert_lifecycle(
            &events,
            &[
                "response.output_item.added",
                "response.content_part.added",
                "response.refusal.delta",
                "response.refusal.done",
                "response.content_part.done",
                "response.output_item.done",
            ],
        );
        let done = events
            .iter()
            .find(|event| event.event.as_deref() == Some("response.output_item.done"))
            .unwrap();
        assert_eq!(done.data["item"]["id"], expected_item_id);
        assert_eq!(done.data["item"]["content"][0]["refusal"], "cannot comply");
        assert!(
            events
                .iter()
                .any(|event| event.event.as_deref() == Some(expected_terminal))
        );
    }
}

#[test]
fn chat_non_success_finishes_render_as_responses_incomplete() {
    for (message, finish_reason, incomplete_reason) in [
        (
            json!({"role":"assistant","content":"truncated"}),
            "length",
            "max_output_tokens",
        ),
        (
            json!({"role":"assistant","content":null,"refusal":"cannot comply"}),
            "content_filter",
            "content_filter",
        ),
    ] {
        let candidate = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::ChatCompletions,
            "physical",
            fixed_reasoning("fixed"),
        );
        let profile = ClientProtocolProfile::for_candidate(&candidate).unwrap();
        let document = json!({
            "id":"r",
            "model":"physical",
            "choices":[{"index":0,"message":message,"finish_reason":finish_reason}],
            "usage":{"prompt_tokens":4,"completion_tokens":2,"total_tokens":6}
        });
        let mut decoder = NativeResponseDecoder::new(&candidate, 200, false).unwrap();
        decoder
            .feed(&serde_json::to_vec(&document).unwrap(), true)
            .unwrap();
        let mut incremental = IncrementalClientSseRenderer::new(profile.clone(), "alias").unwrap();
        let mut incremental_events = Vec::new();
        for event in decoder.take_events() {
            incremental_events.extend(incremental.push(&event).unwrap());
        }
        assert_incomplete_terminal(&incremental_events, incomplete_reason);

        let decoded = decoder.finish().unwrap();
        let RenderedClientResponse::Json { body, .. } =
            ClientResponseRenderer::render_nonstream_with_profile(
                &profile,
                "alias",
                &decoded.response,
            )
            .unwrap()
        else {
            panic!("expected JSON")
        };
        assert_eq!(body["status"], "incomplete");
        assert_eq!(body["incomplete_details"]["reason"], incomplete_reason);

        let RenderedClientResponse::Sse { events, .. } =
            ClientResponseRenderer::render_stream_with_profile(
                &profile,
                "alias",
                &decoded.response,
            )
            .unwrap()
        else {
            panic!("expected SSE")
        };
        assert_incomplete_terminal(&events, incomplete_reason);
    }
}

#[test]
fn native_responses_incomplete_is_accepted_for_json_and_sse() {
    let candidate = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let output = json!([
        {"type":"message","id":"m-text","status":"incomplete","role":"assistant","content":[{"type":"output_text","text":"partial","annotations":[]}]},
        {"type":"message","id":"m-refusal","status":"incomplete","role":"assistant","content":[{"type":"refusal","refusal":"cannot finish"}]},
        {"type":"function_call","id":"fc-native","call_id":"call-native","name":"lookup","arguments":"{\"q\":\"partial\"}","status":"incomplete"}
    ]);
    let json_response = json!({
        "id":"r-incomplete",
        "model":"physical",
        "status":"incomplete",
        "incomplete_details":{"reason":"max_output_tokens"},
        "output":output.clone(),
        "usage":{"input_tokens":4,"output_tokens":2,"total_tokens":6}
    });
    let sse = [
        ("response.created", json!({"type":"response.created","response":{"id":"r-incomplete","model":"physical"}})),
        ("response.output_item.added", json!({"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"m-text","status":"in_progress","role":"assistant","content":[]}})),
        ("response.output_text.delta", json!({"type":"response.output_text.delta","item_id":"m-text","output_index":0,"content_index":0,"delta":"partial"})),
        ("response.output_text.done", json!({"type":"response.output_text.done","item_id":"m-text","output_index":0,"content_index":0,"text":"partial"})),
        ("response.output_item.done", json!({"type":"response.output_item.done","output_index":0,"item":output[0].clone()})),
        ("response.output_item.added", json!({"type":"response.output_item.added","output_index":1,"item":{"type":"message","id":"m-refusal","status":"in_progress","role":"assistant","content":[]}})),
        ("response.refusal.delta", json!({"type":"response.refusal.delta","item_id":"m-refusal","output_index":1,"content_index":0,"delta":"cannot finish"})),
        ("response.refusal.done", json!({"type":"response.refusal.done","item_id":"m-refusal","output_index":1,"content_index":0,"refusal":"cannot finish"})),
        ("response.output_item.done", json!({"type":"response.output_item.done","output_index":1,"item":output[1].clone()})),
        ("response.output_item.added", json!({"type":"response.output_item.added","output_index":2,"item":{"type":"function_call","id":"fc-native","call_id":"call-native","name":"lookup","arguments":"","status":"in_progress"}})),
        ("response.function_call_arguments.delta", json!({"type":"response.function_call_arguments.delta","item_id":"fc-native","output_index":2,"delta":"{\"q\":\"partial\"}"})),
        ("response.function_call_arguments.done", json!({"type":"response.function_call_arguments.done","item_id":"fc-native","output_index":2,"arguments":"{\"q\":\"partial\"}"})),
        ("response.output_item.done", json!({"type":"response.output_item.done","output_index":2,"item":output[2].clone()})),
        ("response.incomplete", json!({"type":"response.incomplete","response":json_response.clone()})),
    ]
    .into_iter()
    .flat_map(|(event, data)| {
        RenderedSseEvent {
            event: Some(event.into()),
            data,
        }
        .wire_bytes()
        .unwrap()
    })
    .collect::<Vec<_>>();
    for (streaming, bytes) in [
        (false, serde_json::to_vec(&json_response).unwrap()),
        (true, sse),
    ] {
        let profile = ClientProtocolProfile::for_candidate(&candidate).unwrap();
        let mut decoder = NativeResponseDecoder::new_for_observation(
            &candidate,
            200,
            streaming,
            super::super::test_tool_projection(),
            None,
        )
        .unwrap();
        assert_eq!(
            decoder.feed(&bytes, true).unwrap(),
            ResponseDecodeStatus::Terminal
        );
        let mut renderer = IncrementalClientSseRenderer::new(profile.clone(), "alias").unwrap();
        let mut events = Vec::new();
        for event in decoder.take_events() {
            events.extend(renderer.push(&event).unwrap());
        }
        assert_incomplete_terminal(&events, "max_output_tokens");
        assert_incomplete_item_statuses(&events, 3);

        let response = decoder.finish().unwrap().response;
        assert_eq!(response.finish_reason, Some(FinishReason::Length));
        assert_eq!(response.blocks.len(), 3);
        assert!(response.blocks.iter().all(|block| match block {
            ResponseBlock::Text { status, .. }
            | ResponseBlock::Reasoning { status, .. }
            | ResponseBlock::Refusal { status, .. }
            | ResponseBlock::ToolCall { status, .. } => {
                *status == ResponseItemStatus::Incomplete
            }
            ResponseBlock::WebSearch { .. } => false,
        }));

        let RenderedClientResponse::Json { body, .. } =
            ClientResponseRenderer::render_nonstream_with_profile(&profile, "alias", &response)
                .unwrap()
        else {
            panic!("expected JSON")
        };
        assert_eq!(body["status"], "incomplete");
        assert!(
            body["output"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["status"] == "incomplete")
        );

        let RenderedClientResponse::Sse { events, .. } =
            ClientResponseRenderer::render_stream_with_profile(&profile, "alias", &response)
                .unwrap()
        else {
            panic!("expected SSE")
        };
        assert_incomplete_terminal(&events, "max_output_tokens");
        assert_incomplete_item_statuses(&events, 3);
    }
}

fn assert_incomplete_item_statuses(events: &[RenderedSseEvent], expected: usize) {
    let done = events
        .iter()
        .filter(|event| event.event.as_deref() == Some("response.output_item.done"))
        .collect::<Vec<_>>();
    assert_eq!(done.len(), expected);
    assert!(
        done.iter()
            .all(|event| event.data["item"]["status"] == "incomplete")
    );
    let terminal = events
        .iter()
        .find(|event| event.event.as_deref() == Some("response.incomplete"))
        .unwrap();
    assert!(
        terminal.data["response"]["output"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["status"] == "incomplete")
    );
}

fn assert_incomplete_terminal(events: &[RenderedSseEvent], reason: &str) {
    assert!(
        events
            .iter()
            .all(|event| event.event.as_deref() != Some("response.completed"))
    );
    let terminal = events
        .iter()
        .find(|event| event.event.as_deref() == Some("response.incomplete"))
        .expect("Responses incomplete terminal event");
    assert_eq!(terminal.data["type"], "response.incomplete");
    assert_eq!(terminal.data["response"]["status"], "incomplete");
    assert_eq!(
        terminal.data["response"]["incomplete_details"]["reason"],
        reason
    );
}

fn render_incrementally(upstream: IngressProtocol, document: Value) -> Vec<RenderedSseEvent> {
    let candidate = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        upstream,
        "physical",
        fixed_reasoning("fixed"),
    );
    let profile = ClientProtocolProfile::for_candidate(&candidate).unwrap();
    let mut decoder = NativeResponseDecoder::new_for_observation(
        &candidate,
        200,
        false,
        super::super::test_tool_projection(),
        None,
    )
    .unwrap();
    decoder
        .feed(&serde_json::to_vec(&document).unwrap(), true)
        .unwrap();
    let mut renderer = IncrementalClientSseRenderer::new(profile, "alias").unwrap();
    let mut rendered = Vec::new();
    for event in decoder.take_events() {
        rendered.extend(renderer.push(&event).unwrap());
    }
    decoder.finish().unwrap();
    rendered
}

fn assert_lifecycle(events: &[RenderedSseEvent], expected: &[&str]) {
    let actual = events
        .iter()
        .filter_map(|event| event.event.as_deref())
        .filter(|event| expected.contains(event))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn three_upstream_tool_completions_emit_one_correlated_responses_item_done() {
    for (upstream, document) in [
        (
            IngressProtocol::Responses,
            json!({"id":"r","model":"physical","status":"completed","output":[{"type":"function_call","id":"native-item","call_id":"native-call","name":"lookup","arguments":"{\"q\":\"test\"}"}],"usage":{"input_tokens":4,"output_tokens":2,"total_tokens":6}}),
        ),
        (
            IngressProtocol::ChatCompletions,
            json!({"id":"r","model":"physical","choices":[{"index":0,"message":{"role":"assistant","tool_calls":[{"id":"native-call","type":"function","function":{"name":"lookup","arguments":"{\"q\":\"test\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":4,"completion_tokens":2,"total_tokens":6}}),
        ),
        (
            IngressProtocol::Messages,
            json!({"id":"r","type":"message","role":"assistant","model":"physical","content":[{"type":"tool_use","id":"native-call","name":"lookup","input":{"q":"test"}}],"stop_reason":"tool_use","stop_sequence":null,"usage":{"input_tokens":4,"output_tokens":2}}),
        ),
    ] {
        let candidate = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            upstream,
            "physical",
            fixed_reasoning("fixed"),
        );
        let profile = ClientProtocolProfile::for_candidate(&candidate).unwrap();
        let mut decoder = NativeResponseDecoder::new_for_observation(
            &candidate,
            200,
            false,
            super::super::test_tool_projection(),
            None,
        )
        .unwrap();
        decoder
            .feed(&serde_json::to_vec(&document).unwrap(), true)
            .unwrap();
        let mut renderer = IncrementalClientSseRenderer::new(profile.clone(), "alias").unwrap();
        let mut wire = Vec::new();
        for event in decoder.take_events() {
            wire.extend(renderer.push(&event).unwrap());
            if matches!(event.event, ModelEvent::ToolCallFinished { .. }) {
                let mut duplicate = event.clone();
                duplicate.sequence += 1;
                assert!(renderer.clone().push(&duplicate).is_err());
            }
        }
        assert_tool_completed(&wire);
        assert_eq!(renderer.buffered_semantic_bytes(), 0);
        let decoded = decoder.finish().unwrap();
        let RenderedClientResponse::Sse { events, .. } =
            ClientResponseRenderer::render_stream_with_profile(
                &profile,
                "alias",
                &decoded.response,
            )
            .unwrap()
        else {
            panic!("expected SSE")
        };
        assert_tool_completed(&events);
    }
}

fn assert_tool_completed(events: &[RenderedSseEvent]) {
    let added = events
        .iter()
        .find(|event| event.event.as_deref() == Some("response.output_item.added"))
        .unwrap();
    let done: Vec<_> = events
        .iter()
        .filter(|event| event.event.as_deref() == Some("response.output_item.done"))
        .collect();
    assert_eq!(done.len(), 1);
    let item = &done[0].data["item"];
    assert_eq!(item["id"], added.data["item"]["id"]);
    assert_eq!(item["call_id"], added.data["item"]["call_id"]);
    assert_eq!(item["name"], "lookup");
    assert_eq!(item["status"], "completed");
    assert_eq!(
        serde_json::from_str::<Value>(item["arguments"].as_str().unwrap()).unwrap(),
        json!({"q":"test"})
    );
    let arguments_done = events
        .iter()
        .position(|event| event.event.as_deref() == Some("response.function_call_arguments.done"))
        .unwrap();
    assert_eq!(events[arguments_done + 1], *done[0]);
}
