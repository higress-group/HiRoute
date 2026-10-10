#[path = "render_responses.rs"]
mod responses;
use responses::{render_responses_json, render_responses_stream};
use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::server::core_runtime::model_ir::{
    FinishReason, ModelError, ModelResponseIRV1, ModelUsage, OpaqueProviderState, ResponseBlock,
    ToolKindV1,
};
use crate::server::core_runtime::profiles::{
    ClientProtocolProfile, Fidelity, StreamingRefusalSemantics,
};
use crate::server::request_plan::IngressProtocol;

use super::{ProtocolAdapterError, client_can_represent_message_phase, responses_terminal};

pub(super) fn render_responses_output_block(
    block: &ResponseBlock,
) -> Result<Value, ProtocolAdapterError> {
    responses::render_responses_block(block)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderedSseEvent {
    pub event: Option<String>,
    pub data: Value,
}

impl RenderedSseEvent {
    pub fn wire_bytes(&self) -> Result<Vec<u8>, ProtocolAdapterError> {
        let mut bytes = Vec::new();
        if let Some(event) = &self.event {
            bytes.extend_from_slice(b"event: ");
            bytes.extend_from_slice(event.as_bytes());
            bytes.push(b'\n');
        }
        bytes.extend_from_slice(b"data: ");
        match &self.data {
            Value::String(value) if value == "[DONE]" => bytes.extend_from_slice(b"[DONE]"),
            value => bytes.extend_from_slice(
                &serde_json::to_vec(value)
                    .map_err(|error| ProtocolAdapterError::Serialization(error.to_string()))?,
            ),
        }
        bytes.extend_from_slice(b"\n\n");
        Ok(bytes)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RenderedClientResponse {
    Json {
        status: u16,
        content_type: &'static str,
        body: Value,
        bytes: Vec<u8>,
    },
    Sse {
        status: u16,
        content_type: &'static str,
        events: Vec<RenderedSseEvent>,
        bytes: Vec<u8>,
    },
}

pub struct ClientResponseRenderer;

impl ClientResponseRenderer {
    pub fn render_nonstream_with_profile(
        profile: &ClientProtocolProfile,
        served_model_alias: &str,
        response: &ModelResponseIRV1,
    ) -> Result<RenderedClientResponse, ProtocolAdapterError> {
        validate_client_profile(profile, response, false)?;
        render_nonstream_validated(profile.protocol, served_model_alias, response)
    }

    pub fn render_nonstream(
        protocol: IngressProtocol,
        served_model_alias: &str,
        response: &ModelResponseIRV1,
    ) -> Result<RenderedClientResponse, ProtocolAdapterError> {
        ensure_alias(served_model_alias)?;
        validate_projection(protocol, response, false)?;
        render_nonstream_validated(protocol, served_model_alias, response)
    }

    pub fn render_stream(
        protocol: IngressProtocol,
        served_model_alias: &str,
        response: &ModelResponseIRV1,
    ) -> Result<RenderedClientResponse, ProtocolAdapterError> {
        ensure_alias(served_model_alias)?;
        validate_projection(protocol, response, true)?;
        render_stream_validated(protocol, served_model_alias, response)
    }

    pub fn render_stream_with_profile(
        profile: &ClientProtocolProfile,
        served_model_alias: &str,
        response: &ModelResponseIRV1,
    ) -> Result<RenderedClientResponse, ProtocolAdapterError> {
        validate_client_profile(profile, response, true)?;
        ensure_alias(served_model_alias)?;
        validate_projection(profile.protocol, response, true)?;
        render_stream_validated(profile.protocol, served_model_alias, response)
    }
}

fn render_stream_validated(
    protocol: IngressProtocol,
    served_model_alias: &str,
    response: &ModelResponseIRV1,
) -> Result<RenderedClientResponse, ProtocolAdapterError> {
    let events = if let Some(error) = &response.error {
        vec![render_stream_error(protocol, error)]
    } else {
        match protocol {
            IngressProtocol::Responses => render_responses_stream(served_model_alias, response)?,
            IngressProtocol::ChatCompletions => render_chat_stream(served_model_alias, response)?,
            IngressProtocol::Messages => render_messages_stream(served_model_alias, response)?,
        }
    };
    let mut bytes = Vec::new();
    for event in &events {
        bytes.extend_from_slice(&event.wire_bytes()?);
    }
    Ok(RenderedClientResponse::Sse {
        status: 200,
        content_type: "text/event-stream",
        events,
        bytes,
    })
}

fn render_nonstream_validated(
    protocol: IngressProtocol,
    served_model_alias: &str,
    response: &ModelResponseIRV1,
) -> Result<RenderedClientResponse, ProtocolAdapterError> {
    ensure_alias(served_model_alias)?;
    validate_projection(protocol, response, false)?;
    let (status, body) = if let Some(error) = &response.error {
        (
            error.status.unwrap_or(500),
            render_error(protocol, served_model_alias, error),
        )
    } else {
        let body = match protocol {
            IngressProtocol::Responses => render_responses_json(served_model_alias, response)?,
            IngressProtocol::ChatCompletions => render_chat_json(served_model_alias, response)?,
            IngressProtocol::Messages => render_messages_json(served_model_alias, response)?,
        };
        (200, body)
    };
    let bytes = serde_json::to_vec(&body)
        .map_err(|error| ProtocolAdapterError::Serialization(error.to_string()))?;
    Ok(RenderedClientResponse::Json {
        status,
        content_type: "application/json",
        body,
        bytes,
    })
}

fn validate_client_profile(
    profile: &ClientProtocolProfile,
    response: &ModelResponseIRV1,
    streaming: bool,
) -> Result<(), ProtocolAdapterError> {
    if !profile.is_complete() {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "client protocol profile is incomplete".into(),
        ));
    }
    let exact = Fidelity::Exact;
    for block in &response.blocks {
        let supported = match block {
            ResponseBlock::WebSearch { .. } => profile.protocol == IngressProtocol::Responses,
            ResponseBlock::Text { phase, .. } => {
                profile.response.text == exact
                    && client_can_represent_message_phase(profile.protocol, phase.as_deref())
            }
            ResponseBlock::Reasoning { .. } => profile.response.reasoning == exact,
            ResponseBlock::Refusal { phase, .. } => {
                profile.response.refusal == exact
                    && client_can_represent_message_phase(profile.protocol, phase.as_deref())
            }
            ResponseBlock::ToolCall {
                tool_kind,
                namespace,
                ..
            } => {
                profile.response.tool_calls == exact
                    && profile.response.logical_tool_id_mapping == exact
                    && (namespace.is_none() || profile.protocol == IngressProtocol::Responses)
                    && (*tool_kind == ToolKindV1::Function
                        || profile.protocol == IngressProtocol::Responses)
            }
        };
        if !supported {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "client protocol profile cannot preserve a response block".into(),
            ));
        }
    }
    if response.error.is_some() && profile.response.typed_error != exact {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "client protocol profile lacks typed errors".into(),
        ));
    }
    if response.error.is_none()
        && (profile.response.usage != exact || profile.response.finish_reason != exact)
    {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "client protocol profile lacks exact usage or finish semantics".into(),
        ));
    }
    if streaming
        && ((response
            .blocks
            .iter()
            .any(|block| matches!(block, ResponseBlock::Refusal { .. }))
            && profile.response.stream_refusal != StreamingRefusalSemantics::ExactDelta)
            || (response.blocks.iter().any(|block| {
                matches!(
                    block,
                    ResponseBlock::Text { .. } | ResponseBlock::Refusal { .. }
                )
            }) && profile.response.stream_text_delta != exact)
            || (response
                .blocks
                .iter()
                .any(|block| matches!(block, ResponseBlock::Reasoning { .. }))
                && profile.response.stream_reasoning_delta != exact)
            || (response
                .blocks
                .iter()
                .any(|block| matches!(block, ResponseBlock::ToolCall { .. }))
                && profile.response.stream_tool_argument_delta != exact)
            || (!response.usage.is_empty() && profile.response.stream_usage != exact))
    {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "client stream profile cannot preserve canonical deltas".into(),
        ));
    }
    Ok(())
}

