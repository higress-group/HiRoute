use serde_json::{Map, Value, json};

use crate::content_ref::{ContentValueExt, JsonValueExt};
use crate::server::core_runtime::model_ir::*;
use crate::server::core_runtime::profiles::{
    CandidateContextDemand, CandidateProtocolProfile, ContextProjectionError, ContextProjector,
    NativeReasoningRender, NativeReasoningValue, ReasoningProfileCapability,
};
use crate::server::request_plan::IngressProtocol;

use super::{ChatToolProjection, ProtocolAdapterError};

mod reader;
mod template;
mod tools;
pub use reader::sequential_attempt_body;
pub(crate) use reader::sequential_replay_body;
pub use template::{PreparedNativeTemplate, project_candidate_request_template};
pub(crate) use template::{
    PreparedReplayTemplate, ReplacementEncoding, RequestedReplacement, prepare_replay_json_template,
};
use tools::{insert_chat_tools, insert_messages_tools, insert_responses_tools};

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedNativeRequest {
    pub protocol: IngressProtocol,
    pub path: String,
    pub body: Value,
    pub bytes: Vec<u8>,
    pub context: CandidateContextDemand,
    pub capability_id: String,
    pub connector_id: String,
    pub adapter_revision: String,
    pub serializer_revision: String,
    pub decoder_revision: String,
    pub reasoning_profile_id: String,
    pub(crate) chat_tool_projection: Option<ChatToolProjection>,
}

/// Validates every critical profile fact and serializes one candidate without
/// acquiring credentials, resolving DNS or opening a connection.
pub fn project_candidate_request(
    request: &ModelRequestIRV1,
    profile: &CandidateProtocolProfile,
) -> Result<PreparedNativeRequest, ProtocolAdapterError> {
    if template::request_has_content_refs(request) {
        return Err(ProtocolAdapterError::Serialization(
            "ContentRef requires the sequential native-request encoder".into(),
        ));
    }
    let requirements = request.requirements();
    let reasoning = profile.validate(&requirements)?;
    validate_message_shapes(request, profile.capability.upstream_protocol)?;
    let chat_tool_projection =
        if profile.capability.upstream_protocol == IngressProtocol::ChatCompletions {
            Some(ChatToolProjection::for_request(request)?)
        } else {
            None
        };
    let body = match profile.capability.upstream_protocol {
        IngressProtocol::Responses => serialize_responses(request, profile, reasoning)?,
        IngressProtocol::ChatCompletions => serialize_chat(
            request,
            profile,
            reasoning,
            chat_tool_projection
                .as_ref()
                .expect("Chat projection was constructed"),
        )?,
        IngressProtocol::Messages => serialize_messages(request, profile, reasoning)?,
    };
    let bytes = serde_json::to_vec(&body)
        .map_err(|error| ProtocolAdapterError::Serialization(error.to_string()))?;
    let context = ContextProjector::project(&bytes, &profile.capability.context, reasoning)?;
    Ok(PreparedNativeRequest {
        protocol: profile.capability.upstream_protocol,
        path: profile.connector.request_path.clone(),
        body,
        bytes,
        context,
        capability_id: profile.capability.capability_id.clone(),
        connector_id: profile.connector.connector_id.clone(),
        adapter_revision: profile.adapter_revision.clone(),
        serializer_revision: profile.serializer_revision.clone(),
        decoder_revision: profile.decoder_revision.clone(),
        reasoning_profile_id: reasoning.profile_id.clone(),
        chat_tool_projection,
    })
}

