use hiroute_gateway::server::core_runtime::adapters::{
    DecodedNativeResponse, NativeResponseDecoder, RenderedClientResponse, RenderedSseEvent,
    ResponseDecodeStatus,
};
use hiroute_gateway::server::core_runtime::model_ir::{
    ContentPart, ModelRequestIRV1, RequestedReasoningControl, SemanticLedgerV1, ToolResultStatusV1,
};
use hiroute_gateway::server::core_runtime::profiles::{
    CandidateProtocolProfile, Fidelity, NativeProviderStateEmission,
    NativeReasoningFieldAssignment, NativeReasoningRender, NativeReasoningValue,
    ReasoningAccounting, ReasoningControlKind, ReasoningProfileCapability,
};
use hiroute_gateway::server::request_plan::IngressProtocol;
use serde_json::{Value, json};

pub const PROTOCOLS: [IngressProtocol; 3] = [
    IngressProtocol::Responses,
    IngressProtocol::ChatCompletions,
    IngressProtocol::Messages,
];

pub fn reasoning_profile(protocol: IngressProtocol) -> ReasoningProfileCapability {
    ReasoningProfileCapability {
        profile_id: format!("agent-plan-high-{protocol:?}"),
        control_kind: ReasoningControlKind::Discrete,
        render: NativeReasoningRender::ExactFields {
            protocol,
            fields: match protocol {
                IngressProtocol::Responses => vec![field(&["reasoning", "effort"], "high")],
                IngressProtocol::ChatCompletions => vec![field(&["reasoning_effort"], "high")],
                IngressProtocol::Messages => vec![
                    field(&["thinking", "type"], "adaptive"),
                    field(&["output_config", "effort"], "high"),
                ],
            },
        },
        accounting: ReasoningAccounting::WithinOutputCap,
        additional_reservation_tokens: 0,
    }
}

fn field(path: &[&str], value: &str) -> NativeReasoningFieldAssignment {
    NativeReasoningFieldAssignment {
        path: path.iter().map(|value| (*value).into()).collect(),
        value: NativeReasoningValue::String(value.into()),
    }
}

pub fn candidate_profile(
    ingress: IngressProtocol,
    upstream: IngressProtocol,
) -> CandidateProtocolProfile {
    CandidateProtocolProfile::exact_portable_path(
        ingress,
        upstream,
        format!("physical-{upstream:?}"),
        reasoning_profile(upstream),
    )
}

pub fn decoder_profile(protocol: IngressProtocol) -> CandidateProtocolProfile {
    let mut profile = candidate_profile(protocol, protocol);
    profile.capability.native_provider_state = NativeProviderStateEmission::Native;
    profile.capability.response.provider_state = Fidelity::Exact;
    profile.capability.request.provider_state = Fidelity::Exact;
    profile
}

pub fn request_fixture(protocol: IngressProtocol) -> Value {
    let schema = json!({
        "type": "object",
        "properties": {"city": {"type": "string"}},
        "required": ["city"]
    });
    match protocol {
        IngressProtocol::Responses => json!({
            "model": "agent/research", "stream": true,
            "instructions": "Answer precisely.",
            "input": [
                {"type":"message","role":"user","content":[
                    {"type":"input_text","text":"weather?"},
                    {"type":"input_image","image_url":"https://img.invalid/map.png"}
                ]},
                {"type":"function_call","call_id":"call_weather","name":"weather","arguments":"{\"city\":\"Paris\"}"},
                {"type":"function_call_output","call_id":"call_weather","output":"sunny"}
            ],
            "tools": [{"type":"function","name":"weather","description":"Look up weather","parameters":schema}],
            "tool_choice": {"type":"function","name":"weather"},
            "parallel_tool_calls": false,
            "reasoning": {"effort":"low"}, "max_output_tokens": 7
        }),
        IngressProtocol::ChatCompletions => json!({
            "model": "agent/research", "stream": true,
            "messages": [
                {"role":"system","content":"Answer precisely."},
                {"role":"user","content":[
                    {"type":"text","text":"weather?"},
                    {"type":"image_url","image_url":{"url":"https://img.invalid/map.png"}}
                ]},
                {"role":"assistant","content":null,"tool_calls":[{"id":"call_weather","type":"function","function":{"name":"weather","arguments":"{\"city\":\"Paris\"}"}}]},
                {"role":"tool","tool_call_id":"call_weather","content":"sunny"}
            ],
            "tools": [{"type":"function","function":{"name":"weather","description":"Look up weather","parameters":schema}}],
            "tool_choice": {"type":"function","function":{"name":"weather"}},
            "parallel_tool_calls": false,
            "stream_options": {"include_usage":true},
            "reasoning_effort": "minimal", "max_completion_tokens": 9
        }),
        IngressProtocol::Messages => json!({
            "model": "agent/research", "stream": true, "max_tokens": 11,
            "system": "Answer precisely.",
            "messages": [
                {"role":"user","content":[
                    {"type":"text","text":"weather?"},
                    {"type":"image","source":{"type":"url","url":"https://img.invalid/map.png"}}
                ]},
                {"role":"assistant","content":[{"type":"tool_use","id":"call_weather","name":"weather","input":{"city":"Paris"}}]},
                {"role":"user","content":[{"type":"tool_result","tool_use_id":"call_weather","content":"sunny"}]}
            ],
            "tools": [{"name":"weather","description":"Look up weather","input_schema":schema}],
            "tool_choice": {"type":"tool","name":"weather","disable_parallel_tool_use":true},
            "thinking": {"type":"enabled","budget_tokens":64},
            "output_config": {"effort":"low"}
        }),
    }
}