fn render_chat_json(
    alias: &str,
    response: &ModelResponseIRV1,
) -> Result<Value, ProtocolAdapterError> {
    let mut message = Map::new();
    message.insert("role".into(), Value::String("assistant".into()));
    let text = collect_text(response);
    message.insert(
        "content".into(),
        if text.is_empty() {
            Value::Null
        } else {
            Value::String(text)
        },
    );
    let reasoning = collect_reasoning(response);
    if !reasoning.is_empty() {
        message.insert("reasoning_content".into(), Value::String(reasoning));
    }
    let refusal = collect_refusal(response);
    if !refusal.is_empty() {
        message.insert("content".into(), Value::Null);
        message.insert("refusal".into(), Value::String(refusal));
    }
    let calls = chat_tool_calls(response)?;
    if !calls.is_empty() {
        message.insert("tool_calls".into(), Value::Array(calls));
    }
    Ok(json!({
        "id": response.response_id,
        "object": "chat.completion",
        "created": 0,
        "model": alias,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_label_chat(response.finish_reason.as_ref()),
            "logprobs": null,
        }],
        "usage": chat_usage(&response.usage),
    }))
}

fn render_messages_json(
    alias: &str,
    response: &ModelResponseIRV1,
) -> Result<Value, ProtocolAdapterError> {
    let mut content = response
        .blocks
        .iter()
        .map(render_messages_block)
        .collect::<Result<Vec<_>, _>>()?;
    append_messages_state(&mut content, &response.provider_state)?;
    content.retain(|block| block["type"] != "thinking" || block.get("signature").is_some());
    Ok(json!({
        "id": response.response_id,
        "type": "message",
        "role": "assistant",
        "content": content,
        "model": alias,
        "stop_reason": finish_label_messages(response.finish_reason.as_ref()),
        "stop_sequence": null,
        "usage": messages_usage(&response.usage),
    }))
}

