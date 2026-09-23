use hiroute_gateway::server::core_runtime::adapters::{
    ClientResponseRenderer, IncrementalClientSseRenderer, IngressRequestBindings,
    NativeResponseDecoder, RenderedClientResponse, RenderedSseEvent, decode_ingress_request,
    decode_ingress_request_with_bindings, project_candidate_request,
};
use hiroute_gateway::server::core_runtime::model_ir::{
    FinishReason, ModelEvent, ModelIrError, ModelStreamEventV1, OpaqueProviderState, ResponseBlock,
    ResponseBlockKind,
};
use hiroute_gateway::server::core_runtime::profiles::{
    CandidateProtocolProfile, ClientProtocolProfile, Fidelity, NativeReasoningFieldAssignment,
    NativeReasoningRender, NativeReasoningValue, ReasoningAccounting, ReasoningControlKind,
    ReasoningProfileCapability, StateAffinity, StreamingRefusalSemantics, fixed_reasoning,
};
use hiroute_gateway::server::request_plan::IngressProtocol;
use serde_json::{Value, json};

use super::support::{
    PROTOCOLS, decode_fragmented, decode_fragmented_for_profile, decoder_profile,
    ledger_from_rendered, native_nonstream, native_stream,
};

#[test]
fn protocol_reviewer_pdf_is_rejected_by_the_exact_path_before_connect() {
    let request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({
            "model":"agent/research",
            "input":[{"type":"message","role":"user","content":[{
                "type":"input_image","image_url":"data:application/pdf;base64,AA=="
            }]}]
        }),
    )
    .unwrap();
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    assert_eq!(
        project_candidate_request(&request, &profile)
            .unwrap_err()
            .code(),
        "PROTOCOL_CAPABILITY_UNSUPPORTED"
    );
}

#[test]
fn protocol_reviewer_path_identity_and_response_discriminants_fail_closed() {
    let request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({
            "model":"agent/research","input":"hello",
            "tools":[{"type":"function","name":"lookup","parameters":{"type":"object"}}],
            "tool_choice":"auto"
        }),
    )
    .unwrap();
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    for invalid in [
        {
            let mut value = profile.clone();
            value.ingress_protocol = IngressProtocol::Messages;
            value
        },
        {
            let mut value = profile.clone();
            value.connector.endpoint_id.clear();
            value
        },
        {
            let mut value = profile.clone();
            value.connector.entitlement_id.clear();
            value
        },
        {
            let mut value = profile.clone();
            value.capability.response.refusal = Fidelity::Unknown;
            value
        },
        {
            let mut value = profile.clone();
            value.capability.response.logical_tool_id_mapping = Fidelity::Unknown;
            value
        },
    ] {
        assert!(
            project_candidate_request(&request, &invalid).is_err(),
            "invalid path unexpectedly passed: {}",
            invalid.path_id
        );
    }
}

#[test]
fn protocol_reviewer_incremental_renderer_is_a_public_bounded_seam() {
    let profile = ClientProtocolProfile::exact_portable(IngressProtocol::Responses);
    let renderer = IncrementalClientSseRenderer::new(profile, "agent/research").unwrap();
    assert_eq!(renderer.buffered_semantic_bytes(), 0);
}

#[test]
fn protocol_reviewer_incremental_sse_is_lossless_for_every_native_client_pair() {
    for source in PROTOCOLS {
        let native = native_stream(source);
        let decoded = decode_fragmented(source, true, &native, &[1, 3, 2, 7]);
        for client in PROTOCOLS {
            let mut renderer = IncrementalClientSseRenderer::new(
                ClientProtocolProfile::exact_portable(client),
                "agent/research",
            )
            .unwrap();
            let mut bytes = Vec::new();
            let mut emitted_before_terminal = false;
            for event in &decoded.events {
                let rendered = renderer.push(event).unwrap();
                if !matches!(
                    event.event,
                    hiroute_gateway::server::core_runtime::model_ir::ModelEvent::ResponseCompleted { .. }
                ) && event.is_semantic_output()
                    && !rendered.is_empty()
                {
                    emitted_before_terminal = true;
                }
                for event in rendered {
                    bytes.extend_from_slice(&event.wire_bytes().unwrap());
                }
                assert_eq!(renderer.buffered_semantic_bytes(), 0);
            }
            assert!(emitted_before_terminal, "{source:?} -> {client:?}");
            let projected = decode_fragmented(client, true, &bytes, &[2, 1, 5, 3]);
            assert_eq!(
                projected.response.semantic_ledger(),
                decoded.response.semantic_ledger(),
                "{source:?} -> {client:?}"
            );
        }
    }
}