pub fn normalized_request(mut request: ModelRequestIRV1) -> ModelRequestIRV1 {
    request.native_body = None;
    request.ingress_protocol = IngressProtocol::Responses;
    request.requested_reasoning = RequestedReasoningControl::absent();
    request.requested_max_output_tokens = None;
    // Messages can report a completed tool result; Chat and an unannotated
    // Responses output cannot. Compare shared request content separately from
    // that protocol-specific status, which the matrix asserts explicitly.
    for message in &mut request.messages {
        for part in &mut message.content {
            if let ContentPart::ToolResult { status, .. } = part {
                *status = ToolResultStatusV1::Unknown;
            }
        }
    }
    request
}

pub fn native_nonstream(protocol: IngressProtocol) -> Vec<u8> {
    let value = match protocol {
        IngressProtocol::Responses => json!({
            "id":"resp_native","object":"response","created_at":1,"status":"completed","error":null,"incomplete_details":null,"model":"physical-responses",
            "output":[
                {"type":"reasoning","id":"rs_0","summary":[{"type":"summary_text","text":"careful"}]},
                {"type":"message","id":"msg_1","status":"completed","role":"assistant","content":[{"type":"output_text","text":"done","annotations":[]}]},
                {"type":"function_call","id":"fc_2","call_id":"responses_native_a","name":"weather","arguments":"{\"city\":\"Paris\"}","status":"completed"},
                {"type":"function_call","id":"fc_3","call_id":"responses_native_b","name":"units","arguments":"{\"unit\":\"C\"}","status":"completed"}
            ],
            "usage":{"input_tokens":10,"output_tokens":20,"total_tokens":30}
        }),
        IngressProtocol::ChatCompletions => json!({
            "id":"chat_native","object":"chat.completion","service_tier":"default","created":1,"model":"physical-chat",
            "choices":[{"index":0,"message":{"role":"assistant","reasoning_content":"careful","content":"done","tool_calls":[
                {"id":"chat_native_a","type":"function","function":{"name":"weather","arguments":"{\"city\":\"Paris\"}"}},
                {"id":"chat_native_b","type":"function","function":{"name":"units","arguments":"{\"unit\":\"C\"}"}}
            ]},"finish_reason":"tool_calls","logprobs":null}],
            "usage":{"prompt_tokens":10,"completion_tokens":20,"total_tokens":30}
        }),
        IngressProtocol::Messages => json!({
            "id":"msg_native","type":"message","role":"assistant","model":"physical-messages",
            "content":[
                {"type":"thinking","thinking":"careful"},
                {"type":"text","text":"done"},
                {"type":"tool_use","id":"messages_native_a","name":"weather","input":{"city":"Paris"}},
                {"type":"tool_use","id":"messages_native_b","name":"units","input":{"unit":"C"}}
            ],
            "stop_reason":"tool_use","stop_sequence":null,
            "usage":{"input_tokens":10,"output_tokens":20}
        }),
    };
    serde_json::to_vec(&value).unwrap()
}

pub fn native_stream(protocol: IngressProtocol) -> Vec<u8> {
    let events = match protocol {
        IngressProtocol::Responses => responses_events(),
        IngressProtocol::ChatCompletions => chat_events(),
        IngressProtocol::Messages => messages_events(),
    };
    let mut bytes = Vec::new();
    for event in events {
        bytes.extend_from_slice(&event.wire_bytes().unwrap());
    }
    bytes
}