fn render_chat_stream(
    alias: &str,
    response: &ModelResponseIRV1,
) -> Result<Vec<RenderedSseEvent>, ProtocolAdapterError> {
    let mut events = Vec::new();
    let prefix = |delta: Value, finish_reason: Value, usage: Value| json!({"id":response.response_id,"object":"chat.completion.chunk","created":0,"model":alias,"choices":[{"index":0,"delta":delta,"finish_reason":finish_reason}],"usage":usage});
    events.push(RenderedSseEvent {
        event: None,
        data: prefix(json!({"role":"assistant"}), Value::Null, Value::Null),
    });
    for block in &response.blocks {
        match block {
            ResponseBlock::WebSearch { .. } => {
                return Err(ProtocolAdapterError::ClientUnrepresentable(
                    "hosted search requires Responses".into(),
                ));
            }
            ResponseBlock::Reasoning { text, .. } => events.push(RenderedSseEvent {
                event: None,
                data: prefix(json!({"reasoning_content":text}), Value::Null, Value::Null),
            }),
            ResponseBlock::Text { text, .. } => events.push(RenderedSseEvent {
                event: None,
                data: prefix(json!({"content":text}), Value::Null, Value::Null),
            }),
            ResponseBlock::Refusal { text, .. } => events.push(RenderedSseEvent {
                event: None,
                data: prefix(json!({"refusal":text}), Value::Null, Value::Null),
            }),
            ResponseBlock::ToolCall {
                index,
                logical_id,
                tool_kind,
                name,
                arguments,
                ..
            } => {
                if *tool_kind != ToolKindV1::Function {
                    return Err(ProtocolAdapterError::ClientUnrepresentable(
                        "custom tool calls require Responses".into(),
                    ));
                }
                let arguments = serde_json::to_string(arguments)
                    .map_err(|error| ProtocolAdapterError::Serialization(error.to_string()))?;
                events.push(RenderedSseEvent { event: None, data: prefix(json!({"tool_calls":[{"index":index,"id":logical_id,"type":"function","function":{"name":name,"arguments":arguments}}]}), Value::Null, Value::Null) });
            }
        }
    }
    events.push(RenderedSseEvent {
        event: None,
        data: prefix(
            json!({}),
            Value::String(finish_label_chat(response.finish_reason.as_ref()).into()),
            chat_usage(&response.usage),
        ),
    });
    events.push(RenderedSseEvent {
        event: None,
        data: Value::String("[DONE]".into()),
    });
    Ok(events)
}

