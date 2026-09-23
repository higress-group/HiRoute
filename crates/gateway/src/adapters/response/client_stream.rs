use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::server::core_runtime::model_ir::{
    ExactProviderPathV1, FinishReason, ModelEvent, ModelIrError, ModelStreamEventV1, ModelUsage,
    OpaqueProviderState, ResponseBlock, ResponseBlockKind, ToolKindV1,
};
use crate::server::core_runtime::profiles::{
    ClientProtocolProfile, Fidelity, StateAffinity, StreamingRefusalSemantics,
};
use crate::server::request_plan::IngressProtocol;

use super::{
    ProtocolAdapterError, RenderedSseEvent, client_can_represent_message_phase, responses_terminal,
};

#[cfg(test)]
#[path = "client_stream_tests.rs"]
mod tests;

#[derive(Clone, Debug)]
struct ToolState {
    logical_id: String,
    kind: ToolKindV1,
    namespace: Option<String>,
    name: String,
    item_id: String,
    client_index: u32,
    finished: bool,
}

/// Incremental canonical-event to client-SSE projection. It retains only
/// identities, indexes, usage and terminal metadata; content and Tool argument
/// deltas are emitted immediately and never accumulated.
#[derive(Clone, Debug)]
pub struct IncrementalClientSseRenderer {
    profile: ClientProtocolProfile,
    alias: String,
    expected_sequence: u64,
    native_sequence: u64,
    response_id: Option<String>,
    source_owner: Option<ExactProviderPathV1>,
    started: bool,
    terminal: bool,
    usage: ModelUsage,
    finish_reason: Option<FinishReason>,
    blocks: BTreeMap<u32, ResponseBlockKind>,
    item_ids: BTreeMap<u32, String>,
    message_phases: BTreeMap<u32, String>,
    tools: BTreeMap<u32, ToolState>,
    next_tool_index: u32,
    pending_responses_reasoning: Option<u32>,
    responses_reasoning_state: BTreeMap<u32, Value>,
    responses_metadata: BTreeMap<String, Value>,
    open_messages_blocks: BTreeSet<u32>,
    pending_messages_reasoning: BTreeSet<u32>,
    retention: super::body_buffer::Retention,
}

impl IncrementalClientSseRenderer {
    pub(crate) fn with_budget(
        mut self,
        budget: hiroute_gateway_core::runtime::body::StreamBudget,
    ) -> Self {
        self.retention = super::body_buffer::Retention::new(budget);
        self
    }

