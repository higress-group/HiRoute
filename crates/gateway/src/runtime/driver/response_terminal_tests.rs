use super::*;
use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};
use hiroute_gateway_core::runtime::body::{BodyDirection, BodyPlan, BodyPlanExecutor, BudgetTree};
use serde_json::json;

#[test]
fn accepted_codex_terminal_event_crosses_transport_frames_and_keeps_eos() {
    let tree = BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024).unwrap();
    let budget = tree.stream(8 * 1024 * 1024).unwrap();
    let plan = BodyPlan::PassThrough {
        max_chunk_bytes: 64 * 1024,
    };
    let prefix =
        ChargedBodyQueue::new(&budget, MemoryRole::ResponsePrefix, &plan, 256 * 1024, 64).unwrap();
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let projector = adapters::NativeResponseProjector::new_for_attempt(
        &profile,
        true,
        "alias".into(),
        None,
        budget.clone(),
    )
    .unwrap();
    let mut readiness = ProductionReadiness {
        response_status: StatusCode::OK,
        content_type: "text/event-stream",
        prefix,
        terminal_body: None,
        decoder: None,
        renderer: None,
        projector: Some(Box::new(projector)),
        _decoder_budget: None,
        _chat_tool_projection_budget: None,
        budget: budget.clone(),
        streaming: true,
        prefix_eos_pending: false,
        prefix_terminal_chunks: None,
        semantic_terminal: None,
    };
    // CPA repeats the request's instructions and tool schema in the
    // terminal response. This legal SSE event is larger than one 64 KiB
    // accepted-response transport frame.
    let terminal = json!({
        "type":"response.completed",
        "response":{
            "id":"response","model":"physical","status":"completed",
            "instructions":"x".repeat(100_000),
            "output":[{"type":"message","id":"message","role":"assistant",
                "status":"completed","content":[{"type":"output_text","text":"DONE"}]}]
        }
    });
    let wire = format!("event: response.completed\ndata: {terminal}\n\ndata: [DONE]\n\n");
    let input =
        ChargedBytes::copy_from_opaque(&budget, MemoryRole::ResponsePrefix, wire.as_bytes())
            .unwrap();
    let first = encode_accepted_event(
        &mut readiness,
        ProviderAcceptedEvent::Raw(PrecommitEvent::Body(input)),
    )
    .unwrap();
    let mut accepted =
        BodyPlanExecutor::new(BodyDirection::AcceptedResponse, plan, 1024 * 1024).unwrap();
    let mut reconstructed = Vec::new();
    let mut frames = 0;
    let mut eos = 0;
    let mut next = first;
    let mut source_end_sent = false;
    loop {
        let frame = match next.take() {
            Some(frame) => frame,
            None => match take_accepted_prefix(&mut readiness) {
                Some(event) => encode_accepted_event(&mut readiness, event)
                    .unwrap()
                    .unwrap(),
                None if !source_end_sent => {
                    source_end_sent = true;
                    encode_accepted_event(
                        &mut readiness,
                        ProviderAcceptedEvent::Raw(PrecommitEvent::EndStream),
                    )
                    .unwrap()
                    .unwrap()
                }
                None => break,
            },
        };
        if let Some(output) = frame.output {
            let bytes = output.bytes.bytes();
            accepted.admit_chunk(bytes.len()).unwrap();
            reconstructed.extend_from_slice(bytes);
            frames += 1;
        }
        eos += usize::from(frame.end_stream);
        if frame.end_stream {
            assert_eq!(accepted.finish().unwrap(), reconstructed.len());
        }
    }
    assert!(frames >= 2);
    assert_eq!(eos, 1);
    assert!(reconstructed.starts_with(b"event: response.completed\n"));
    assert!(reconstructed.ends_with(b"data: [DONE]\n\n"));
    assert!(
        std::str::from_utf8(&reconstructed)
            .unwrap()
            .contains("\"model\":\"alias\"")
    );
    assert_eq!(
        readiness.semantic_terminal,
        Some(SemanticTerminalOutcome::Complete)
    );
}