fn render_messages_stream(
    alias: &str,
    response: &ModelResponseIRV1,
) -> Result<Vec<RenderedSseEvent>, ProtocolAdapterError> {
    let signatures = messages_signatures(&response.provider_state)?;
    if signatures.keys().any(|index| {
        !response.blocks.iter().any(|block| {
            matches!(block, ResponseBlock::Reasoning { index: block_index, .. } if block_index == index)
        })
    }) {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "Messages streaming signature has no reasoning block".into(),
        ));
    }
    let mut events = Vec::new();
    push_event(
        &mut events,
        "message_start",
        json!({"type":"message_start","message":{"id":response.response_id,"type":"message","role":"assistant","content":[],"model":alias,"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":response.usage.input_tokens.unwrap_or(0),"output_tokens":0}}}),
    );
    for block in &response.blocks {
        match block {
            ResponseBlock::WebSearch { .. } => {
                return Err(ProtocolAdapterError::ClientUnrepresentable(
                    "hosted search requires Responses".into(),
                ));
            }
            ResponseBlock::Text { index, text, .. } => {
                push_event(
                    &mut events,
                    "content_block_start",
                    json!({"type":"content_block_start","index":index,"content_block":{"type":"text","text":""}}),
                );
                push_event(
                    &mut events,
                    "content_block_delta",
                    json!({"type":"content_block_delta","index":index,"delta":{"type":"text_delta","text":text}}),
                );
                push_event(
                    &mut events,
                    "content_block_stop",
                    json!({"type":"content_block_stop","index":index}),
                );
            }
            ResponseBlock::Reasoning { index, text, .. } => {
                if !signatures.contains_key(index) {
                    continue;
                }
                push_event(
                    &mut events,
                    "content_block_start",
                    json!({"type":"content_block_start","index":index,"content_block":{"type":"thinking","thinking":""}}),
                );
                if !text.is_empty() {
                    push_event(
                        &mut events,
                        "content_block_delta",
                        json!({"type":"content_block_delta","index":index,"delta":{"type":"thinking_delta","thinking":text}}),
                    );
                }
                if let Some(signature) = signatures.get(index) {
                    push_event(
                        &mut events,
                        "content_block_delta",
                        json!({"type":"content_block_delta","index":index,"delta":{"type":"signature_delta","signature":signature}}),
                    );
                }
                push_event(
                    &mut events,
                    "content_block_stop",
                    json!({"type":"content_block_stop","index":index}),
                );
            }
            ResponseBlock::Refusal { index, text, .. } => {
                push_event(
                    &mut events,
                    "content_block_start",
                    json!({"type":"content_block_start","index":index,"content_block":{"type":"text","text":""}}),
                );
                push_event(
                    &mut events,
                    "content_block_delta",
                    json!({"type":"content_block_delta","index":index,"delta":{"type":"text_delta","text":text}}),
                );
                push_event(
                    &mut events,
                    "content_block_stop",
                    json!({"type":"content_block_stop","index":index}),
                );
            }
            ResponseBlock::ToolCall {
                index,
                logical_id,
                tool_kind,
                name,
                arguments,
                ..
            } => {
                if *tool_kind != ToolKindV1::Function {
                    return Err(ProtocolAdapterError::ClientUnrepresentable(
                        "custom tool calls require Responses".into(),
                    ));
                }
                let arguments = serde_json::to_string(arguments)
                    .map_err(|error| ProtocolAdapterError::Serialization(error.to_string()))?;
                push_event(
                    &mut events,
                    "content_block_start",
                    json!({"type":"content_block_start","index":index,"content_block":{"type":"tool_use","id":logical_id,"name":name,"input":{}}}),
                );
                push_event(
                    &mut events,
                    "content_block_delta",
                    json!({"type":"content_block_delta","index":index,"delta":{"type":"input_json_delta","partial_json":arguments}}),
                );
                push_event(
                    &mut events,
                    "content_block_stop",
                    json!({"type":"content_block_stop","index":index}),
                );
            }
        }
    }
    push_event(
        &mut events,
        "message_delta",
        json!({"type":"message_delta","delta":{"stop_reason":finish_label_messages(response.finish_reason.as_ref()),"stop_sequence":null},"usage":{"output_tokens":response.usage.output_tokens.unwrap_or(0)}}),
    );
    push_event(&mut events, "message_stop", json!({"type":"message_stop"}));
    // Omitted unsigned reasoning must not leave holes in Messages block indices.
    let mut indices = BTreeMap::new();
    for event in &mut events {
        if let Some(index) = event.data.get("index").and_then(Value::as_u64) {
            let next = indices.len();
            let wire_index = *indices.entry(index).or_insert(next);
            event.data["index"] = json!(wire_index);
        }
    }
    Ok(events)
}