    pub fn new(
        profile: ClientProtocolProfile,
        served_model_alias: impl Into<String>,
    ) -> Result<Self, ProtocolAdapterError> {
        let alias = served_model_alias.into();
        if !profile.is_complete() || alias.trim().is_empty() {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "client stream profile or served model alias is incomplete".into(),
            ));
        }
        Ok(Self {
            profile,
            retention: super::body_buffer::Retention::new(super::body_buffer::standalone_budget()),
            alias,
            expected_sequence: 0,
            native_sequence: 0,
            response_id: None,
            source_owner: None,
            started: false,
            terminal: false,
            usage: ModelUsage::default(),
            finish_reason: None,
            blocks: BTreeMap::new(),
            item_ids: BTreeMap::new(),
            message_phases: BTreeMap::new(),
            tools: BTreeMap::new(),
            next_tool_index: 0,
            pending_responses_reasoning: None,
            responses_reasoning_state: BTreeMap::new(),
            responses_metadata: BTreeMap::new(),
            open_messages_blocks: BTreeSet::new(),
            pending_messages_reasoning: BTreeSet::new(),
        })
    }

    /// Bytes of semantic output retained for later emission. This remains zero
    /// by construction; only bounded routing metadata is retained.
    pub const fn buffered_semantic_bytes(&self) -> usize {
        0
    }

    pub fn push(
        &mut self,
        event: &ModelStreamEventV1,
    ) -> Result<Vec<RenderedSseEvent>, ProtocolAdapterError> {
        if self.terminal || event.sequence != self.expected_sequence {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "client stream received a terminal or out-of-order canonical event".into(),
            )
            .into());
        }
        self.validate_event_capability(&event.event)?;
        if let ModelEvent::ResponsesMetadata { metadata } = &event.event {
            self.charge_metadata(
                serde_json::to_vec(metadata)
                    .map_err(|error| ProtocolAdapterError::Serialization(error.to_string()))?
                    .len(),
            )?;
            self.responses_metadata.extend(metadata.clone());
            self.expected_sequence += 1;
            return Ok(Vec::new());
        }
        let mut output = match self.profile.protocol {
            IngressProtocol::Responses => self.push_responses(&event.event),
            IngressProtocol::ChatCompletions => self.push_chat(&event.event),
            IngressProtocol::Messages => self.push_messages(&event.event),
        }?;
        if self.profile.protocol == IngressProtocol::Responses {
            for event in &mut output {
                if let Some(response) = event
                    .data
                    .get_mut("response")
                    .and_then(Value::as_object_mut)
                {
                    for (key, value) in &self.responses_metadata {
                        response.entry(key.clone()).or_insert_with(|| value.clone());
                    }
                }
            }
        }
        self.expected_sequence = self.expected_sequence.checked_add(1).ok_or_else(|| {
            ModelIrError::InvalidResponseLifecycle("client event sequence overflow".into())
        })?;
        Ok(output)
    }

    fn push_responses(
        &mut self,
        event: &ModelEvent,
    ) -> Result<Vec<RenderedSseEvent>, ProtocolAdapterError> {
        let mut output = Vec::new();
        let state_fills_pending = matches!(event,
            ModelEvent::ProviderState { state }
                if state.kind == "encrypted_content"
                    && state.block_index == self.pending_responses_reasoning
        );
        if !state_fills_pending {
            self.flush_responses_reasoning(&mut output)?;
        }
        match event {
            ModelEvent::ResponsesMetadata { .. } => {}
            ModelEvent::TextAnnotation {
                index,
                annotation_index,
                annotation,
            } => {
                let item_id = self.item_id(*index)?.to_owned();
                self.named(&mut output,
                    "response.output_text.annotation.added", json!({"type":"response.output_text.annotation.added","sequence_number":self.native_sequence,"item_id":item_id,"output_index":index,"content_index":0,"annotation_index":annotation_index,"annotation":annotation}));
            }
            ModelEvent::TextFinished {
                index,
                text,
                annotations,
                status,
            } => {
                let item_id = self.item_id(*index)?.to_owned();
                let part = json!({"type":"output_text","text":text,"annotations":annotations});
                self.named(&mut output, "response.output_text.done", json!({"type":"response.output_text.done","sequence_number":self.native_sequence,"item_id":item_id,"output_index":index,"content_index":0,"text":text}));
                self.named(&mut output, "response.content_part.done", json!({"type":"response.content_part.done","sequence_number":self.native_sequence,"item_id":item_id,"output_index":index,"content_index":0,"part":part}));
                let mut item = json!({"type":"message","id":item_id,"role":"assistant","status":status.as_str(),"content":[part]});
                if let Some(phase) = self.message_phases.get(index) {
                    item["phase"] = Value::String(phase.clone());
                }
                self.named(&mut output, "response.output_item.done", json!({"type":"response.output_item.done","sequence_number":self.native_sequence,"output_index":index,"item":item}));
            }
            ModelEvent::ReasoningFinished {
                index,
                text,
                status,
            } => {
                let item_id = self.item_id(*index)?.to_owned();
                let part = json!({"type":"summary_text","text":text});
                self.named(&mut output, "response.reasoning_summary_text.done", json!({"type":"response.reasoning_summary_text.done","sequence_number":self.native_sequence,"item_id":item_id,"output_index":index,"summary_index":0,"text":text}));
                self.named(&mut output, "response.reasoning_summary_part.done", json!({"type":"response.reasoning_summary_part.done","sequence_number":self.native_sequence,"item_id":item_id,"output_index":index,"summary_index":0,"part":part}));
                let mut item = json!({"type":"reasoning","id":item_id,"status":status.as_str(),"summary":[part]});
                if let Some(state) = self.responses_reasoning_state.get(index) {
                    item["encrypted_content"] = state.clone();
                }
                self.named(&mut output, "response.output_item.done", json!({"type":"response.output_item.done","sequence_number":self.native_sequence,"output_index":index,"item":item}));
            }
            ModelEvent::RefusalFinished {
                index,
                text,
                status,
            } => {
                let item_id = self.item_id(*index)?.to_owned();
                let part = json!({"type":"refusal","refusal":text});
                self.named(&mut output, "response.refusal.done", json!({"type":"response.refusal.done","sequence_number":self.native_sequence,"item_id":item_id,"output_index":index,"content_index":0,"refusal":text}));
                self.named(&mut output, "response.content_part.done", json!({"type":"response.content_part.done","sequence_number":self.native_sequence,"item_id":item_id,"output_index":index,"content_index":0,"part":part}));
                let mut item = json!({"type":"message","id":item_id,"role":"assistant","status":status.as_str(),"content":[part]});
                if let Some(phase) = self.message_phases.get(index) {
                    item["phase"] = Value::String(phase.clone());
                }
                self.named(&mut output, "response.output_item.done", json!({"type":"response.output_item.done","sequence_number":self.native_sequence,"output_index":index,"item":item}));
            }
            ModelEvent::WebSearch {
                index,
                phase,
                item,
                native_id,
                owner,
            } => {
                use crate::server::core_runtime::model_ir::WebSearchPhase;
                if *phase == WebSearchPhase::Added {
                    self.observe_source_owner(owner, native_id)?;
                    self.add_tool(
                        *index,
                        &item.id,
                        ToolKindV1::Function,
                        None,
                        "web_search",
                        *index,
                        Some(&item.id),
                    )?;
                } else if *phase == WebSearchPhase::Done {
                    self.validate_tool(*index, &item.id, ToolKindV1::Function, None, "web_search")?;
                }
                let kind = match phase {
                    WebSearchPhase::Added => "response.output_item.added",
                    WebSearchPhase::Done => "response.output_item.done",
                    WebSearchPhase::InProgress => "response.web_search_call.in_progress",
                    WebSearchPhase::Searching => "response.web_search_call.searching",
                    WebSearchPhase::Completed => "response.web_search_call.completed",
                };
                let mut value = json!({"type":kind,"sequence_number":self.native_sequence,"output_index":index});
                if matches!(phase, WebSearchPhase::Added | WebSearchPhase::Done) {
                    value["item"] = item.wire_value();
                } else {
                    value["item_id"] = item.id.clone().into();
                }
                self.named(&mut output, kind, value);
            }
            ModelEvent::ResponseStarted { response_id, .. } => {
                self.start(response_id)?;
                self.named(
                    &mut output,
                    "response.created",
                    json!({"type":"response.created","sequence_number":self.native_sequence,"response":{"id":response_id,"model":self.alias}}),
                );
            }
            ModelEvent::ContentBlockStarted {
                index,
                block_kind,
                item_id,
                phase,
            } => {
                let item_id = self.start_block(*index, *block_kind, item_id.as_deref())?;
                if let Some(phase) = phase {
                    if !matches!(
                        block_kind,
                        ResponseBlockKind::Text | ResponseBlockKind::Refusal
                    ) || self.message_phases.insert(*index, phase.clone()).is_some()
                    {
                        return Err(invalid("Responses message phase binding"));
                    }
                    self.charge_metadata(phase.len())?;
                }
                let mut item = match block_kind {
                    ResponseBlockKind::Text | ResponseBlockKind::Refusal => {
                        json!({"type":"message","id":item_id,"role":"assistant","status":"in_progress","content":[]})
                    }
                    ResponseBlockKind::Reasoning => {
                        self.pending_responses_reasoning = Some(*index);
                        return Ok(output);
                    }
                    ResponseBlockKind::ToolCall => return Err(invalid("Tool block start")),
                };
                if let Some(phase) = phase {
                    item["phase"] = Value::String(phase.clone());
                }
                self.named(&mut output, "response.output_item.added", json!({"type":"response.output_item.added","sequence_number":self.native_sequence,"output_index":index,"item":item}));
                let part = match block_kind {
                    ResponseBlockKind::Text => {
                        json!({"type":"output_text","text":"","annotations":[]})
                    }
                    ResponseBlockKind::Refusal => json!({"type":"refusal","refusal":""}),
                    _ => unreachable!("reasoning and Tool starts returned above"),
                };
                self.named(&mut output, "response.content_part.added", json!({"type":"response.content_part.added","sequence_number":self.native_sequence,"item_id":item_id,"output_index":index,"content_index":0,"part":part}));
            }
            ModelEvent::TextDelta { index, text } => {
                let item_id = self.item_id(*index)?.to_owned();
                self.named(&mut output, "response.output_text.delta", json!({"type":"response.output_text.delta","sequence_number":self.native_sequence,"item_id":item_id,"output_index":index,"content_index":0,"delta":text}));
            }
            ModelEvent::ReasoningDelta { index, text } => {
                let item_id = self.item_id(*index)?.to_owned();
                self.named(&mut output, "response.reasoning_summary_text.delta", json!({"type":"response.reasoning_summary_text.delta","sequence_number":self.native_sequence,"item_id":item_id,"output_index":index,"summary_index":0,"delta":text}));
            }
            ModelEvent::RefusalDelta { index, text } => {
                let item_id = self.item_id(*index)?.to_owned();
                self.named(&mut output, "response.refusal.delta", json!({"type":"response.refusal.delta","sequence_number":self.native_sequence,"item_id":item_id,"output_index":index,"content_index":0,"delta":text}));
            }
            ModelEvent::ToolCallStarted {
                index,
                logical_id,
                native_id,
                tool_kind,
                namespace,
                name,
                owner,
                item_id,
            } => {
                self.observe_source_owner(owner, native_id)?;
                self.add_tool(
                    *index,
                    logical_id,
                    *tool_kind,
                    namespace.as_deref(),
                    name,
                    *index,
                    item_id.as_deref(),
                )?;
                let item_id = self.tool_item_id(*index)?.to_owned();
                let (item_type, payload_field) = match tool_kind {
                    ToolKindV1::Function => ("function_call", "arguments"),
                    ToolKindV1::Custom => ("custom_tool_call", "input"),
                };
                let mut item = json!({"type":item_type,"id":item_id,"call_id":logical_id,"name":name,"status":"in_progress"});
                item[payload_field] = Value::String(String::new());
                if let Some(namespace) = namespace {
                    item["namespace"] = Value::String(namespace.clone());
                }
                self.named(&mut output, "response.output_item.added", json!({"type":"response.output_item.added","sequence_number":self.native_sequence,"output_index":index,"item":item}));
            }
            ModelEvent::ToolArgumentsDelta { index, delta, .. } => {
                let tool = self
                    .tools
                    .get(index)
                    .ok_or_else(|| invalid("unknown Responses Tool delta"))?;
                let item_id = tool.item_id.clone();
                let event = match tool.kind {
                    ToolKindV1::Function => "response.function_call_arguments.delta",
                    ToolKindV1::Custom => "response.custom_tool_call_input.delta",
                };
                self.named(&mut output, event, json!({"type":event,"sequence_number":self.native_sequence,"item_id":item_id,"output_index":index,"delta":delta}));
            }
            ModelEvent::ToolCallFinished {
                index,
                logical_id,
                tool_kind,
                namespace,
                name,
                arguments,
                status,
            } => {
                self.validate_tool(*index, logical_id, *tool_kind, namespace.as_deref(), name)?;
                let item_id = self.tool_item_id(*index)?.to_owned();
                let (item_type, payload_field, done_event, arguments) = match tool_kind {
                    ToolKindV1::Function => (
                        "function_call",
                        "arguments",
                        "response.function_call_arguments.done",
                        serde_json::to_string(arguments).map_err(|error| {
                            ProtocolAdapterError::Serialization(error.to_string())
                        })?,
                    ),
                    ToolKindV1::Custom => (
                        "custom_tool_call",
                        "input",
                        "response.custom_tool_call_input.done",
                        arguments
                            .as_str()
                            .ok_or_else(|| unrepresentable("custom tool input is not a string"))?
                            .to_owned(),
                    ),
                };
                let mut done = json!({"type":done_event,"sequence_number":self.native_sequence,"item_id":item_id,"output_index":index});
                done[payload_field] = Value::String(arguments.clone());
                self.named(&mut output, done_event, done);
                let mut item = json!({"type":item_type,"id":item_id,"call_id":logical_id,"name":name,"status":status.as_str()});
                item[payload_field] = Value::String(arguments);
                if let Some(namespace) = namespace {
                    item["namespace"] = Value::String(namespace.clone());
                }
                self.named(&mut output, "response.output_item.done", json!({"type":"response.output_item.done","sequence_number":self.native_sequence,"output_index":index,"item":item}));
            }
            ModelEvent::ProviderState { state } => {
                self.render_responses_state(&mut output, state)?
            }
            ModelEvent::UsageUpdated { usage } => self.usage.merge_from(usage),
            ModelEvent::FinishReason { reason } => self.set_finish(reason)?,
            ModelEvent::ResponseCompleted { output: snapshot } => {
                let id = self.response_id()?.to_owned();
                let reason = self
                    .finish_reason
                    .as_ref()
                    .ok_or_else(|| invalid("missing finish reason"))?;
                let terminal = responses_terminal(reason)?;
                let usage = responses_usage(&self.usage)?;
                let mut completed_output = snapshot
                    .iter()
                    .map(super::render::render_responses_output_block)
                    .collect::<Result<Vec<_>, _>>()?;
                for (block, item) in snapshot.iter().zip(&mut completed_output) {
                    if let ResponseBlock::Reasoning { index, .. } = block
                        && let Some(state) = self.responses_reasoning_state.get(index)
                    {
                        item["encrypted_content"] = state.clone();
                    }
                }
                self.named(&mut output, terminal.event, json!({"type":terminal.event,"sequence_number":self.native_sequence,"response":{"id":id,"model":self.alias,"status":terminal.status,"incomplete_details":terminal.incomplete_details,"output":completed_output,"usage":usage}}));
                self.terminal = true;
            }
            ModelEvent::ResponseFailed { error } => {
                output.push(stream_error(IngressProtocol::Responses, error));
                self.terminal = true;
            }
        }
        Ok(output)
    }

    fn push_chat(
        &mut self,
        event: &ModelEvent,
    ) -> Result<Vec<RenderedSseEvent>, ProtocolAdapterError> {
        let mut output = Vec::new();
        match event {
            ModelEvent::ResponsesMetadata { .. } => {}
            ModelEvent::TextAnnotation { .. } => {
                return Err(unrepresentable("URL citations require Responses"));
            }
            ModelEvent::TextFinished { .. }
            | ModelEvent::ReasoningFinished { .. }
            | ModelEvent::RefusalFinished { .. } => {}
            ModelEvent::WebSearch { .. } => {
                return Err(unrepresentable("hosted search requires Responses"));
            }
            ModelEvent::ResponseStarted { response_id, .. } => {
                self.start(response_id)?;
                output.push(self.chat_chunk(
                    json!({"role":"assistant"}),
                    Value::Null,
                    Value::Null,
                )?);
            }
            ModelEvent::ContentBlockStarted {
                index, block_kind, ..
            } => {
                self.start_block(*index, *block_kind, None)?;
            }
            ModelEvent::TextDelta { text, .. } => {
                output.push(self.chat_chunk(json!({"content":text}), Value::Null, Value::Null)?)
            }
            ModelEvent::ReasoningDelta { text, .. } => output.push(self.chat_chunk(
                json!({"reasoning_content":text}),
                Value::Null,
                Value::Null,
            )?),
            ModelEvent::RefusalDelta { text, .. } => {
                output.push(self.chat_chunk(json!({"refusal":text}), Value::Null, Value::Null)?)
            }
            ModelEvent::ToolCallStarted {
                index,
                logical_id,
                native_id,
                tool_kind,
                namespace,
                name,
                owner,
                ..
            } => {
                if namespace.is_some() || *tool_kind != ToolKindV1::Function {
                    return Err(unrepresentable(
                        "namespaced or custom Tool call requires Responses",
                    ));
                }
                self.observe_source_owner(owner, native_id)?;
                let client_index = self.next_tool_index;
                self.next_tool_index = self
                    .next_tool_index
                    .checked_add(1)
                    .ok_or_else(|| invalid("Tool index overflow"))?;
                self.add_tool(
                    *index,
                    logical_id,
                    *tool_kind,
                    None,
                    name,
                    client_index,
                    None,
                )?;
                output.push(self.chat_chunk(json!({"tool_calls":[{"index":client_index,"id":logical_id,"type":"function","function":{"name":name,"arguments":""}}]}), Value::Null, Value::Null)?);
            }
            ModelEvent::ToolArgumentsDelta { index, delta, .. } => {
                let tool = self
                    .tools
                    .get(index)
                    .ok_or_else(|| invalid("unknown Tool delta"))?;
                output.push(self.chat_chunk(json!({"tool_calls":[{"index":tool.client_index,"function":{"arguments":delta}}]}), Value::Null, Value::Null)?);
            }
            ModelEvent::ToolCallFinished {
                index,
                logical_id,
                tool_kind,
                namespace,
                name,
                ..
            } => {
                if namespace.is_some() || *tool_kind != ToolKindV1::Function {
                    return Err(unrepresentable(
                        "namespaced or custom Tool call requires Responses",
                    ));
                }
                self.validate_tool(*index, logical_id, *tool_kind, None, name)?;
            }
            ModelEvent::ProviderState { .. } => {
                return Err(unrepresentable("Chat stream provider state"));
            }
            ModelEvent::UsageUpdated { usage } => self.usage.merge_from(usage),
            ModelEvent::FinishReason { reason } => self.set_finish(reason)?,
            ModelEvent::ResponseCompleted { .. } => {
                let reason = self
                    .finish_reason
                    .as_ref()
                    .ok_or_else(|| invalid("missing finish reason"))?;
                output.push(self.chat_chunk(
                    json!({}),
                    Value::String(chat_finish(reason).into()),
                    chat_usage(&self.usage)?,
                )?);
                output.push(RenderedSseEvent {
                    event: None,
                    data: Value::String("[DONE]".into()),
                });
                self.terminal = true;
            }
            ModelEvent::ResponseFailed { error } => {
                output.push(stream_error(IngressProtocol::ChatCompletions, error));
                self.terminal = true;
            }
        }
        Ok(output)
    }

    fn push_messages(
        &mut self,
        event: &ModelEvent,
    ) -> Result<Vec<RenderedSseEvent>, ProtocolAdapterError> {
        let mut output = Vec::new();
        let state_fills_pending = match event {
            ModelEvent::ProviderState { state }
                if matches!(
                    state.kind.as_str(),
                    "thinking_signature" | "encrypted_content"
                ) =>
            {
                state.block_index
            }
            _ => None,
        };
        self.flush_messages_reasoning_except(state_fills_pending, &mut output)?;
        match event {
            ModelEvent::ResponsesMetadata { .. } => {}
            ModelEvent::TextAnnotation { .. } => return Err(unrepresentable("URL citations require Responses")),
            ModelEvent::TextFinished { index, .. }
            | ModelEvent::RefusalFinished { index, .. } => {
                self.close_messages_block(*index, &mut output);
            }
            ModelEvent::ReasoningFinished { index, .. } => {
                if !self.open_messages_blocks.contains(index)
                    || !self.pending_messages_reasoning.insert(*index)
                {
                    return Err(invalid("Messages reasoning completion"));
                }
            }
            ModelEvent::WebSearch { .. } => return Err(unrepresentable("hosted search requires Responses")),
            ModelEvent::ResponseStarted { response_id, .. } => {
                self.start(response_id)?;
                self.start_messages_wire(&mut output)?;
            }
            ModelEvent::UsageUpdated { usage } => {
                self.usage.merge_from(usage);
            }
            ModelEvent::ContentBlockStarted { index, block_kind, .. } => {
                self.require_messages_started()?;
                self.start_block(*index, *block_kind, None)?;
                let block = match block_kind {
                    ResponseBlockKind::Text | ResponseBlockKind::Refusal => json!({"type":"text","text":""}),
                    ResponseBlockKind::Reasoning => json!({"type":"thinking","thinking":""}),
                    ResponseBlockKind::ToolCall => return Err(invalid("Tool block start")),
                };
                self.named(&mut output, "content_block_start", json!({"type":"content_block_start","index":index,"content_block":block}));
                self.open_messages_blocks.insert(*index);
            }
            ModelEvent::TextDelta { index, text } | ModelEvent::RefusalDelta { index, text } => self.named(&mut output, "content_block_delta", json!({"type":"content_block_delta","index":index,"delta":{"type":"text_delta","text":text}})),
            ModelEvent::ReasoningDelta { index, text } => self.named(&mut output, "content_block_delta", json!({"type":"content_block_delta","index":index,"delta":{"type":"thinking_delta","thinking":text}})),
            ModelEvent::ToolCallStarted { index, logical_id, native_id, tool_kind, namespace, name, owner, .. } => {
                if namespace.is_some() || *tool_kind != ToolKindV1::Function {
                    return Err(unrepresentable("namespaced or custom Tool call requires Responses"));
                }
                self.observe_source_owner(owner, native_id)?;
                self.require_messages_started()?;
                self.add_tool(*index, logical_id, *tool_kind, None, name, *index, None)?;
                self.named(&mut output, "content_block_start", json!({"type":"content_block_start","index":index,"content_block":{"type":"tool_use","id":logical_id,"name":name,"input":{}}}));
                self.open_messages_blocks.insert(*index);
            }
            ModelEvent::ToolArgumentsDelta { index, delta, .. } => self.named(&mut output, "content_block_delta", json!({"type":"content_block_delta","index":index,"delta":{"type":"input_json_delta","partial_json":delta}})),
            ModelEvent::ToolCallFinished { index, logical_id, tool_kind, namespace, name, .. } => {
                if namespace.is_some() || *tool_kind != ToolKindV1::Function {
                    return Err(unrepresentable("namespaced or custom Tool call requires Responses"));
                }
                self.validate_tool(*index, logical_id, *tool_kind, None, name)?;
                self.close_messages_block(*index, &mut output);
            }
            ModelEvent::ProviderState { state } => self.render_messages_state(&mut output, state)?,
            ModelEvent::FinishReason { reason } => self.set_finish(reason)?,
            ModelEvent::ResponseCompleted { .. } => {
                self.require_messages_started()?;
                self.close_all_messages_blocks(&mut output);
                let reason = self.finish_reason.as_ref().ok_or_else(|| invalid("missing finish reason"))?;
                let usage = messages_usage(&self.usage)?;
                self.named(&mut output, "message_delta", json!({"type":"message_delta","delta":{"stop_reason":messages_finish(reason),"stop_sequence":null},"usage":usage}));
                self.named(&mut output, "message_stop", json!({"type":"message_stop"}));
                self.terminal = true;
            }
            ModelEvent::ResponseFailed { error } => {
                output.push(stream_error(IngressProtocol::Messages, error));
                self.terminal = true;
            }
        }
        Ok(output)
    }

    fn start(&mut self, response_id: &str) -> Result<(), ProtocolAdapterError> {
        if self.response_id.is_some() || response_id.is_empty() {
            return Err(invalid("response start"));
        }
        self.charge_metadata(response_id.len())?;
        self.response_id = Some(response_id.into());
        self.started = self.profile.protocol != IngressProtocol::Messages;
        Ok(())
    }

    fn validate_event_capability(&self, event: &ModelEvent) -> Result<(), ProtocolAdapterError> {
        let response = &self.profile.response;
        let exact = Fidelity::Exact;
        let supported = match event {
            ModelEvent::TextAnnotation { .. } => {
                self.profile.protocol == IngressProtocol::Responses
            }
            ModelEvent::TextFinished { .. }
            | ModelEvent::ReasoningFinished { .. }
            | ModelEvent::RefusalFinished { .. } => true,
            ModelEvent::WebSearch { .. } => self.profile.protocol == IngressProtocol::Responses,
            ModelEvent::ResponsesMetadata { .. }
            | ModelEvent::ResponseStarted { .. }
            | ModelEvent::ResponseCompleted { .. } => true,
            ModelEvent::ContentBlockStarted {
                block_kind, phase, ..
            } => match block_kind {
                ResponseBlockKind::Text => {
                    response.text == exact
                        && client_can_represent_message_phase(
                            self.profile.protocol,
                            phase.as_deref(),
                        )
                }
                ResponseBlockKind::Reasoning => response.reasoning == exact,
                ResponseBlockKind::Refusal => {
                    response.refusal == exact
                        && client_can_represent_message_phase(
                            self.profile.protocol,
                            phase.as_deref(),
                        )
                }
                ResponseBlockKind::ToolCall => false,
            },
            ModelEvent::TextDelta { .. } => {
                response.text == exact && response.stream_text_delta == exact
            }
            ModelEvent::ReasoningDelta { .. } => {
                response.reasoning == exact && response.stream_reasoning_delta == exact
            }
            ModelEvent::RefusalDelta { .. } => {
                response.refusal == exact
                    && response.stream_text_delta == exact
                    && response.stream_refusal == StreamingRefusalSemantics::ExactDelta
            }
            ModelEvent::ToolCallStarted {
                tool_kind,
                namespace,
                ..
            }
            | ModelEvent::ToolCallFinished {
                tool_kind,
                namespace,
                ..
            } => {
                response.tool_calls == exact
                    && response.logical_tool_id_mapping == exact
                    && response.stream_tool_argument_delta == exact
                    && (namespace.is_none() || self.profile.protocol == IngressProtocol::Responses)
                    && (*tool_kind == ToolKindV1::Function
                        || self.profile.protocol == IngressProtocol::Responses)
            }
            ModelEvent::ToolArgumentsDelta { .. } => {
                response.tool_calls == exact
                    && response.logical_tool_id_mapping == exact
                    && response.stream_tool_argument_delta == exact
            }
            ModelEvent::ProviderState { .. } => true,
            ModelEvent::UsageUpdated { .. } => {
                response.usage == exact && response.stream_usage == exact
            }
            ModelEvent::FinishReason { .. } => response.finish_reason == exact,
            ModelEvent::ResponseFailed { .. } => response.typed_error == exact,
        };
        if supported {
            Ok(())
        } else {
            Err(unrepresentable(
                "client stream profile cannot preserve canonical event",
            ))
        }
    }

    fn start_messages_wire(
        &mut self,
        output: &mut Vec<RenderedSseEvent>,
    ) -> Result<(), ProtocolAdapterError> {
        let id = self.response_id()?.to_owned();
        self.named(output, "message_start", json!({"type":"message_start","message":{"id":id,"type":"message","role":"assistant","content":[],"model":self.alias,"stop_reason":null,"stop_sequence":null,"usage":{}}}));
        self.started = true;
        Ok(())
    }

    fn require_messages_started(&self) -> Result<(), ProtocolAdapterError> {
        if self.started {
            Ok(())
        } else {
            Err(unrepresentable(
                "Messages stream content before exact input usage",
            ))
        }
    }

    fn start_block(
        &mut self,
        index: u32,
        kind: ResponseBlockKind,
        native_item_id: Option<&str>,
    ) -> Result<String, ProtocolAdapterError> {
        if self.blocks.insert(index, kind).is_some() {
            return Err(invalid("duplicate content block"));
        }
        let prefix = if kind == ResponseBlockKind::Reasoning {
            "rs"
        } else {
            "msg"
        };
        let item_id = self.bind_client_item_id(index, prefix, native_item_id)?;
        Ok(item_id)
    }

    // Renderer identity is checked component-by-component before any event is
    // emitted, so keep the complete tuple visible at the call boundary.
    #[allow(clippy::too_many_arguments)]
    fn add_tool(
        &mut self,
        index: u32,
        logical_id: &str,
        kind: ToolKindV1,
        namespace: Option<&str>,
        name: &str,
        client_index: u32,
        native_item_id: Option<&str>,
    ) -> Result<(), ProtocolAdapterError> {
        if logical_id.is_empty() || name.is_empty() || self.tools.contains_key(&index) {
            return Err(invalid("Tool identity"));
        }
        let prefix = if kind == ToolKindV1::Custom {
            "ct"
        } else {
            "fc"
        };
        let item_id = self.bind_client_item_id(index, prefix, native_item_id)?;
        self.charge_metadata(
            logical_id
                .len()
                .saturating_add(namespace.map_or(0, str::len))
                .saturating_add(name.len()),
        )?;
        self.tools.insert(
            index,
            ToolState {
                logical_id: logical_id.into(),
                kind,
                namespace: namespace.map(str::to_owned),
                name: name.into(),
                item_id,
                client_index,
                finished: false,
            },
        );
        Ok(())
    }

    fn bind_client_item_id(
        &mut self,
        index: u32,
        prefix: &str,
        native_item_id: Option<&str>,
    ) -> Result<String, ProtocolAdapterError> {
        let item_id = native_item_id
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{prefix}_{index}"));
        if item_id.is_empty()
            || item_id.chars().any(char::is_control)
            || self.item_ids.contains_key(&index)
            || self.item_ids.values().any(|current| current == &item_id)
            || self.tools.values().any(|tool| tool.item_id == item_id)
        {
            return Err(invalid("duplicate or invalid Responses item ID"));
        }
        self.charge_metadata(item_id.len())?;
        self.item_ids.insert(index, item_id.clone());
        Ok(item_id)
    }

    fn item_id(&self, index: u32) -> Result<&str, ProtocolAdapterError> {
        self.item_ids
            .get(&index)
            .map(String::as_str)
            .ok_or_else(|| invalid("unknown Responses content item"))
    }

    fn tool_item_id(&self, index: u32) -> Result<&str, ProtocolAdapterError> {
        self.tools
            .get(&index)
            .map(|tool| tool.item_id.as_str())
            .ok_or_else(|| invalid("unknown Responses Tool item"))
    }

    fn validate_tool(
        &mut self,
        index: u32,
        logical_id: &str,
        kind: ToolKindV1,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<(), ProtocolAdapterError> {
        match self.tools.get_mut(&index) {
            Some(tool)
                if tool.logical_id == logical_id
                    && tool.kind == kind
                    && tool.namespace.as_deref() == namespace
                    && tool.name == name
                    && !tool.finished =>
            {
                tool.finished = true;
                Ok(())
            }
            _ => Err(invalid("Tool completion identity")),
        }
    }

    fn observe_source_owner(
        &mut self,
        owner: &ExactProviderPathV1,
        native_id: &str,
    ) -> Result<(), ProtocolAdapterError> {
        if !owner.is_complete()
            || native_id.trim().is_empty()
            || self
                .source_owner
                .as_ref()
                .is_some_and(|current| current != owner)
        {
            return Err(invalid("Tool physical owner binding"));
        }
        if self.source_owner.is_none() {
            self.source_owner = Some(owner.clone());
        }
        self.charge_metadata(native_id.len())
    }

    fn set_finish(&mut self, reason: &FinishReason) -> Result<(), ProtocolAdapterError> {
        if self.finish_reason.replace(reason.clone()).is_some() {
            return Err(invalid("duplicate finish reason"));
        }
        Ok(())
    }

    fn close_messages_block(&mut self, index: u32, output: &mut Vec<RenderedSseEvent>) {
        if self.open_messages_blocks.remove(&index) {
            self.named(
                output,
                "content_block_stop",
                json!({"type":"content_block_stop","index":index}),
            );
        }
    }

    fn close_all_messages_blocks(&mut self, output: &mut Vec<RenderedSseEvent>) {
        self.pending_messages_reasoning.clear();
        for index in std::mem::take(&mut self.open_messages_blocks) {
            self.named(
                output,
                "content_block_stop",
                json!({"type":"content_block_stop","index":index}),
            );
        }
    }

    fn flush_messages_reasoning_except(
        &mut self,
        preserve: Option<u32>,
        output: &mut Vec<RenderedSseEvent>,
    ) -> Result<(), ProtocolAdapterError> {
        let pending = std::mem::take(&mut self.pending_messages_reasoning);
        for index in pending {
            if preserve == Some(index) {
                self.pending_messages_reasoning.insert(index);
            } else if self.open_messages_blocks.contains(&index) {
                self.close_messages_block(index, output);
            } else {
                return Err(invalid("Messages pending reasoning block"));
            }
        }
        Ok(())
    }

    fn render_responses_state(
        &mut self,
        output: &mut Vec<RenderedSseEvent>,
        state: &OpaqueProviderState,
    ) -> Result<(), ProtocolAdapterError> {
        self.check_state_owner(state)?;
        if state.kind != "encrypted_content" {
            return Err(unrepresentable("Responses provider-state kind"));
        }
        let index = state
            .block_index
            .filter(|index| self.blocks.get(index) == Some(&ResponseBlockKind::Reasoning))
            .ok_or_else(|| unrepresentable("Responses provider-state block binding"))?;
        let fills_pending = self.pending_responses_reasoning == Some(index);
        if fills_pending {
            self.pending_responses_reasoning = None;
        }
        let item_id = self.item_id(index)?.to_owned();
        let state_bytes = serde_json::to_vec(&state.value)
            .map_err(|error| ProtocolAdapterError::Serialization(error.to_string()))?
            .len();
        self.charge_metadata(state_bytes)?;
        if self
            .responses_reasoning_state
            .insert(index, state.value.clone())
            .is_some()
        {
            return Err(invalid("duplicate Responses reasoning state"));
        }
        if fills_pending {
            self.named(output, "response.output_item.added", json!({"type":"response.output_item.added","sequence_number":self.native_sequence,"output_index":index,"item":{"type":"reasoning","id":item_id,"status":"in_progress","summary":[],"encrypted_content":state.value}}));
            self.named(output, "response.reasoning_summary_part.added", json!({"type":"response.reasoning_summary_part.added","sequence_number":self.native_sequence,"item_id":item_id,"output_index":index,"summary_index":0,"part":{"type":"summary_text","text":""}}));
        }
        Ok(())
    }

    fn flush_responses_reasoning(
        &mut self,
        output: &mut Vec<RenderedSseEvent>,
    ) -> Result<(), ProtocolAdapterError> {
        if let Some(index) = self.pending_responses_reasoning.take() {
            let item_id = self.item_id(index)?.to_owned();
            self.named(output, "response.output_item.added", json!({"type":"response.output_item.added","sequence_number":self.native_sequence,"output_index":index,"item":{"type":"reasoning","id":item_id,"status":"in_progress","summary":[]}}));
            self.named(output, "response.reasoning_summary_part.added", json!({"type":"response.reasoning_summary_part.added","sequence_number":self.native_sequence,"item_id":item_id,"output_index":index,"summary_index":0,"part":{"type":"summary_text","text":""}}));
        }
        Ok(())
    }

    fn render_messages_state(
        &mut self,
        output: &mut Vec<RenderedSseEvent>,
        state: &OpaqueProviderState,
    ) -> Result<(), ProtocolAdapterError> {
        self.check_state_owner(state)?;
        let index = state
            .block_index
            .ok_or_else(|| unrepresentable("Messages provider-state block binding"))?;
        match (state.owner.upstream_protocol, state.kind.as_str()) {
            (IngressProtocol::Messages, "thinking_signature")
            | (IngressProtocol::Responses, "encrypted_content") => {
                let signature = state
                    .value
                    .as_str()
                    .filter(|signature| !signature.is_empty())
                    .ok_or_else(|| unrepresentable("Messages thinking signature value"))?;
                if !self.open_messages_blocks.contains(&index) {
                    return Err(unrepresentable("Messages thinking signature lifecycle"));
                }
                self.named(
                    output,
                    "content_block_delta",
                    json!({"type":"content_block_delta","index":index,"delta":{"type":"signature_delta","signature":signature}}),
                );
                if self.pending_messages_reasoning.remove(&index) {
                    self.close_messages_block(index, output);
                }
            }
            (IngressProtocol::Messages, "redacted_thinking") => {
                self.named(
                    output,
                    "content_block_start",
                    json!({"type":"content_block_start","index":index,"content_block":state.value}),
                );
                self.named(
                    output,
                    "content_block_stop",
                    json!({"type":"content_block_stop","index":index}),
                );
            }
            _ => return Err(unrepresentable("Messages streaming provider-state kind")),
        }
        Ok(())
    }

    fn check_state_owner(
        &mut self,
        state: &OpaqueProviderState,
    ) -> Result<(), ProtocolAdapterError> {
        if self.profile.response.provider_state != Fidelity::Exact
            || self.profile.response.state_affinity != StateAffinity::ExactOwner
            || self.profile.state_owner.as_ref() != Some(&state.owner)
            || self
                .source_owner
                .as_ref()
                .is_some_and(|owner| owner != &state.owner)
        {
            return Err(unrepresentable("provider-state exact owner"));
        }
        if self.source_owner.is_none() {
            self.source_owner = Some(state.owner.clone());
        }
        Ok(())
    }

    fn response_id(&self) -> Result<&str, ProtocolAdapterError> {
        self.response_id
            .as_deref()
            .ok_or_else(|| invalid("response identity"))
    }

    fn chat_chunk(
        &self,
        delta: Value,
        finish_reason: Value,
        usage: Value,
    ) -> Result<RenderedSseEvent, ProtocolAdapterError> {
        Ok(RenderedSseEvent {
            event: None,
            data: json!({"id":self.response_id()?,"object":"chat.completion.chunk","created":0,"model":self.alias,"choices":[{"index":0,"delta":delta,"finish_reason":finish_reason}],"usage":usage}),
        })
    }

    fn named(&mut self, output: &mut Vec<RenderedSseEvent>, event: &str, data: Value) {
        output.push(RenderedSseEvent {
            event: Some(event.into()),
            data,
        });
        self.native_sequence += 1;
    }

    fn charge_metadata(&mut self, bytes: usize) -> Result<(), ProtocolAdapterError> {
        self.retention.add(bytes)
    }
}

