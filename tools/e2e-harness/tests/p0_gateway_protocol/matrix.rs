use std::sync::atomic::{AtomicUsize, Ordering};

use hiroute_gateway::server::core_runtime::adapters::{
    ClientResponseRenderer, NativeResponseDecoder, RenderedClientResponse, RenderedSseEvent,
    ResponseDecodeStatus, decode_ingress_request, project_candidate_request,
};
use hiroute_gateway::server::core_runtime::model_ir::{
    ContentPart, ModelEvent, ModelIrError, RequestedReasoningDisposition, ToolResultStatusV1,
};
use hiroute_gateway::server::core_runtime::profiles::{
    CandidateProtocolProfile, ClientProtocolProfile, CriticalFact, NativeProviderStateEmission,
    NativeReasoningFieldAssignment, NativeReasoningRender, NativeReasoningValue,
    ReasoningAccounting, ReasoningControlKind,
};
use hiroute_gateway::server::request_plan::IngressProtocol;
use serde_json::{Value, json};

use super::support::{
    PROTOCOLS, candidate_profile, decode_fragmented, decoder_profile, ledger_from_rendered,
    native_nonstream, native_stream, normalized_request, reasoning_profile, request_fixture,
};

#[test]
fn protocol_request_matrix_preserves_shared_content_and_exact_status_semantics() {
    let requests = PROTOCOLS
        .iter()
        .map(|protocol| {
            let request = decode_ingress_request(*protocol, &request_fixture(*protocol)).unwrap();
            assert_eq!(
                request.requested_reasoning.disposition,
                RequestedReasoningDisposition::OverriddenByAgentPlan
            );
            request
        })
        .collect::<Vec<_>>();
    for (request, expected_status) in requests.iter().zip([
        ToolResultStatusV1::Unknown,
        ToolResultStatusV1::Unknown,
        ToolResultStatusV1::Completed,
    ]) {
        let statuses = request
            .messages
            .iter()
            .flat_map(|message| &message.content)
            .filter_map(|part| match part {
                ContentPart::ToolResult { status, .. } => Some(*status),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(statuses, vec![expected_status]);
    }
    let expected = normalized_request(requests[0].clone());
    for request in &requests[1..] {
        assert_eq!(normalized_request(request.clone()), expected);
    }

    for target in PROTOCOLS {
        let projected = requests
            .iter()
            .map(|request| {
                let profile = candidate_profile(request.ingress_protocol, target);
                project_candidate_request(request, &profile).unwrap()
            })
            .collect::<Vec<_>>();
        for request in &projected[1..] {
            assert_eq!(request.bytes, projected[0].bytes);
            assert_eq!(request.context, projected[0].context);
        }
        let body = &projected[0].body;
        assert_eq!(body["model"], format!("physical-{target:?}"));
        assert_ne!(body["model"], "agent/research");
        match target {
            IngressProtocol::Responses => {
                assert_eq!(body["reasoning"], json!({"effort":"high"}));
                assert_eq!(body["max_output_tokens"], 256);
            }
            IngressProtocol::ChatCompletions => {
                assert_eq!(body["reasoning_effort"], "high");
                assert_eq!(body["max_completion_tokens"], 256);
                assert_eq!(body["stream_options"], json!({"include_usage":true}));
            }
            IngressProtocol::Messages => {
                assert_eq!(body["thinking"], json!({"type":"adaptive"}));
                assert_eq!(body["output_config"], json!({"effort":"high"}));
                assert_eq!(body["max_tokens"], 256);
                assert_eq!(body["tool_choice"]["disable_parallel_tool_use"], true);
            }
        }
    }
}

#[test]
fn protocol_context_and_capability_boundaries_fail_before_connect() {
    let mut request = decode_ingress_request(
        IngressProtocol::Responses,
        &request_fixture(IngressProtocol::Responses),
    )
    .unwrap();
    let mut profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        reasoning_profile(IngressProtocol::Responses),
    );
    let baseline = project_candidate_request(&request, &profile).unwrap();
    let input_n = baseline.context.target_serialized_input_upper_bound;
    let total_n = baseline.context.required_total;

    profile.capability.context.max_input_tokens = CriticalFact::Exact(input_n);
    assert_eq!(
        project_candidate_request(&request, &profile)
            .unwrap()
            .context
            .target_serialized_input_upper_bound,
        input_n
    );
    profile.capability.context.max_input_tokens = CriticalFact::Exact(input_n - 1);
    assert_eq!(
        project_candidate_request(&request, &profile)
            .unwrap_err()
            .code(),
        "CONTEXT_TOO_LARGE"
    );

    profile.capability.context.max_input_tokens = CriticalFact::Exact(input_n);
    profile.capability.context.max_total_tokens = CriticalFact::Exact(Some(total_n));
    project_candidate_request(&request, &profile).unwrap();
    profile.capability.context.max_total_tokens = CriticalFact::Exact(Some(total_n - 1));
    assert_eq!(
        project_candidate_request(&request, &profile)
            .unwrap_err()
            .code(),
        "CONTEXT_TOO_LARGE"
    );

    let mut max_low = request.clone();
    max_low.requested_max_output_tokens = Some(1);
    let mut max_high = request.clone();
    max_high.requested_max_output_tokens = Some(1_000_000);
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        reasoning_profile(IngressProtocol::Responses),
    );
    assert_eq!(
        project_candidate_request(&max_low, &profile).unwrap().bytes,
        project_candidate_request(&max_high, &profile)
            .unwrap()
            .bytes
    );

    let connects = AtomicUsize::new(0);
    let mut unknown_reasoning = profile.clone();
    unknown_reasoning.capability.selected_reasoning_profile_id = "missing".into();
    reject_before_connect(
        &request,
        &unknown_reasoning,
        &connects,
        "REASONING_PROFILE_MISMATCH",
    );

    let mut wrong_reasoning = profile.clone();
    wrong_reasoning.capability.reasoning_profiles[0].render = NativeReasoningRender::ExactFields {
        protocol: IngressProtocol::Messages,
        fields: vec![NativeReasoningFieldAssignment {
            path: vec!["thinking".into(), "type".into()],
            value: NativeReasoningValue::String("adaptive".into()),
        }],
    };
    reject_before_connect(
        &request,
        &wrong_reasoning,
        &connects,
        "REASONING_PROFILE_MISMATCH",
    );

    let mut unknown_auth = profile.clone();
    unknown_auth.connector.authentication = CriticalFact::Unknown;
    reject_before_connect(
        &request,
        &unknown_auth,
        &connects,
        "PROTOCOL_CAPABILITY_UNSUPPORTED",
    );

    let mut wrong_path = profile.clone();
    wrong_path.connector.request_path = "/v1/messages?unregistered=true".into();
    reject_before_connect(
        &request,
        &wrong_path,
        &connects,
        "PROTOCOL_CAPABILITY_UNSUPPORTED",
    );

    let mut may_emit_state = profile.clone();
    may_emit_state.capability.native_provider_state = NativeProviderStateEmission::Unknown;
    reject_before_connect(
        &request,
        &may_emit_state,
        &connects,
        "PROTOCOL_CAPABILITY_UNSUPPORTED",
    );

    let mut strict = request.clone();
    strict.tools[0].strict = Some(true);
    let messages = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Messages,
        "physical-messages",
        reasoning_profile(IngressProtocol::Messages),
    );
    reject_before_connect(
        &strict,
        &messages,
        &connects,
        "PROTOCOL_CAPABILITY_UNSUPPORTED",
    );

    request.stream = false;
    let mut additive = reasoning_profile(IngressProtocol::Responses);
    additive.control_kind = ReasoningControlKind::Discrete;
    additive.accounting = ReasoningAccounting::Additive;
    additive.additional_reservation_tokens = 37;
    let additive = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        additive,
    );
    assert_eq!(
        project_candidate_request(&request, &additive)
            .unwrap()
            .context
            .additional_reasoning_reservation,
        37
    );
    assert_eq!(connects.load(Ordering::SeqCst), 0);
}