fn serialize_responses(
    request: &ModelRequestIRV1,
    profile: &CandidateProtocolProfile,
    reasoning: &ReasoningProfileCapability,
) -> Result<Value, ProtocolAdapterError> {
    let mut body = Map::new();
    if let Some(options) = &request.responses_options {
        if let Some(store) = options.store {
            body.insert("store".into(), Value::Bool(store));
        }
        if let Some(include) = &options.include {
            body.insert(
                "include".into(),
                serde_json::to_value(include)
                    .map_err(|_| ProtocolAdapterError::Serialization("Responses include".into()))?,
            );
        }
        if let Some(prompt_cache_key) = &options.prompt_cache_key {
            body.insert(
                "prompt_cache_key".into(),
                Value::String(prompt_cache_key.clone()),
            );
        }
        if let Some(client_metadata) = &options.client_metadata {
            body.insert(
                "client_metadata".into(),
                serde_json::to_value(client_metadata).map_err(|_| {
                    ProtocolAdapterError::Serialization("Responses client metadata".into())
                })?,
            );
        }
        if options.reasoning_summary.is_some() || options.reasoning_context.is_some() {
            let mut native = Map::new();
            if let Some(summary) = &options.reasoning_summary {
                native.insert("summary".into(), Value::String(summary.wire_value()));
            }
            if let Some(context) = &options.reasoning_context {
                native.insert("context".into(), Value::String(context.clone()));
            }
            body.insert("reasoning".into(), Value::Object(native));
        }
    }
    body.insert(
        "model".into(),
        Value::String(profile.capability.native_model.clone()),
    );
    body.insert("stream".into(), Value::Bool(request.stream));
    if !request.instructions.is_empty() {
        let text = instruction_text(&request.instructions, false)?;
        body.insert("instructions".into(), Value::String(text));
    }
    let mut input = Vec::new();
    for (message_index, message) in request.messages.iter().enumerate() {
        if let Some(search) = request.responses_search_history.get(&message_index) {
            let mut search = search.clone();
            search.id = native_tool_id(request, profile, &search.id)?;
            input.push(with_responses_item_fields(
                request,
                message_index,
                false,
                false,
                search.wire_value(),
            )?);
            continue;
        }
        if message.content.is_empty()
            && let Some(history) = request.responses_reasoning_history.get(&message_index)
        {
            if message.role != MessageRole::Assistant
                || history.encrypted_content == ResponsesReasoningEncryptedContentV1::Opaque
            {
                return Err(ProtocolAdapterError::ClientUnrepresentable(
                    "Responses reasoning history shape is inconsistent".into(),
                ));
            }
            let mut item = responses_reasoning_native_fields(history);
            item["type"] = json!("reasoning");
            match history.encrypted_content {
                ResponsesReasoningEncryptedContentV1::Absent => {}
                ResponsesReasoningEncryptedContentV1::Null => {
                    item["encrypted_content"] = Value::Null;
                }
                ResponsesReasoningEncryptedContentV1::Empty => {
                    item["encrypted_content"] = Value::String(String::new());
                }
                ResponsesReasoningEncryptedContentV1::Opaque => unreachable!(),
            }
            input.push(with_responses_item_fields(
                request,
                message_index,
                true,
                false,
                item,
            )?);
            continue;
        }
        let mut message_content = Vec::new();
        for (part_index, part) in message.content.iter().enumerate() {
            match part {
                ContentPart::Text { text } => message_content.push((part_index, json!({
                    "type": if message.role == MessageRole::Assistant { "output_text" } else { "input_text" },
                    "text": text.wire_value(),
                }))),
                ContentPart::Image { source } => message_content.push((part_index, json!({
                    "type": "input_image",
                    "image_url": render_image_url(source),
                }))),
                ContentPart::ToolCall {
                    logical_id,
                    tool_kind,
                    namespace,
                    name,
                    arguments,
                } => {
                    flush_responses_message(
                        &mut input,
                        request,
                        message_index,
                        message,
                        &mut message_content,
                    )?;
                    let native_arguments = match tool_kind {
                        ToolKindV1::Function => compact_json(arguments)?,
                        ToolKindV1::Custom => arguments
                            .wire_value()
                            .as_str()
                            .ok_or_else(|| {
                                ProtocolAdapterError::ClientUnrepresentable(
                                    "custom tool input must be a freeform string".into(),
                                )
                            })?
                            .to_owned(),
                    };
                    let mut item = json!({
                        "type": match tool_kind { ToolKindV1::Function => "function_call", ToolKindV1::Custom => "custom_tool_call" },
                        "call_id": native_tool_call_id(
                            request,
                            profile,
                            logical_id,
                            *tool_kind,
                            namespace.as_deref(),
                            name,
                        )?,
                        "name": name,
                    });
                    item[match tool_kind {
                        ToolKindV1::Function => "arguments",
                        ToolKindV1::Custom => "input",
                    }] = Value::String(native_arguments);
                    if let Some(namespace) = namespace {
                        item["namespace"] = Value::String(namespace.clone());
                    }
                    input.push(with_responses_item_fields(
                        request,
                        message_index,
                        true,
                        false,
                        item,
                    )?);
                }
                ContentPart::ToolResult {
                    logical_id,
                    tool_kind,
                    namespace,
                    output,
                    ..
                } => {
                    flush_responses_message(
                        &mut input,
                        request,
                        message_index,
                        message,
                        &mut message_content,
                    )?;
                    input.push(with_responses_item_fields(
                        request,
                        message_index,
                        true,
                        false,
                        json!({
                            "type": match tool_kind { ToolKindV1::Function => "function_call_output", ToolKindV1::Custom => "custom_tool_call_output" },
                            "call_id": native_tool_result_id(
                                request,
                                profile,
                                logical_id,
                                *tool_kind,
                                namespace.as_deref(),
                            )?,
                            "output": render_tool_output(output)?,
                        }),
                    )?);
                }
                ContentPart::ProviderState { state } => {
                    flush_responses_message(
                        &mut input,
                        request,
                        message_index,
                        message,
                        &mut message_content,
                    )?;
                    ensure_state_owner(state, profile)?;
                    if message.role != MessageRole::Assistant
                        || state.kind != "encrypted_content"
                        || !(state
                            .value
                            .as_str()
                            .is_some_and(|value| !value.is_empty())
                            || state
                                .value
                                .content_ref()
                                .is_some_and(|content| content.byte_len() > 2))
                    {
                        return Err(ProtocolAdapterError::ClientUnrepresentable(
                            "Responses reasoning continuation state is not exact".into(),
                        ));
                    }
                    let history = request.responses_reasoning_history.get(&message_index);
                    if history.is_some_and(|history| {
                        history.encrypted_content
                            != ResponsesReasoningEncryptedContentV1::Opaque
                    }) {
                        return Err(ProtocolAdapterError::ClientUnrepresentable(
                            "Responses reasoning history shape is inconsistent".into(),
                        ));
                    }
                    let mut item = history.map_or_else(
                        || json!({"summary":[],"content":null}),
                        responses_reasoning_native_fields,
                    );
                    item["type"] = json!("reasoning");
                    item["encrypted_content"] = state.value.wire_value();
                    input.push(with_responses_item_fields(
                        request,
                        message_index,
                        true,
                        false,
                        item,
                    )?);
                }
            }
        }
        flush_responses_message(
            &mut input,
            request,
            message_index,
            message,
            &mut message_content,
        )?;
    }
    for state in &request.provider_state {
        ensure_state_owner(state, profile)?;
        if !matches!(state.kind.as_str(), "previous_response_id" | "conversation") {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "Responses provider-state kind is not representable".into(),
            ));
        }
        if body
            .insert(state.kind.clone(), state.value.wire_value())
            .is_some()
        {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "Responses provider-state key collides with request fields".into(),
            ));
        }
    }
    body.insert("input".into(), Value::Array(input));
    insert_responses_tools(&mut body, request)?;
    render_reasoning(&mut body, reasoning, IngressProtocol::Responses)?;
    body.insert(
        "max_output_tokens".into(),
        Value::from(candidate_max_output(profile)?),
    );
    Ok(Value::Object(body))
}