fn responses_usage(usage: &ModelUsage) -> Result<Value, ProtocolAdapterError> {
    let input = usage
        .input_tokens
        .ok_or_else(|| unrepresentable("Responses input usage"))?;
    let output = usage
        .output_tokens
        .ok_or_else(|| unrepresentable("Responses output usage"))?;
    if usage.cache_write_tokens.is_some() {
        return Err(unrepresentable("Responses cache-write usage"));
    }
    let mut value = json!({"input_tokens":input,"output_tokens":output,"total_tokens":input.saturating_add(output)});
    if let Some(cache) = usage.cache_read_tokens {
        value["input_tokens_details"] = json!({"cached_tokens":cache});
    }
    if let Some(reasoning) = usage.reasoning_tokens {
        value["output_tokens_details"] = json!({"reasoning_tokens":reasoning});
    }
    Ok(value)
}

fn chat_usage(usage: &ModelUsage) -> Result<Value, ProtocolAdapterError> {
    let input = usage
        .input_tokens
        .ok_or_else(|| unrepresentable("Chat input usage"))?;
    let output = usage
        .output_tokens
        .ok_or_else(|| unrepresentable("Chat output usage"))?;
    if usage.cache_write_tokens.is_some() {
        return Err(unrepresentable("Chat cache-write usage"));
    }
    let mut value = json!({"prompt_tokens":input,"completion_tokens":output,"total_tokens":input.saturating_add(output)});
    if let Some(cache) = usage.cache_read_tokens {
        value["prompt_tokens_details"] = json!({"cached_tokens":cache});
    }
    if let Some(reasoning) = usage.reasoning_tokens {
        value["completion_tokens_details"] = json!({"reasoning_tokens":reasoning});
    }
    Ok(value)
}