#[test]
fn protocol_native_nonstream_and_sse_fragmentation_share_one_ledger() {
    let mut expected = None;
    for protocol in PROTOCOLS {
        let nonstream = native_nonstream(protocol);
        let whole = decode_fragmented(protocol, false, &nonstream, &[nonstream.len()]);
        let one_byte = decode_fragmented(protocol, false, &nonstream, &[1]);
        assert_eq!(
            whole.response.semantic_ledger(),
            one_byte.response.semantic_ledger()
        );

        let stream = native_stream(protocol);
        let prime_fragments = decode_fragmented(protocol, true, &stream, &[1, 2, 3, 5, 7, 11]);
        let whole_stream = decode_fragmented(protocol, true, &stream, &[stream.len()]);
        assert_eq!(
            whole.response.semantic_ledger(),
            prime_fragments.response.semantic_ledger()
        );
        assert_eq!(
            whole.response.semantic_ledger(),
            whole_stream.response.semantic_ledger()
        );
        let complexity = prime_fragments.sse_complexity.unwrap();
        assert!(complexity.scanned_bytes <= stream.len());
        assert!(
            prime_fragments
                .events
                .iter()
                .any(|event| { matches!(event.event, ModelEvent::ReasoningDelta { .. }) })
        );
        assert!(
            prime_fragments
                .events
                .iter()
                .any(|event| { matches!(event.event, ModelEvent::ToolArgumentsDelta { .. }) })
        );
        assert!(
            prime_fragments
                .events
                .iter()
                .any(|event| { matches!(event.event, ModelEvent::UsageUpdated { .. }) })
        );
        assert!(
            prime_fragments
                .events
                .iter()
                .any(|event| { matches!(event.event, ModelEvent::FinishReason { .. }) })
        );
        // Different providers choose different IDs. Check preservation first,
        // then compare the remaining semantics using block identity locally.
        let mut comparable = whole.response.semantic_ledger();
        for event in &mut comparable.events {
            use hiroute_gateway::server::core_runtime::model_ir::{
                ResponseBlock, SemanticLedgerEvent,
            };
            if let SemanticLedgerEvent::Block(ResponseBlock::ToolCall {
                index, logical_id, ..
            }) = event
            {
                assert!(
                    whole
                        .response
                        .tool_id_map
                        .iter()
                        .any(|entry| entry.native_id == *logical_id
                            && entry.logical_id == *logical_id)
                );
                *logical_id = format!("fixture-call-{index}");
            }
        }
        if let Some(expected) = &expected {
            assert_eq!(&comparable, expected);
        } else {
            expected = Some(comparable);
        }
    }
}

