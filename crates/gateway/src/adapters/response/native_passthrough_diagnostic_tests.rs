use super::*;
use hiroute_diagnostics::event::ResponseEventKind;
use hiroute_gateway_core::runtime::body::MemoryRole;
use serde_json::{Value, json};

fn projector(protocol: IngressProtocol, streaming: bool) -> NativeResponseProjector {
    use crate::server::core_runtime::{adapters::ToolIdProjection, profiles};
    let profile = profiles::CandidateProtocolProfile::exact_portable_path(
        protocol,
        protocol,
        "physical",
        profiles::fixed_reasoning("fixed"),
    );
    NativeResponseProjector::new_for_observation(
        &profile,
        streaming,
        "alias".into(),
        None,
        ToolIdProjection::new(protocol),
    )
    .unwrap()
}

fn sse(value: &Value) -> Vec<u8> {
    format!("data: {value}\n\n").into_bytes()
}

fn item(text: &str) -> Value {
    json!({"type":"message","id":"PRIVATE_ITEM","role":"assistant","status":"completed",
        "content":[{"type":"output_text","text":text}]})
}

fn terminal(output: Value) -> Value {
    json!({"type":"response.completed","response":{
        "id":"PRIVATE_RESPONSE","model":"physical","status":"completed","output":output
    }})
}

#[test]
fn native_completion_diagnostic_keeps_first_frame_and_emits_only_after_unknown_eof() {
    let mut stream = projector(IngressProtocol::Responses, true);
    let mut bytes = b": transport comment\n\n".to_vec();
    bytes.extend(sse(
        &json!({"type":"response.output_text.delta","output_index":2,
        "content_index":1,"item_id":"PRIVATE_ITEM","delta":{"PRIVATE_BODY":"SECRET"}}),
    ));
    stream.feed(&bytes, false).unwrap();
    assert!(stream.take_completion_diagnostic().is_none());
    // Later loss of terminal status must not replace the original delta error.
    let mut done = terminal(json!([item("PRIVATE_TEXT")]));
    done["response"]["status"] = json!("PRIVATE_STATUS");
    stream.feed(&sse(&done), false).unwrap();
    assert!(stream.take_completion_diagnostic().is_none());
    let units = stream.feed(&[], true).unwrap();
    assert_eq!(
        units.last().unwrap().terminal,
        Some(NativeTerminalOutcome::Unknown)
    );
    let (protocol, evidence) = stream.take_completion_diagnostic().unwrap();
    assert_eq!(protocol, DiagnosticProtocol::OpenAiResponses);
    assert_eq!(evidence.reason, Reason::InvalidDelta);
    assert_eq!(evidence.frame_index, Some(2));
    let shape = evidence.event.unwrap();
    assert_eq!(shape.event, ResponseEventKind::TextDelta);
    assert_eq!(shape.output_index, Some(2));
    assert_eq!(shape.content_index, Some(1));
    let safe = serde_json::to_string(&evidence).unwrap();
    assert!(!safe.contains("PRIVATE") && !safe.contains("SECRET"));
    assert!(stream.take_completion_diagnostic().is_none());
}

#[test]
fn native_completion_diagnostic_identifies_snapshot_and_delta_evidence_losses() {
    let original = item("original");
    for (final_output, expected) in [
        (Value::Null, Reason::MissingOutput),
        (json!([]), Reason::MissingOutputItem),
        (
            json!([{"type":"message","id":"changed"}]),
            Reason::ItemIdentity,
        ),
        (json!([item("changed")]), Reason::ItemSnapshot),
    ] {
        let mut stream = projector(IngressProtocol::Responses, true);
        let mut bytes = sse(
            &json!({"type":"response.output_item.done", "output_index":0,
            "item":original}),
        );
        bytes.extend(sse(&terminal(final_output)));
        stream.feed(&bytes, true).unwrap();
        let (_, evidence) = stream.take_completion_diagnostic().unwrap();
        assert_eq!(evidence.reason, expected);
        assert_eq!(evidence.frame_index, Some(2));
        assert_eq!(evidence.event.unwrap().event, ResponseEventKind::Terminal);
    }
    for (final_item, expected) in [
        (item("different"), Reason::DeltaFinalText),
        (
            json!({"type":"message","id":"wrong","content":[{"text":"observed"}]}),
            Reason::DeltaFinalIdentity,
        ),
        (
            json!({"type":"reasoning","id":"PRIVATE_ITEM","summary":[]}),
            Reason::DeltaFinalType,
        ),
    ] {
        let mut stream = projector(IngressProtocol::Responses, true);
        let mut bytes = sse(
            &json!({"type":"response.output_text.delta", "output_index":0,
            "content_index":0,"item_id":"PRIVATE_ITEM","delta":"observed"}),
        );
        bytes.extend(sse(&terminal(json!([final_item]))));
        stream.feed(&bytes, true).unwrap();
        let (_, evidence) = stream.take_completion_diagnostic().unwrap();
        assert_eq!(evidence.reason, expected);
        assert_eq!(evidence.event.unwrap().output_index, Some(0));
    }
}