#[test]
fn protocol_reviewer_pre_output_sse_errors_are_incremental_under_fragmentation() {
    for source in PROTOCOLS {
        let event = RenderedSseEvent {
            event: (source != IngressProtocol::ChatCompletions).then(|| "error".into()),
            data: json!({
                "type":"error",
                "error":{"type":"rate_limit","message":"slow"}
            }),
        };
        let decoded = decode_fragmented(source, true, &event.wire_bytes().unwrap(), &[1, 2, 3]);
        assert_eq!(
            decoded.response.error.as_ref().unwrap().code.as_deref(),
            Some("rate_limit")
        );
        for client in PROTOCOLS {
            let mut renderer = IncrementalClientSseRenderer::new(
                ClientProtocolProfile::exact_portable(client),
                "agent/research",
            )
            .unwrap();
            let wire = renderer
                .push(&decoded.events[0])
                .unwrap()
                .into_iter()
                .flat_map(|event| event.wire_bytes().unwrap())
                .collect::<Vec<_>>();
            let projected = decode_fragmented(client, true, &wire, &[2, 1]);
            assert_eq!(projected.response.error, decoded.response.error);
        }
    }
}

#[test]
fn protocol_reviewer_reasoning_catalog_renders_fixed_toggle_discrete_and_budget_exactly() {
    let responses_request = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({"model":"agent/research","input":"hello"}),
    )
    .unwrap();
    let fixed = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "fixed-model",
        fixed_reasoning("fixed"),
    );
    let fixed_body = project_candidate_request(&responses_request, &fixed)
        .unwrap()
        .body;
    assert!(fixed_body.get("reasoning").is_none());

    let chat_request = decode_ingress_request(
        IngressProtocol::ChatCompletions,
        &json!({"model":"agent/research","messages":[{"role":"user","content":"hello"}]}),
    )
    .unwrap();
    let toggle = exact_reasoning(
        "nested-toggle",
        ReasoningControlKind::Toggle,
        IngressProtocol::ChatCompletions,
        vec![bool_field(&["thinking", "enabled"], true)],
    );
    let toggle_profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::ChatCompletions,
        IngressProtocol::ChatCompletions,
        "toggle-model",
        toggle,
    );
    assert_eq!(
        project_candidate_request(&chat_request, &toggle_profile)
            .unwrap()
            .body["thinking"],
        json!({"enabled":true})
    );

    for (name, fields, expected) in [
        (
            "qwen",
            vec![
                bool_field(&["enable_thinking"], true),
                string_field(&["reasoning_effort"], "high"),
            ],
            json!({"enable_thinking":true,"reasoning_effort":"high"}),
        ),
        (
            "deepseek",
            vec![
                string_field(&["thinking", "type"], "enabled"),
                string_field(&["reasoning_effort"], "high"),
            ],
            json!({"thinking":{"type":"enabled"},"reasoning_effort":"high"}),
        ),
    ] {
        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::ChatCompletions,
            IngressProtocol::ChatCompletions,
            format!("{name}-model"),
            exact_reasoning(
                name,
                ReasoningControlKind::Discrete,
                IngressProtocol::ChatCompletions,
                fields,
            ),
        );
        let body = project_candidate_request(&chat_request, &profile)
            .unwrap()
            .body;
        for (key, value) in expected.as_object().unwrap() {
            assert_eq!(&body[key], value);
        }
    }

    let messages_request = decode_ingress_request(
        IngressProtocol::Messages,
        &json!({"model":"agent/research","max_tokens":1,"messages":[{"role":"user","content":"hello"}]}),
    )
    .unwrap();
    let budget = ReasoningProfileCapability {
        profile_id: "budget-1024".into(),
        control_kind: ReasoningControlKind::Budget,
        render: NativeReasoningRender::ExactBudget {
            protocol: IngressProtocol::Messages,
            fields: vec![
                string_field(&["thinking", "type"], "enabled"),
                u64_field(&["thinking", "budget_tokens"], 1024),
            ],
            budget_path: vec!["thinking".into(), "budget_tokens".into()],
            selected_tokens: 1024,
            min_tokens: 512,
            max_tokens: 4096,
            step_tokens: 512,
        },
        accounting: ReasoningAccounting::WithinOutputCap,
        additional_reservation_tokens: 0,
    };
    let budget_profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Messages,
        IngressProtocol::Messages,
        "budget-model",
        budget,
    );
    assert_eq!(
        project_candidate_request(&messages_request, &budget_profile)
            .unwrap()
            .body["thinking"],
        json!({"type":"enabled","budget_tokens":1024})
    );
    let mut invalid_budget = budget_profile;
    if let NativeReasoningRender::ExactBudget {
        selected_tokens, ..
    } = &mut invalid_budget.capability.reasoning_profiles[0].render
    {
        *selected_tokens = 1025;
    }
    assert_eq!(
        project_candidate_request(&messages_request, &invalid_budget)
            .unwrap_err()
            .code(),
        "REASONING_PROFILE_MISMATCH"
    );
}

