#[path = "responses_ingress.rs"]
mod responses;
use responses::{decode_responses_input, decode_responses_tools};

#[path = "ingress/continuation.rs"]
mod continuation;
pub use continuation::IngressRequestBindings;
use continuation::validate_bindings;

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::content_ref::{ContentRef, JsonValueExt};
use crate::server::core_runtime::model_ir::*;
use crate::server::request_plan::IngressProtocol;

#[path = "responses_options.rs"]
mod responses_options;

pub fn decode_ingress_request(
    protocol: IngressProtocol,
    body: &Value,
) -> Result<ModelRequestIRV1, ModelIrError> {
    decode_ingress_request_with_bindings(protocol, body, &IngressRequestBindings::default())
}

pub fn decode_ingress_request_with_bindings(
    protocol: IngressProtocol,
    body: &Value,
    bindings: &IngressRequestBindings,
) -> Result<ModelRequestIRV1, ModelIrError> {
    validate_bindings(bindings)?;
    if bindings.provider_state_owner.as_ref().is_some_and(|owner| {
        owner.upstream_protocol != protocol
            && !(protocol == IngressProtocol::Messages
                && owner.upstream_protocol == IngressProtocol::Responses)
    }) {
        return Err(ModelIrError::ProviderStateNotPortable);
    }
    let request = match protocol {
        IngressProtocol::Responses => {
            decode_responses(body, bindings.provider_state_owner.as_ref())
        }
        IngressProtocol::ChatCompletions => {
            decode_chat(body, bindings.provider_state_owner.as_ref())
        }
        IngressProtocol::Messages => decode_messages(body, bindings.provider_state_owner.as_ref()),
    }?;
    Ok(request)
}

fn validate_responses_named_choice(
    tools: &responses::DecodedResponsesTools,
    choice: &ToolChoice,
) -> Result<(), ModelIrError> {
    let mut namespace_names = BTreeSet::new();
    for namespace in &tools.namespaces {
        if !namespace_names.insert(namespace.name.as_str()) {
            return Err(ModelIrError::InvalidField("duplicate namespace name"));
        }
        let mut child_names = BTreeSet::new();
        if namespace
            .tools
            .iter()
            .any(|tool| !child_names.insert(tool.name.as_str()))
        {
            return Err(ModelIrError::InvalidField("duplicate namespace tool name"));
        }
    }
    let ToolChoice::RequiredNamed { tool_kind, name } = choice else {
        return Ok(());
    };
    let flat_matches = tools
        .tools
        .iter()
        .filter(|tool| tool.kind == *tool_kind && tool.name == *name)
        .count();
    let namespace_matches = tools
        .namespaces
        .iter()
        .flat_map(|namespace| namespace.tools.iter())
        .filter(|tool| tool.kind == *tool_kind && tool.name == *name)
        .count();
    if flat_matches.saturating_add(namespace_matches) != 1 {
        return Err(ModelIrError::UnsupportedField(
            "ambiguous responses named tool choice".into(),
        ));
    }
    Ok(())
}