fn responses_events() -> Vec<RenderedSseEvent> {
    vec![
        named(
            "response.created",
            json!({"type":"response.created","sequence_number":0,"response":{"id":"resp_native","model":"physical-responses"}}),
        ),
        named(
            "response.output_item.added",
            json!({"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":{"type":"reasoning"}}),
        ),
        named(
            "response.reasoning_summary_text.delta",
            json!({"type":"response.reasoning_summary_text.delta","sequence_number":2,"item_id":"rs_0","output_index":0,"summary_index":0,"delta":"care"}),
        ),
        named(
            "response.reasoning_summary_text.delta",
            json!({"type":"response.reasoning_summary_text.delta","sequence_number":3,"item_id":"rs_0","output_index":0,"summary_index":0,"delta":"ful"}),
        ),
        named(
            "response.output_item.added",
            json!({"type":"response.output_item.added","sequence_number":4,"output_index":1,"item":{"type":"message"}}),
        ),
        named(
            "response.output_text.delta",
            json!({"type":"response.output_text.delta","sequence_number":5,"item_id":"msg_1","output_index":1,"content_index":0,"delta":"do"}),
        ),
        named(
            "response.output_text.delta",
            json!({"type":"response.output_text.delta","sequence_number":6,"item_id":"msg_1","output_index":1,"content_index":0,"delta":"ne"}),
        ),
        named(
            "response.output_item.added",
            json!({"type":"response.output_item.added","sequence_number":7,"output_index":2,"item":{"type":"function_call","id":"fc_2","call_id":"responses_native_a","name":"weather","arguments":"","status":"in_progress"}}),
        ),
        named(
            "response.output_item.added",
            json!({"type":"response.output_item.added","sequence_number":8,"output_index":3,"item":{"type":"function_call","id":"fc_3","call_id":"responses_native_b","name":"units","arguments":"","status":"in_progress"}}),
        ),
        named(
            "response.function_call_arguments.delta",
            json!({"type":"response.function_call_arguments.delta","sequence_number":9,"item_id":"fc_2","output_index":2,"delta":"{\"city\":"}),
        ),
        named(
            "response.function_call_arguments.delta",
            json!({"type":"response.function_call_arguments.delta","sequence_number":10,"item_id":"fc_3","output_index":3,"delta":"{\"unit\":"}),
        ),
        named(
            "response.function_call_arguments.delta",
            json!({"type":"response.function_call_arguments.delta","sequence_number":11,"item_id":"fc_2","output_index":2,"delta":"\"Paris\"}"}),
        ),
        named(
            "response.function_call_arguments.delta",
            json!({"type":"response.function_call_arguments.delta","sequence_number":12,"item_id":"fc_3","output_index":3,"delta":"\"C\"}"}),
        ),
        named(
            "response.function_call_arguments.done",
            json!({"type":"response.function_call_arguments.done","sequence_number":13,"item_id":"fc_2","output_index":2,"arguments":"{\"city\":\"Paris\"}"}),
        ),
        named(
            "response.function_call_arguments.done",
            json!({"type":"response.function_call_arguments.done","sequence_number":14,"item_id":"fc_3","output_index":3,"arguments":"{\"unit\":\"C\"}"}),
        ),
        named(
            "response.completed",
            json!({"type":"response.completed","sequence_number":15,"response":{"id":"resp_native","model":"physical-responses","status":"completed","incomplete_details":null,"usage":{"input_tokens":10,"output_tokens":20,"total_tokens":30}}}),
        ),
    ]
}