fn render_messages_block(block: &ResponseBlock) -> Result<Value, ProtocolAdapterError> {
    Ok(match block {
        ResponseBlock::WebSearch { .. } => {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "hosted search requires Responses".into(),
            ));
        }
        ResponseBlock::Text { text, .. } => json!({"type":"text","text":text}),
        ResponseBlock::Reasoning { text, .. } => json!({"type":"thinking","thinking":text}),
        ResponseBlock::Refusal { text, .. } => json!({"type":"text","text":text}),
        ResponseBlock::ToolCall {
            logical_id,
            tool_kind,
            name,
            arguments,
            ..
        } => {
            if *tool_kind != ToolKindV1::Function {
                return Err(ProtocolAdapterError::ClientUnrepresentable(
                    "custom tool calls require Responses".into(),
                ));
            }
            json!({"type":"tool_use","id":logical_id,"name":name,"input":arguments})
        }
    })
}

fn chat_tool_calls(response: &ModelResponseIRV1) -> Result<Vec<Value>, ProtocolAdapterError> {
    response.blocks.iter().filter_map(|block| match block {
        ResponseBlock::ToolCall { logical_id, tool_kind, name, arguments, .. } => Some(if *tool_kind != ToolKindV1::Function {
            Err(ProtocolAdapterError::ClientUnrepresentable("custom tool calls require Responses".into()))
        } else {
            serde_json::to_string(arguments).map(|arguments| json!({"id":logical_id,"type":"function","function":{"name":name,"arguments":arguments}})).map_err(|error| ProtocolAdapterError::Serialization(error.to_string()))
        }),
        _ => None,
    }).collect()
}

fn append_responses_state(
    output: &mut Vec<Value>,
    state: &[OpaqueProviderState],
) -> Result<(), ProtocolAdapterError> {
    for (state_index, state) in state.iter().enumerate() {
        if state.kind != "encrypted_content" {
            continue;
        }
        if let Some(block_index) = state
            .block_index
            .and_then(|value| usize::try_from(value).ok())
        {
            let Some(block) = output.get_mut(block_index).and_then(Value::as_object_mut) else {
                return Err(ProtocolAdapterError::ClientUnrepresentable(
                    "Responses state has no exact reasoning block binding".into(),
                ));
            };
            if block.get("type").and_then(Value::as_str) != Some("reasoning")
                || block
                    .insert("encrypted_content".into(), state.value.clone())
                    .is_some()
            {
                return Err(ProtocolAdapterError::ClientUnrepresentable(
                    "Responses state block binding is not unique".into(),
                ));
            }
        } else {
            output.push(json!({"type":"reasoning","id":format!("state_{state_index}"),"summary":[],"encrypted_content":state.value}));
        }
    }
    Ok(())
}

