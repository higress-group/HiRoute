//! Responses item/content lifecycle validation shared by streaming and batch decoders.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde_json::{Map, Value};

use super::protocols::ResponsesToolIdentity;
use super::wire::*;
use super::{DecoderCore, NativeBlockKind, ProtocolAdapterError, invalid_block};
use crate::server::core_runtime::model_ir::{
    FinishReason, ModelIrError, ModelStreamEventV1, MutableResponseBlock, ResponseItemStatus,
    ToolKindV1, UrlCitationV1,
};

pub(super) fn register_item(
    core: &mut DecoderCore,
    native_index: u32,
    item: &Map<String, Value>,
) -> Result<(), ProtocolAdapterError> {
    if let Some(item_id) = optional_str(item, "id")? {
        core.register_native_item_id(native_index, item_id)
    } else {
        Ok(())
    }
}

pub(super) fn event_index(
    core: &mut DecoderCore,
    object: &Map<String, Value>,
) -> Result<u32, ProtocolAdapterError> {
    let native_index = required_u32(object, "output_index")?;
    if let Some(item_id) = optional_str(object, "item_id")? {
        observe_item_id(core, native_index, item_id)?;
    }
    Ok(native_index)
}

pub(super) fn content_part(
    core: &mut DecoderCore,
    object: &Map<String, Value>,
    done: bool,
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
    let native_index = event_index(core, object)?;
    if required_u32(object, "content_index")? != 0 {
        return Err(ModelIrError::InvalidField("content_index").into());
    }
    finish_message_part(
        core,
        native_index,
        object_field(object, "part")?,
        done,
        None,
        output,
    )
}

pub(super) fn reasoning_part(
    core: &mut DecoderCore,
    object: &Map<String, Value>,
    done: bool,
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
    let native_index = event_index(core, object)?;
    if required_u32(object, "summary_index")? != 0 {
        return Err(ModelIrError::InvalidField("summary_index").into());
    }
    let part = object_field(object, "part")?;
    allow(part, &["type", "text"])?;
    if required_str(part, "type")? != "summary_text" {
        return Err(unsupported(
            "Responses reasoning summary part",
            required_str(part, "type")?,
        ));
    }
    let text = required_str(part, "text")?;
    if done {
        core.reconcile_reasoning(native_index, text, output)
            .map(drop)
    } else {
        core.reasoning_delta(native_index, text.into(), output)
    }
}

pub(super) fn output_item_done(
    core: &mut DecoderCore,
    tools: &BTreeMap<u32, ResponsesToolIdentity>,
    object: &Map<String, Value>,
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
    let native_index = required_u32(object, "output_index")?;
    let item = object_field(object, "item")?;
    finish_output_item(core, tools, native_index, item, false, output)
}

fn finish_output_item(
    core: &mut DecoderCore,
    tools: &BTreeMap<u32, ResponsesToolIdentity>,
    native_index: u32,
    item: &Map<String, Value>,
    final_snapshot: bool,
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
    observe_item_id(core, native_index, required_str(item, "id")?)?;
    let status = item_status(item)?;
    match required_str(item, "type")? {
        "message" => finish_message_item(core, native_index, item, status, final_snapshot, output),
        "reasoning" => {
            finish_reasoning_item(core, native_index, item, status, final_snapshot, output)
        }
        "function_call" | "custom_tool_call" => {
            finish_tool_item(core, tools, native_index, item, status, output)
        }
        other => Err(unsupported("Responses completed output item", other)),
    }
}