#[test]
fn protocol_reviewer_exact_owner_is_required_for_provider_state_and_tool_continuation() {
    let mut profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    profile.capability.request.provider_state = Fidelity::Exact;
    profile.capability.request.state_affinity = StateAffinity::ExactOwner;
    let body = json!({
        "model":"agent/research",
        "input":"continue",
        "conversation":"conv_previous"
    });
    assert_eq!(
        decode_ingress_request(IngressProtocol::Responses, &body).unwrap_err(),
        ModelIrError::ProviderStateOwnershipRequired
    );
    let owner = profile.exact_provider_path().unwrap();
    let request = decode_ingress_request_with_bindings(
        IngressProtocol::Responses,
        &body,
        &IngressRequestBindings {
            provider_state_owner: Some(owner.clone()),
        },
    )
    .unwrap();
    assert_eq!(
        project_candidate_request(&request, &profile).unwrap().body["conversation"],
        "conv_previous"
    );
    let mut other = profile.clone();
    other.connector.endpoint_id = "other-endpoint".into();
    assert_eq!(
        project_candidate_request(&request, &other)
            .unwrap_err()
            .code(),
        "PROTOCOL_SEMANTICS_UNSUPPORTED"
    );
}

#[test]
fn protocol_reviewer_provider_state_owner_and_block_survive_sse_fragmentation() {
    let nonstream = serde_json::to_vec(&json!({
        "id":"state","object":"response","created_at":1,"status":"completed",
        "error":null,"incomplete_details":null,"model":"physical-Responses",
        "output":[{"type":"reasoning","id":"rs","summary":[{"type":"summary_text","text":"careful"}],"encrypted_content":"opaque"}],
        "usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}
    }))
    .unwrap();
    let expected = decode_fragmented(IngressProtocol::Responses, false, &nonstream, &[1, 3, 2]);
    let events = [
        RenderedSseEvent {
            event: Some("response.created".into()),
            data: json!({"type":"response.created","response":{"id":"state","model":"physical-Responses"}}),
        },
        RenderedSseEvent {
            event: Some("response.output_item.added".into()),
            data: json!({"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs","summary":[],"encrypted_content":"opaque"}}),
        },
        RenderedSseEvent {
            event: Some("response.reasoning_summary_text.delta".into()),
            data: json!({"type":"response.reasoning_summary_text.delta","output_index":0,"delta":"careful"}),
        },
        RenderedSseEvent {
            event: Some("response.output_item.done".into()),
            data: json!({"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","id":"rs","summary":[{"type":"summary_text","text":"careful"}],"encrypted_content":"opaque"}}),
        },
        RenderedSseEvent {
            event: Some("response.completed".into()),
            data: json!({"type":"response.completed","response":{"id":"state","model":"physical-Responses","status":"completed","error":null,"incomplete_details":null,"usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}}}),
        },
    ];
    let native_wire = events
        .iter()
        .flat_map(|event| event.wire_bytes().unwrap())
        .collect::<Vec<_>>();
    let streamed = decode_fragmented(IngressProtocol::Responses, true, &native_wire, &[1, 2, 5]);
    assert_eq!(
        streamed.response.semantic_ledger(),
        expected.response.semantic_ledger()
    );
    assert_eq!(streamed.response.provider_state[0].block_index, Some(0));

    let owner = streamed.response.provider_state[0].owner.clone();
    let mut renderer = IncrementalClientSseRenderer::new(
        ClientProtocolProfile::exact_owner_affine(IngressProtocol::Responses, owner),
        "agent/research",
    )
    .unwrap();
    let projected_wire = streamed
        .events
        .iter()
        .flat_map(|event| {
            renderer
                .push(event)
                .unwrap()
                .into_iter()
                .flat_map(|event| event.wire_bytes().unwrap())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        String::from_utf8_lossy(&projected_wire)
            .matches("event: response.output_item.added")
            .count(),
        1
    );
    assert_eq!(
        decode_fragmented(
            IngressProtocol::Responses,
            true,
            &projected_wire,
            &[2, 1, 4],
        )
        .response
        .semantic_ledger(),
        streamed.response.semantic_ledger()
    );
}

#[test]
fn protocol_reviewer_json_tool_result_uses_exact_native_binding_for_all_targets() {
    let canonical = decode_ingress_request(
        IngressProtocol::Responses,
        &json!({
            "model":"agent/research",
            "tools":[{
                "type":"function",
                "name":"weather",
                "parameters":{
                    "type":"object",
                    "properties":{"city":{"type":"string"}},
                    "required":["city"],
                    "additionalProperties":false
                }
            }],
            "input":[
                {"type":"function_call","call_id":"logical_weather","name":"weather","arguments":"{\"city\":\"Paris\"}"},
                {"type":"function_call_output","call_id":"logical_weather","output":{
                    "z":1,"temperature":21,"nested":{"z":2,"a":1}
                }}
            ]
        }),
    )
    .unwrap();
    for target in PROTOCOLS {
        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            target,
            format!("physical-{target:?}"),
            fixed_reasoning("fixed"),
        );
        let request = canonical.clone();
        let body = project_candidate_request(&request, &profile).unwrap().body;
        let (native_id, native_wire, output) = native_tool_result(target, &body);
        assert_eq!(native_id, "logical_weather");
        assert_eq!(
            native_wire,
            r#"{"nested":{"a":1,"z":2},"temperature":21,"z":1}"#
        );
        assert_eq!(
            output,
            json!({"z":1,"temperature":21,"nested":{"z":2,"a":1}})
        );
    }

    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::ChatCompletions,
        "physical-chat-without-declaration",
        fixed_reasoning("fixed"),
    );
    let mut missing_declaration = canonical;
    missing_declaration.tools.clear();
    missing_declaration.responses_tool_order.clear();
    assert_eq!(
        project_candidate_request(&missing_declaration, &profile)
            .unwrap_err()
            .code(),
        "CLIENT_PROTOCOL_UNREPRESENTABLE"
    );
}