fn decode_responses(
    body: &Value,
    state_owner: Option<&ExactProviderPathV1>,
) -> Result<ModelRequestIRV1, ModelIrError> {
    let object = checked_object(
        body,
        &[
            "model",
            "stream",
            "instructions",
            "input",
            "tools",
            "tool_choice",
            "parallel_tool_calls",
            "reasoning",
            "max_output_tokens",
            "previous_response_id",
            "conversation",
            "store",
            "include",
            "prompt_cache_key",
            "client_metadata",
        ],
        "responses request",
    )?;
    let served_model_id = required_string(object, "model")?;
    let instructions = object
        .get("instructions")
        .map(|value| decode_instruction(value, InstructionRole::System))
        .transpose()?
        .into_iter()
        .collect();
    let mut messages = Vec::new();
    let mut additional_tools = Vec::new();
    let mut responses_search_history = std::collections::BTreeMap::new();
    let mut responses_item_statuses = std::collections::BTreeMap::new();
    let mut responses_message_phases = std::collections::BTreeMap::new();
    let mut responses_internal_chat_message_metadata = std::collections::BTreeMap::new();
    let mut responses_reasoning_history = std::collections::BTreeMap::new();
    match object.get("input") {
        Some(Value::String(text)) => messages.push(CanonicalMessage {
            role: MessageRole::User,
            content: vec![ContentPart::Text { text: text.clone() }],
            name: None,
        }),
        Some(Value::Array(items)) => {
            for item in items {
                if item.get("type").and_then(Value::as_str) == Some("additional_tools") {
                    let additional = checked_object(item, &["type", "tools"], "additional_tools")?;
                    additional_tools.push(
                        additional
                            .get("tools")
                            .ok_or(ModelIrError::InvalidField("additional_tools.tools"))?,
                    );
                    continue;
                }
                let message_index = messages.len();
                let internal_metadata = responses_options::internal_chat_message_metadata(item)?;
                let item_type = item.get("type").and_then(Value::as_str);
                if item_type == Some("web_search_call") {
                    let mut native_item = item.clone();
                    native_item
                        .as_object_mut()
                        .ok_or(ModelIrError::InvalidField("web_search_call"))?
                        .remove("internal_chat_message_metadata_passthrough");
                    let call = WebSearchCallV1::from_wire(&native_item)?;
                    if !matches!(
                        call.status,
                        WebSearchStatus::Completed | WebSearchStatus::Failed
                    ) {
                        return Err(ModelIrError::InvalidField("unfinished search history"));
                    }
                    responses_search_history.insert(messages.len(), call);
                    messages.push(CanonicalMessage {
                        role: MessageRole::Assistant,
                        content: Vec::new(),
                        name: None,
                    });
                } else {
                    let item_status = item
                        .get("status")
                        .map(|status| {
                            status
                                .as_str()
                                .ok_or(ModelIrError::InvalidField("Responses input item status"))
                        })
                        .transpose()?;
                    if let Some(status) = item_status
                        && status != "completed"
                        && !(item_type == Some("function_call_output")
                            && matches!(status, "incomplete" | "in_progress"))
                    {
                        return Err(ModelIrError::InvalidField("Responses input item status"));
                    }
                    decode_responses_input(item, &mut messages, state_owner, item_status)?;
                    if let Some(status) = item_status {
                        responses_item_statuses.insert(message_index, status.into());
                    }
                }
                if let Some(metadata) = internal_metadata {
                    responses_internal_chat_message_metadata.insert(message_index, metadata);
                }
                if item_type == Some("message")
                    && let Some(phase) = responses_options::message_phase(item)?
                {
                    responses_message_phases.insert(message_index, phase);
                }
                if item_type == Some("reasoning") {
                    responses_reasoning_history
                        .insert(message_index, responses_options::reasoning_history(item)?);
                }
            }
        }
        Some(_) => return Err(ModelIrError::InvalidField("input")),
        None => return Err(ModelIrError::InvalidField("input")),
    }
    let provider_state = decode_responses_state(object, state_owner)?;
    let tools = decode_responses_tools(object.get("tools"), &additional_tools)?;
    let tool_choice = decode_tool_choice(IngressProtocol::Responses, object.get("tool_choice"))?;
    validate_responses_named_choice(&tools, &tool_choice)?;
    Ok(ModelRequestIRV1 {
        schema_version: MODEL_REQUEST_IR_SCHEMA.into(),
        ingress_protocol: IngressProtocol::Responses,
        responses_options: responses_options::decode(object)?,
        responses_item_ids: responses_options::item_ids(object.get("input"))?,
        served_model_id,
        stream: optional_bool(object, "stream")?.unwrap_or(false),
        instructions,
        messages,
        tools: tools.tools,
        tool_namespaces: tools.namespaces,
        responses_tool_order: tools.order,
        web_search: tools.web_search,
        responses_search_history,
        responses_annotations: responses_options::annotations(object.get("input"))?,
        responses_item_statuses,
        responses_message_phases,
        responses_internal_chat_message_metadata,
        responses_reasoning_history,
        tool_choice,
        parallel_tool_calls: optional_bool(object, "parallel_tool_calls")?.unwrap_or(false),
        requested_reasoning: object
            .get("reasoning")
            .cloned()
            .map(RequestedReasoningControl::overridden)
            .unwrap_or_else(RequestedReasoningControl::absent),
        requested_max_output_tokens: optional_u64(object, "max_output_tokens")?,
        provider_state,
    })
}

