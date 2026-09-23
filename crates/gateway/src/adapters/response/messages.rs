use std::collections::{BTreeMap, VecDeque};

use serde_json::{Map, Value};

use crate::server::core_runtime::model_ir::{
    FinishReason, ModelIrError, ModelStreamEventV1, ToolKindV1,
};
use crate::server::request_plan::IngressProtocol;

use super::wire::*;
use super::{DecoderCore, ProtocolAdapterError};

#[derive(Clone, Debug)]
pub(super) struct MessagesState {
    blocks: BTreeMap<u32, NativeBlock>,
    next_order: u32,
    pub(super) retention: super::body_buffer::Retention,
    terminal_classified: bool,
}

#[derive(Clone, Debug)]
struct NativeBlock {
    order: u32,
    stopped: bool,
    content: Option<NativeContent>,
}

#[derive(Clone, Debug)]
enum NativeContent {
    Text(String),
    Thinking {
        text: String,
        signature: String,
    },
    RedactedThinking(Value),
    Tool {
        native_id: String,
        name: String,
        arguments: String,
    },
}

impl MessagesState {
    pub(super) fn new() -> Self {
        Self {
            blocks: BTreeMap::new(),
            next_order: 0,
            retention: super::body_buffer::Retention::new(super::body_buffer::standalone_budget()),
            terminal_classified: false,
        }
    }