fn messages_usage(usage: &ModelUsage) -> Result<Value, ProtocolAdapterError> {
    let input = usage
        .input_tokens
        .ok_or_else(|| unrepresentable("Messages input usage"))?;
    let output = usage
        .output_tokens
        .ok_or_else(|| unrepresentable("Messages output usage"))?;
    // Anthropic Messages has no field for the Responses reasoning-token
    // breakdown. Those tokens are already included in `output_tokens`, so the
    // protocol projection keeps the exact billable total while the canonical
    // event remains available to HiRoute observation.
    let mut value = json!({"input_tokens":input,"output_tokens":output});
    if let Some(cache) = usage.cache_read_tokens {
        value["cache_read_input_tokens"] = Value::from(cache);
    }
    if let Some(cache) = usage.cache_write_tokens {
        value["cache_creation_input_tokens"] = Value::from(cache);
    }
    Ok(value)
}

fn chat_finish(reason: &FinishReason) -> &'static str {
    match reason {
        FinishReason::Length => "length",
        FinishReason::ToolCall => "tool_calls",
        FinishReason::Refusal => "content_filter",
        _ => "stop",
    }
}

fn messages_finish(reason: &FinishReason) -> &'static str {
    match reason {
        FinishReason::Length => "max_tokens",
        FinishReason::ToolCall => "tool_use",
        FinishReason::Refusal => "refusal",
        _ => "end_turn",
    }
}

fn stream_error(
    protocol: IngressProtocol,
    error: &crate::server::core_runtime::model_ir::ModelError,
) -> RenderedSseEvent {
    RenderedSseEvent {
        event: (protocol != IngressProtocol::ChatCompletions).then(|| "error".into()),
        data: json!({"type":"error","error":{"type":error.code.clone().unwrap_or_else(|| "provider_error".into()),"code":error.code,"message":error.message}}),
    }
}

fn invalid(label: &str) -> ProtocolAdapterError {
    ModelIrError::InvalidResponseLifecycle(label.into()).into()
}

fn unrepresentable(label: &str) -> ProtocolAdapterError {
    ProtocolAdapterError::ClientUnrepresentable(label.into())
}