fn decode_chat(
    body: &Value,
    state_owner: Option<&ExactProviderPathV1>,
) -> Result<ModelRequestIRV1, ModelIrError> {
    let object = checked_object(
        body,
        &[
            "model",
            "stream",
            "messages",
            "tools",
            "tool_choice",
            "parallel_tool_calls",
            "reasoning_effort",
            "max_tokens",
            "max_completion_tokens",
            "stream_options",
        ],
        "chat request",
    )?;
    if let Some(stream_options) = object.get("stream_options") {
        let options = checked_object(stream_options, &["include_usage"], "stream_options")?;
        if optional_bool(options, "include_usage")? == Some(false) {
            return Err(ModelIrError::UnsupportedValue(
                "stream_options.include_usage=false".into(),
            ));
        }
    }
    let mut instructions = Vec::new();
    let mut messages = Vec::new();
    for value in required_array(object, "messages")? {
        decode_chat_message(value, &mut instructions, &mut messages, state_owner)?;
    }
    let requested_reasoning = object
        .get("reasoning_effort")
        .cloned()
        .map(RequestedReasoningControl::overridden)
        .unwrap_or_else(RequestedReasoningControl::absent);
    Ok(ModelRequestIRV1 {
        schema_version: MODEL_REQUEST_IR_SCHEMA.into(),
        ingress_protocol: IngressProtocol::ChatCompletions,
        responses_options: None,
        responses_item_ids: Default::default(),
        responses_item_statuses: Default::default(),
        served_model_id: required_string(object, "model")?,
        stream: optional_bool(object, "stream")?.unwrap_or(false),
        instructions,
        messages,
        tools: decode_chat_tools(object.get("tools"))?,
        tool_namespaces: Vec::new(),
        responses_tool_order: Vec::new(),
        web_search: None,
        responses_search_history: Default::default(),
        responses_annotations: Default::default(),
        responses_message_phases: Default::default(),
        responses_internal_chat_message_metadata: Default::default(),
        responses_reasoning_history: Default::default(),
        tool_choice: decode_tool_choice(
            IngressProtocol::ChatCompletions,
            object.get("tool_choice"),
        )?,
        parallel_tool_calls: optional_bool(object, "parallel_tool_calls")?.unwrap_or(false),
        requested_reasoning,
        requested_max_output_tokens: compatible_max_output(object)?,
        provider_state: Vec::new(),
    })
}

fn decode_chat_message(
    value: &Value,
    instructions: &mut Vec<CanonicalInstruction>,
    messages: &mut Vec<CanonicalMessage>,
    state_owner: Option<&ExactProviderPathV1>,
) -> Result<(), ModelIrError> {
    let object = checked_object(
        value,
        &[
            "role",
            "content",
            "name",
            "tool_calls",
            "tool_call_id",
            "reasoning_content",
        ],
        "chat message",
    )?;
    let role = required_string(object, "role")?;
    if role == "system" || role == "developer" {
        ensure_absent(
            object,
            &["name", "tool_calls", "tool_call_id", "reasoning_content"],
            "chat instruction",
        )?;
        let instruction_role = if role == "system" {
            InstructionRole::System
        } else {
            InstructionRole::Developer
        };
        let content = object
            .get("content")
            .ok_or(ModelIrError::InvalidField("content"))?;
        let instruction = decode_instruction(content, instruction_role)?;
        if messages.is_empty() {
            instructions.push(instruction);
        } else {
            messages.push(CanonicalMessage {
                role: if role == "system" {
                    MessageRole::System
                } else {
                    MessageRole::Developer
                },
                content: instruction.content,
                name: None,
            });
        }
        return Ok(());
    }
    if role == "tool" {
        ensure_absent(
            object,
            &["tool_calls", "reasoning_content"],
            "chat Tool message",
        )?;
        messages.push(CanonicalMessage {
            role: MessageRole::User,
            content: vec![ContentPart::ToolResult {
                logical_id: required_string(object, "tool_call_id")?,
                tool_kind: ToolKindV1::Function,
                output: decode_tool_output(
                    object
                        .get("content")
                        .ok_or(ModelIrError::InvalidField("content"))?,
                ),
                status: ToolResultStatusV1::Unknown,
            }],
            name: optional_string(object, "name")?,
        });
        return Ok(());
    }
    let mut content = object
        .get("content")
        .map(decode_chat_content)
        .transpose()?
        .unwrap_or_default();
    if let Some(reasoning) = object.get("reasoning_content") {
        if role != "assistant" || (!reasoning.is_string() && reasoning.content_ref().is_none()) {
            return Err(ModelIrError::InvalidField("reasoning_content"));
        }
        content.push(ContentPart::ProviderState {
            state: Box::new(OpaqueProviderState {
                owner: require_state_owner(state_owner)?,
                block_index: None,
                kind: "reasoning_content".into(),
                value: reasoning.clone(),
            }),
        });
    }
    if let Some(tool_calls) = object.get("tool_calls") {
        if role != "assistant" {
            return Err(ModelIrError::InvalidField("tool_calls"));
        }
        for value in tool_calls
            .as_array()
            .ok_or(ModelIrError::InvalidField("tool_calls"))?
        {
            let call = checked_object(value, &["id", "type", "function"], "chat tool call")?;
            if required_string(call, "type")? != "function" {
                return Err(ModelIrError::UnsupportedValue(
                    "chat non-function Tool".into(),
                ));
            }
            let function = checked_object(
                call.get("function")
                    .ok_or(ModelIrError::InvalidField("function"))?,
                &["name", "arguments"],
                "chat function",
            )?;
            content.push(ContentPart::ToolCall {
                logical_id: required_string(call, "id")?,
                tool_kind: ToolKindV1::Function,
                namespace: None,
                name: required_string(function, "name")?,
                arguments: parse_json_string(required_string(function, "arguments")?, "arguments")?,
            });
        }
    }
    messages.push(CanonicalMessage {
        role: decode_role(&role)?,
        content,
        name: optional_string(object, "name")?,
    });
    Ok(())
}