    fn insert(&mut self, index: u32, content: NativeContent) -> Result<(), ProtocolAdapterError> {
        if self.terminal_classified || self.blocks.contains_key(&index) {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "duplicate or late Messages block".into(),
            )
            .into());
        }
        self.retention.add(256)?;
        let order = self.next_order;
        self.next_order = self.next_order.checked_add(1).ok_or_else(|| {
            ModelIrError::InvalidResponseLifecycle("Messages block order overflow".into())
        })?;
        self.blocks.insert(
            index,
            NativeBlock {
                order,
                stopped: false,
                content: Some(content),
            },
        );
        Ok(())
    }

    fn charge(&mut self, bytes: usize) -> Result<(), ProtocolAdapterError> {
        self.retention.add(bytes)
    }

    fn append_text(&mut self, index: u32, text: &str) -> Result<(), ProtocolAdapterError> {
        if !matches!(
            self.blocks.get(&index),
            Some(NativeBlock {
                stopped: false,
                content: Some(NativeContent::Text(_)),
                ..
            })
        ) {
            return Err(invalid_block(index, "text delta"));
        }
        self.charge(text.len())?;
        match self.blocks.get_mut(&index) {
            Some(NativeBlock {
                content: Some(NativeContent::Text(output)),
                ..
            }) => output.push_str(text),
            _ => unreachable!("validated Messages text block changed"),
        }
        Ok(())
    }

    fn append_thinking(&mut self, index: u32, text: &str) -> Result<(), ProtocolAdapterError> {
        if !matches!(
            self.blocks.get(&index),
            Some(NativeBlock {
                stopped: false,
                content: Some(NativeContent::Thinking { .. }),
                ..
            })
        ) {
            return Err(invalid_block(index, "thinking delta"));
        }
        self.charge(text.len())?;
        match self.blocks.get_mut(&index) {
            Some(NativeBlock {
                content: Some(NativeContent::Thinking { text: output, .. }),
                ..
            }) => output.push_str(text),
            _ => unreachable!("validated Messages thinking block changed"),
        }
        Ok(())
    }

    fn append_signature(&mut self, index: u32, value: &str) -> Result<(), ProtocolAdapterError> {
        if !matches!(
            self.blocks.get(&index),
            Some(NativeBlock {
                stopped: false,
                content: Some(NativeContent::Thinking { .. }),
                ..
            })
        ) {
            return Err(invalid_block(index, "thinking signature delta"));
        }
        self.charge(value.len())?;
        match self.blocks.get_mut(&index) {
            Some(NativeBlock {
                content: Some(NativeContent::Thinking { signature, .. }),
                ..
            }) => signature.push_str(value),
            _ => unreachable!("validated Messages thinking block changed"),
        }
        Ok(())
    }

    fn append_arguments(&mut self, index: u32, value: &str) -> Result<(), ProtocolAdapterError> {
        if !matches!(
            self.blocks.get(&index),
            Some(NativeBlock {
                stopped: false,
                content: Some(NativeContent::Tool { .. }),
                ..
            })
        ) {
            return Err(invalid_block(index, "Tool argument delta"));
        }
        self.charge(value.len())?;
        match self.blocks.get_mut(&index) {
            Some(NativeBlock {
                content: Some(NativeContent::Tool { arguments, .. }),
                ..
            }) => arguments.push_str(value),
            _ => unreachable!("validated Messages Tool block changed"),
        }
        Ok(())
    }

    fn stop(&mut self, index: u32) -> Result<(), ProtocolAdapterError> {
        match self.blocks.get_mut(&index) {
            Some(block) if !block.stopped => {
                block.stopped = true;
                Ok(())
            }
            _ => Err(invalid_block(index, "duplicate or unknown block stop")),
        }
    }

    fn flush(
        &mut self,
        core: &mut DecoderCore,
        refusal: bool,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        if self.terminal_classified || self.blocks.values().any(|block| !block.stopped) {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "Messages terminal classification is duplicate or precedes block stop".into(),
            )
            .into());
        }
        let mut ordered = self
            .blocks
            .iter()
            .map(|(index, block)| (block.order, *index))
            .collect::<Vec<_>>();
        ordered.sort_unstable();
        for (_, native_index) in ordered {
            let content = self
                .blocks
                .get_mut(&native_index)
                .and_then(|block| block.content.take())
                .ok_or_else(|| invalid_block(native_index, "terminal replay"))?;
            match content {
                NativeContent::Text(text) if refusal => {
                    core.refusal_delta(native_index, text, output)?
                }
                NativeContent::Text(text) => core.text_delta(native_index, text, output)?,
                NativeContent::Thinking { text, signature } => {
                    core.reasoning_delta(native_index, text, output)?;
                    if !signature.is_empty() {
                        core.provider_state(
                            IngressProtocol::Messages,
                            "thinking_signature",
                            Value::String(signature),
                            Some(native_index),
                            output,
                        )?;
                    }
                }
                NativeContent::RedactedThinking(value) => core.provider_state(
                    IngressProtocol::Messages,
                    "redacted_thinking",
                    value,
                    Some(native_index),
                    output,
                )?,
                NativeContent::Tool {
                    native_id,
                    name,
                    arguments,
                } => {
                    let arguments = if arguments.is_empty() {
                        "{}".to_owned()
                    } else {
                        arguments
                    };
                    core.tool_delta(
                        native_index,
                        native_id.clone(),
                        ToolKindV1::Function,
                        None,
                        name.clone(),
                        arguments,
                        output,
                    )?;
                    core.finish_tool(
                        native_index,
                        native_id,
                        ToolKindV1::Function,
                        None,
                        name,
                        output,
                    )?;
                }
            }
        }
        self.terminal_classified = true;
        Ok(())
    }
}

pub(super) fn decode_sse(
    core: &mut DecoderCore,
    state: &mut MessagesState,
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
            "Messages SSE event field disagrees with JSON type".into(),
        )
        .into());
    }
    match native_type {
        "message_start" => decode_start(core, object, output),
        "content_block_start" => decode_block_start(state, object),
        "content_block_delta" => decode_block_delta(state, object),
        "content_block_stop" => {
            allow(object, &["type", "index"])?;
            state.stop(required_u32(object, "index")?)
        }
        "message_delta" => decode_message_delta(core, state, object, output),
        "message_stop" => {
            allow(object, &["type"])?;
            if !state.terminal_classified {
                return Err(ModelIrError::MissingTerminalEvent.into());
            }
            // Some Messages-compatible providers keep the HTTP stream open after the
            // terminal message_delta, while Anthropic also emits message_stop. The
            // terminal delta is sufficient to complete the canonical response; accept
            // a following message_stop as an idempotent wire terminator.
            if core.accumulator.terminal {
                Ok(())
            } else {
                core.complete(output)
            }
        }
        "error" => {
            allow(object, &["type", "error"])?;
            if !state.blocks.is_empty() {
                return Err(ModelIrError::InvalidResponseLifecycle(
                    "Messages error cannot classify already received content".into(),
                )
                .into());
            }
            core.fail(decode_error(&value, None)?, output)
        }
        "ping" => allow(object, &["type"]),
        other => Err(unsupported("Messages SSE event", other)),
    }
}