fn append_messages_state(
    content: &mut Vec<Value>,
    state: &[OpaqueProviderState],
) -> Result<(), ProtocolAdapterError> {
    for state in state {
        match state.kind.as_str() {
            "redacted_thinking" => content.push(state.value.clone()),
            "thinking_signature" => {
                if !state.value.is_string() {
                    return Err(ProtocolAdapterError::ClientUnrepresentable(
                        "thinking signature is not a string".into(),
                    ));
                }
                let Some(index) = state
                    .block_index
                    .and_then(|value| usize::try_from(value).ok())
                else {
                    return Err(ProtocolAdapterError::ClientUnrepresentable(
                        "thinking signature has no exact block binding".into(),
                    ));
                };
                let Some(block) = content
                    .get_mut(index)
                    .filter(|block| block.get("type") == Some(&Value::String("thinking".into())))
                else {
                    return Err(ProtocolAdapterError::ClientUnrepresentable(
                        "thinking signature has no reasoning block".into(),
                    ));
                };
                block
                    .as_object_mut()
                    .expect("rendered block is object")
                    .insert("signature".into(), state.value.clone());
            }
            _ => {}
        }
    }
    Ok(())
}

fn messages_signatures(
    state: &[OpaqueProviderState],
) -> Result<std::collections::BTreeMap<u32, String>, ProtocolAdapterError> {
    let mut signatures = std::collections::BTreeMap::new();
    for state in state {
        if state.kind != "thinking_signature" {
            continue;
        }
        let index = state.block_index.ok_or_else(|| {
            ProtocolAdapterError::ClientUnrepresentable(
                "Messages streaming provider state has no reasoning block".into(),
            )
        })?;
        let signature = state.value.as_str().ok_or_else(|| {
            ProtocolAdapterError::ClientUnrepresentable(
                "Messages streaming signature is not a string".into(),
            )
        })?;
        if signatures.insert(index, signature.to_owned()).is_some() {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "Messages streaming reasoning state is not unique".into(),
            ));
        }
    }
    Ok(signatures)
}