fn responses_reasoning_native_fields(history: &ResponsesReasoningHistoryV1) -> Value {
    Value::Object(
        history
            .native_fields
            .iter()
            .map(|(key, value)| (key.clone(), value.wire_value()))
            .collect(),
    )
}

fn flush_responses_message(
    input: &mut Vec<Value>,
    request: &ModelRequestIRV1,
    message_index: usize,
    message: &CanonicalMessage,
    content: &mut Vec<(usize, Value)>,
) -> Result<(), ProtocolAdapterError> {
    if content.is_empty() {
        return Ok(());
    }
    if let Some(parts) = request.responses_annotations.get(&message_index) {
        for (original_index, part) in content.iter_mut() {
            if let Some(annotations) = parts.get(original_index) {
                part["annotations"] = serde_json::to_value(annotations)
                    .map_err(|_| ProtocolAdapterError::Serialization("annotations".into()))?;
            }
        }
    }
    let mut object = Map::new();
    object.insert("type".into(), Value::String("message".into()));
    object.insert(
        "role".into(),
        Value::String(role_label(&message.role).into()),
    );
    object.insert(
        "content".into(),
        Value::Array(content.drain(..).map(|(_, value)| value).collect()),
    );
    if let Some(name) = &message.name {
        object.insert("name".into(), Value::String(name.clone()));
    }
    input.push(with_responses_item_fields(
        request,
        message_index,
        true,
        true,
        Value::Object(object),
    )?);
    Ok(())
}

