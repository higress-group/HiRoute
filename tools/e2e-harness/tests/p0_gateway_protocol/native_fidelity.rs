use hiroute_gateway::server::core_runtime::adapters::{
    ClientResponseRenderer, IncrementalClientSseRenderer, NativeResponseDecoder,
    ProtocolAdapterError, RenderedClientResponse, ResponseDecodeStatus, decode_ingress_request,
    project_candidate_request,
};
use hiroute_gateway::server::core_runtime::model_ir::{ModelEvent, ModelIrError};
use hiroute_gateway::server::core_runtime::profiles::{
    CandidateProtocolProfile, ClientProtocolProfile, fixed_reasoning,
};
use hiroute_gateway::server::request_plan::IngressProtocol;
use serde_json::{Value, json};

#[test]
fn native_codex_request_has_only_owned_same_protocol_rewrites() {
    let native = json!({
        "model": "served-alias",
        "stream": true,
        "instructions": "isolated Worker",
        "input": [
            {"type":"message","id":"developer-message","role":"developer","content":[{"type":"input_text","text":"bounds"}]},
            {"type":"message","id":"user-message","role":"user","content":[{"type":"input_text","text":"goal"}]},
            {"type":"function_call","id":"call-item","call_id":"native-call","namespace":"tools","name":"lookup","arguments":"{\"q\":\"test\"}"},
            {"type":"function_call_output","id":"result-item","call_id":"native-call","output":"done"}
        ],
        "store": false,
        "include": ["reasoning.encrypted_content"],
        "prompt_cache_key": "session",
        "client_metadata": {"session_id":"session"},
        "reasoning": {"effort":"low","summary":"auto"},
        "max_output_tokens": 2048,
        "parallel_tool_calls": false,
        "tool_choice": "auto",
        "tools": [{
            "type":"namespace",
            "name":"tools",
            "description":"native tools",
            "tools":[{"type":"function","name":"lookup","description":"look up","parameters":{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]},"strict":true}]
        }]
    });
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical-model",
        fixed_reasoning("fixed"),
    );
    let request = decode_ingress_request(IngressProtocol::Responses, &native).unwrap();
    let projected = project_candidate_request(&request, &profile).unwrap().body;
    let mut expected = native;
    expected["model"] = json!("physical-model");
    expected["reasoning"] = json!({"summary":"auto"});
    expected["max_output_tokens"] = json!(256);

    assert_eq!(projected, expected);
}

#[test]
fn responses_phase_and_full_output_survive_batch_and_stream_projection() {
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical-model",
        fixed_reasoning("fixed"),
    );
    let native_item = json!({
        "type": "message",
        "id": "native-message",
        "status": "completed",
        "role": "assistant",
        "phase": "final_answer",
        "content": [{"type":"output_text","text":"answer","annotations":[]}]
    });
    let document = json!({
        "id":"native-response",
        "model":"physical-model",
        "status":"completed",
        "output":[native_item.clone()],
        "usage":{"input_tokens":3,"output_tokens":1,"total_tokens":4}
    });
    let mut decoder = NativeResponseDecoder::new(&profile, 200, false).unwrap();
    assert_eq!(
        decoder
            .feed(&serde_json::to_vec(&document).unwrap(), true)
            .unwrap(),
        ResponseDecodeStatus::Terminal
    );
    let canonical_events = decoder.take_events();
    let decoded = decoder.finish().unwrap();
    let client = ClientProtocolProfile::for_candidate(&profile).unwrap();

    let mut incremental =
        IncrementalClientSseRenderer::new(client.clone(), "served-alias").unwrap();
    let projected_events = canonical_events
        .iter()
        .flat_map(|event| incremental.push(event).unwrap())
        .collect::<Vec<_>>();
    assert_completed_output(&projected_events, &native_item);

    let RenderedClientResponse::Sse { events, .. } =
        ClientResponseRenderer::render_stream_with_profile(
            &client,
            "served-alias",
            &decoded.response,
        )
        .unwrap()
    else {
        panic!("expected Responses SSE")
    };
    assert_completed_output(&events, &native_item);

    let RenderedClientResponse::Json { body, .. } =
        ClientResponseRenderer::render_nonstream_with_profile(
            &client,
            "served-alias",
            &decoded.response,
        )
        .unwrap()
    else {
        panic!("expected Responses JSON")
    };
    assert_eq!(body["output"], json!([native_item]));
}

fn assert_completed_output(
    events: &[hiroute_gateway::server::core_runtime::adapters::RenderedSseEvent],
    expected: &Value,
) {
    let completed = events
        .iter()
        .find(|event| event.event.as_deref() == Some("response.completed"))
        .unwrap();
    assert_eq!(completed.data["response"]["output"], json!([expected]));
}