pub(super) fn completed_output(
    core: &mut DecoderCore,
    tools: &BTreeMap<u32, ResponsesToolIdentity>,
    response: &Map<String, Value>,
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
    let Some(value) = response.get("output") else {
        return Ok(());
    };
    let items = value.as_array().ok_or(ModelIrError::InvalidField(
        "response.completed.response.output",
    ))?;
    let mut expected_indexes = BTreeSet::new();
    for (position, value) in items.iter().enumerate() {
        let native_index = u32::try_from(position)
            .map_err(|_| ModelIrError::InvalidField("response.completed.response.output"))?;
        expected_indexes.insert(native_index);
        let item = checked_object(value)?;
        if required_str(item, "type")? == "web_search_call" {
            core.finish_search_snapshot(native_index, value, output)?;
        } else {
            finish_output_item(core, tools, native_index, item, true, output)?;
        }
    }

    let mut observed_indexes = core
        .native_blocks
        .keys()
        .map(|(_, native_index)| *native_index)
        .collect::<BTreeSet<_>>();
    observed_indexes.extend(core.native_item_ids.keys().copied());
    observed_indexes.extend(core.native_message_phases.keys().copied());
    observed_indexes.extend(tools.keys().copied());
    if observed_indexes != expected_indexes {
        return Err(ModelIrError::InvalidResponseLifecycle(
            "Responses completed output items disagree with the accepted stream".into(),
        )
        .into());
    }
    Ok(())
}

fn observe_item_id(
    core: &mut DecoderCore,
    native_index: u32,
    item_id: &str,
) -> Result<(), ProtocolAdapterError> {
    if core.native_item_ids.contains_key(&native_index) {
        core.validate_native_item_id(native_index, item_id)
    } else {
        core.register_native_item_id(native_index, item_id)
    }
}

pub(super) fn nonstream_item(
    core: &mut DecoderCore,
    native_index: u32,
    item: &Map<String, Value>,
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
    let status = item_status(item)?;
    match required_str(item, "type")? {
        "message" => {
            register_item(core, native_index, item)?;
            core.observe_message_phase(native_index, optional_str(item, "phase")?, false)?;
            finish_message_item(core, native_index, item, status, true, output)
        }
        "reasoning" => {
            register_item(core, native_index, item)?;
            finish_reasoning_item(core, native_index, item, status, true, output)
        }
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
            register_item(core, native_index, item)?;
            let call_id = required_str(item, "call_id")?.to_owned();
            let namespace = optional_str(item, "namespace")?.map(str::to_owned);
            let name = required_str(item, "name")?.to_owned();
            core.finish_tool_with_arguments_status(
                native_index,
                call_id,
                kind,
                namespace,
                name,
                required_str(item, payload_field)?,
                status,
                output,
            )
        }
        other => Err(unsupported("Responses output", other)),
    }
}

fn finish_message_item(
    core: &mut DecoderCore,
    native_index: u32,
    item: &Map<String, Value>,
    status: ResponseItemStatus,
    final_snapshot: bool,
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
    allow(item, &["type", "id", "status", "role", "content", "phase"])?;
    if optional_str(item, "role")?.is_some_and(|role| role != "assistant") {
        return Err(ModelIrError::InvalidField("response output message role").into());
    }
    core.observe_message_phase(native_index, optional_str(item, "phase")?, true)?;
    let content = array_field(item, "content", false)?;
    if content.len() > 1 {
        return Err(ModelIrError::UnsupportedField(
            "response output message with multiple content parts".into(),
        )
        .into());
    }
    if let Some(part) = content.first() {
        finish_message_part(
            core,
            native_index,
            checked_object(part)?,
            true,
            Some(status),
            output,
        )
    } else {
        finish_message_without_part(core, native_index, status, final_snapshot, output)
    }
}