fn collect_text(response: &ModelResponseIRV1) -> String {
    response
        .blocks
        .iter()
        .filter_map(|block| match block {
            ResponseBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn collect_reasoning(response: &ModelResponseIRV1) -> String {
    response
        .blocks
        .iter()
        .filter_map(|block| match block {
            ResponseBlock::Reasoning { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn collect_refusal(response: &ModelResponseIRV1) -> String {
    response
        .blocks
        .iter()
        .filter_map(|block| match block {
            ResponseBlock::Refusal { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn responses_usage(usage: &ModelUsage) -> Value {
    let mut value = json!({"input_tokens":usage.input_tokens.unwrap_or(0),"output_tokens":usage.output_tokens.unwrap_or(0),"total_tokens":usage.input_tokens.unwrap_or(0).saturating_add(usage.output_tokens.unwrap_or(0))});
    let object = value
        .as_object_mut()
        .expect("Responses usage renderer creates an object");
    if let Some(tokens) = usage.cache_read_tokens {
        object.insert(
            "input_tokens_details".into(),
            json!({"cached_tokens":tokens}),
        );
    }
    if let Some(tokens) = usage.reasoning_tokens {
        object.insert(
            "output_tokens_details".into(),
            json!({"reasoning_tokens":tokens}),
        );
    }
    value
}

fn chat_usage(usage: &ModelUsage) -> Value {
    let mut value = json!({"prompt_tokens":usage.input_tokens.unwrap_or(0),"completion_tokens":usage.output_tokens.unwrap_or(0),"total_tokens":usage.input_tokens.unwrap_or(0).saturating_add(usage.output_tokens.unwrap_or(0))});
    let object = value
        .as_object_mut()
        .expect("Chat usage renderer creates an object");
    if let Some(tokens) = usage.cache_read_tokens {
        object.insert(
            "prompt_tokens_details".into(),
            json!({"cached_tokens":tokens}),
        );
    }
    if let Some(tokens) = usage.reasoning_tokens {
        object.insert(
            "completion_tokens_details".into(),
            json!({"reasoning_tokens":tokens}),
        );
    }
    value
}

fn messages_usage(usage: &ModelUsage) -> Value {
    let mut value = json!({"input_tokens":usage.input_tokens.unwrap_or(0),"output_tokens":usage.output_tokens.unwrap_or(0)});
    let object = value
        .as_object_mut()
        .expect("Messages usage renderer creates an object");
    if let Some(tokens) = usage.cache_read_tokens {
        object.insert("cache_read_input_tokens".into(), Value::from(tokens));
    }
    if let Some(tokens) = usage.cache_write_tokens {
        object.insert("cache_creation_input_tokens".into(), Value::from(tokens));
    }
    value
}

fn finish_label_chat(reason: Option<&FinishReason>) -> &'static str {
    match reason {
        Some(FinishReason::Length) => "length",
        Some(FinishReason::ToolCall) => "tool_calls",
        Some(FinishReason::Refusal) => "content_filter",
        _ => "stop",
    }
}

fn finish_label_messages(reason: Option<&FinishReason>) -> &'static str {
    match reason {
        Some(FinishReason::Length) => "max_tokens",
        Some(FinishReason::ToolCall) => "tool_use",
        Some(FinishReason::Refusal) => "refusal",
        _ => "end_turn",
    }
}

fn render_error(protocol: IngressProtocol, _alias: &str, error: &ModelError) -> Value {
    let error = json!({"type":error.code.clone().unwrap_or_else(|| "provider_error".into()),"code":error.code,"message":error.message});
    match protocol {
        IngressProtocol::Responses | IngressProtocol::ChatCompletions => {
            json!({"error":error})
        }
        IngressProtocol::Messages => json!({"type":"error","error":error}),
    }
}

fn render_stream_error(protocol: IngressProtocol, error: &ModelError) -> RenderedSseEvent {
    let native = json!({"type":"error","error":{"type":error.code.clone().unwrap_or_else(|| "provider_error".into()),"code":error.code,"message":error.message}});
    RenderedSseEvent {
        event: (protocol != IngressProtocol::ChatCompletions).then(|| "error".into()),
        data: native,
    }
}

fn push_event(events: &mut Vec<RenderedSseEvent>, event: &str, data: Value) {
    events.push(RenderedSseEvent {
        event: Some(event.into()),
        data,
    });
}

fn ensure_alias(alias: &str) -> Result<(), ProtocolAdapterError> {
    if alias.is_empty() {
        Err(ProtocolAdapterError::ClientUnrepresentable(
            "served model alias is empty".into(),
        ))
    } else {
        Ok(())
    }
}

fn validate_projection(
    protocol: IngressProtocol,
    response: &ModelResponseIRV1,
    streaming: bool,
) -> Result<(), ProtocolAdapterError> {
    if response.completed != response.error.is_none() {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "canonical terminal state is inconsistent".into(),
        ));
    }
    if protocol != IngressProtocol::Responses
        && response
            .blocks
            .iter()
            .any(|block| matches!(block, ResponseBlock::WebSearch { .. }))
    {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "hosted search requires Responses".into(),
        ));
    }
    if protocol != IngressProtocol::Responses && response.blocks.iter().any(|block|
        matches!(block, ResponseBlock::Text { annotations, .. } if !annotations.is_empty())) {
        return Err(ProtocolAdapterError::ClientUnrepresentable("URL citations require Responses".into()));
    }
    if response.blocks.iter().any(|block| match block {
        ResponseBlock::Text { phase, .. } | ResponseBlock::Refusal { phase, .. } => {
            !client_can_represent_message_phase(protocol, phase.as_deref())
        }
        _ => false,
    }) {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "Responses message phase requires Responses".into(),
        ));
    }
    if protocol != IngressProtocol::Responses
        && response.blocks.iter().any(|block| {
            matches!(
                block,
                ResponseBlock::ToolCall {
                    namespace: Some(_),
                    ..
                }
            )
        })
    {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "namespaced Tool call requires Responses".into(),
        ));
    }
    if protocol != IngressProtocol::Responses
        && response.blocks.iter().any(|block| {
            matches!(
                block,
                ResponseBlock::ToolCall {
                    tool_kind: ToolKindV1::Custom,
                    ..
                }
            )
        })
    {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "custom Tool call requires Responses".into(),
        ));
    }
    let tool_blocks = response
        .blocks
        .iter()
        .filter_map(|block| match block {
            ResponseBlock::ToolCall {
                logical_id,
                tool_kind,
                namespace,
                name,
                ..
            } => Some((*tool_kind, logical_id, namespace.as_deref(), name.as_str())),
            ResponseBlock::WebSearch { item, .. } => {
                Some((ToolKindV1::Function, &item.id, None, "web_search"))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if response.tool_id_map.len() != tool_blocks.len()
        || tool_blocks
            .iter()
            .any(|(kind, logical_id, namespace, name)| {
                response
                    .tool_id_map
                    .iter()
                    .filter(|binding| {
                        &binding.logical_id == *logical_id
                            && binding.kind == *kind
                            && binding.namespace.as_deref() == *namespace
                            && binding.name == *name
                            && !binding.native_id.trim().is_empty()
                            && binding.owner.is_complete()
                            && binding.owner.upstream_protocol == response.source_protocol
                    })
                    .count()
                    != 1
            })
    {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "canonical Tool call lacks one exact logical/native ID binding".into(),
        ));
    }
    if response.error.is_some()
        && (!response.blocks.is_empty()
            || !response.provider_state.is_empty()
            || !response.usage.is_empty()
            || response.finish_reason.is_some())
    {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "batch renderer cannot discard semantics emitted before an error".into(),
        ));
    }
    for (position, block) in response.blocks.iter().enumerate() {
        if usize::try_from(block.index()).ok() != Some(position) {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "canonical response block indexes are not contiguous".into(),
            ));
        }
    }
    if matches!(
        response.finish_reason,
        Some(FinishReason::Cancelled | FinishReason::Other(_))
    ) {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "client protocol has no exact finish reason".into(),
        ));
    }
    if response.error.is_none()
        && (response.usage.input_tokens.is_none() || response.usage.output_tokens.is_none())
    {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "P0 client projection requires exact input and output usage".into(),
        ));
    }
    match protocol {
        IngressProtocol::Responses | IngressProtocol::ChatCompletions
            if response.usage.cache_write_tokens.is_some() =>
        {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "client protocol cannot represent cache-write usage".into(),
            ));
        }
        // Messages has no separate reasoning-token counter. As in the
        // incremental renderer, retain the billable output total; the canonical
        // usage still records its reasoning breakdown for observation.
        _ => {}
    }
    if protocol == IngressProtocol::ChatCompletions {
        let reasoning_blocks = response
            .blocks
            .iter()
            .filter(|block| matches!(block, ResponseBlock::Reasoning { .. }))
            .count();
        let text_blocks = response
            .blocks
            .iter()
            .filter(|block| matches!(block, ResponseBlock::Text { .. }))
            .count();
        let refusal_blocks = response
            .blocks
            .iter()
            .filter(|block| matches!(block, ResponseBlock::Refusal { .. }))
            .count();
        let rank = |block: &ResponseBlock| match block {
            ResponseBlock::WebSearch { .. } => 3,
            ResponseBlock::Reasoning { .. } => 0,
            ResponseBlock::Text { .. } | ResponseBlock::Refusal { .. } => 1,
            ResponseBlock::ToolCall { .. } => 2,
        };
        if reasoning_blocks > 1
            || text_blocks > 1
            || refusal_blocks > 1
            || (text_blocks > 0 && refusal_blocks > 0)
            || response
                .blocks
                .windows(2)
                .any(|blocks| rank(&blocks[0]) > rank(&blocks[1]))
        {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "Chat cannot preserve canonical content-block ordering".into(),
            ));
        }
    }
    if let Some(error) = &response.error
        && (error.retryable.is_some() || (streaming && error.status.is_some()))
    {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "client error envelope cannot preserve typed error metadata".into(),
        ));
    }
    Ok(())
}