fn with_responses_item_fields(
    request: &ModelRequestIRV1,
    index: usize,
    preserve_client_item_id: bool,
    allow_phase: bool,
    mut item: Value,
) -> Result<Value, ProtocolAdapterError> {
    if preserve_client_item_id && let Some(id) = request.responses_item_ids.get(&index) {
        item["id"] = Value::String(id.clone());
    }
    if let Some(status) = request.responses_item_statuses.get(&index) {
        let item_type = item.get("type").and_then(Value::as_str);
        if status != "completed"
            && !(item_type == Some("function_call_output")
                && matches!(status.as_str(), "incomplete" | "in_progress"))
        {
            return Err(ModelIrError::InvalidField("Responses input item status").into());
        }
        item["status"] = Value::String(status.clone());
    }
    if let Some(metadata) = request.responses_internal_chat_message_metadata.get(&index) {
        item["internal_chat_message_metadata_passthrough"] = serde_json::to_value(metadata)
            .map_err(|_| {
                ProtocolAdapterError::Serialization(
                    "Responses internal chat message metadata".into(),
                )
            })?;
    }
    if let Some(phase) = request.responses_message_phases.get(&index) {
        if !allow_phase {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "Responses message phase belongs to a non-message item".into(),
            ));
        }
        item["phase"] = serde_json::to_value(phase)
            .map_err(|_| ProtocolAdapterError::Serialization("Responses message phase".into()))?;
    }
    Ok(item)
}

fn serialize_chat(
    request: &ModelRequestIRV1,
    profile: &CandidateProtocolProfile,
    reasoning: &ReasoningProfileCapability,
    tool_projection: &ChatToolProjection,
) -> Result<Value, ProtocolAdapterError> {
    let mut messages = Vec::new();
    for instruction in &request.instructions {
        messages.push(json!({
            "role": instruction_role_label(&instruction.role),
            "content": content_text_only(&instruction.content)?,
        }));
    }
    for message in &request.messages {
        let mut base_content = Vec::new();
        let mut tool_calls = Vec::new();
        let mut provider_state = Vec::new();
        for part in &message.content {
            match part {
                ContentPart::Text { text } => base_content.push(json!({
                    "type": "text",
                    "text": text.wire_value(),
                })),
                ContentPart::Image { source } => base_content.push(json!({
                    "type": "image_url",
                    "image_url": {"url": render_image_url(source)},
                })),
                ContentPart::ToolCall {
                    logical_id,
                    tool_kind,
                    namespace,
                    name,
                    arguments,
                } => {
                    let emitted_name =
                        tool_projection.emitted_for(*tool_kind, namespace.as_deref(), name)?;
                    let identity = tool_projection
                        .resolve_emitted(emitted_name)
                        .expect("emitted name came from this projection");
                    let chat_arguments = ChatToolProjection::chat_arguments(identity, arguments)?;
                    tool_calls.push(json!({
                        "id": native_tool_call_id(request, profile, logical_id, *tool_kind, namespace.as_deref(), name)?,
                        "type": "function",
                        "function": {"name": emitted_name, "arguments": compact_json(&chat_arguments)?},
                    }));
                }
                ContentPart::ToolResult {
                    logical_id,
                    tool_kind,
                    namespace,
                    output,
                    ..
                } => {
                    if message.content.len() != 1 || message.role != MessageRole::User {
                        return Err(ProtocolAdapterError::ClientUnrepresentable(
                            "Chat Tool result must be one request message".into(),
                        ));
                    }
                    let mut object = Map::new();
                    object.insert("role".into(), Value::String("tool".into()));
                    object.insert(
                        "tool_call_id".into(),
                        Value::String(native_tool_result_id(
                            request,
                            profile,
                            logical_id,
                            *tool_kind,
                            namespace.as_deref(),
                        )?),
                    );
                    object.insert("content".into(), render_tool_output(output)?);
                    if let Some(name) = &message.name {
                        object.insert("name".into(), Value::String(name.clone()));
                    }
                    messages.push(Value::Object(object));
                }
                ContentPart::ProviderState { state } => {
                    ensure_state_owner(state, profile)?;
                    if message.role != MessageRole::Assistant
                        || state.kind != "reasoning_content"
                        || (!state.value.is_string() && state.value.content_ref().is_none())
                    {
                        return Err(ProtocolAdapterError::ClientUnrepresentable(
                            "Chat provider-state value is not exact reasoning_content".into(),
                        ));
                    }
                    provider_state.push((state.kind.clone(), state.value.wire_value()));
                }
            }
        }
        if !base_content.is_empty() || !tool_calls.is_empty() || !provider_state.is_empty() {
            let mut object = Map::new();
            object.insert(
                "role".into(),
                Value::String(role_label(&message.role).into()),
            );
            if let Some(name) = &message.name {
                object.insert("name".into(), Value::String(name.clone()));
            }
            if !base_content.is_empty() {
                object.insert("content".into(), collapse_chat_content(base_content));
            }
            if !tool_calls.is_empty() {
                object.insert("tool_calls".into(), Value::Array(tool_calls));
            }
            for (key, value) in provider_state {
                object.insert(key, value);
            }
            if object.len() == 2
                && object.get("role").and_then(Value::as_str) == Some("assistant")
                && object.get("tool_calls").is_some_and(Value::is_array)
                && let Some(previous) = messages.last_mut().and_then(Value::as_object_mut)
                && previous.len() == 2
                && previous.get("role").and_then(Value::as_str) == Some("assistant")
                && let Some(previous_calls) =
                    previous.get_mut("tool_calls").and_then(Value::as_array_mut)
            {
                previous_calls.extend(
                    object
                        .remove("tool_calls")
                        .and_then(|calls| calls.as_array().cloned())
                        .expect("tool-only Chat message has an array of calls"),
                );
                continue;
            }
            messages.push(Value::Object(object));
        }
    }
    if !request.provider_state.is_empty() {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "Chat has no request-level provider-state field".into(),
        ));
    }
    let mut body = Map::new();
    body.insert(
        "model".into(),
        Value::String(profile.capability.native_model.clone()),
    );
    body.insert("stream".into(), Value::Bool(request.stream));
    body.insert("messages".into(), Value::Array(messages));
    if request.stream {
        body.insert("stream_options".into(), json!({"include_usage": true}));
    }
    insert_chat_tools(&mut body, request, tool_projection)?;
    render_reasoning(&mut body, reasoning, IngressProtocol::ChatCompletions)?;
    body.insert(
        "max_completion_tokens".into(),
        Value::from(candidate_max_output(profile)?),
    );
    Ok(Value::Object(body))
}