fn finish_message_part(
    core: &mut DecoderCore,
    native_index: u32,
    part: &Map<String, Value>,
    done: bool,
    status: Option<ResponseItemStatus>,
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
    match required_str(part, "type")? {
        "output_text" => {
            allow(part, &["type", "text", "annotations", "logprobs"])?;
            reject_nonempty_array(part, "logprobs")?;
            let text = required_str(part, "text")?;
            if done {
                core.reconcile_text(native_index, text, output)?;
            } else {
                core.text_delta(native_index, text.into(), output)?;
            }
            reconcile_annotations(core, native_index, part, output)?;
            if let Some(status) = status {
                core.finish_text_with_status(native_index, text, status, output)?;
            }
            Ok(())
        }
        "refusal" => {
            allow(part, &["type", "refusal"])?;
            let refusal = required_str(part, "refusal")?;
            if done {
                core.reconcile_refusal(native_index, refusal, output)?;
                if let Some(status) = status {
                    core.finish_refusal_with_status(native_index, refusal, status, output)?;
                }
                Ok(())
            } else {
                core.refusal_delta(native_index, refusal.into(), output)
            }
        }
        "reasoning_text" => {
            allow(part, &["type", "text"])?;
            let text = required_str(part, "text")?;
            if done {
                core.reconcile_reasoning(native_index, text, output)?;
                if let Some(status) = status {
                    core.finish_reasoning_with_status(native_index, text, status, output)?;
                }
                Ok(())
            } else {
                core.reasoning_delta(native_index, text.into(), output)
            }
        }
        other => Err(unsupported("Responses output content", other)),
    }
}

fn reconcile_annotations(
    core: &mut DecoderCore,
    native_index: u32,
    part: &Map<String, Value>,
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
    let expected = array_field(part, "annotations", true)?
        .iter()
        .map(|value| {
            serde_json::from_value::<UrlCitationV1>(value.clone())
                .map_err(|_| ModelIrError::InvalidField("url citation").into())
        })
        .collect::<Result<Vec<_>, ProtocolAdapterError>>()?;
    let (index, fresh) = core.block_index(NativeBlockKind::Text, native_index)?;
    if fresh {
        return Err(invalid_block(index, "text annotations before text"));
    }
    let current = core
        .accumulator
        .annotations
        .get(&index)
        .cloned()
        .unwrap_or_default();
    if current == expected {
        return Ok(());
    }
    if !current.is_empty() || core.accumulator.finished_text.contains(&index) {
        return Err(invalid_block(
            index,
            "final text annotations disagree with deltas",
        ));
    }
    for (position, annotation) in expected.iter().enumerate() {
        let value = serde_json::to_value(annotation)
            .map_err(|error| ModelIrError::InvalidJson(error.to_string()))?;
        core.text_annotation(native_index, position as u32, &value, output)?;
    }
    Ok(())
}

fn finish_reasoning_item(
    core: &mut DecoderCore,
    native_index: u32,
    item: &Map<String, Value>,
    status: ResponseItemStatus,
    final_snapshot: bool,
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
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
    let encrypted = item
        .get("encrypted_content")
        .filter(|value| !value.is_null());
    core.finalize_responses_encrypted_content(native_index, encrypted, final_snapshot, output)?;
    let summary = array_field(item, "summary", true)?;
    if summary.len() > 1 {
        return Err(ModelIrError::UnsupportedField(
            "response reasoning with multiple summary parts".into(),
        )
        .into());
    }
    if let Some(part) = summary.first() {
        let part = checked_object(part)?;
        if required_str(part, "type")? != "summary_text" {
            return Err(unsupported(
                "Responses reasoning summary",
                required_str(part, "type")?,
            ));
        }
        core.finish_reasoning_with_status(
            native_index,
            required_str(part, "text")?,
            status,
            output,
        )?;
    } else {
        let expected = if final_snapshot && item.contains_key("summary") {
            String::new()
        } else {
            core.native_blocks
                .get(&(NativeBlockKind::Reasoning, native_index))
                .and_then(|index| core.accumulator.blocks.get(index))
                .and_then(|block| match block {
                    MutableResponseBlock::Reasoning(text) => Some(text.clone()),
                    _ => None,
                })
                .unwrap_or_default()
        };
        core.finish_reasoning_with_status(native_index, &expected, status, output)?;
    }
    Ok(())
}