#[test]
fn responses_conflicting_terminal_snapshot_fails_before_completion() {
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
    let error = decoder
        .feed(
            br#"event: response.created
data: {"type":"response.created","response":{"id":"r","model":"physical"}}

event: response.output_item.added
data: {"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"m","status":"in_progress","role":"assistant","content":[],"phase":"final_answer"}}

event: response.output_text.delta
data: {"type":"response.output_text.delta","item_id":"m","output_index":0,"content_index":0,"delta":"accepted"}

event: response.completed
data: {"type":"response.completed","response":{"id":"r","model":"physical","status":"completed","output":[{"type":"message","id":"m","status":"completed","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"conflict","annotations":[]}]}]}}

"#,
            true,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        ProtocolAdapterError::ModelIr(ModelIrError::InvalidResponseLifecycle(_))
    ));
    assert!(
        !decoder
            .take_events()
            .iter()
            .any(|event| matches!(event.event, ModelEvent::ResponseCompleted { .. }))
    );
}

#[test]
fn responses_explicit_empty_terminal_snapshots_reject_accepted_content() {
    struct Case {
        stream: &'static [u8],
        accepted_content: fn(&ModelEvent) -> bool,
    }

    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let cases = [
        Case {
            stream: br#"event: response.created
data: {"type":"response.created","response":{"id":"r","model":"physical"}}

event: response.output_item.added
data: {"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"m","status":"in_progress","role":"assistant","content":[],"phase":"final_answer"}}

event: response.output_text.delta
data: {"type":"response.output_text.delta","item_id":"m","output_index":0,"content_index":0,"delta":"accepted"}

event: response.output_text.done
data: {"type":"response.output_text.done","item_id":"m","output_index":0,"content_index":0,"text":"accepted"}

event: response.completed
data: {"type":"response.completed","response":{"id":"r","model":"physical","status":"completed","output":[{"type":"message","id":"m","status":"completed","role":"assistant","phase":"final_answer","content":[]}]}}

"#,
            accepted_content: |event| {
                matches!(event, ModelEvent::TextDelta { text, .. } if text == "accepted")
            },
        },
        Case {
            stream: br#"event: response.created
data: {"type":"response.created","response":{"id":"r","model":"physical"}}

event: response.output_item.added
data: {"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"reasoning","summary":[],"content":[]}}

event: response.reasoning_summary_text.delta
data: {"type":"response.reasoning_summary_text.delta","item_id":"reasoning","output_index":0,"summary_index":0,"delta":"accepted"}

event: response.reasoning_summary_text.done
data: {"type":"response.reasoning_summary_text.done","item_id":"reasoning","output_index":0,"summary_index":0,"text":"accepted"}

event: response.completed
data: {"type":"response.completed","response":{"id":"r","model":"physical","status":"completed","output":[{"type":"reasoning","id":"reasoning","status":"completed","summary":[],"content":[]}]}}

"#,
            accepted_content: |event| {
                matches!(event, ModelEvent::ReasoningDelta { text, .. } if text == "accepted")
            },
        },
    ];

    for case in cases {
        let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
        let error = decoder.feed(case.stream, true).unwrap_err();
        assert!(matches!(
            error,
            ProtocolAdapterError::ModelIr(ModelIrError::InvalidResponseLifecycle(_))
        ));
        let events = decoder.take_events();
        assert!(
            events
                .iter()
                .any(|event| (case.accepted_content)(&event.event))
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event.event, ModelEvent::ResponseCompleted { .. }))
        );
    }
}

#[test]
fn responses_matching_terminal_snapshot_does_not_repeat_blocks_or_usage() {
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
    assert_eq!(
        decoder
            .feed(
                br#"event: response.created
data: {"type":"response.created","response":{"id":"r","model":"physical"}}

event: response.output_item.added
data: {"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"reasoning","summary":[],"content":[]}}

event: response.reasoning_summary_text.delta
data: {"type":"response.reasoning_summary_text.delta","item_id":"reasoning","output_index":0,"summary_index":0,"delta":"think"}

event: response.reasoning_summary_text.done
data: {"type":"response.reasoning_summary_text.done","item_id":"reasoning","output_index":0,"summary_index":0,"text":"think"}

event: response.output_item.added
data: {"type":"response.output_item.added","output_index":1,"item":{"type":"message","id":"m","status":"in_progress","role":"assistant","content":[],"phase":"final_answer"}}

event: response.output_text.delta
data: {"type":"response.output_text.delta","item_id":"m","output_index":1,"content_index":0,"delta":"answer"}

event: response.output_text.done
data: {"type":"response.output_text.done","item_id":"m","output_index":1,"content_index":0,"text":"answer"}

event: response.completed
data: {"type":"response.completed","response":{"id":"r","model":"physical","status":"completed","output":[{"type":"reasoning","id":"reasoning","status":"completed","summary":[{"type":"summary_text","text":"think"}],"content":[]},{"type":"message","id":"m","status":"completed","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"answer","annotations":[]}]}],"usage":{"input_tokens":5,"output_tokens":2,"total_tokens":7,"output_tokens_details":{"reasoning_tokens":1}}}}

"#,
                true,
            )
            .unwrap(),
        ResponseDecodeStatus::Terminal
    );
    let events = decoder.take_events();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.event, ModelEvent::ContentBlockStarted { .. }))
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.event, ModelEvent::TextDelta { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.event, ModelEvent::ReasoningDelta { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.event, ModelEvent::TextFinished { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.event, ModelEvent::ReasoningFinished { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.event, ModelEvent::UsageUpdated { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.event, ModelEvent::ResponseCompleted { .. }))
            .count(),
        1
    );
    decoder.finish().unwrap();
}