fn serialize_messages(
    request: &ModelRequestIRV1,
    profile: &CandidateProtocolProfile,
    reasoning: &ReasoningProfileCapability,
) -> Result<Value, ProtocolAdapterError> {
    let mut body = Map::new();
    body.insert(
        "model".into(),
        Value::String(profile.capability.native_model.clone()),
    );
    body.insert("stream".into(), Value::Bool(request.stream));
    body.insert(
        "max_tokens".into(),
        Value::from(candidate_max_output(profile)?),
    );
    if !request.instructions.is_empty() {
        body.insert(
            "system".into(),
            Value::String(instruction_text(&request.instructions, true)?),
        );
    }
    let mut messages = Vec::new();
    for message in &request.messages {
        if message.name.is_some() {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "Messages cannot preserve message names".into(),
            ));
        }
        let mut content = Vec::new();
        for part in &message.content {
            match part {
                ContentPart::Text { text } => {
                    content.push(json!({"type": "text", "text": text.wire_value()}));
                }
                ContentPart::Image { source } => content.push(match source {
                    ImageSource::Url { url } => {
                        json!({"type": "image", "source": {"type": "url", "url": url}})
                    }
                    ImageSource::Base64 { media_type, data } => json!({
                        "type": "image",
                        "source": {"type": "base64", "media_type": media_type, "data": data.wire_value()},
                    }),
                }),
                ContentPart::ToolCall {
                    logical_id,
                    tool_kind,
                    namespace,
                    name,
                    arguments,
                } => {
                    if *tool_kind != ToolKindV1::Function {
                        return Err(ProtocolAdapterError::ClientUnrepresentable(
                            "Messages cannot express custom tool calls".into(),
                        ));
                    }
                    reject_namespace(namespace, IngressProtocol::Messages)?;
                    content.push(json!({
                        "type": "tool_use",
                        "id": native_tool_call_id(request, profile, logical_id, *tool_kind, None, name)?,
                        "name": name,
                        "input": arguments.wire_value(),
                    }));
                }
                ContentPart::ToolResult {
                    logical_id,
                    tool_kind,
                    namespace,
                    output,
                    status,
                } => {
                    if *tool_kind != ToolKindV1::Function {
                        return Err(ProtocolAdapterError::ClientUnrepresentable(
                            "Messages cannot express custom tool results".into(),
                        ));
                    }
                    reject_namespace(namespace, IngressProtocol::Messages)?;
                    let mut tool_result = json!({
                        "type": "tool_result",
                        "tool_use_id": native_tool_result_id(request, profile, logical_id, *tool_kind, None)?,
                        "content": render_tool_output(output)?,
                    });
                    if *status == ToolResultStatusV1::Failed {
                        tool_result["is_error"] = Value::Bool(true);
                    }
                    content.push(tool_result);
                }
                ContentPart::ProviderState { state } => {
                    ensure_state_owner(state, profile)?;
                    if !matches!(state.kind.as_str(), "thinking" | "redacted_thinking")
                        || (state.value.content_ref().is_none()
                            && state.value.get("type").and_then(Value::as_str)
                                != Some(state.kind.as_str()))
                    {
                        return Err(ProtocolAdapterError::ClientUnrepresentable(
                            "Messages provider-state block is not exact".into(),
                        ));
                    }
                    content.push(state.value.wire_value());
                }
            }
        }
        messages.push(json!({
            "role": role_label_messages(&message.role)?,
            "content": content,
        }));
    }
    if !request.provider_state.is_empty() {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "Messages has no request-level provider-state field".into(),
        ));
    }
    body.insert("messages".into(), Value::Array(messages));
    insert_messages_tools(&mut body, request)?;
    render_reasoning(&mut body, reasoning, IngressProtocol::Messages)?;
    Ok(Value::Object(body))
}