fn decode_start(
    core: &mut DecoderCore,
    object: &Map<String, Value>,
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
    allow(object, &["type", "message"])?;
    let message = object_field(object, "message")?;
    allow(
        message,
        &[
            "id",
            "type",
            "role",
            "content",
            "model",
            "stop_reason",
            "stop_sequence",
            "usage",
        ],
    )?;
    reject_nonempty_array(message, "content")?;
    reject_non_null(message, "stop_reason")?;
    reject_non_null(message, "stop_sequence")?;
    core.start_response(
        required_str(message, "id")?.into(),
        required_str(message, "model")?.into(),
        output,
    )?;
    if let Some(usage) = message.get("usage") {
        core.usage(decode_messages_usage(usage)?, output)?;
    }
    Ok(())
}

fn decode_block_start(
    state: &mut MessagesState,
    object: &Map<String, Value>,
) -> Result<(), ProtocolAdapterError> {
    allow(object, &["type", "index", "content_block"])?;
    let native_index = required_u32(object, "index")?;
    let block = object_field(object, "content_block")?;
    let content = match required_str(block, "type")? {
        "text" => {
            allow(block, &["type", "text", "citations"])?;
            reject_nonempty_array(block, "citations")?;
            let text = optional_str(block, "text")?.unwrap_or_default().to_owned();
            state.charge(text.len())?;
            NativeContent::Text(text)
        }
        "thinking" => {
            allow(block, &["type", "thinking", "signature"])?;
            let text = optional_str(block, "thinking")?
                .unwrap_or_default()
                .to_owned();
            let signature = optional_str(block, "signature")?
                .unwrap_or_default()
                .to_owned();
            state.charge(text.len().saturating_add(signature.len()))?;
            NativeContent::Thinking { text, signature }
        }
        "redacted_thinking" => {
            let value = Value::Object(block.clone());
            state.charge(
                serde_json::to_vec(&value)
                    .map_err(|error| ModelIrError::InvalidJson(error.to_string()))?
                    .len(),
            )?;
            NativeContent::RedactedThinking(value)
        }
        "tool_use" => {
            allow(block, &["type", "id", "name", "input"])?;
            let native_id = required_str(block, "id")?.to_owned();
            let name = required_str(block, "name")?.to_owned();
            let input = block
                .get("input")
                .ok_or(ModelIrError::InvalidField("input"))?;
            let arguments = if input == &Value::Object(Map::new()) {
                String::new()
            } else {
                serde_json::to_string(input)
                    .map_err(|error| ModelIrError::InvalidJson(error.to_string()))?
            };
            state.charge(
                native_id
                    .len()
                    .saturating_add(name.len())
                    .saturating_add(arguments.len()),
            )?;
            NativeContent::Tool {
                native_id,
                name,
                arguments,
            }
        }
        other => return Err(unsupported("Messages content block", other)),
    };
    state.insert(native_index, content)
}

fn decode_block_delta(
    state: &mut MessagesState,
    object: &Map<String, Value>,
) -> Result<(), ProtocolAdapterError> {
    allow(object, &["type", "index", "delta"])?;
    let native_index = required_u32(object, "index")?;
    let delta = object_field(object, "delta")?;
    match required_str(delta, "type")? {
        "text_delta" => {
            allow(delta, &["type", "text"])?;
            state.append_text(native_index, required_str(delta, "text")?)
        }
        "thinking_delta" => {
            allow(delta, &["type", "thinking"])?;
            state.append_thinking(native_index, required_str(delta, "thinking")?)
        }
        "signature_delta" => {
            allow(delta, &["type", "signature"])?;
            state.append_signature(native_index, required_str(delta, "signature")?)
        }
        "input_json_delta" => {
            allow(delta, &["type", "partial_json"])?;
            state.append_arguments(native_index, required_str(delta, "partial_json")?)
        }
        other => Err(unsupported("Messages content delta", other)),
    }
}