fn decode_chat_content(value: &Value) -> Result<Vec<ContentPart>, ModelIrError> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::String(text) => Ok(vec![ContentPart::Text { text: text.clone() }]),
        Value::Array(values) => values
            .iter()
            .map(|value| {
                let object = checked_object(value, &["type", "text", "image_url"], "chat content")?;
                match required_string(object, "type")?.as_str() {
                    "text" => Ok(ContentPart::Text {
                        text: required_string(object, "text")?,
                    }),
                    "image_url" => {
                        let image = checked_object(
                            object
                                .get("image_url")
                                .ok_or(ModelIrError::InvalidField("image_url"))?,
                            &["url"],
                            "chat image_url",
                        )?;
                        Ok(ContentPart::Image {
                            source: decode_image_url(required_string(image, "url")?)?,
                        })
                    }
                    other => Err(ModelIrError::UnsupportedValue(format!(
                        "chat content {other}"
                    ))),
                }
            })
            .collect(),
        _ => Err(ModelIrError::InvalidField("content")),
    }
}

fn decode_messages(
    body: &Value,
    state_owner: Option<&ExactProviderPathV1>,
) -> Result<ModelRequestIRV1, ModelIrError> {
    let object = checked_object(
        body,
        &[
            "model",
            "stream",
            "max_tokens",
            "system",
            "messages",
            "tools",
            "tool_choice",
            "thinking",
            "output_config",
            "metadata",
            "context_management",
        ],
        "messages request",
    )?;
    validate_messages_metadata(object.get("metadata"))?;
    validate_messages_context_management(object.get("context_management"))?;
    let mut instructions = object
        .get("system")
        .map(|value| decode_instruction(value, InstructionRole::System))
        .transpose()?
        .into_iter()
        .collect::<Vec<_>>();
    let mut messages = Vec::new();
    for value in required_array(object, "messages")? {
        let message = decode_messages_message(value, state_owner)?;
        match message.role {
            // Claude Code 2.1.231 emits an instruction reminder as a `system` message even though
            // the public Messages wire represents instructions at the top level. Normalize this
            // client extension back into the canonical instruction layer so both a Messages
            // target and a cross-protocol target preserve its authority without inventing an
            // unsupported mid-conversation Messages role.
            MessageRole::System | MessageRole::Developer => {
                instructions.push(CanonicalInstruction {
                    role: if message.role == MessageRole::System {
                        InstructionRole::System
                    } else {
                        InstructionRole::Developer
                    },
                    content: message.content,
                });
            }
            MessageRole::User | MessageRole::Assistant => messages.push(message),
        }
    }
    let requested_reasoning = match (object.get("thinking"), object.get("output_config")) {
        (None, None) => RequestedReasoningControl::absent(),
        (thinking, output_config) => RequestedReasoningControl::overridden(serde_json::json!({
            "thinking": thinking.cloned(),
            "output_config": output_config.cloned(),
        })),
    };
    Ok(ModelRequestIRV1 {
        schema_version: MODEL_REQUEST_IR_SCHEMA.into(),
        ingress_protocol: IngressProtocol::Messages,
        responses_options: None,
        responses_item_ids: Default::default(),
        responses_item_statuses: Default::default(),
        served_model_id: required_string(object, "model")?,
        stream: optional_bool(object, "stream")?.unwrap_or(false),
        instructions,
        messages,
        tools: decode_messages_tools(object.get("tools"))?,
        tool_namespaces: Vec::new(),
        responses_tool_order: Vec::new(),
        web_search: None,
        responses_search_history: Default::default(),
        responses_annotations: Default::default(),
        responses_message_phases: Default::default(),
        responses_internal_chat_message_metadata: Default::default(),
        responses_reasoning_history: Default::default(),
        tool_choice: decode_tool_choice(IngressProtocol::Messages, object.get("tool_choice"))?,
        parallel_tool_calls: decode_messages_parallel(object.get("tool_choice"))?,
        requested_reasoning,
        requested_max_output_tokens: optional_u64(object, "max_tokens")?,
        provider_state: Vec::new(),
    })
}

