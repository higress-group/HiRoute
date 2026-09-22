use std::collections::{BTreeMap, VecDeque};

use serde_json::{Map, Value};

use crate::server::core_runtime::model_ir::{
    ExactProviderPathV1, FinishReason, ModelIrError, ModelStreamEventV1, ToolKindV1,
};
use crate::server::core_runtime::profiles::NativeProviderStateEmission;
use crate::server::request_plan::IngressProtocol;

use super::messages::{self, MessagesState};
use super::responses_lifecycle;
use super::wire::*;
use super::{DecoderCore, ProtocolAdapterError};
use crate::server::core_runtime::adapters::{ChatToolIdentity, ChatToolProjection};

#[derive(Clone, Debug)]
pub(super) enum ProtocolState {
    Responses {
        core: DecoderCore,
        tools: BTreeMap<u32, ResponsesToolIdentity>,
        done_seen: bool,
    },
    Chat {
        core: DecoderCore,
        tools: BTreeMap<u32, ChatToolCallState>,
        projection: Option<ChatToolProjection>,
    },
    Messages {
        core: DecoderCore,
        state: MessagesState,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ResponsesToolIdentity {
    pub(super) native_id: String,
    pub(super) kind: ToolKindV1,
    pub(super) namespace: Option<String>,
    pub(super) name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ChatToolCallState {
    native_id: String,
    identity: ChatToolIdentity,
    wire_arguments: String,
    finished: bool,
}

impl ProtocolState {
    pub(super) fn new(
        protocol: IngressProtocol,
        owner: ExactProviderPathV1,
        state_emission: NativeProviderStateEmission,
        terminal_refusal_buffer: usize,
        terminal_refusal_blocks: u32,
        tool_id_projection: Option<super::super::continuation::ToolIdProjection>,
        chat_tool_projection: Option<ChatToolProjection>,
    ) -> Self {
        match protocol {
            IngressProtocol::Responses => Self::Responses {
                core: DecoderCore::new(owner, state_emission, tool_id_projection),
                tools: BTreeMap::new(),
                done_seen: false,
            },
            IngressProtocol::ChatCompletions => Self::Chat {
                core: DecoderCore::new(owner, state_emission, tool_id_projection),
                tools: BTreeMap::new(),
                projection: chat_tool_projection,
            },
            IngressProtocol::Messages => Self::Messages {
                core: DecoderCore::new(owner, state_emission, tool_id_projection),
                state: MessagesState::new(terminal_refusal_buffer, terminal_refusal_blocks),
            },
        }
    }

    pub(super) fn decode_sse(
        &mut self,
        event_type: Option<&str>,
        data: &[u8],
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        match self {
            Self::Responses {
                core,
                tools,
                done_seen,
            } => {
                if data == b"[DONE]" {
                    if event_type.is_some() || !core.accumulator.terminal || *done_seen {
                        return Err(ModelIrError::InvalidResponseLifecycle(
                            "Responses [DONE] must follow one terminal event".into(),
                        )
                        .into());
                    }
                    *done_seen = true;
                    return Ok(());
                }
                decode_responses_sse(core, tools, event_type, data, output)
            }
            Self::Chat {
                core,
                tools,
                projection,
            } => decode_chat_sse(core, tools, projection.as_ref(), data, output),
            Self::Messages { core, state } => {
                messages::decode_sse(core, state, event_type, data, output)
            }
        }
    }

    pub(super) fn decode_nonstream(
        &mut self,
        status: u16,
        body: &[u8],
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        let value: Value = serde_json::from_slice(body)
            .map_err(|error| ModelIrError::InvalidJson(error.to_string()))?;
        if !(200..300).contains(&status) {
            return self.fail_http(status, &value, output);
        }
        match self {
            Self::Responses { core, .. } => decode_responses_nonstream(core, &value, output),
            Self::Chat {
                core, projection, ..
            } => decode_chat_nonstream(core, projection.as_ref(), &value, output),
            Self::Messages { core, .. } => messages::decode_nonstream(core, &value, output),
        }
    }

    pub(super) fn fail_http(
        &mut self,
        status: u16,
        value: &Value,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        let error = decode_error(value, Some(status))?;
        self.core_mut().fail(error, output)
    }

    pub(super) fn core(&self) -> &DecoderCore {
        match self {
            Self::Responses { core, .. }
            | Self::Chat { core, .. }
            | Self::Messages { core, .. } => core,
        }
    }

    pub(super) fn into_core(self) -> DecoderCore {
        match self {
            Self::Responses { core, .. }
            | Self::Chat { core, .. }
            | Self::Messages { core, .. } => core,
        }
    }

    fn core_mut(&mut self) -> &mut DecoderCore {
        match self {
            Self::Responses { core, .. }
            | Self::Chat { core, .. }
            | Self::Messages { core, .. } => core,
        }
    }
}

fn decode_responses_sse(
    core: &mut DecoderCore,
    tools: &mut BTreeMap<u32, ResponsesToolIdentity>,
    event_type: Option<&str>,
    data: &[u8],
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
    let value = parse_data(data)?;
    let object = checked_object(&value)?;
    let native_type = required_str(object, "type")?;
    if let Some(sse_type) = event_type
        && sse_type != native_type
    {
        return Err(ModelIrError::InvalidResponseLifecycle(
            "Responses SSE event field disagrees with JSON type".into(),
        )
        .into());
    }
    match native_type {
        "response.created" | "response.in_progress" => {
            allow(object, &["type", "sequence_number", "response"])?;
            let response = object_field(object, "response")?;
            validate_responses_envelope_metadata(response)?;
            core.responses_metadata(response, output)?;
            core.start_response(
                required_str(response, "id")?.into(),
                required_str(response, "model")?.into(),
                output,
            )?;
        }
        "response.output_item.added" => {
            allow(object, &["type", "sequence_number", "output_index", "item"])?;
            let native_index = required_u32(object, "output_index")?;
            let item = object_field(object, "item")?;
            match required_str(item, "type")? {
                "web_search_call" => core.search_item(
                    native_index,
                    &Value::Object(item.clone()),
                    crate::server::core_runtime::model_ir::WebSearchPhase::Added,
                    output,
                )?,
                "function_call" | "custom_tool_call" => {
                    let kind = if required_str(item, "type")? == "function_call" {
                        ToolKindV1::Function
                    } else {
                        ToolKindV1::Custom
                    };
                    let payload_field = match kind {
                        ToolKindV1::Function => "arguments",
                        ToolKindV1::Custom => "input",
                    };
                    allow(
                        item,
                        &[
                            "type",
                            "id",
                            "call_id",
                            "namespace",
                            "name",
                            payload_field,
                            "status",
                        ],
                    )?;
                    let identity = ResponsesToolIdentity {
                        native_id: required_str(item, "call_id")?.into(),
                        kind,
                        namespace: optional_str(item, "namespace")?.map(str::to_owned),
                        name: required_str(item, "name")?.into(),
                    };
                    responses_lifecycle::register_item(core, native_index, item)?;
                    if tools.insert(native_index, identity.clone()).is_some() {
                        return Err(ModelIrError::InvalidResponseLifecycle(
                            "Responses output item index reused".into(),
                        )
                        .into());
                    }
                    core.start_tool(
                        native_index,
                        identity.native_id,
                        identity.kind,
                        identity.namespace,
                        identity.name,
                        output,
                    )?;
                }
                "message" => {
                    allow(item, &["type", "id", "status", "role", "content", "phase"])?;
                    if item
                        .get("content")
                        .and_then(Value::as_array)
                        .is_some_and(|content| !content.is_empty())
                    {
                        return Err(ModelIrError::UnsupportedField(
                            "response.output_item.added.item.content".into(),
                        )
                        .into());
                    }
                    responses_lifecycle::register_item(core, native_index, item)?;
                    core.observe_message_phase(native_index, optional_str(item, "phase")?, false)?;
                }
                "reasoning" => {
                    allow(
                        item,
                        &[
                            "type",
                            "id",
                            "status",
                            "summary",
                            "content",
                            "encrypted_content",
                        ],
                    )?;
                    if item
                        .get("summary")
                        .and_then(Value::as_array)
                        .is_some_and(|summary| !summary.is_empty())
                    {
                        return Err(ModelIrError::UnsupportedField(
                            "response.output_item.added.item.summary".into(),
                        )
                        .into());
                    }
                    if item
                        .get("content")
                        .and_then(Value::as_array)
                        .is_some_and(|content| !content.is_empty())
                    {
                        return Err(ModelIrError::UnsupportedField(
                            "response.output_item.added.item.content".into(),
                        )
                        .into());
                    }
                    responses_lifecycle::register_item(core, native_index, item)?;
                    if let Some(encrypted) = item.get("encrypted_content")
                        && !encrypted.is_null()
                    {
                        core.observe_responses_encrypted_fallback(native_index, encrypted.clone())?;
                    }
                }
                other => return Err(unsupported("Responses output item", other)),
            }
        }
        "response.web_search_call.in_progress"
        | "response.web_search_call.searching"
        | "response.web_search_call.completed" => {
            allow(
                object,
                &["type", "sequence_number", "output_index", "item_id"],
            )?;
            use crate::server::core_runtime::model_ir::WebSearchPhase;
            let phase = match native_type {
                "response.web_search_call.in_progress" => WebSearchPhase::InProgress,
                "response.web_search_call.searching" => WebSearchPhase::Searching,
                _ => WebSearchPhase::Completed,
            };
            core.search_progress(
                required_u32(object, "output_index")?,
                required_str(object, "item_id")?,
                phase,
                output,
            )?;
        }
        "response.output_item.done"
            if object
                .get("item")
                .and_then(|item| item.get("type"))
                .and_then(Value::as_str)
                == Some("web_search_call") =>
        {
            allow(object, &["type", "sequence_number", "output_index", "item"])?;
            core.search_item(
                required_u32(object, "output_index")?,
                &object["item"],
                crate::server::core_runtime::model_ir::WebSearchPhase::Done,
                output,
            )?;
        }
        "response.output_text.annotation.added" => {
            allow(
                object,
                &[
                    "type",
                    "sequence_number",
                    "item_id",
                    "output_index",
                    "content_index",
                    "annotation_index",
                    "annotation",
                ],
            )?;
            if required_u32(object, "content_index")? != 0 {
                return Err(ModelIrError::InvalidField("content_index").into());
            }
            let native_index = responses_lifecycle::event_index(core, object)?;
            core.text_annotation(
                native_index,
                required_u32(object, "annotation_index")?,
                &object["annotation"],
                output,
            )?;
        }
        "response.output_text.done" => {
            allow(
                object,
                &[
                    "type",
                    "sequence_number",
                    "item_id",
                    "output_index",
                    "content_index",
                    "text",
                    "logprobs",
                ],
            )?;
            if required_u32(object, "content_index")? != 0 {
                return Err(ModelIrError::InvalidField("content_index").into());
            }
            reject_nonempty_array(object, "logprobs")?;
            let native_index = responses_lifecycle::event_index(core, object)?;
            core.reconcile_text(native_index, required_str(object, "text")?, output)?;
        }
        "response.refusal.done" => {
            allow(
                object,
                &[
                    "type",
                    "sequence_number",
                    "item_id",
                    "output_index",
                    "content_index",
                    "refusal",
                ],
            )?;
            if required_u32(object, "content_index")? != 0 {
                return Err(ModelIrError::InvalidField("content_index").into());
            }
            let native_index = responses_lifecycle::event_index(core, object)?;
            core.reconcile_refusal(native_index, required_str(object, "refusal")?, output)?;
        }
        "response.reasoning_text.done" | "response.reasoning_summary_text.done" => {
            allow(
                object,
                &[
                    "type",
                    "sequence_number",
                    "item_id",
                    "output_index",
                    "content_index",
                    "summary_index",
                    "text",
                ],
            )?;
            let part_index = if native_type == "response.reasoning_text.done" {
                optional_part_index(object, "content_index")?
            } else {
                optional_part_index(object, "summary_index")?
            };
            if part_index != 0 {
                return Err(ModelIrError::InvalidField("reasoning part index").into());
            }
            let native_index = responses_lifecycle::event_index(core, object)?;
            core.reconcile_reasoning(native_index, required_str(object, "text")?, output)?;
        }
        "response.output_text.delta" => {
            allow(
                object,
                &[
                    "type",
                    "sequence_number",
                    "item_id",
                    "output_index",
                    "content_index",
                    "delta",
                    "logprobs",
                    "obfuscation",
                ],
            )?;
            if required_u32(object, "content_index")? != 0 {
                return Err(ModelIrError::InvalidField("content_index").into());
            }
            reject_nonempty_array(object, "logprobs")?;
            let native_index = responses_lifecycle::event_index(core, object)?;
            core.text_delta(native_index, required_str(object, "delta")?.into(), output)?;
        }
        "response.refusal.delta" => {
            allow(
                object,
                &[
                    "type",
                    "sequence_number",
                    "item_id",
                    "output_index",
                    "content_index",
                    "delta",
                ],
            )?;
            if required_u32(object, "content_index")? != 0 {
                return Err(ModelIrError::InvalidField("content_index").into());
            }
            let native_index = responses_lifecycle::event_index(core, object)?;
            core.refusal_delta(native_index, required_str(object, "delta")?.into(), output)?;
        }
        "response.reasoning_text.delta" | "response.reasoning_summary_text.delta" => {
            allow(
                object,
                &[
                    "type",
                    "sequence_number",
                    "item_id",
                    "output_index",
                    "content_index",
                    "summary_index",
                    "delta",
                ],
            )?;
            let part_index = if native_type == "response.reasoning_text.delta" {
                optional_part_index(object, "content_index")?
            } else {
                optional_part_index(object, "summary_index")?
            };
            if part_index != 0 {
                return Err(ModelIrError::InvalidField("reasoning part index").into());
            }
            let native_index = responses_lifecycle::event_index(core, object)?;
            core.reasoning_delta(native_index, required_str(object, "delta")?.into(), output)?;
        }
        "response.function_call_arguments.delta" | "response.custom_tool_call_input.delta" => {
            allow(
                object,
                &[
                    "type",
                    "sequence_number",
                    "item_id",
                    "output_index",
                    "delta",
                ],
            )?;
            let native_index = responses_lifecycle::event_index(core, object)?;
            let identity = tools.get(&native_index).cloned().ok_or_else(|| {
                ModelIrError::MissingToolIdentity(format!("responses index {native_index}"))
            })?;
            let expected_kind = if native_type == "response.function_call_arguments.delta" {
                ToolKindV1::Function
            } else {
                ToolKindV1::Custom
            };
            if identity.kind != expected_kind {
                return Err(ModelIrError::InvalidResponseLifecycle(
                    "Responses tool delta kind changed".into(),
                )
                .into());
            }
            core.tool_delta(
                native_index,
                identity.native_id,
                identity.kind,
                identity.namespace,
                identity.name,
                required_str(object, "delta")?.into(),
                output,
            )?;
        }
        "response.function_call_arguments.done" | "response.custom_tool_call_input.done" => {
            let payload_field = if native_type == "response.function_call_arguments.done" {
                "arguments"
            } else {
                "input"
            };
            allow(
                object,
                &[
                    "type",
                    "sequence_number",
                    "item_id",
                    "output_index",
                    payload_field,
                ],
            )?;
            let native_index = responses_lifecycle::event_index(core, object)?;
            let identity = tools.get(&native_index).cloned().ok_or_else(|| {
                ModelIrError::MissingToolIdentity(format!("responses index {native_index}"))
            })?;
            let expected_kind = if native_type == "response.function_call_arguments.done" {
                ToolKindV1::Function
            } else {
                ToolKindV1::Custom
            };
            if identity.kind != expected_kind {
                return Err(ModelIrError::InvalidResponseLifecycle(
                    "Responses completed tool input kind changed".into(),
                )
                .into());
            }
            core.reconcile_tool_with_arguments(
                native_index,
                identity.native_id,
                identity.kind,
                identity.namespace,
                identity.name,
                required_str(object, payload_field)?,
                output,
            )?;
        }
        "response.completed" | "response.incomplete" => {
            allow(object, &["type", "sequence_number", "response"])?;
            let response = object_field(object, "response")?;
            let expected_status = if native_type == "response.completed" {
                "completed"
            } else {
                "incomplete"
            };
            if required_str(response, "status")? != expected_status {
                return Err(ModelIrError::InvalidResponseLifecycle(
                    "Responses terminal event disagrees with response status".into(),
                )
                .into());
            }
            let mut staged = core.clone();
            let mut staged_events = VecDeque::new();
            staged.start_response(
                required_str(response, "id")?.into(),
                required_str(response, "model")?.into(),
                &mut staged_events,
            )?;
            responses_lifecycle::completed_output(
                &mut staged,
                tools,
                response,
                &mut staged_events,
            )?;
            responses_lifecycle::decode_terminal(&mut staged, response, &mut staged_events)?;
            staged.complete(&mut staged_events)?;
            *core = staged;
            output.extend(staged_events);
        }
        "response.failed" | "error" => {
            allow(
                object,
                &[
                    "type",
                    "sequence_number",
                    "response",
                    "error",
                    "code",
                    "message",
                    "param",
                ],
            )?;
            let error_value = object
                .get("error")
                .or_else(|| {
                    object
                        .get("response")
                        .and_then(Value::as_object)
                        .and_then(|response| response.get("error"))
                })
                .unwrap_or(&value);
            core.fail(decode_error(error_value, None)?, output)?;
        }
        "response.content_part.added" | "response.content_part.done" => {
            allow(
                object,
                &[
                    "type",
                    "sequence_number",
                    "item_id",
                    "output_index",
                    "content_index",
                    "summary_index",
                    "text",
                    "logprobs",
                    "part",
                    "item",
                ],
            )?;
            responses_lifecycle::content_part(
                core,
                object,
                native_type == "response.content_part.done",
                output,
            )?;
        }
        "response.reasoning_summary_part.added" | "response.reasoning_summary_part.done" => {
            allow(
                object,
                &[
                    "type",
                    "sequence_number",
                    "item_id",
                    "output_index",
                    "summary_index",
                    "part",
                ],
            )?;
            responses_lifecycle::reasoning_part(
                core,
                object,
                native_type == "response.reasoning_summary_part.done",
                output,
            )?;
        }
        "response.output_item.done" => {
            allow(object, &["type", "sequence_number", "output_index", "item"])?;
            responses_lifecycle::output_item_done(core, tools, object, output)?;
        }
        other => return Err(unsupported("Responses SSE event", other)),
    }
    Ok(())
}

fn optional_part_index(
    object: &Map<String, Value>,
    key: &'static str,
) -> Result<u32, ProtocolAdapterError> {
    object
        .get(key)
        .map(|_| required_u32(object, key))
        .transpose()
        .map(|index| index.unwrap_or_default())
}

fn resolve_chat_tool_identity(
    projection: Option<&ChatToolProjection>,
    emitted_name: &str,
) -> Result<ChatToolIdentity, ProtocolAdapterError> {
    match projection {
        Some(projection) => projection
            .resolve_emitted(emitted_name)
            .cloned()
            .ok_or_else(|| {
                ModelIrError::MissingToolIdentity(format!(
                    "unknown projected Chat tool {emitted_name}"
                ))
                .into()
            }),
        None => Ok(ChatToolIdentity {
            emitted_name: emitted_name.into(),
            kind: ToolKindV1::Function,
            namespace: None,
            local_name: emitted_name.into(),
        }),
    }
}

fn append_chat_tool_arguments(
    tools: &mut BTreeMap<u32, ChatToolCallState>,
    native_index: u32,
    delta: &str,
) -> Result<(), ProtocolAdapterError> {
    tools
        .values()
        .try_fold(0_usize, |total, tool| {
            total.checked_add(tool.wire_arguments.len())
        })
        .and_then(|total| total.checked_add(delta.len()))
        .filter(|total| *total <= super::MAX_CANONICAL_SEMANTIC_BYTES)
        .ok_or(ModelIrError::BufferLimit(
            super::MAX_CANONICAL_SEMANTIC_BYTES,
        ))?;
    tools
        .get_mut(&native_index)
        .expect("Chat tool identity was established before its arguments")
        .wire_arguments
        .push_str(delta);
    Ok(())
}

fn decode_chat_tool_arguments(
    identity: &ChatToolIdentity,
    wire_arguments: &str,
) -> Result<String, ProtocolAdapterError> {
    match identity.kind {
        ToolKindV1::Function => Ok(wire_arguments.into()),
        ToolKindV1::Custom => {
            let value = serde_json::from_str(wire_arguments)
                .map_err(|_| ModelIrError::InvalidToolArguments(identity.emitted_name.clone()))?;
            let canonical = ChatToolProjection::canonical_arguments(identity, value)?;
            canonical.as_str().map(str::to_owned).ok_or_else(|| {
                ModelIrError::InvalidToolArguments(identity.emitted_name.clone()).into()
            })
        }
    }
}

fn decode_chat_sse(
    core: &mut DecoderCore,
    tools: &mut BTreeMap<u32, ChatToolCallState>,
    projection: Option<&ChatToolProjection>,
    data: &[u8],
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
    if data == b"[DONE]" {
        return core.complete(output);
    }
    let value = parse_data(data)?;
    let object = checked_object(&value)?;
    if object.contains_key("error") {
        return core.fail(decode_error(&value, None)?, output);
    }
    allow(
        object,
        &[
            "id",
            "object",
            "created",
            "model",
            "choices",
            "usage",
            "system_fingerprint",
            "service_tier",
        ],
    )?;
    core.start_response(
        required_str(object, "id")?.into(),
        required_str(object, "model")?.into(),
        output,
    )?;
    reject_non_null(object, "system_fingerprint")?;
    reject_non_null(object, "service_tier")?;
    if let Some(usage) = object.get("usage")
        && !usage.is_null()
    {
        core.usage(decode_chat_usage(usage)?, output)?;
    }
    for choice in array_field(object, "choices", true)? {
        let choice = checked_object(choice)?;
        allow(choice, &["index", "delta", "finish_reason", "logprobs"])?;
        reject_non_null(choice, "logprobs")?;
        let choice_index = required_u32(choice, "index")?;
        let delta = object_field(choice, "delta")?;
        allow(
            delta,
            &[
                "role",
                "content",
                "reasoning_content",
                "tool_calls",
                "refusal",
            ],
        )?;
        if let Some(role) = optional_str(delta, "role")?
            && role != "assistant"
        {
            return Err(unsupported("Chat delta role", role));
        }
        if let Some(reasoning) = optional_str(delta, "reasoning_content")? {
            core.reasoning_delta(choice_index, reasoning.into(), output)?;
        }
        if let Some(content) = optional_str(delta, "content")? {
            core.text_delta(choice_index, content.into(), output)?;
        }
        if let Some(refusal) = optional_str(delta, "refusal")? {
            core.refusal_delta(choice_index, refusal.into(), output)?;
        }
        if let Some(calls) = delta.get("tool_calls") {
            for call in array(calls, "tool_calls")? {
                let call = checked_object(call)?;
                allow(call, &["index", "id", "type", "function"])?;
                if let Some(call_type) = optional_str(call, "type")?
                    && call_type != "function"
                {
                    return Err(unsupported("Chat Tool call", call_type));
                }
                let native_index = required_u32(call, "index")?;
                let function = object_field(call, "function")?;
                allow(function, &["name", "arguments"])?;
                let tool = if let (Some(id), Some(name)) =
                    (optional_str(call, "id")?, optional_str(function, "name")?)
                {
                    let identity = resolve_chat_tool_identity(projection, name)?;
                    let candidate = ChatToolCallState {
                        native_id: id.into(),
                        identity,
                        wire_arguments: String::new(),
                        finished: false,
                    };
                    if let Some(previous) = tools.get(&native_index) {
                        if previous.native_id != candidate.native_id
                            || previous.identity != candidate.identity
                        {
                            return Err(ModelIrError::InvalidResponseLifecycle(
                                "Chat tool identity changed".into(),
                            )
                            .into());
                        }
                    } else {
                        tools.insert(native_index, candidate);
                    }
                    tools
                        .get(&native_index)
                        .cloned()
                        .expect("Chat tool identity was inserted")
                } else {
                    tools.get(&native_index).cloned().ok_or_else(|| {
                        ModelIrError::MissingToolIdentity(format!("chat index {native_index}"))
                    })?
                };
                if tool.finished {
                    return Err(ModelIrError::InvalidResponseLifecycle(
                        "Chat tool arguments arrived after completion".into(),
                    )
                    .into());
                }
                let delta = optional_str(function, "arguments")?.unwrap_or_default();
                match tool.identity.kind {
                    ToolKindV1::Function => core.tool_delta(
                        native_index,
                        tool.native_id,
                        ToolKindV1::Function,
                        tool.identity.namespace,
                        tool.identity.local_name,
                        delta.into(),
                        output,
                    )?,
                    ToolKindV1::Custom => {
                        append_chat_tool_arguments(tools, native_index, delta)?;
                        core.start_tool(
                            native_index,
                            tool.native_id,
                            ToolKindV1::Custom,
                            tool.identity.namespace,
                            tool.identity.local_name,
                            output,
                        )?;
                    }
                }
            }
        }
        if let Some(reason) = optional_str(choice, "finish_reason")? {
            if reason == "tool_calls" {
                for native_index in tools.keys().copied().collect::<Vec<_>>() {
                    let tool = tools
                        .get(&native_index)
                        .cloned()
                        .expect("Chat tool index came from the same map");
                    if tool.finished {
                        continue;
                    }
                    if tool.identity.kind == ToolKindV1::Custom {
                        let input =
                            decode_chat_tool_arguments(&tool.identity, &tool.wire_arguments)?;
                        core.tool_delta(
                            native_index,
                            tool.native_id.clone(),
                            ToolKindV1::Custom,
                            tool.identity.namespace.clone(),
                            tool.identity.local_name.clone(),
                            input,
                            output,
                        )?;
                    }
                    core.finish_tool(
                        native_index,
                        tool.native_id,
                        tool.identity.kind,
                        tool.identity.namespace,
                        tool.identity.local_name,
                        output,
                    )?;
                    tools
                        .get_mut(&native_index)
                        .expect("Chat tool index came from the same map")
                        .finished = true;
                }
            }
            core.finish_reason(
                if core.has_refusal() {
                    FinishReason::Refusal
                } else {
                    decode_finish_reason(reason)
                },
                output,
            )?;
        }
    }
    Ok(())
}

fn decode_responses_nonstream(
    core: &mut DecoderCore,
    value: &Value,
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
    let object = checked_object(value)?;
    allow(
        object,
        &[
            "id",
            "object",
            "created_at",
            "status",
            "error",
            "incomplete_details",
            "instructions",
            "max_output_tokens",
            "model",
            "output",
            "parallel_tool_calls",
            "previous_response_id",
            "reasoning",
            "store",
            "temperature",
            "text",
            "tool_choice",
            "tools",
            "top_p",
            "truncation",
            "usage",
            "user",
            "metadata",
            "service_tier",
        ],
    )?;
    // Empty metadata and the provider's `auto` service-tier marker carry no
    // client-visible value. Non-empty metadata and other tier values remain
    // fail-closed until a profile can name an exact client projection.
    validate_responses_envelope_metadata(object)?;
    core.start_response(
        required_str(object, "id")?.into(),
        required_str(object, "model")?.into(),
        output,
    )?;
    for (native_index, item) in array_field(object, "output", false)?.iter().enumerate() {
        let native_index =
            u32::try_from(native_index).map_err(|_| ModelIrError::InvalidField("output"))?;
        let item = checked_object(item)?;
        match required_str(item, "type")? {
            "web_search_call" => {
                use crate::server::core_runtime::model_ir::WebSearchPhase;
                let final_item = Value::Object(item.clone());
                let mut start = final_item.clone();
                start["status"] = "in_progress".into();
                start.as_object_mut().unwrap().remove("action");
                core.search_item(native_index, &start, WebSearchPhase::Added, output)?;
                core.search_item(native_index, &final_item, WebSearchPhase::Done, output)?;
            }
            _ => responses_lifecycle::nonstream_item(core, native_index, item, output)?,
        }
    }
    responses_lifecycle::decode_terminal(core, object, output)?;
    match required_str(object, "status")? {
        "completed" | "incomplete" => core.complete(output),
        "failed" | "cancelled" => core.fail(
            decode_error(
                object
                    .get("error")
                    .ok_or(ModelIrError::InvalidField("error"))?,
                None,
            )?,
            output,
        ),
        status => Err(unsupported("Responses terminal status", status)),
    }
}

fn decode_chat_nonstream(
    core: &mut DecoderCore,
    projection: Option<&ChatToolProjection>,
    value: &Value,
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
    let object = checked_object(value)?;
    allow(
        object,
        &[
            "id",
            "object",
            "created",
            "model",
            "choices",
            "usage",
            "system_fingerprint",
            "service_tier",
        ],
    )?;
    reject_non_null(object, "system_fingerprint")?;
    reject_non_null(object, "service_tier")?;
    core.start_response(
        required_str(object, "id")?.into(),
        required_str(object, "model")?.into(),
        output,
    )?;
    for choice in array_field(object, "choices", false)? {
        let choice = checked_object(choice)?;
        allow(choice, &["index", "message", "finish_reason", "logprobs"])?;
        reject_non_null(choice, "logprobs")?;
        let native_index = required_u32(choice, "index")?;
        let message = object_field(choice, "message")?;
        allow(
            message,
            &[
                "role",
                "content",
                "reasoning_content",
                "tool_calls",
                "refusal",
                "annotations",
                "audio",
            ],
        )?;
        reject_nonempty_array(message, "annotations")?;
        reject_non_null(message, "audio")?;
        if required_str(message, "role")? != "assistant" {
            return Err(ModelIrError::InvalidField("role").into());
        }
        if let Some(reasoning) = optional_str(message, "reasoning_content")? {
            core.reasoning_delta(native_index, reasoning.into(), output)?;
        }
        if let Some(content) = optional_str(message, "content")? {
            core.text_delta(native_index, content.into(), output)?;
        }
        if let Some(refusal) = optional_str(message, "refusal")? {
            core.refusal_delta(native_index, refusal.into(), output)?;
        }
        if let Some(calls) = message.get("tool_calls") {
            for (tool_position, call) in array(calls, "tool_calls")?.iter().enumerate() {
                let native_tool_index = native_index
                    .checked_mul(10_000)
                    .and_then(|base| base.checked_add(u32::try_from(tool_position).ok()?))
                    .ok_or(ModelIrError::InvalidField("tool_calls"))?;
                let call = checked_object(call)?;
                allow(call, &["id", "type", "function"])?;
                if required_str(call, "type")? != "function" {
                    return Err(unsupported("Chat Tool call", required_str(call, "type")?));
                }
                let function = object_field(call, "function")?;
                allow(function, &["name", "arguments"])?;
                let logical_id = required_str(call, "id")?.to_owned();
                let identity =
                    resolve_chat_tool_identity(projection, required_str(function, "name")?)?;
                let arguments =
                    decode_chat_tool_arguments(&identity, required_str(function, "arguments")?)?;
                core.finish_tool_with_arguments(
                    native_tool_index,
                    logical_id,
                    identity.kind,
                    identity.namespace,
                    identity.local_name,
                    &arguments,
                    output,
                )?;
            }
        }
        let native_reason = required_str(choice, "finish_reason")?;
        core.finish_reason(
            if core.has_refusal() {
                FinishReason::Refusal
            } else {
                decode_finish_reason(native_reason)
            },
            output,
        )?;
    }
    if let Some(usage) = object.get("usage") {
        core.usage(decode_chat_usage(usage)?, output)?;
    }
    core.complete(output)
}