fn decode_message_delta(
    core: &mut DecoderCore,
    state: &mut MessagesState,
    object: &Map<String, Value>,
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
    allow(object, &["type", "delta", "usage"])?;
    let delta = object_field(object, "delta")?;
    allow(delta, &["stop_reason", "stop_sequence"])?;
    reject_non_null(delta, "stop_sequence")?;
    if let Some(reason) = optional_str(delta, "stop_reason")? {
        let reason = decode_finish_reason(reason);
        state.flush(core, reason == FinishReason::Refusal, output)?;
        core.finish_reason(reason, output)?;
    }
    if let Some(usage) = object.get("usage") {
        core.usage(decode_messages_usage(usage)?, output)?;
    }
    if state.terminal_classified && !core.accumulator.terminal {
        core.complete(output)?;
    }
    Ok(())
}

pub(super) fn decode_nonstream(
    core: &mut DecoderCore,
    value: &Value,
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
    let object = checked_object(value)?;
    allow(
        object,
        &[
            "id",
            "type",
            "role",
            "content",
            "model",
            "stop_reason",
            "stop_sequence",
            "usage",
        ],
    )?;
    core.start_response(
        required_str(object, "id")?.into(),
        required_str(object, "model")?.into(),
        output,
    )?;
    reject_non_null(object, "stop_sequence")?;
    let finish_reason = decode_finish_reason(required_str(object, "stop_reason")?);
    for (native_index, block) in array_field(object, "content", false)?.iter().enumerate() {
        let native_index =
            u32::try_from(native_index).map_err(|_| ModelIrError::InvalidField("content"))?;
        let block = checked_object(block)?;
        match required_str(block, "type")? {
            "text" => {
                allow(block, &["type", "text", "citations"])?;
                reject_nonempty_array(block, "citations")?;
                let text = required_str(block, "text")?.into();
                if finish_reason == FinishReason::Refusal {
                    core.refusal_delta(native_index, text, output)?;
                } else {
                    core.text_delta(native_index, text, output)?;
                }
            }
            "thinking" => {
                allow(block, &["type", "thinking", "signature"])?;
                core.reasoning_delta(
                    native_index,
                    required_str(block, "thinking")?.into(),
                    output,
                )?;
                if let Some(signature) = optional_str(block, "signature")?
                    && !signature.is_empty()
                {
                    core.provider_state(
                        IngressProtocol::Messages,
                        "thinking_signature",
                        Value::String(signature.into()),
                        Some(native_index),
                        output,
                    )?;
                }
            }
            "redacted_thinking" => core.provider_state(
                IngressProtocol::Messages,
                "redacted_thinking",
                Value::Object(block.clone()),
                Some(native_index),
                output,
            )?,
            "tool_use" => {
                allow(block, &["type", "id", "name", "input"])?;
                let native_id = required_str(block, "id")?.to_owned();
                let name = required_str(block, "name")?.to_owned();
                let arguments = serde_json::to_string(
                    block
                        .get("input")
                        .ok_or(ModelIrError::InvalidField("input"))?,
                )
                .map_err(|error| ModelIrError::InvalidJson(error.to_string()))?;
                core.tool_delta(
                    native_index,
                    native_id.clone(),
                    ToolKindV1::Function,
                    None,
                    name.clone(),
                    arguments,
                    output,
                )?;
                core.finish_tool(
                    native_index,
                    native_id,
                    ToolKindV1::Function,
                    None,
                    name,
                    output,
                )?;
            }
            other => return Err(unsupported("Messages content", other)),
        }
    }
    core.finish_reason(finish_reason, output)?;
    core.usage(
        decode_messages_usage(
            object
                .get("usage")
                .ok_or(ModelIrError::InvalidField("usage"))?,
        )?,
        output,
    )?;
    core.complete(output)
}

fn invalid_block(index: u32, operation: &str) -> ProtocolAdapterError {
    ModelIrError::InvalidResponseLifecycle(format!(
        "Messages {operation} does not match content block {index}"
    ))
    .into()
}