#[test]
fn protocol_three_by_three_client_renderers_preserve_ledger_and_alias() {
    for source in PROTOCOLS {
        let decoded = decode_fragmented(source, true, &native_stream(source), &[1, 4, 2, 9]);
        let expected = decoded.response.semantic_ledger();
        for client in PROTOCOLS {
            for rendered in [
                ClientResponseRenderer::render_nonstream(
                    client,
                    "agent/research",
                    &decoded.response,
                )
                .unwrap(),
                ClientResponseRenderer::render_stream(client, "agent/research", &decoded.response)
                    .unwrap(),
            ] {
                let bytes = match &rendered {
                    RenderedClientResponse::Json { bytes, .. }
                    | RenderedClientResponse::Sse { bytes, .. } => bytes,
                };
                let text = String::from_utf8_lossy(bytes);
                assert!(text.contains("agent/research"));
                assert!(!text.contains("physical-"));
                assert_eq!(ledger_from_rendered(client, &rendered), expected);
            }
        }
    }
}

#[test]
fn protocol_errors_malformed_sse_and_unknown_semantics_fail_closed() {
    for protocol in PROTOCOLS {
        let body = match protocol {
            IngressProtocol::Responses | IngressProtocol::ChatCompletions => {
                json!({"error":{"code":"rate_limit","message":"slow"}})
            }
            IngressProtocol::Messages => {
                json!({"type":"error","error":{"type":"rate_limit","message":"slow"}})
            }
        };
        let bytes = serde_json::to_vec(&body).unwrap();
        let profile = decoder_profile(protocol);
        let mut decoder = NativeResponseDecoder::new(&profile, 429, false).unwrap();
        let chunks = bytes.chunks(2).collect::<Vec<_>>();
        for (index, chunk) in chunks.iter().enumerate() {
            decoder.feed(chunk, index + 1 == chunks.len()).unwrap();
        }
        let response = decoder.finish().unwrap().response;
        assert_eq!(response.error.as_ref().unwrap().status, Some(429));
        assert_eq!(
            response.error.as_ref().unwrap().code.as_deref(),
            Some("rate_limit")
        );
    }

    let malformed = b"dataoops: {\"type\":\"response.completed\"}\n\n";
    let profile = decoder_profile(IngressProtocol::Responses);
    let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
    assert!(matches!(
        decoder.feed(malformed, true),
        Err(error) if error.code() == "PROTOCOL_SEMANTICS_UNSUPPORTED"
    ));

    let incomplete = b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"r\",\"model\":\"m\"}}\n\n";
    let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
    assert!(matches!(
        decoder.feed(incomplete, true),
        Err(error) if error == ModelIrError::MissingTerminalEvent.into()
    ));

    let request = request_fixture(IngressProtocol::Responses);
    let mut request = request.as_object().unwrap().clone();
    request.insert("temperature".into(), Value::from(0.1));
    assert!(matches!(
        decode_ingress_request(IngressProtocol::Responses, &Value::Object(request)),
        Err(ModelIrError::UnsupportedField(_))
    ));
}