fn validate_message_shapes(
    request: &ModelRequestIRV1,
    target: IngressProtocol,
) -> Result<(), ProtocolAdapterError> {
    if (!request.responses_annotations.is_empty()
        || !request.responses_search_history.is_empty()
        || request.web_search.is_some()
        || request.responses_options.is_some()
        || !request.responses_item_ids.is_empty()
        || !request.responses_item_statuses.is_empty()
        || !request.responses_message_phases.is_empty()
        || !request.responses_internal_chat_message_metadata.is_empty()
        || !request.responses_reasoning_history.is_empty()
        || (target == IngressProtocol::Messages
            && (!request.tool_namespaces.is_empty() || !request.responses_tool_order.is_empty())))
        && target != IngressProtocol::Responses
    {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "native Responses options require a Responses upstream".into(),
        ));
    }
    if request
        .responses_message_phases
        .keys()
        .chain(request.responses_item_statuses.keys())
        .chain(request.responses_internal_chat_message_metadata.keys())
        .chain(request.responses_reasoning_history.keys())
        .any(|index| *index >= request.messages.len())
    {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "Responses input-item metadata has no canonical message".into(),
        ));
    }
    for index in request.responses_message_phases.keys() {
        let message = &request.messages[*index];
        if message.content.is_empty()
            || message
                .content
                .iter()
                .any(|part| !matches!(part, ContentPart::Text { .. } | ContentPart::Image { .. }))
        {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "Responses message phase belongs to a non-message item".into(),
            ));
        }
    }
    for index in request.responses_reasoning_history.keys() {
        let message = &request.messages[*index];
        let history = &request.responses_reasoning_history[index];
        let shape_matches = match history.encrypted_content {
            ResponsesReasoningEncryptedContentV1::Opaque => {
                message.content.len() == 1
                    && matches!(
                        message.content.first(),
                        Some(ContentPart::ProviderState { .. })
                    )
            }
            ResponsesReasoningEncryptedContentV1::Absent
            | ResponsesReasoningEncryptedContentV1::Null
            | ResponsesReasoningEncryptedContentV1::Empty => message.content.is_empty(),
        };
        if message.role != MessageRole::Assistant || !shape_matches {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "Responses reasoning history belongs to a non-reasoning item".into(),
            ));
        }
    }
    for message in &request.messages {
        let mut native_items = 0_usize;
        let mut message_parts = 0_usize;
        for part in &message.content {
            match part {
                ContentPart::Text { .. } => message_parts += 1,
                ContentPart::Image { .. } => {
                    if message.role != MessageRole::User {
                        return Err(ProtocolAdapterError::ClientUnrepresentable(
                            "image input must belong to a user message".into(),
                        ));
                    }
                    message_parts += 1;
                }
                ContentPart::ToolCall { .. } => {
                    if message.role != MessageRole::Assistant {
                        return Err(ProtocolAdapterError::ClientUnrepresentable(
                            "Tool call must belong to an assistant message".into(),
                        ));
                    }
                    native_items += 1;
                }
                ContentPart::ToolResult { .. } => {
                    if message.role != MessageRole::User {
                        return Err(ProtocolAdapterError::ClientUnrepresentable(
                            "Tool result must belong to a user message".into(),
                        ));
                    }
                    native_items += 1;
                }
                ContentPart::ProviderState { .. } => {}
            }
        }
        if target == IngressProtocol::Responses && native_items != 0 && message_parts != 0 {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "Responses cannot preserve mixed message and item ordering".into(),
            ));
        }
        if target == IngressProtocol::ChatCompletions
            && message
                .content
                .iter()
                .filter(|part| matches!(part, ContentPart::ToolResult { .. }))
                .count()
                > 1
        {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "Chat requires one message per Tool result".into(),
            ));
        }
    }
    Ok(())
}