fn decode_messages_message(
    value: &Value,
    state_owner: Option<&ExactProviderPathV1>,
) -> Result<CanonicalMessage, ModelIrError> {
    let object = checked_object(value, &["role", "content"], "messages message")?;
    let role = decode_role(required_string(object, "role")?.as_str())?;
    let values = object
        .get("content")
        .ok_or(ModelIrError::InvalidField("content"))?;
    let content = match values {
        Value::String(text) => vec![ContentPart::Text { text: text.clone() }],
        Value::Array(values) => values
            .iter()
            .map(|value| decode_messages_content(value, state_owner))
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(ModelIrError::InvalidField("content")),
    };
    Ok(CanonicalMessage {
        role,
        content,
        name: None,
    })
}

fn decode_messages_content(
    value: &Value,
    state_owner: Option<&ExactProviderPathV1>,
) -> Result<ContentPart, ModelIrError> {
    let object = value
        .as_object()
        .ok_or(ModelIrError::InvalidField("messages content"))?;
    match required_string(object, "type")?.as_str() {
        "text" => {
            ensure_keys(object, &["type", "text", "cache_control"], "messages text")?;
            validate_messages_cache_control(object.get("cache_control"))?;
            Ok(ContentPart::Text {
                text: required_string(object, "text")?,
            })
        }
        "image" => {
            ensure_keys(object, &["type", "source"], "messages image")?;
            let source = checked_object(
                object
                    .get("source")
                    .ok_or(ModelIrError::InvalidField("source"))?,
                &["type", "media_type", "data", "url"],
                "messages image source",
            )?;
            let source = match required_string(source, "type")?.as_str() {
                "base64" => ImageSource::Base64 {
                    media_type: required_string(source, "media_type")?,
                    data: required_string(source, "data")?,
                },
                "url" => ImageSource::Url {
                    url: decode_http_image_url(required_string(source, "url")?)?,
                },
                other => {
                    return Err(ModelIrError::UnsupportedValue(format!(
                        "messages image source {other}"
                    )));
                }
            };
            Ok(ContentPart::Image { source })
        }
        "tool_use" => {
            ensure_keys(
                object,
                &["type", "id", "name", "input"],
                "messages tool_use",
            )?;
            Ok(ContentPart::ToolCall {
                logical_id: required_string(object, "id")?,
                tool_kind: ToolKindV1::Function,
                namespace: None,
                name: required_string(object, "name")?,
                arguments: object
                    .get("input")
                    .cloned()
                    .ok_or(ModelIrError::InvalidField("input"))?,
            })
        }
        "tool_result" => {
            ensure_keys(
                object,
                &[
                    "type",
                    "tool_use_id",
                    "content",
                    "is_error",
                    "cache_control",
                ],
                "messages tool_result",
            )?;
            validate_messages_cache_control(object.get("cache_control"))?;
            Ok(ContentPart::ToolResult {
                logical_id: required_string(object, "tool_use_id")?,
                tool_kind: ToolKindV1::Function,
                output: decode_tool_output(
                    object
                        .get("content")
                        .ok_or(ModelIrError::InvalidField("content"))?,
                ),
                status: if optional_bool(object, "is_error")?.unwrap_or(false) {
                    ToolResultStatusV1::Failed
                } else {
                    ToolResultStatusV1::Completed
                },
            })
        }
        "thinking" | "redacted_thinking" => {
            let owner = require_state_owner(state_owner)?;
            if owner.upstream_protocol == IngressProtocol::Responses {
                if required_string(object, "type")? != "thinking" {
                    return Err(ModelIrError::ProviderStateNotPortable);
                }
                ensure_keys(
                    object,
                    &["type", "thinking", "signature"],
                    "messages thinking",
                )?;
                let signature = required_string(object, "signature")?;
                if signature.is_empty() {
                    return Err(ModelIrError::InvalidField("thinking signature"));
                }
                Ok(ContentPart::ProviderState {
                    state: Box::new(OpaqueProviderState {
                        owner,
                        block_index: None,
                        kind: "encrypted_content".into(),
                        value: Value::String(signature),
                    }),
                })
            } else {
                Ok(ContentPart::ProviderState {
                    state: Box::new(OpaqueProviderState {
                        owner,
                        block_index: None,
                        kind: required_string(object, "type")?,
                        value: value.clone(),
                    }),
                })
            }
        }
        other => Err(ModelIrError::UnsupportedValue(format!(
            "messages content {other}"
        ))),
    }
}