#[test]
fn protocol_reviewer_messages_terminal_refusal_is_typed_before_client_emission() {
    let exact_profile = decoder_profile(IngressProtocol::Messages);
    let request = decode_ingress_request(
        IngressProtocol::Messages,
        &json!({
            "model":"agent/research","stream":true,"max_tokens":1,
            "messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]
        }),
    )
    .unwrap();
    assert!(matches!(
        exact_profile.capability.response.stream_refusal,
        StreamingRefusalSemantics::TerminalClassified
    ));
    for semantics in [
        StreamingRefusalSemantics::Unknown,
        StreamingRefusalSemantics::Unsupported,
    ] {
        let mut invalid = exact_profile.clone();
        invalid.capability.response.stream_refusal = semantics;
        assert_eq!(
            project_candidate_request(&request, &invalid)
                .unwrap_err()
                .code(),
            "PROTOCOL_CAPABILITY_UNSUPPORTED"
        );
        assert_eq!(
            NativeResponseDecoder::new(&invalid, 200, true)
                .err()
                .unwrap()
                .code(),
            "PROTOCOL_CAPABILITY_UNSUPPORTED"
        );
    }

    let native = wire(&messages_refusal_events());
    let decoded = decode_fragmented(IngressProtocol::Messages, true, &native, &[1, 2, 5, 3]);
    let canonical = decoded
        .events
        .iter()
        .filter_map(|event| match &event.event {
            ModelEvent::ContentBlockStarted {
                index, block_kind, ..
            } => Some(("start", *index, format!("{block_kind:?}"))),
            ModelEvent::RefusalDelta { index, text } => Some(("refusal", *index, text.clone())),
            ModelEvent::TextDelta { index, text } => Some(("text", *index, text.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        canonical,
        vec![
            ("start", 0, "Refusal".into()),
            ("refusal", 0, "cannot comply".into()),
        ]
    );

    for client in PROTOCOLS {
        let mut renderer = IncrementalClientSseRenderer::new(
            ClientProtocolProfile::exact_portable(client),
            "agent/research",
        )
        .unwrap();
        let projected = decoded
            .events
            .iter()
            .flat_map(|event| renderer.push(event).unwrap())
            .collect::<Vec<_>>();
        match client {
            IngressProtocol::Responses => {
                let deltas = projected
                    .iter()
                    .filter(|event| event.event.as_deref() == Some("response.refusal.delta"))
                    .map(|event| {
                        (
                            event.data["output_index"].as_u64().unwrap(),
                            event.data["delta"].as_str().unwrap(),
                        )
                    })
                    .collect::<Vec<_>>();
                assert_eq!(deltas, vec![(0, "cannot comply")]);
                assert!(
                    !projected
                        .iter()
                        .any(|event| event.event.as_deref() == Some("response.output_text.delta"))
                );
            }
            IngressProtocol::ChatCompletions => {
                let deltas = projected
                    .iter()
                    .filter_map(|event| event.data["choices"][0]["delta"]["refusal"].as_str())
                    .collect::<Vec<_>>();
                assert_eq!(deltas, vec!["cannot comply"]);
                assert!(!projected.iter().any(|event| {
                    event.data["choices"][0]["delta"]["content"]
                        .as_str()
                        .is_some()
                }));
            }
            IngressProtocol::Messages => {
                assert!(projected.iter().any(|event| {
                    event.event.as_deref() == Some("content_block_start")
                        && event.data["index"] == 0
                        && event.data["content_block"]["type"] == "text"
                }));
                assert_eq!(
                    projected
                        .iter()
                        .find(|event| event.event.as_deref() == Some("message_delta"))
                        .unwrap()
                        .data["delta"]["stop_reason"],
                    "refusal"
                );
            }
        }
    }
}

#[test]
fn protocol_reviewer_messages_signature_is_one_canonical_state_for_json_and_sse() {
    let nonstream = serde_json::to_vec(&json!({
        "id":"signature","type":"message","role":"assistant","model":"physical-Messages",
        "content":[{"type":"thinking","thinking":"careful","signature":"sigABC"}],
        "stop_reason":"end_turn","stop_sequence":null,
        "usage":{"input_tokens":2,"output_tokens":3}
    }))
    .unwrap();
    let expected = decode_fragmented(IngressProtocol::Messages, false, &nonstream, &[2, 1, 4]);
    let streamed = decode_fragmented(
        IngressProtocol::Messages,
        true,
        &wire(&messages_signature_events()),
        &[1, 3, 2, 5],
    );
    assert_eq!(
        streamed.response.semantic_ledger(),
        expected.response.semantic_ledger()
    );
    assert_eq!(streamed.response.provider_state.len(), 1);
    assert_eq!(
        streamed.response.provider_state[0].kind,
        "thinking_signature"
    );
    assert_eq!(streamed.response.provider_state[0].value, "sigABC");
    assert!(!streamed.events.iter().any(|event| matches!(
        &event.event,
        ModelEvent::ProviderState { state }
            if state.kind == "thinking_signature_delta"
    )));

    let owner = streamed.response.provider_state[0].owner.clone();
    let replayed = ClientResponseRenderer::render_nonstream_with_profile(
        &ClientProtocolProfile::exact_owner_affine(IngressProtocol::Messages, owner),
        "agent/research",
        &streamed.response,
    )
    .unwrap();
    let RenderedClientResponse::Json { body, .. } = replayed else {
        panic!("non-stream replay must be JSON")
    };
    assert_eq!(body["content"][0]["signature"], "sigABC");

    let tampered = wire(&messages_signature_on_text_events());
    let profile = decoder_profile(IngressProtocol::Messages);
    let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
    assert_eq!(
        decoder.feed(&tampered, false).unwrap_err().code(),
        "PROTOCOL_SEMANTICS_UNSUPPORTED"
    );
}

#[test]
fn protocol_reviewer_provider_state_never_advances_the_commit_boundary() {
    let profile = decoder_profile(IngressProtocol::Responses);
    let owner = profile.exact_provider_path().unwrap();
    let state = ModelStreamEventV1::new(
        0,
        ModelEvent::ProviderState {
            state: Box::new(OpaqueProviderState {
                owner,
                block_index: Some(0),
                kind: "encrypted_content".into(),
                value: json!("opaque"),
            }),
        },
    );
    let block_start = ModelStreamEventV1::new(
        1,
        ModelEvent::ContentBlockStarted {
            index: 0,
            block_kind: ResponseBlockKind::Reasoning,
            item_id: None,
            phase: None,
        },
    );
    let reasoning = ModelStreamEventV1::new(
        2,
        ModelEvent::ReasoningDelta {
            index: 0,
            text: "visible".into(),
        },
    );
    assert!(!state.is_semantic_output());
    assert!(!block_start.is_semantic_output());
    assert!(reasoning.is_semantic_output());
    assert_eq!(
        [&state, &block_start, &reasoning]
            .into_iter()
            .position(ModelStreamEventV1::is_semantic_output),
        Some(2)
    );
}

#[test]
fn protocol_reviewer_native_tool_ids_are_physical_and_refusal_is_typed_three_by_three() {
    let mut native_ids = Vec::new();
    for source in PROTOCOLS {
        let native = native_nonstream(source);
        let decoded = decode_fragmented(source, false, &native, &[2, 1, 7]);
        native_ids.push(
            decoded
                .response
                .tool_id_map
                .iter()
                .map(|binding| binding.native_id.clone())
                .collect::<Vec<_>>(),
        );
        assert!(decoded.response.blocks.iter().any(|block| matches!(
            block,
            ResponseBlock::ToolCall { logical_id, .. } if native_ids.last().unwrap().contains(logical_id)
        )));
    }
    assert_ne!(native_ids[0], native_ids[1]);
    assert_ne!(native_ids[1], native_ids[2]);

    for source in PROTOCOLS {
        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            source,
            format!("physical-{source:?}"),
            fixed_reasoning("fixed"),
        );
        let decoded =
            decode_fragmented_for_profile(&profile, false, &native_nonstream(source), &[3, 1]);
        let binding = decoded.response.tool_id_map[0].clone();
        assert_eq!(profile.exact_provider_path().unwrap(), binding.owner);
        let continuation = decode_ingress_request(
            IngressProtocol::Responses,
            &json!({
                "model":"agent/research",
                "input":[{"type":"function_call_output","call_id":binding.native_id,"output":{"ok":true}}]
            }),
        )
        .unwrap();
        let projected = project_candidate_request(&continuation, &profile).unwrap();
        assert_eq!(
            native_tool_result(source, &projected.body).0,
            binding.native_id
        );
    }

    for source in PROTOCOLS {
        let bytes = refusal_native(source);
        let decoded = decode_fragmented(source, false, &bytes, &[1, 4, 2]);
        assert!(matches!(
            decoded.response.blocks.as_slice(),
            [ResponseBlock::Refusal { text, .. }] if text == "cannot comply"
        ));
        assert_eq!(decoded.response.finish_reason, Some(FinishReason::Refusal));
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
                assert_eq!(
                    ledger_from_rendered(client, &rendered),
                    decoded.response.semantic_ledger(),
                    "{source:?} -> {client:?}"
                );
            }
            let mut incremental = IncrementalClientSseRenderer::new(
                ClientProtocolProfile::exact_portable(client),
                "agent/research",
            )
            .unwrap();
            let mut wire = Vec::new();
            for event in &decoded.events {
                for rendered in incremental.push(event).unwrap() {
                    wire.extend_from_slice(&rendered.wire_bytes().unwrap());
                }
            }
            assert_eq!(
                decode_fragmented(client, true, &wire, &[1, 2, 4])
                    .response
                    .semantic_ledger(),
                decoded.response.semantic_ledger()
            );
        }
    }
}