fn render_reasoning(
    body: &mut Map<String, Value>,
    reasoning: &ReasoningProfileCapability,
    protocol: IngressProtocol,
) -> Result<(), ProtocolAdapterError> {
    match &reasoning.render {
        NativeReasoningRender::NoControlParameter => {}
        NativeReasoningRender::ExactFields {
            protocol: owner,
            fields,
        }
        | NativeReasoningRender::ExactBudget {
            protocol: owner,
            fields,
            ..
        } if *owner == protocol => {
            for field in fields {
                insert_exact_field(body, &field.path, native_reasoning_value(&field.value))?;
            }
        }
        NativeReasoningRender::ExactFields { .. } | NativeReasoningRender::ExactBudget { .. } => {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "reasoning render belongs to another upstream protocol".into(),
            ));
        }
    }
    Ok(())
}

fn native_reasoning_value(value: &NativeReasoningValue) -> Value {
    match value {
        NativeReasoningValue::Bool(value) => Value::Bool(*value),
        NativeReasoningValue::String(value) => Value::String(value.clone()),
        NativeReasoningValue::U64(value) => Value::from(*value),
    }
}

fn insert_exact_field(
    body: &mut Map<String, Value>,
    path: &[String],
    value: Value,
) -> Result<(), ProtocolAdapterError> {
    let Some((root, tail)) = path.split_first() else {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "reasoning field path is empty".into(),
        ));
    };
    if tail.is_empty() {
        if body.insert(root.clone(), value).is_some() {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "reasoning field collides with native request".into(),
            ));
        }
        return Ok(());
    }
    let root_value = body
        .entry(root.clone())
        .or_insert_with(|| Value::Object(Map::new()));
    let mut object = root_value.as_object_mut().ok_or_else(|| {
        ProtocolAdapterError::ClientUnrepresentable(
            "reasoning path collides with a non-object native field".into(),
        )
    })?;
    for part in &tail[..tail.len() - 1] {
        let child = object
            .entry(part.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        object = child.as_object_mut().ok_or_else(|| {
            ProtocolAdapterError::ClientUnrepresentable(
                "reasoning path collides with a non-object native field".into(),
            )
        })?;
    }
    if object.insert(tail[tail.len() - 1].clone(), value).is_some() {
        return Err(ProtocolAdapterError::ClientUnrepresentable(
            "reasoning field is assigned more than once".into(),
        ));
    }
    Ok(())
}

fn candidate_max_output(profile: &CandidateProtocolProfile) -> Result<u64, ProtocolAdapterError> {
    profile
        .capability
        .context
        .max_output_tokens
        .exact()
        .copied()
        .ok_or_else(|| ContextProjectionError::UnknownLimit("max_output").into())
}

fn instruction_text(
    instructions: &[CanonicalInstruction],
    messages_target: bool,
) -> Result<String, ProtocolAdapterError> {
    let mut output = Vec::new();
    for instruction in instructions {
        if instruction.role == InstructionRole::Developer {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                if messages_target {
                    "Messages cannot preserve a distinct developer instruction"
                } else {
                    "Responses cannot preserve a distinct developer instruction in instructions"
                }
                .into(),
            ));
        }
        output.push(content_text_only(&instruction.content)?);
    }
    Ok(output.join("\n"))
}

fn content_text_only(content: &[ContentPart]) -> Result<String, ProtocolAdapterError> {
    let mut output = String::new();
    for part in content {
        let ContentPart::Text { text } = part else {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "instruction content is not text".into(),
            ));
        };
        output.push_str(&text.wire_value());
    }
    Ok(output)
}

fn compact_json(value: &Value) -> Result<String, ProtocolAdapterError> {
    if let Some(content) = value.content_ref() {
        return Ok(content.wire_marker());
    }
    serde_json::to_string(value)
        .map_err(|error| ProtocolAdapterError::Serialization(error.to_string()))
}

fn render_tool_output(output: &ToolOutput) -> Result<Value, ProtocolAdapterError> {
    match output {
        ToolOutput::Text(value) => Ok(Value::String(value.wire_value())),
        ToolOutput::Json(value) => Ok(Value::String(match value.content_ref() {
            Some(content) => content.wire_marker(),
            None => canonical_json(value)?,
        })),
    }
}

fn canonical_json(value: &Value) -> Result<String, ProtocolAdapterError> {
    fn ordered(value: &Value) -> Value {
        match value {
            Value::Array(values) => Value::Array(values.iter().map(ordered).collect()),
            Value::Object(object) => {
                let mut keys = object.keys().collect::<Vec<_>>();
                keys.sort_unstable();
                let mut output = Map::new();
                for key in keys {
                    output.insert(key.clone(), ordered(&object[key]));
                }
                Value::Object(output)
            }
            value => value.clone(),
        }
    }

    serde_json::to_string(&ordered(value))
        .map_err(|error| ProtocolAdapterError::Serialization(error.to_string()))
}