#[test]
fn protocol_native_provider_state_stays_typed_or_projection_is_rejected() {
    let cases = [
        (
            IngressProtocol::Responses,
            json!({
                "id":"state_response","object":"response","created_at":1,
                "status":"completed","error":null,"incomplete_details":null,
                "model":"physical-responses","output":[{
                    "type":"reasoning","id":"rs_0",
                    "summary":[{"type":"summary_text","text":"careful"}],
                    "encrypted_content":"opaque-state"
                }],"usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5}
            }),
        ),
        (
            IngressProtocol::Messages,
            json!({
                "id":"state_message","type":"message","role":"assistant",
                "model":"physical-messages","content":[{
                    "type":"thinking","thinking":"careful","signature":"signed-state"
                }],"stop_reason":"end_turn","stop_sequence":null,
                "usage":{"input_tokens":2,"output_tokens":3}
            }),
        ),
    ];
    for (protocol, value) in cases {
        let bytes = serde_json::to_vec(&value).unwrap();
        let decoded = decode_fragmented(protocol, false, &bytes, &[1, 3, 2]);
        assert_eq!(decoded.response.provider_state.len(), 1);
        assert_eq!(
            decoded.response.provider_state[0].owner.upstream_protocol,
            protocol
        );

        assert_eq!(
            ClientResponseRenderer::render_nonstream(
                protocol,
                "agent/research",
                &decoded.response,
            )
            .unwrap_err()
            .code(),
            "CLIENT_PROTOCOL_UNREPRESENTABLE"
        );
        let client_profile = ClientProtocolProfile::exact_owner_affine(
            protocol,
            decoded.response.provider_state[0].owner.clone(),
        );
        let same = ClientResponseRenderer::render_nonstream_with_profile(
            &client_profile,
            "agent/research",
            &decoded.response,
        )
        .unwrap();
        assert_eq!(
            ledger_from_rendered(protocol, &same),
            decoded.response.semantic_ledger()
        );
        let other = if protocol == IngressProtocol::Responses {
            IngressProtocol::Messages
        } else {
            IngressProtocol::Responses
        };
        let cross_protocol = ClientResponseRenderer::render_nonstream_with_profile(
            &ClientProtocolProfile::exact_owner_affine(
                other,
                decoded.response.provider_state[0].owner.clone(),
            ),
            "agent/research",
            &decoded.response,
        );
        if protocol == IngressProtocol::Responses {
            let RenderedClientResponse::Json { body, .. } = cross_protocol.unwrap() else {
                panic!("Responses state projected to Messages must be non-stream JSON")
            };
            assert_eq!(
                body["content"],
                json!([{"type":"thinking","thinking":"careful","signature":"opaque-state"}])
            );
        } else {
            assert_eq!(
                cross_protocol.unwrap_err().code(),
                "CLIENT_PROTOCOL_UNREPRESENTABLE"
            );
        }
    }

    let chat_with_unprofiled_state = serde_json::to_vec(&json!({
        "id":"chat","object":"chat.completion","created":1,"model":"physical-chat",
        "system_fingerprint":"fp_123",
        "choices":[{"index":0,"message":{"role":"assistant","content":"done"},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
    }))
    .unwrap();
    let profile = decoder_profile(IngressProtocol::ChatCompletions);
    let mut decoder = NativeResponseDecoder::new(&profile, 200, false).unwrap();
    assert!(decoder.feed(&chat_with_unprofiled_state, true).is_err());
}

#[test]
fn protocol_sse_decoder_applies_bounded_backpressure_without_rescanning() {
    let mut bytes = Vec::new();
    for index in 0..80 {
        bytes.extend_from_slice(
            &RenderedSseEvent {
                event: None,
                data: json!({
                    "id":"chat_backpressure","object":"chat.completion.chunk",
                    "created":1,"model":"physical-chat",
                    "choices":[{"index":0,"delta":{
                        "role": (index == 0).then_some("assistant"),
                        "content":"x"
                    },"finish_reason":null}],"usage":null
                }),
            }
            .wire_bytes()
            .unwrap(),
        );
    }
    for event in [
        RenderedSseEvent {
            event: None,
            data: json!({
                "id":"chat_backpressure","object":"chat.completion.chunk",
                "created":1,"model":"physical-chat",
                "choices":[{"index":0,"delta":{},"finish_reason":"stop"}],
                "usage":{"prompt_tokens":1,"completion_tokens":80,"total_tokens":81}
            }),
        },
        RenderedSseEvent {
            event: None,
            data: Value::String("[DONE]".into()),
        },
    ] {
        bytes.extend_from_slice(&event.wire_bytes().unwrap());
    }

    let profile = decoder_profile(IngressProtocol::ChatCompletions);
    let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
    assert_eq!(
        decoder.feed(&bytes, true).unwrap(),
        ResponseDecodeStatus::NeedDrain
    );
    let mut drained = decoder.take_events().len();
    loop {
        match decoder.resume().unwrap() {
            ResponseDecodeStatus::NeedDrain => drained += decoder.take_events().len(),
            ResponseDecodeStatus::Terminal => break,
            ResponseDecodeStatus::Complete => {}
        }
    }
    let complexity = decoder.sse_complexity().unwrap();
    let response = decoder.finish().unwrap().response;
    assert!(drained >= 32);
    assert_eq!(
        response.blocks.iter().find_map(|block| match block {
            hiroute_gateway::server::core_runtime::model_ir::ResponseBlock::Text {
                text, ..
            } => Some(text.len()),
            _ => None,
        }),
        Some(80)
    );
    assert!(
        complexity.scanned_bytes <= bytes.len().saturating_mul(2),
        "bounded resume must stay linear: {complexity:?} for {} bytes",
        bytes.len()
    );
}

fn reject_before_connect(
    request: &hiroute_gateway::server::core_runtime::model_ir::ModelRequestIRV1,
    profile: &CandidateProtocolProfile,
    connects: &AtomicUsize,
    code: &str,
) {
    match project_candidate_request(request, profile) {
        Ok(_) => {
            connects.fetch_add(1, Ordering::SeqCst);
            panic!("candidate unexpectedly passed pre-connect projection")
        }
        Err(error) => assert_eq!(error.code(), code),
    }
}