#[test]
fn protocol_reviewer_refusal_finish_follows_every_native_delta() {
    let mut native = Vec::new();
    for data in [
        json!({"id":"refusal","object":"chat.completion.chunk","created":1,"model":"physical","choices":[{"index":0,"delta":{"role":"assistant","refusal":"cannot "},"finish_reason":null}],"usage":null}),
        json!({"id":"refusal","object":"chat.completion.chunk","created":1,"model":"physical","choices":[{"index":0,"delta":{"refusal":"comply"},"finish_reason":null}],"usage":null}),
        json!({"id":"refusal","object":"chat.completion.chunk","created":1,"model":"physical","choices":[{"index":0,"delta":{},"finish_reason":"content_filter"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}),
        Value::String("[DONE]".into()),
    ] {
        native.extend_from_slice(&RenderedSseEvent { event: None, data }.wire_bytes().unwrap());
    }
    let decoded = decode_fragmented(IngressProtocol::ChatCompletions, true, &native, &[1, 3, 2]);
    let last_delta = decoded
        .events
        .iter()
        .rposition(|event| matches!(event.event, ModelEvent::RefusalDelta { .. }))
        .unwrap();
    let finish = decoded
        .events
        .iter()
        .position(|event| matches!(event.event, ModelEvent::FinishReason { .. }))
        .unwrap();
    assert!(last_delta < finish);
    assert!(matches!(
        decoded.response.blocks.as_slice(),
        [ResponseBlock::Refusal { text, .. }] if text == "cannot comply"
    ));
    assert_eq!(decoded.response.finish_reason, Some(FinishReason::Refusal));
}

fn exact_reasoning(
    id: &str,
    control_kind: ReasoningControlKind,
    protocol: IngressProtocol,
    fields: Vec<NativeReasoningFieldAssignment>,
) -> ReasoningProfileCapability {
    ReasoningProfileCapability {
        profile_id: id.into(),
        control_kind,
        render: NativeReasoningRender::ExactFields { protocol, fields },
        accounting: ReasoningAccounting::WithinOutputCap,
        additional_reservation_tokens: 0,
    }
}

fn bool_field(path: &[&str], value: bool) -> NativeReasoningFieldAssignment {
    field(path, NativeReasoningValue::Bool(value))
}

fn string_field(path: &[&str], value: &str) -> NativeReasoningFieldAssignment {
    field(path, NativeReasoningValue::String(value.into()))
}

fn u64_field(path: &[&str], value: u64) -> NativeReasoningFieldAssignment {
    field(path, NativeReasoningValue::U64(value))
}

fn field(path: &[&str], value: NativeReasoningValue) -> NativeReasoningFieldAssignment {
    NativeReasoningFieldAssignment {
        path: path.iter().map(|part| (*part).into()).collect(),
        value,
    }
}

fn native_tool_result(protocol: IngressProtocol, body: &Value) -> (String, String, Value) {
    let (native_id, output) = match protocol {
        IngressProtocol::Responses => {
            let output = body["input"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["type"] == "function_call_output")
                .unwrap();
            (
                output["call_id"].as_str().unwrap().into(),
                output["output"].clone(),
            )
        }
        IngressProtocol::ChatCompletions => {
            let output = body["messages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|message| message["role"] == "tool")
                .unwrap();
            (
                output["tool_call_id"].as_str().unwrap().into(),
                output["content"].clone(),
            )
        }
        IngressProtocol::Messages => {
            let output = body["messages"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|message| message["content"].as_array().unwrap())
                .find(|part| part["type"] == "tool_result")
                .unwrap();
            (
                output["tool_use_id"].as_str().unwrap().into(),
                output["content"].clone(),
            )
        }
    };
    let schema_text = output
        .as_str()
        .expect("native Tool-result schema requires string content");
    let canonical = serde_json::from_str(schema_text)
        .expect("canonical JSON Tool result must round-trip from native text");
    (native_id, schema_text.into(), canonical)
}

fn wire(events: &[RenderedSseEvent]) -> Vec<u8> {
    events
        .iter()
        .flat_map(|event| event.wire_bytes().unwrap())
        .collect()
}

fn messages_refusal_events() -> Vec<RenderedSseEvent> {
    vec![
        messages_event(
            "message_start",
            json!({"type":"message_start","message":{"id":"refusal","type":"message","role":"assistant","content":[],"model":"physical-Messages","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":0}}}),
        ),
        messages_event(
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        ),
        messages_event(
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"cannot "}}),
        ),
        messages_event(
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"comply"}}),
        ),
        messages_event(
            "content_block_stop",
            json!({"type":"content_block_stop","index":0}),
        ),
        messages_event(
            "message_delta",
            json!({"type":"message_delta","delta":{"stop_reason":"refusal","stop_sequence":null},"usage":{"output_tokens":2}}),
        ),
        messages_event("message_stop", json!({"type":"message_stop"})),
    ]
}