fn render_image_url(source: &ImageSource) -> String {
    match source {
        ImageSource::Url { url } => url.clone(),
        ImageSource::Base64 { media_type, data } => {
            format!("data:{media_type};base64,{data}")
        }
    }
}

fn ensure_state_owner(
    state: &OpaqueProviderState,
    profile: &CandidateProtocolProfile,
) -> Result<(), ProtocolAdapterError> {
    if state.owner == profile.exact_provider_path()? {
        Ok(())
    } else {
        Err(ModelIrError::ProviderStateNotPortable.into())
    }
}

fn native_tool_id(
    request: &ModelRequestIRV1,
    profile: &CandidateProtocolProfile,
    logical_id: &str,
) -> Result<String, ProtocolAdapterError> {
    let owner = profile.exact_provider_path()?;
    let mut matches = request
        .tool_id_map
        .iter()
        .filter(|binding| binding.logical_id == logical_id && binding.owner == owner);
    let binding = matches
        .next()
        .ok_or_else(|| ModelIrError::ToolIdBindingRequired(logical_id.into()))?;
    if binding.native_id.trim().is_empty() || matches.next().is_some() {
        return Err(ModelIrError::ToolIdBindingRequired(logical_id.into()).into());
    }
    Ok(binding.native_id.clone())
}

fn native_tool_call_id(
    request: &ModelRequestIRV1,
    profile: &CandidateProtocolProfile,
    logical_id: &str,
    kind: ToolKindV1,
    namespace: Option<&str>,
    name: &str,
) -> Result<String, ProtocolAdapterError> {
    let binding = native_tool_binding(request, profile, logical_id)?;
    if binding.kind != kind || binding.name != name || binding.namespace.as_deref() != namespace {
        return Err(ModelIrError::ToolContinuationConflict.into());
    }
    Ok(binding.native_id.clone())
}

fn native_tool_result_id(
    request: &ModelRequestIRV1,
    profile: &CandidateProtocolProfile,
    logical_id: &str,
    kind: ToolKindV1,
    namespace: Option<&str>,
) -> Result<String, ProtocolAdapterError> {
    let binding = native_tool_binding(request, profile, logical_id)?;
    if binding.kind != kind || binding.namespace.as_deref() != namespace {
        return Err(ModelIrError::ToolContinuationConflict.into());
    }
    Ok(binding.native_id.clone())
}

fn native_tool_binding<'a>(
    request: &'a ModelRequestIRV1,
    profile: &CandidateProtocolProfile,
    logical_id: &str,
) -> Result<&'a ToolIdMapEntryV1, ProtocolAdapterError> {
    let owner = profile.exact_provider_path()?;
    let mut matches = request
        .tool_id_map
        .iter()
        .filter(|binding| binding.logical_id == logical_id && binding.owner == owner);
    let binding = matches
        .next()
        .ok_or_else(|| ModelIrError::ToolIdBindingRequired(logical_id.into()))?;
    if binding.native_id.trim().is_empty() || matches.next().is_some() {
        return Err(ModelIrError::ToolIdBindingRequired(logical_id.into()).into());
    }
    Ok(binding)
}

fn reject_namespace(
    namespace: &Option<String>,
    protocol: IngressProtocol,
) -> Result<(), ProtocolAdapterError> {
    if namespace.is_some() {
        Err(ProtocolAdapterError::ClientUnrepresentable(format!(
            "namespace Tool identity is not representable by {protocol:?}"
        )))
    } else {
        Ok(())
    }
}

fn collapse_chat_content(content: Vec<Value>) -> Value {
    if content.len() == 1 && content[0]["type"] == "text" {
        content[0]["text"].clone()
    } else {
        Value::Array(content)
    }
}

fn role_label(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
        MessageRole::Developer => "developer",
    }
}

fn instruction_role_label(role: &InstructionRole) -> &'static str {
    match role {
        InstructionRole::System => "system",
        InstructionRole::Developer => "developer",
    }
}

fn role_label_messages(role: &MessageRole) -> Result<&'static str, ProtocolAdapterError> {
    match role {
        MessageRole::User => Ok("user"),
        MessageRole::Assistant => Ok("assistant"),
        MessageRole::System | MessageRole::Developer => {
            Err(ProtocolAdapterError::ClientUnrepresentable(
                "Messages cannot preserve a distinct Responses input system/developer role or its position".into(),
            ))
        }
    }
}