fn decode_instruction(
    value: &Value,
    role: InstructionRole,
) -> Result<CanonicalInstruction, ModelIrError> {
    let content = match value {
        Value::String(text) => vec![ContentPart::Text { text: text.clone() }],
        Value::Array(values) => values
            .iter()
            .map(|value| {
                let object = checked_object(
                    value,
                    &["type", "text", "cache_control"],
                    "instruction content",
                )?;
                if required_string(object, "type")? != "text" {
                    return Err(ModelIrError::UnsupportedValue(
                        "non-text instruction".into(),
                    ));
                }
                validate_messages_cache_control(object.get("cache_control"))?;
                Ok(ContentPart::Text {
                    text: required_string(object, "text")?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(ModelIrError::InvalidField("instructions")),
    };
    Ok(CanonicalInstruction { role, content })
}

// Claude Code sends these standard Messages transport hints even for a minimal prompt. They do
// not alter the logical task and are intentionally consumed at ingress: HiRoute owns cache
// affinity across candidates, while forwarding an Anthropic user identifier to another provider
// would be incorrect. Their shapes remain fail-closed so an unknown semantic field cannot be
// silently erased.
fn validate_messages_metadata(value: Option<&Value>) -> Result<(), ModelIrError> {
    let Some(value) = value else {
        return Ok(());
    };
    let object = checked_object(value, &["user_id"], "messages metadata")?;
    if let Some(user_id) = object.get("user_id")
        && !user_id.is_string()
    {
        return Err(ModelIrError::InvalidField("metadata.user_id"));
    }
    Ok(())
}

fn validate_messages_context_management(value: Option<&Value>) -> Result<(), ModelIrError> {
    let Some(value) = value else {
        return Ok(());
    };
    let object = checked_object(value, &["edits"], "messages context_management")?;
    let edits = object
        .get("edits")
        .and_then(Value::as_array)
        .ok_or(ModelIrError::InvalidField("context_management.edits"))?;
    if edits.len() != 1 {
        return Err(ModelIrError::UnsupportedValue(
            "messages context_management edits".into(),
        ));
    }
    let edit = checked_object(
        &edits[0],
        &["type", "keep"],
        "messages context_management edit",
    )?;
    if required_string(edit, "type")? != "clear_thinking_20251015"
        || required_string(edit, "keep")? != "all"
    {
        return Err(ModelIrError::UnsupportedValue(
            "messages context_management edit".into(),
        ));
    }

    // Claude Code emits this exact hint even when it retains every thinking block. Because
    // `keep: all` performs no edit, it is safe to consume before a cross-protocol projection.
    // Every behavior-changing context-management shape remains fail-closed above.
    Ok(())
}

fn validate_messages_cache_control(value: Option<&Value>) -> Result<(), ModelIrError> {
    let Some(value) = value else {
        return Ok(());
    };
    let object = checked_object(value, &["type", "ttl"], "messages cache_control")?;
    if required_string(object, "type")? != "ephemeral" {
        return Err(ModelIrError::UnsupportedValue(
            "messages cache_control type".into(),
        ));
    }
    if let Some(ttl) = object.get("ttl").map(Value::as_str) {
        match ttl {
            Some("5m" | "1h") => {}
            _ => return Err(ModelIrError::InvalidField("cache_control.ttl")),
        }
    }
    Ok(())
}

fn decode_chat_tools(value: Option<&Value>) -> Result<Vec<CanonicalTool>, ModelIrError> {
    decode_tools(value, |object| {
        ensure_keys(object, &["type", "function"], "chat Tool")?;
        if required_string(object, "type")? != "function" {
            return Err(ModelIrError::UnsupportedValue("chat hosted Tool".into()));
        }
        let function = checked_object(
            object
                .get("function")
                .ok_or(ModelIrError::InvalidField("function"))?,
            &["name", "description", "parameters", "strict"],
            "chat function Tool",
        )?;
        Ok(CanonicalTool {
            kind: ToolKindV1::Function,
            name: required_string(function, "name")?,
            description: optional_string(function, "description")?,
            input_schema: Some(
                function
                    .get("parameters")
                    .cloned()
                    .ok_or(ModelIrError::InvalidField("parameters"))?,
            ),
            strict: optional_bool(function, "strict")?,
            format: None,
        })
    })
}

fn decode_messages_tools(value: Option<&Value>) -> Result<Vec<CanonicalTool>, ModelIrError> {
    decode_tools(value, |object| {
        ensure_keys(
            object,
            &["name", "description", "input_schema", "strict"],
            "messages Tool",
        )?;
        Ok(CanonicalTool {
            kind: ToolKindV1::Function,
            name: required_string(object, "name")?,
            description: optional_string(object, "description")?,
            input_schema: Some(
                object
                    .get("input_schema")
                    .cloned()
                    .ok_or(ModelIrError::InvalidField("input_schema"))?,
            ),
            strict: optional_bool(object, "strict")?,
            format: None,
        })
    })
}

fn decode_tools(
    value: Option<&Value>,
    decode: impl Fn(&Map<String, Value>) -> Result<CanonicalTool, ModelIrError>,
) -> Result<Vec<CanonicalTool>, ModelIrError> {
    value
        .map(|value| {
            value
                .as_array()
                .ok_or(ModelIrError::InvalidField("tools"))?
                .iter()
                .map(|value| {
                    decode(
                        value
                            .as_object()
                            .ok_or(ModelIrError::InvalidField("tools[]"))?,
                    )
                })
                .collect()
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

fn decode_tool_choice(
    protocol: IngressProtocol,
    value: Option<&Value>,
) -> Result<ToolChoice, ModelIrError> {
    let Some(value) = value else {
        return Ok(ToolChoice::Auto);
    };
    if let Some(value) = value.as_str() {
        return match value {
            "none" => Ok(ToolChoice::None),
            "auto" => Ok(ToolChoice::Auto),
            "required" | "any" => Ok(ToolChoice::RequiredAny),
            other => Err(ModelIrError::UnsupportedValue(format!(
                "Tool choice {other}"
            ))),
        };
    }
    let object = value
        .as_object()
        .ok_or(ModelIrError::InvalidField("tool_choice"))?;
    match protocol {
        IngressProtocol::Messages => {
            ensure_keys(
                object,
                &["type", "name", "disable_parallel_tool_use"],
                "messages tool_choice",
            )?;
            match required_string(object, "type")?.as_str() {
                "auto" => Ok(ToolChoice::Auto),
                "any" => Ok(ToolChoice::RequiredAny),
                "none" => Ok(ToolChoice::None),
                "tool" => Ok(ToolChoice::RequiredNamed {
                    tool_kind: ToolKindV1::Function,
                    name: required_string(object, "name")?,
                }),
                other => Err(ModelIrError::UnsupportedValue(format!(
                    "messages Tool choice {other}"
                ))),
            }
        }
        IngressProtocol::Responses => {
            ensure_keys(object, &["type", "name"], "responses tool_choice")?;
            let kind = match required_string(object, "type")?.as_str() {
                "function" => ToolKindV1::Function,
                "custom" => ToolKindV1::Custom,
                other => {
                    return Err(ModelIrError::UnsupportedValue(format!(
                        "responses Tool choice {other}"
                    )));
                }
            };
            Ok(ToolChoice::RequiredNamed {
                tool_kind: kind,
                name: required_string(object, "name")?,
            })
        }
        IngressProtocol::ChatCompletions => {
            ensure_keys(object, &["type", "function"], "chat tool_choice")?;
            if required_string(object, "type")? != "function" {
                return Err(ModelIrError::UnsupportedValue(
                    "chat non-function Tool choice".into(),
                ));
            }
            let function = checked_object(
                object
                    .get("function")
                    .ok_or(ModelIrError::InvalidField("function"))?,
                &["name"],
                "chat tool_choice function",
            )?;
            Ok(ToolChoice::RequiredNamed {
                tool_kind: ToolKindV1::Function,
                name: required_string(function, "name")?,
            })
        }
    }
}

fn decode_responses_state(
    object: &Map<String, Value>,
    state_owner: Option<&ExactProviderPathV1>,
) -> Result<Vec<OpaqueProviderState>, ModelIrError> {
    let mut state = Vec::new();
    if let Some(value) = object.get("previous_response_id") {
        match value {
            Value::Null => {}
            Value::String(_) | Value::Object(_) => {
                return Err(ModelIrError::ResponsesPreviousResponseIdUnsupported);
            }
            Value::Bool(_) | Value::Number(_) | Value::Array(_) => {
                return Err(ModelIrError::InvalidField("previous_response_id"));
            }
        }
    }
    if let Some(value) = object.get("conversation") {
        if !value.is_string() && !value.is_object() {
            return Err(ModelIrError::InvalidField("conversation"));
        }
        state.push(OpaqueProviderState {
            owner: require_state_owner(state_owner)?,
            block_index: None,
            kind: "conversation".into(),
            value: value.clone(),
        });
    }
    Ok(state)
}

fn require_state_owner(
    owner: Option<&ExactProviderPathV1>,
) -> Result<ExactProviderPathV1, ModelIrError> {
    owner
        .filter(|owner| owner.is_complete())
        .cloned()
        .ok_or(ModelIrError::ProviderStateOwnershipRequired)
}

fn decode_image_url(value: String) -> Result<ImageSource, ModelIrError> {
    if let Some(rest) = value.strip_prefix("data:") {
        let (media_type, data) = rest
            .split_once(";base64,")
            .ok_or(ModelIrError::InvalidField("image_url"))?;
        if media_type.is_empty() || data.is_empty() {
            return Err(ModelIrError::InvalidField("image_url"));
        }
        Ok(ImageSource::Base64 {
            media_type: media_type.into(),
            data: data.into(),
        })
    } else {
        Ok(ImageSource::Url {
            url: decode_http_image_url(value)?,
        })
    }
}

fn decode_http_image_url(value: String) -> Result<String, ModelIrError> {
    if ContentRef::from_wire_marker(&value).is_some()
        || value.starts_with("https://")
        || value.starts_with("http://")
    {
        return Ok(value);
    }
    Err(ModelIrError::UnsupportedValue(
        "non-HTTP image source".into(),
    ))
}

fn decode_tool_output(value: &Value) -> ToolOutput {
    match value {
        Value::String(value) => ToolOutput::Text(value.clone()),
        value => ToolOutput::Json(value.clone()),
    }
}

fn parse_json_string(value: String, field: &'static str) -> Result<Value, ModelIrError> {
    if let Some(reference) = ContentRef::from_wire_marker(&value) {
        return Ok(reference.json_marker());
    }
    serde_json::from_str(&value).map_err(|_| ModelIrError::InvalidField(field))
}

fn decode_role(value: &str) -> Result<MessageRole, ModelIrError> {
    match value {
        "user" => Ok(MessageRole::User),
        "assistant" => Ok(MessageRole::Assistant),
        "system" => Ok(MessageRole::System),
        "developer" => Ok(MessageRole::Developer),
        other => Err(ModelIrError::UnsupportedValue(format!("role {other}"))),
    }
}

fn checked_object<'a>(
    value: &'a Value,
    keys: &[&str],
    label: &str,
) -> Result<&'a Map<String, Value>, ModelIrError> {
    let object = value.as_object().ok_or(ModelIrError::ExpectedObject)?;
    ensure_keys(object, keys, label)?;
    Ok(object)
}

fn ensure_keys(
    object: &Map<String, Value>,
    keys: &[&str],
    label: &str,
) -> Result<(), ModelIrError> {
    let allowed = keys.iter().copied().collect::<BTreeSet<_>>();
    if let Some(key) = object.keys().find(|key| !allowed.contains(key.as_str())) {
        return Err(ModelIrError::UnsupportedField(format!("{label}.{key}")));
    }
    Ok(())
}

fn ensure_absent(
    object: &Map<String, Value>,
    fields: &[&str],
    label: &str,
) -> Result<(), ModelIrError> {
    if let Some(field) = fields.iter().find(|field| object.contains_key(**field)) {
        return Err(ModelIrError::UnsupportedField(format!("{label}.{field}")));
    }
    Ok(())
}

fn compatible_max_output(object: &Map<String, Value>) -> Result<Option<u64>, ModelIrError> {
    let completion = optional_u64(object, "max_completion_tokens")?;
    let legacy = optional_u64(object, "max_tokens")?;
    if completion.is_some() && legacy.is_some() && completion != legacy {
        return Err(ModelIrError::UnsupportedValue(
            "conflicting Chat max output fields".into(),
        ));
    }
    Ok(completion.or(legacy))
}

fn decode_messages_parallel(value: Option<&Value>) -> Result<bool, ModelIrError> {
    let Some(object) = value.and_then(Value::as_object) else {
        return Ok(false);
    };
    Ok(!optional_bool(object, "disable_parallel_tool_use")?.unwrap_or(true))
}

fn required_string(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<String, ModelIrError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or(ModelIrError::InvalidField(field))
}

fn optional_string(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, ModelIrError> {
    object
        .get(field)
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or(ModelIrError::InvalidField(field))
        })
        .transpose()
}

fn optional_bool(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<bool>, ModelIrError> {
    object
        .get(field)
        .map(|value| value.as_bool().ok_or(ModelIrError::InvalidField(field)))
        .transpose()
}

fn optional_u64(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<u64>, ModelIrError> {
    object
        .get(field)
        .map(|value| value.as_u64().ok_or(ModelIrError::InvalidField(field)))
        .transpose()
}

fn required_array<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a Vec<Value>, ModelIrError> {
    object
        .get(field)
        .and_then(Value::as_array)
        .ok_or(ModelIrError::InvalidField(field))
}