#[test]
fn native_completion_diagnostic_records_budget_refusal_without_another_allocation() {
    let mut stream = projector(IngressProtocol::Responses, true);
    let budget = stream.state.budget.clone();
    let before = budget.snapshot().unwrap();
    let pressure = budget
        .reserve(MemoryRole::TransportInflight, 1024 * 1024 - before.live)
        .unwrap();
    stream.state.begin_native_diagnostic_frame();
    let value = json!({"type":"response.output_item.done","output_index":0,"item":item("answer")});
    stream.state.completion_diagnostic.shape = Some(super::super::super::diagnostic_shape::shape(
        None,
        Some(&value),
    ));
    stream
        .state
        .observe_response_item(0, value["item"].as_object().unwrap(), true);
    let after = budget.snapshot().unwrap();
    assert_eq!(after.rejected, before.rejected + 1);
    assert_eq!(after.live, 1024 * 1024);
    drop(pressure);
    stream
        .feed(&sse(&terminal(json!([item("answer")]))), true)
        .unwrap();
    let (_, evidence) = stream.take_completion_diagnostic().unwrap();
    assert_eq!(evidence.reason, Reason::EvidenceBudget);
    assert_eq!(evidence.frame_index, Some(1));
    assert_eq!(
        evidence.event.unwrap().event,
        ResponseEventKind::OutputItemDone
    );
}

#[test]
fn native_completion_diagnostic_separates_transport_result_from_capture_and_generic_protocols() {
    let mut stream = projector(IngressProtocol::Responses, true);
    let mut bytes = sse(&json!({"type":"response.vendor_extension","PRIVATE":"SECRET"}));
    bytes.extend(sse(&terminal(json!([item("answer")]))));
    let units = stream.feed(&bytes, true).unwrap();
    assert_eq!(
        units.last().unwrap().terminal,
        Some(NativeTerminalOutcome::Complete)
    );
    assert!(stream.take_completion_diagnostic().is_none());

    for (event, status, outcome) in [
        ("response.failed", "failed", NativeTerminalOutcome::Failed),
        (
            "response.incomplete",
            "incomplete",
            NativeTerminalOutcome::Incomplete,
        ),
    ] {
        let mut stream = projector(IngressProtocol::Responses, true);
        let mut bytes = sse(&json!({"type":"response.output_text.delta","delta":false}));
        let mut done = terminal(json!([]));
        done["type"] = json!(event);
        done["response"]["status"] = json!(status);
        bytes.extend(sse(&done));
        let units = stream.feed(&bytes, true).unwrap();
        assert_eq!(units.last().unwrap().terminal, Some(outcome));
        assert!(stream.take_completion_diagnostic().is_none());
    }

    let mut stream = projector(IngressProtocol::Responses, true);
    let mut bytes = sse(&terminal(json!([item("answer")])));
    bytes.extend(sse(&json!({"type":"response.vendor_extension"})));
    stream.feed(&bytes, true).unwrap();
    let (_, evidence) = stream.take_completion_diagnostic().unwrap();
    assert_eq!(evidence.reason, Reason::PostTerminal);
    assert_eq!(evidence.frame_index, Some(2));

    let mut chat = projector(IngressProtocol::ChatCompletions, false);
    chat.feed(
        &serde_json::to_vec(&json!({"id":"PRIVATE","model":"physical","choices":[{
            "index":0,"message":{"role":"assistant","content":"answer"},"finish_reason":"PRIVATE"
        }]}))
        .unwrap(),
        true,
    )
    .unwrap();
    let (_, evidence) = chat.take_completion_diagnostic().unwrap();
    assert_eq!(evidence.reason, Reason::Generic);
    assert!(evidence.frame_index.is_none() && evidence.event.is_none());

    let mut responses = projector(IngressProtocol::Responses, false);
    let mut body = terminal(json!([item("answer")]))["response"].clone();
    body["status"] = json!("PRIVATE_STATUS");
    responses
        .feed(&serde_json::to_vec(&body).unwrap(), true)
        .unwrap();
    let (_, evidence) = responses.take_completion_diagnostic().unwrap();
    assert_eq!(evidence.reason, Reason::TerminalStatus);
    assert!(evidence.frame_index.is_none());
    assert!(
        !serde_json::to_string(&evidence)
            .unwrap()
            .contains("PRIVATE")
    );
}