fn finish_tool_item(
    core: &mut DecoderCore,
    tools: &BTreeMap<u32, ResponsesToolIdentity>,
    native_index: u32,
    item: &Map<String, Value>,
    status: ResponseItemStatus,
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
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
    let call_id = required_str(item, "call_id")?.to_owned();
    let namespace = optional_str(item, "namespace")?.map(str::to_owned);
    let name = required_str(item, "name")?.to_owned();
    let identity = ResponsesToolIdentity {
        native_id: call_id.clone(),
        kind,
        namespace: namespace.clone(),
        name: name.clone(),
    };
    match tools.get(&native_index) {
        Some(current) if current == &identity => {}
        None if !core
            .native_blocks
            .contains_key(&(NativeBlockKind::Tool, native_index)) => {}
        _ => {
            return Err(ModelIrError::MissingToolIdentity(format!(
                "responses index {native_index}"
            ))
            .into());
        }
    }
    core.finish_tool_with_arguments_status(
        native_index,
        call_id,
        kind,
        namespace,
        name,
        required_str(item, payload_field)?,
        status,
        output,
    )
}

fn finish_message_without_part(
    core: &mut DecoderCore,
    native_index: u32,
    status: ResponseItemStatus,
    final_snapshot: bool,
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
    if let Some(index) = core
        .native_blocks
        .get(&(NativeBlockKind::Text, native_index))
        .copied()
    {
        let expected = match core.accumulator.blocks.get(&index) {
            Some(MutableResponseBlock::Text(_)) if final_snapshot => String::new(),
            Some(MutableResponseBlock::Text(text)) => text.clone(),
            _ => return Err(invalid_block(index, "completed message text")),
        };
        core.finish_text_with_status(native_index, &expected, status, output)
    } else if let Some(index) = core
        .native_blocks
        .get(&(NativeBlockKind::Refusal, native_index))
        .copied()
    {
        let expected = match core.accumulator.blocks.get(&index) {
            Some(MutableResponseBlock::Refusal(_)) if final_snapshot => String::new(),
            Some(MutableResponseBlock::Refusal(text)) => text.clone(),
            _ => return Err(invalid_block(index, "completed message refusal")),
        };
        core.finish_refusal_with_status(native_index, &expected, status, output)
    } else {
        core.finish_text_with_status(native_index, "", status, output)
    }
}

fn item_status(item: &Map<String, Value>) -> Result<ResponseItemStatus, ProtocolAdapterError> {
    match optional_str(item, "status")?.unwrap_or("completed") {
        "completed" => Ok(ResponseItemStatus::Completed),
        "incomplete" => Ok(ResponseItemStatus::Incomplete),
        _ => Err(ModelIrError::InvalidField("response output item status").into()),
    }
}

pub(super) fn decode_terminal(
    core: &mut DecoderCore,
    response: &Map<String, Value>,
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<(), ProtocolAdapterError> {
    validate_responses_envelope_metadata(response)?;
    core.responses_metadata(response, output)?;
    core.start_response(
        required_str(response, "id")?.into(),
        required_str(response, "model")?.into(),
        output,
    )?;
    if let Some(usage) = response.get("usage")
        && !usage.is_null()
    {
        core.usage(decode_responses_usage(usage)?, output)?;
    }
    let status = required_str(response, "status")?;
    let incomplete_reason = response
        .get("incomplete_details")
        .filter(|details| !details.is_null())
        .and_then(|details| details.get("reason"))
        .and_then(Value::as_str);
    let reason = match status {
        "completed" if incomplete_reason.is_some() => {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "completed Responses terminal carries incomplete details".into(),
            )
            .into());
        }
        "completed" => Some(
            if core.accumulator.blocks.values().any(|block| {
                matches!(
                    block,
                    crate::server::core_runtime::model_ir::MutableResponseBlock::ToolCall { .. }
                )
            }) {
                FinishReason::ToolCall
            } else {
                FinishReason::Stop
            },
        ),
        "incomplete" => incomplete_reason
            .map(decode_finish_reason)
            .or_else(|| core.has_refusal().then_some(FinishReason::Refusal)),
        "failed" | "cancelled" => None,
        other => return Err(unsupported("Responses terminal status", other)),
    };
    if let Some(reason) = reason {
        core.finish_reason(reason, output)?;
    }
    Ok(())
}