fn messages_signature_events() -> Vec<RenderedSseEvent> {
    vec![
        messages_event(
            "message_start",
            json!({"type":"message_start","message":{"id":"signature","type":"message","role":"assistant","content":[],"model":"physical-Messages","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2,"output_tokens":0}}}),
        ),
        messages_event(
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
        ),
        messages_event(
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"care"}}),
        ),
        messages_event(
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig"}}),
        ),
        messages_event(
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"ful"}}),
        ),
        messages_event(
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"ABC"}}),
        ),
        messages_event(
            "content_block_stop",
            json!({"type":"content_block_stop","index":0}),
        ),
        messages_event(
            "message_delta",
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":3}}),
        ),
        messages_event("message_stop", json!({"type":"message_stop"})),
    ]
}

fn messages_signature_on_text_events() -> Vec<RenderedSseEvent> {
    vec![
        messages_event(
            "message_start",
            json!({"type":"message_start","message":{"id":"tampered","type":"message","role":"assistant","content":[],"model":"physical-Messages","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":0}}}),
        ),
        messages_event(
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        ),
        messages_event(
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"forged"}}),
        ),
    ]
}

fn messages_event(event: &str, data: Value) -> RenderedSseEvent {
    RenderedSseEvent {
        event: Some(event.into()),
        data,
    }
}

fn refusal_native(protocol: IngressProtocol) -> Vec<u8> {
    let value = match protocol {
        IngressProtocol::Responses => json!({
            "id":"refusal","object":"response","created_at":1,"status":"incomplete",
            "error":null,"incomplete_details":{"reason":"content_filter"},"model":"physical",
            "output":[{"type":"message","id":"m","status":"incomplete","role":"assistant","content":[{"type":"refusal","refusal":"cannot comply"}]}],
            "usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}
        }),
        IngressProtocol::ChatCompletions => json!({
            "id":"refusal","object":"chat.completion","created":1,"model":"physical",
            "choices":[{"index":0,"message":{"role":"assistant","content":null,"refusal":"cannot comply"},"finish_reason":"content_filter","logprobs":null}],
            "usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}
        }),
        IngressProtocol::Messages => json!({
            "id":"refusal","type":"message","role":"assistant","model":"physical",
            "content":[{"type":"text","text":"cannot comply"}],"stop_reason":"refusal","stop_sequence":null,
            "usage":{"input_tokens":1,"output_tokens":2}
        }),
    };
    serde_json::to_vec(&value).unwrap()
}