fn chat_events() -> Vec<RenderedSseEvent> {
    vec![
        unnamed(
            json!({"id":"chat_native","object":"chat.completion.chunk","created":1,"model":"physical-chat","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"care"},"finish_reason":null}],"usage":null}),
        ),
        unnamed(
            json!({"id":"chat_native","object":"chat.completion.chunk","created":1,"model":"physical-chat","choices":[{"index":0,"delta":{"reasoning_content":"ful","content":"do"},"finish_reason":null}],"usage":null}),
        ),
        unnamed(
            json!({"id":"chat_native","object":"chat.completion.chunk","created":1,"model":"physical-chat","choices":[{"index":0,"delta":{"content":"ne","tool_calls":[
            {"index":0,"id":"chat_native_a","type":"function","function":{"name":"weather","arguments":"{\"city\":"}},
            {"index":1,"id":"chat_native_b","type":"function","function":{"name":"units","arguments":"{\"unit\":"}}
        ]},"finish_reason":null}],"usage":null}),
        ),
        unnamed(
            json!({"id":"chat_native","object":"chat.completion.chunk","created":1,"model":"physical-chat","choices":[{"index":0,"delta":{"tool_calls":[
            {"index":1,"function":{"arguments":"\"C\"}"}},
            {"index":0,"function":{"arguments":"\"Paris\"}"}}
        ]},"finish_reason":null}],"usage":null}),
        ),
        unnamed(
            json!({"id":"chat_native","object":"chat.completion.chunk","created":1,"model":"physical-chat","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"service_tier":"default","usage":null}),
        ),
        unnamed(
            json!({"id":"chat_native","object":"chat.completion.chunk","created":1,"model":"physical-chat","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":20,"total_tokens":30}}),
        ),
        RenderedSseEvent {
            event: None,
            data: Value::String("[DONE]".into()),
        },
    ]
}

fn messages_events() -> Vec<RenderedSseEvent> {
    vec![
        named(
            "message_start",
            json!({"type":"message_start","message":{"id":"msg_native","type":"message","role":"assistant","content":[],"model":"physical-messages","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":0}}}),
        ),
        named(
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
        ),
        named(
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"careful"}}),
        ),
        named(
            "content_block_stop",
            json!({"type":"content_block_stop","index":0}),
        ),
        named(
            "content_block_start",
            json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
        ),
        named(
            "content_block_delta",
            json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"done"}}),
        ),
        named(
            "content_block_stop",
            json!({"type":"content_block_stop","index":1}),
        ),
        named(
            "content_block_start",
            json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"messages_native_a","name":"weather","input":{}}}),
        ),
        named(
            "content_block_start",
            json!({"type":"content_block_start","index":3,"content_block":{"type":"tool_use","id":"messages_native_b","name":"units","input":{}}}),
        ),
        named(
            "content_block_delta",
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"city\":"}}),
        ),
        named(
            "content_block_delta",
            json!({"type":"content_block_delta","index":3,"delta":{"type":"input_json_delta","partial_json":"{\"unit\":"}}),
        ),
        named(
            "content_block_delta",
            json!({"type":"content_block_delta","index":3,"delta":{"type":"input_json_delta","partial_json":"\"C\"}"}}),
        ),
        named(
            "content_block_delta",
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"\"Paris\"}"}}),
        ),
        named(
            "content_block_stop",
            json!({"type":"content_block_stop","index":2}),
        ),
        named(
            "content_block_stop",
            json!({"type":"content_block_stop","index":3}),
        ),
        named(
            "message_delta",
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":20}}),
        ),
        named("message_stop", json!({"type":"message_stop"})),
    ]
}

fn named(event: &str, data: Value) -> RenderedSseEvent {
    RenderedSseEvent {
        event: Some(event.into()),
        data,
    }
}

fn unnamed(data: Value) -> RenderedSseEvent {
    RenderedSseEvent { event: None, data }
}

pub fn decode_fragmented(
    protocol: IngressProtocol,
    streaming: bool,
    bytes: &[u8],
    pattern: &[usize],
) -> DecodedNativeResponse {
    let profile = decoder_profile(protocol);
    decode_fragmented_for_profile(&profile, streaming, bytes, pattern)
}

pub fn decode_fragmented_for_profile(
    profile: &CandidateProtocolProfile,
    streaming: bool,
    bytes: &[u8],
    pattern: &[usize],
) -> DecodedNativeResponse {
    let mut decoder = NativeResponseDecoder::new(profile, 200, streaming).unwrap();
    let mut offset = 0;
    let mut step = 0;
    let mut observed = Vec::new();
    while offset < bytes.len() {
        let width = pattern[step % pattern.len()].max(1);
        let end = (offset + width).min(bytes.len());
        let status = decoder
            .feed(&bytes[offset..end], end == bytes.len())
            .unwrap();
        if status == ResponseDecodeStatus::NeedDrain {
            observed.extend(decoder.take_events());
            while decoder.resume().unwrap() == ResponseDecodeStatus::NeedDrain {
                observed.extend(decoder.take_events());
            }
        }
        offset = end;
        step += 1;
    }
    let mut decoded = decoder.finish().unwrap();
    observed.append(&mut decoded.events);
    decoded.events = observed;
    decoded
}

pub fn ledger_from_rendered(
    protocol: IngressProtocol,
    rendered: &RenderedClientResponse,
) -> SemanticLedgerV1 {
    match rendered {
        RenderedClientResponse::Json { status, bytes, .. } => {
            let profile = decoder_profile(protocol);
            let mut decoder = NativeResponseDecoder::new(&profile, *status, false).unwrap();
            let chunks = bytes.chunks(3).collect::<Vec<_>>();
            for (index, chunk) in chunks.iter().enumerate() {
                decoder.feed(chunk, index + 1 == chunks.len()).unwrap();
            }
            decoder.finish().unwrap().response.semantic_ledger()
        }
        RenderedClientResponse::Sse { status, bytes, .. } => {
            assert_eq!(*status, 200);
            decode_fragmented(protocol, true, bytes, &[1, 2, 5, 3, 8])
                .response
                .semantic_ledger()
        }
    }
}
