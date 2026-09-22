//! Stateful native response decoding and client protocol rendering.

mod client_stream;
mod completion;
mod decoder;
mod messages;
mod native_passthrough;
mod native_passthrough_sse;
mod protocols;
mod render;
mod responses_lifecycle;
mod search;
mod wire;

#[cfg(test)]
#[path = "native_passthrough_tests.rs"]
mod native_passthrough_tests;

#[cfg(test)]
pub(super) fn test_tool_projection() -> super::continuation::ToolIdProjection {
    // Production installs the process key before requests. Golden response tests must supply
    // their own request-bound projection so parallel runtime tests cannot change their result.
    super::continuation::ToolIdProjection::new(IngressProtocol::Responses)
}

pub use decoder::{DecodedNativeResponse, NativeResponseDecoder, ResponseDecodeStatus};
pub(crate) use native_passthrough::{NativeResponseProjector, NativeTerminalOutcome};
pub use render::{ClientResponseRenderer, RenderedClientResponse, RenderedSseEvent};

use std::collections::{BTreeMap, VecDeque};

use serde_json::{Map, Value};

use crate::server::core_runtime::model_ir::{
    ExactProviderPathV1, FinishReason, ModelError, ModelEvent, ModelIrError, ModelResponseIRV1,
    ModelStreamEventV1, ModelUsage, MutableResponseBlock, OpaqueProviderState, ResponseAccumulator,
    ResponseBlockKind, ResponseItemStatus, ToolKindV1,
};
use crate::server::core_runtime::profiles::NativeProviderStateEmission;
use crate::server::request_plan::IngressProtocol;

use super::ProtocolAdapterError;

const MAX_CANONICAL_SEMANTIC_BYTES: usize = 16 * 1024 * 1024;
const MAX_CANONICAL_BLOCKS: u32 = 65_536;

/// Responses labels its ordinary assistant answer with `final_answer`. Messages and Chat
/// Completions have only that assistant-output channel, so the label is losslessly implicit in
/// those protocols. Every other named phase remains Responses-only and must fail closed.
fn client_can_represent_message_phase(protocol: IngressProtocol, phase: Option<&str>) -> bool {
    phase.is_none() || protocol == IngressProtocol::Responses || phase == Some("final_answer")
}

struct ResponsesTerminalProjection {
    event: &'static str,
    status: &'static str,
    incomplete_details: Value,
}

fn responses_terminal(
    reason: &FinishReason,
) -> Result<ResponsesTerminalProjection, ProtocolAdapterError> {
    let terminal = match reason {
        FinishReason::Stop | FinishReason::ToolCall => ResponsesTerminalProjection {
            event: "response.completed",
            status: "completed",
            incomplete_details: Value::Null,
        },
        FinishReason::Length => ResponsesTerminalProjection {
            event: "response.incomplete",
            status: "incomplete",
            incomplete_details: serde_json::json!({"reason":"max_output_tokens"}),
        },
        FinishReason::Refusal => ResponsesTerminalProjection {
            event: "response.incomplete",
            status: "incomplete",
            incomplete_details: serde_json::json!({"reason":"content_filter"}),
        },
        FinishReason::Cancelled | FinishReason::Other(_) => {
            return Err(ProtocolAdapterError::ClientUnrepresentable(
                "Responses has no exact terminal projection for the finish reason".into(),
            ));
        }
    };
    Ok(terminal)
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum NativeBlockKind {
    Search,
    Text,
    Reasoning,
    Refusal,
    Tool,
}

#[derive(Clone, Debug)]
struct DecoderCore {
    owner: ExactProviderPathV1,
    tool_id_projection: Option<super::continuation::ToolIdProjection>,
    provider_state_emission: NativeProviderStateEmission,
    accumulator: ResponseAccumulator,
    native_blocks: BTreeMap<(NativeBlockKind, u32), u32>,
    native_item_ids: BTreeMap<u32, String>,
    native_item_id_values: std::collections::BTreeSet<String>,
    responses_encrypted_fallback: BTreeMap<u32, Value>,
    responses_encrypted_final: BTreeMap<u32, Option<Value>>,
    responses_encrypted_buffered_bytes: usize,
    native_message_phases: BTreeMap<u32, String>,
    search_ids: std::collections::BTreeSet<String>,
    next_block: u32,
    next_sequence: u64,
    semantic_bytes: usize,
}

impl DecoderCore {
    fn new(
        owner: ExactProviderPathV1,
        provider_state_emission: NativeProviderStateEmission,
        tool_id_projection: Option<super::continuation::ToolIdProjection>,
    ) -> Self {
        Self {
            owner,
            tool_id_projection,
            provider_state_emission,
            accumulator: ResponseAccumulator::default(),
            native_blocks: BTreeMap::new(),
            native_item_ids: BTreeMap::new(),
            native_item_id_values: std::collections::BTreeSet::new(),
            responses_encrypted_fallback: BTreeMap::new(),
            responses_encrypted_final: BTreeMap::new(),
            responses_encrypted_buffered_bytes: 0,
            native_message_phases: BTreeMap::new(),
            search_ids: std::collections::BTreeSet::new(),
            next_block: 0,
            next_sequence: 0,
            semantic_bytes: 0,
        }
    }

    fn block_index(
        &mut self,
        kind: NativeBlockKind,
        native_index: u32,
    ) -> Result<(u32, bool), ProtocolAdapterError> {
        let conflict = if kind == NativeBlockKind::Search {
            [
                NativeBlockKind::Text,
                NativeBlockKind::Reasoning,
                NativeBlockKind::Refusal,
                NativeBlockKind::Tool,
            ]
            .iter()
            .any(|previous| self.native_blocks.contains_key(&(*previous, native_index)))
        } else {
            self.native_blocks
                .contains_key(&(NativeBlockKind::Search, native_index))
        };
        if conflict {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "search output index reused by another item kind".into(),
            )
            .into());
        }
        if let Some(index) = self.native_blocks.get(&(kind, native_index)) {
            return Ok((*index, false));
        }
        let index = self.next_block;
        if index >= MAX_CANONICAL_BLOCKS {
            return Err(ModelIrError::BufferLimit(MAX_CANONICAL_BLOCKS as usize).into());
        }
        self.next_block = self.next_block.checked_add(1).ok_or_else(|| {
            ModelIrError::InvalidResponseLifecycle("content block index overflow".into())
        })?;
        self.native_blocks.insert((kind, native_index), index);
        Ok((index, true))
    }

    fn emit(
        &mut self,
        event: ModelEvent,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        self.charge_semantics(&event)?;
        self.apply(&event)?;
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.checked_add(1).ok_or_else(|| {
            ModelIrError::InvalidResponseLifecycle("event sequence overflow".into())
        })?;
        output.push_back(ModelStreamEventV1::new(sequence, event));
        Ok(())
    }

    fn register_native_item_id(
        &mut self,
        native_index: u32,
        item_id: &str,
    ) -> Result<(), ProtocolAdapterError> {
        if item_id.is_empty()
            || item_id.len() > 256
            || item_id.chars().any(char::is_control)
            || self.native_item_ids.len() >= MAX_CANONICAL_BLOCKS as usize
            || self.native_item_ids.contains_key(&native_index)
            || !self.native_item_id_values.insert(item_id.into())
        {
            return Err(ModelIrError::InvalidField("response output item id").into());
        }
        self.native_item_ids.insert(native_index, item_id.into());
        Ok(())
    }

    fn validate_native_item_id(
        &self,
        native_index: u32,
        item_id: &str,
    ) -> Result<(), ProtocolAdapterError> {
        if self.native_item_ids.get(&native_index).map(String::as_str) == Some(item_id) {
            Ok(())
        } else {
            Err(ModelIrError::InvalidResponseLifecycle(
                "Responses event item ID changed during the stream".into(),
            )
            .into())
        }
    }

    fn observe_responses_encrypted_fallback(
        &mut self,
        native_index: u32,
        value: Value,
    ) -> Result<(), ProtocolAdapterError> {
        if !self.native_item_ids.contains_key(&native_index)
            || self.responses_encrypted_final.contains_key(&native_index)
            || self
                .responses_encrypted_fallback
                .contains_key(&native_index)
        {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "invalid Responses encrypted reasoning fallback".into(),
            )
            .into());
        }
        let bytes = serde_json::to_vec(&value)
            .map_err(|error| ModelIrError::InvalidJson(error.to_string()))?
            .len();
        self.responses_encrypted_buffered_bytes = self
            .responses_encrypted_buffered_bytes
            .checked_add(bytes)
            .filter(|next| *next <= MAX_CANONICAL_SEMANTIC_BYTES)
            .ok_or(ModelIrError::BufferLimit(MAX_CANONICAL_SEMANTIC_BYTES))?;
        self.responses_encrypted_fallback
            .insert(native_index, value);
        Ok(())
    }

    fn finalize_responses_encrypted_content(
        &mut self,
        native_index: u32,
        authoritative: Option<&Value>,
        final_snapshot: bool,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        if let Some(established) = self.responses_encrypted_final.get(&native_index) {
            if authoritative.is_some() && established.as_ref() != authoritative {
                return Err(ModelIrError::InvalidResponseLifecycle(
                    "Responses final encrypted reasoning state disagrees with output_item.done"
                        .into(),
                )
                .into());
            }
            return Ok(());
        }

        let fallback = self.responses_encrypted_fallback.remove(&native_index);
        if let Some(value) = fallback.as_ref() {
            self.responses_encrypted_buffered_bytes =
                self.responses_encrypted_buffered_bytes.saturating_sub(
                    serde_json::to_vec(value)
                        .map_err(|error| ModelIrError::InvalidJson(error.to_string()))?
                        .len(),
                );
        }
        let selected = authoritative.cloned().or(fallback);
        if let Some(value) = selected.as_ref() {
            self.provider_state(
                IngressProtocol::Responses,
                "encrypted_content",
                value.clone(),
                Some(native_index),
                output,
            )?;
        }
        self.responses_encrypted_final
            .insert(native_index, selected);

        if !final_snapshot && !self.native_item_ids.contains_key(&native_index) {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "Responses encrypted reasoning state lacks an item identity".into(),
            )
            .into());
        }
        Ok(())
    }

    fn native_item_id(&self, native_index: u32) -> Option<String> {
        self.native_item_ids.get(&native_index).cloned()
    }

    fn observe_message_phase(
        &mut self,
        native_index: u32,
        phase: Option<&str>,
        complete_item: bool,
    ) -> Result<(), ProtocolAdapterError> {
        let Some(phase) = phase else {
            if complete_item && self.native_message_phases.contains_key(&native_index) {
                return Err(ModelIrError::InvalidResponseLifecycle(
                    "Responses completed message omitted its established phase".into(),
                )
                .into());
            }
            return Ok(());
        };
        if phase.is_empty() || phase.len() > 64 || phase.chars().any(char::is_control) {
            return Err(ModelIrError::InvalidField("response output message phase").into());
        }
        if let Some(current) = self.native_message_phases.get(&native_index) {
            if current == phase {
                return Ok(());
            }
            return Err(ModelIrError::InvalidResponseLifecycle(
                "Responses message phase changed during the stream".into(),
            )
            .into());
        }
        if self
            .native_blocks
            .contains_key(&(NativeBlockKind::Text, native_index))
            || self
                .native_blocks
                .contains_key(&(NativeBlockKind::Refusal, native_index))
        {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "Responses message phase arrived after message content".into(),
            )
            .into());
        }
        if self.native_message_phases.len() >= MAX_CANONICAL_BLOCKS as usize {
            return Err(ModelIrError::BufferLimit(MAX_CANONICAL_BLOCKS as usize).into());
        }
        self.semantic_bytes = self
            .semantic_bytes
            .checked_add(phase.len())
            .filter(|next| *next <= MAX_CANONICAL_SEMANTIC_BYTES)
            .ok_or(ModelIrError::BufferLimit(MAX_CANONICAL_SEMANTIC_BYTES))?;
        self.native_message_phases
            .insert(native_index, phase.to_owned());
        Ok(())
    }

    fn native_message_phase(&self, native_index: u32) -> Option<String> {
        self.native_message_phases.get(&native_index).cloned()
    }

    fn charge_semantics(&mut self, event: &ModelEvent) -> Result<(), ProtocolAdapterError> {
        let added = match event {
            ModelEvent::ResponsesMetadata { metadata } => serde_json::to_vec(metadata)
                .map_err(|error| ModelIrError::InvalidJson(error.to_string()))?
                .len(),
            ModelEvent::TextAnnotation { annotation, .. } => serde_json::to_vec(annotation)
                .map_err(|error| ModelIrError::InvalidJson(error.to_string()))?
                .len(),
            ModelEvent::TextFinished { .. }
            | ModelEvent::ReasoningFinished { .. }
            | ModelEvent::RefusalFinished { .. } => 0,
            ModelEvent::WebSearch {
                item, native_id, ..
            } => serde_json::to_vec(item)
                .map_err(|error| ModelIrError::InvalidJson(error.to_string()))?
                .len()
                .saturating_add(native_id.len()),
            ModelEvent::ResponseStarted {
                response_id,
                provider_model,
            } => response_id.len().saturating_add(provider_model.len()),
            ModelEvent::TextDelta { text, .. }
            | ModelEvent::ReasoningDelta { text, .. }
            | ModelEvent::RefusalDelta { text, .. }
            | ModelEvent::ToolArgumentsDelta { delta: text, .. } => text.len(),
            ModelEvent::ToolCallStarted {
                logical_id,
                native_id,
                namespace,
                name,
                ..
            } => logical_id
                .len()
                .saturating_add(native_id.len())
                .saturating_add(namespace.as_ref().map_or(0, String::len))
                .saturating_add(name.len()),
            ModelEvent::ProviderState { state } => serde_json::to_vec(&state.value)
                .map_err(|error| ModelIrError::InvalidJson(error.to_string()))?
                .len()
                .saturating_add(state.kind.len()),
            ModelEvent::ResponseFailed { error } => error
                .code
                .as_ref()
                .map_or(0, String::len)
                .saturating_add(error.message.as_ref().map_or(0, String::len)),
            ModelEvent::ContentBlockStarted { .. }
            | ModelEvent::ToolCallFinished { .. }
            | ModelEvent::UsageUpdated { .. }
            | ModelEvent::FinishReason { .. }
            | ModelEvent::ResponseCompleted { .. } => 0,
        };
        let next = self
            .semantic_bytes
            .checked_add(added)
            .ok_or(ModelIrError::BufferLimit(MAX_CANONICAL_SEMANTIC_BYTES))?;
        if next > MAX_CANONICAL_SEMANTIC_BYTES {
            return Err(ModelIrError::BufferLimit(MAX_CANONICAL_SEMANTIC_BYTES).into());
        }
        self.semantic_bytes = next;
        Ok(())
    }

    fn apply(&mut self, event: &ModelEvent) -> Result<(), ProtocolAdapterError> {
        if self.accumulator.terminal {
            return Err(ModelIrError::DuplicateTerminalEvent.into());
        }
        match event {
            ModelEvent::ResponsesMetadata { metadata } => {
                self.accumulator.responses_metadata.extend(metadata.clone())
            }
            ModelEvent::TextAnnotation {
                index,
                annotation_index,
                annotation,
            } => {
                if !matches!(
                    self.accumulator.blocks.get(index),
                    Some(MutableResponseBlock::Text(_))
                ) || self.accumulator.finished_text.contains(index)
                    || annotation.start_index > annotation.end_index
                {
                    return Err(invalid_block(*index, "citation"));
                }
                let annotations = self.accumulator.annotations.entry(*index).or_default();
                if *annotation_index as usize != annotations.len() {
                    return Err(invalid_block(*index, "citation index"));
                }
                annotations.push(annotation.clone());
            }
            ModelEvent::TextFinished {
                index,
                text,
                annotations,
                status,
            } => {
                if !matches!(self.accumulator.blocks.get(index), Some(MutableResponseBlock::Text(current)) if current == text)
                    || !self.accumulator.finished_text.insert(*index)
                    || annotations
                        .iter()
                        .any(|annotation| annotation.start_index > annotation.end_index)
                {
                    return Err(invalid_block(*index, "text completion"));
                }
                self.bind_item_status(*index, *status)?;
            }
            ModelEvent::ReasoningFinished {
                index,
                text,
                status,
            } => {
                if !matches!(
                    self.accumulator.blocks.get(index),
                    Some(MutableResponseBlock::Reasoning(current)) if current == text
                ) || !self.accumulator.finished_reasoning.insert(*index)
                {
                    return Err(invalid_block(*index, "reasoning completion"));
                }
                self.bind_item_status(*index, *status)?;
            }
            ModelEvent::RefusalFinished {
                index,
                text,
                status,
            } => {
                if !matches!(
                    self.accumulator.blocks.get(index),
                    Some(MutableResponseBlock::Refusal(current)) if current == text
                ) || !self.accumulator.finished_refusal.insert(*index)
                {
                    return Err(invalid_block(*index, "refusal completion"));
                }
                self.bind_item_status(*index, *status)?;
            }
            ModelEvent::WebSearch {
                index,
                phase,
                item,
                native_id,
                owner,
            } => {
                search::apply(
                    &mut self.accumulator,
                    *index,
                    *phase,
                    item,
                    native_id,
                    owner,
                    &self.owner,
                )?;
            }
            ModelEvent::ResponseStarted {
                response_id,
                provider_model,
            } => {
                if self.accumulator.response_id.is_some() {
                    return Err(ModelIrError::InvalidResponseLifecycle(
                        "response started more than once".into(),
                    )
                    .into());
                }
                self.accumulator.response_id = Some(response_id.clone());
                self.accumulator.provider_model = Some(provider_model.clone());
            }
            ModelEvent::ContentBlockStarted {
                index,
                block_kind,
                item_id,
                phase,
            } => {
                let value = match block_kind {
                    ResponseBlockKind::Text => MutableResponseBlock::Text(String::new()),
                    ResponseBlockKind::Reasoning => MutableResponseBlock::Reasoning(String::new()),
                    ResponseBlockKind::Refusal => MutableResponseBlock::Refusal(String::new()),
                    ResponseBlockKind::ToolCall => {
                        return Err(ModelIrError::InvalidResponseLifecycle(
                            "tool blocks require ToolCallStarted".into(),
                        )
                        .into());
                    }
                };
                if self.accumulator.blocks.insert(*index, value).is_some() {
                    return Err(ModelIrError::InvalidResponseLifecycle(format!(
                        "content block {index} started twice"
                    ))
                    .into());
                }
                self.bind_item_id(*index, item_id)?;
                match (block_kind, phase) {
                    (ResponseBlockKind::Text | ResponseBlockKind::Refusal, Some(phase)) => {
                        if self
                            .accumulator
                            .message_phases
                            .insert(*index, phase.clone())
                            .is_some()
                        {
                            return Err(invalid_block(*index, "message phase"));
                        }
                    }
                    (ResponseBlockKind::Reasoning | ResponseBlockKind::ToolCall, Some(_)) => {
                        return Err(invalid_block(*index, "message phase on non-message block"));
                    }
                    (_, None) => {}
                }
            }
            ModelEvent::TextDelta { index, .. }
                if self.accumulator.finished_text.contains(index) =>
            {
                return Err(invalid_block(*index, "text after completion"));
            }
            ModelEvent::TextDelta { index, text } => match self.accumulator.blocks.get_mut(index) {
                Some(MutableResponseBlock::Text(output)) => output.push_str(text),
                _ => return Err(invalid_block(*index, "text delta")),
            },
            ModelEvent::ReasoningDelta { index, text } => {
                if self.accumulator.finished_reasoning.contains(index) {
                    return Err(invalid_block(*index, "reasoning after completion"));
                }
                match self.accumulator.blocks.get_mut(index) {
                    Some(MutableResponseBlock::Reasoning(output)) => output.push_str(text),
                    _ => return Err(invalid_block(*index, "reasoning delta")),
                }
            }
            ModelEvent::RefusalDelta { index, text } => {
                if self.accumulator.finished_refusal.contains(index) {
                    return Err(invalid_block(*index, "refusal after completion"));
                }
                match self.accumulator.blocks.get_mut(index) {
                    Some(MutableResponseBlock::Refusal(output)) => output.push_str(text),
                    _ => return Err(invalid_block(*index, "refusal delta")),
                }
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
                if logical_id.is_empty()
                    || native_id.is_empty()
                    || name.is_empty()
                    || owner.as_ref() != &self.owner
                {
                    return Err(ModelIrError::MissingToolIdentity(logical_id.clone()).into());
                }
                let block = MutableResponseBlock::ToolCall {
                    logical_id: logical_id.clone(),
                    native_id: native_id.clone(),
                    tool_kind: *tool_kind,
                    namespace: namespace.clone(),
                    name: name.clone(),
                    owner: owner.clone(),
                    arguments: String::new(),
                    finished: false,
                };
                if self.accumulator.blocks.insert(*index, block).is_some() {
                    return Err(ModelIrError::InvalidResponseLifecycle(format!(
                        "tool block {index} started twice"
                    ))
                    .into());
                }
                self.bind_item_id(*index, item_id)?;
            }
            ModelEvent::ToolArgumentsDelta {
                index,
                logical_id,
                delta,
            } => match self.accumulator.blocks.get_mut(index) {
                Some(MutableResponseBlock::ToolCall {
                    logical_id: current,
                    arguments,
                    finished: false,
                    ..
                }) if current == logical_id => arguments.push_str(delta),
                _ => return Err(invalid_block(*index, "tool argument delta")),
            },
            ModelEvent::ToolCallFinished {
                index,
                logical_id,
                tool_kind,
                namespace,
                name,
                arguments,
                status,
            } => {
                match self.accumulator.blocks.get_mut(index) {
                    Some(MutableResponseBlock::ToolCall {
                        logical_id: current_id,
                        tool_kind: current_kind,
                        namespace: current_namespace,
                        name: current_name,
                        arguments: current_arguments,
                        finished,
                        ..
                    }) if current_id == logical_id
                        && current_kind == tool_kind
                        && current_namespace == namespace
                        && current_name == name
                        && !*finished =>
                    {
                        let parsed =
                            canonical_tool_arguments(*tool_kind, current_arguments, logical_id)?;
                        if &parsed != arguments {
                            return Err(ModelIrError::InvalidResponseLifecycle(format!(
                                "tool call {logical_id} final arguments disagree with deltas"
                            ))
                            .into());
                        }
                        *finished = true;
                    }
                    _ => return Err(invalid_block(*index, "tool completion")),
                }
                self.bind_item_status(*index, *status)?;
            }
            ModelEvent::ProviderState { state } => {
                if state.owner != self.owner
                    || self.provider_state_emission != NativeProviderStateEmission::ExactOwnerAffine
                {
                    return Err(ModelIrError::ProviderStateNotPortable.into());
                }
                self.accumulator.provider_state.push(state.as_ref().clone());
            }
            ModelEvent::UsageUpdated { usage } => self.accumulator.usage.merge_from(usage),
            ModelEvent::FinishReason { reason } => {
                if self
                    .accumulator
                    .finish_reason
                    .replace(reason.clone())
                    .is_some()
                {
                    return Err(ModelIrError::InvalidResponseLifecycle(
                        "finish reason emitted twice".into(),
                    )
                    .into());
                }
                if reason == &FinishReason::Refusal
                    && !self
                        .accumulator
                        .blocks
                        .values()
                        .any(|block| matches!(block, MutableResponseBlock::Refusal(_)))
                {
                    for block in self.accumulator.blocks.values_mut() {
                        if let MutableResponseBlock::Text(text) = block {
                            *block = MutableResponseBlock::Refusal(std::mem::take(text));
                        }
                    }
                }
            }
            ModelEvent::ResponseCompleted { output } => {
                let mut expected = self.accumulator.clone();
                expected.terminal = true;
                let expected = expected.finish(self.owner.upstream_protocol)?.blocks;
                if &expected != output {
                    return Err(ModelIrError::InvalidResponseLifecycle(
                        "response completion snapshot disagrees with canonical output".into(),
                    )
                    .into());
                }
                self.accumulator.terminal = true;
            }
            ModelEvent::ResponseFailed { error } => {
                self.accumulator.error = Some(error.clone());
                self.accumulator.terminal = true;
            }
        }
        Ok(())
    }

    fn bind_item_id(
        &mut self,
        index: u32,
        item_id: &Option<String>,
    ) -> Result<(), ProtocolAdapterError> {
        let Some(item_id) = item_id else {
            return Ok(());
        };
        if item_id.is_empty()
            || item_id.len() > 256
            || item_id.chars().any(char::is_control)
            || self
                .accumulator
                .item_ids
                .values()
                .any(|value| value == item_id)
            || self
                .accumulator
                .item_ids
                .insert(index, item_id.clone())
                .is_some()
        {
            return Err(ModelIrError::InvalidField("response output item id").into());
        }
        Ok(())
    }

    fn bind_item_status(
        &mut self,
        index: u32,
        status: ResponseItemStatus,
    ) -> Result<(), ProtocolAdapterError> {
        if self
            .accumulator
            .item_statuses
            .insert(index, status)
            .is_some()
        {
            return Err(invalid_block(index, "item status"));
        }
        Ok(())
    }

    fn validate_item_status(
        &self,
        index: u32,
        status: ResponseItemStatus,
    ) -> Result<(), ProtocolAdapterError> {
        if self.accumulator.item_statuses.get(&index) != Some(&status) {
            return Err(invalid_block(index, "item status"));
        }
        Ok(())
    }

    fn start_response(
        &mut self,
        response_id: String,
        provider_model: String,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        if let (Some(current_id), Some(current_model)) = (
            self.accumulator.response_id.as_deref(),
            self.accumulator.provider_model.as_deref(),
        ) {
            if current_id != response_id || current_model != provider_model {
                return Err(ModelIrError::InvalidResponseLifecycle(
                    "native response identity changed during the stream".into(),
                )
                .into());
            }
        } else {
            self.emit(
                ModelEvent::ResponseStarted {
                    response_id,
                    provider_model,
                },
                output,
            )?;
        }
        Ok(())
    }

    fn text_delta(
        &mut self,
        native_index: u32,
        text: String,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        let (index, fresh) = self.block_index(NativeBlockKind::Text, native_index)?;
        if fresh {
            let item_id = self.native_item_id(native_index);
            let phase = self.native_message_phase(native_index);
            self.emit(
                ModelEvent::ContentBlockStarted {
                    index,
                    block_kind: ResponseBlockKind::Text,
                    item_id,
                    phase,
                },
                output,
            )?;
        }
        if !text.is_empty() {
            self.emit(ModelEvent::TextDelta { index, text }, output)?;
        }
        Ok(())
    }

    fn reasoning_delta(
        &mut self,
        native_index: u32,
        text: String,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        let (index, fresh) = self.block_index(NativeBlockKind::Reasoning, native_index)?;
        if fresh {
            let item_id = self.native_item_id(native_index);
            self.emit(
                ModelEvent::ContentBlockStarted {
                    index,
                    block_kind: ResponseBlockKind::Reasoning,
                    item_id,
                    phase: None,
                },
                output,
            )?;
        }
        if !text.is_empty() {
            self.emit(ModelEvent::ReasoningDelta { index, text }, output)?;
        }
        Ok(())
    }

    fn refusal_delta(
        &mut self,
        native_index: u32,
        text: String,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        let (index, fresh) = self.block_index(NativeBlockKind::Refusal, native_index)?;
        if fresh {
            let item_id = self.native_item_id(native_index);
            let phase = self.native_message_phase(native_index);
            self.emit(
                ModelEvent::ContentBlockStarted {
                    index,
                    block_kind: ResponseBlockKind::Refusal,
                    item_id,
                    phase,
                },
                output,
            )?;
        }
        if !text.is_empty() {
            self.emit(ModelEvent::RefusalDelta { index, text }, output)?;
        }
        Ok(())
    }

    fn reconcile_reasoning(
        &mut self,
        native_index: u32,
        expected: &str,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(u32, String), ProtocolAdapterError> {
        let (index, fresh) = self.block_index(NativeBlockKind::Reasoning, native_index)?;
        if fresh {
            let item_id = self.native_item_id(native_index);
            self.emit(
                ModelEvent::ContentBlockStarted {
                    index,
                    block_kind: ResponseBlockKind::Reasoning,
                    item_id,
                    phase: None,
                },
                output,
            )?;
            if !expected.is_empty() {
                self.emit(
                    ModelEvent::ReasoningDelta {
                        index,
                        text: expected.into(),
                    },
                    output,
                )?;
            }
        }
        let suffix = match self.accumulator.blocks.get(&index) {
            Some(MutableResponseBlock::Reasoning(text)) => expected
                .strip_prefix(text)
                .filter(|suffix| !suffix.is_empty())
                .map(str::to_owned),
            _ => None,
        };
        if let Some(text) = suffix {
            self.emit(ModelEvent::ReasoningDelta { index, text }, output)?;
        }
        let text = match self.accumulator.blocks.get(&index) {
            Some(MutableResponseBlock::Reasoning(text)) if text == expected => text.clone(),
            _ => {
                return Err(invalid_block(
                    index,
                    "final reasoning disagrees with deltas",
                ));
            }
        };
        Ok((index, text))
    }

    fn finish_reasoning_with_status(
        &mut self,
        native_index: u32,
        expected: &str,
        status: ResponseItemStatus,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        let (index, text) = self.reconcile_reasoning(native_index, expected, output)?;
        if self.accumulator.finished_reasoning.contains(&index) {
            return self.validate_item_status(index, status);
        }
        self.emit(
            ModelEvent::ReasoningFinished {
                index,
                text,
                status,
            },
            output,
        )
    }

    fn reconcile_refusal(
        &mut self,
        native_index: u32,
        expected: &str,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(u32, String), ProtocolAdapterError> {
        let (index, fresh) = self.block_index(NativeBlockKind::Refusal, native_index)?;
        if fresh {
            let item_id = self.native_item_id(native_index);
            let phase = self.native_message_phase(native_index);
            self.emit(
                ModelEvent::ContentBlockStarted {
                    index,
                    block_kind: ResponseBlockKind::Refusal,
                    item_id,
                    phase,
                },
                output,
            )?;
            if !expected.is_empty() {
                self.emit(
                    ModelEvent::RefusalDelta {
                        index,
                        text: expected.into(),
                    },
                    output,
                )?;
            }
        }
        let suffix = match self.accumulator.blocks.get(&index) {
            Some(MutableResponseBlock::Refusal(text)) => expected
                .strip_prefix(text)
                .filter(|suffix| !suffix.is_empty())
                .map(str::to_owned),
            _ => None,
        };
        if let Some(text) = suffix {
            self.emit(ModelEvent::RefusalDelta { index, text }, output)?;
        }
        let text = match self.accumulator.blocks.get(&index) {
            Some(MutableResponseBlock::Refusal(text)) if text == expected => text.clone(),
            _ => return Err(invalid_block(index, "final refusal disagrees with deltas")),
        };
        Ok((index, text))
    }

    fn finish_refusal_with_status(
        &mut self,
        native_index: u32,
        expected: &str,
        status: ResponseItemStatus,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        let (index, text) = self.reconcile_refusal(native_index, expected, output)?;
        if self.accumulator.finished_refusal.contains(&index) {
            return self.validate_item_status(index, status);
        }
        self.emit(
            ModelEvent::RefusalFinished {
                index,
                text,
                status,
            },
            output,
        )
    }

    fn has_refusal(&self) -> bool {
        self.accumulator
            .blocks
            .values()
            .any(|block| matches!(block, MutableResponseBlock::Refusal(_)))
    }

    fn start_tool(
        &mut self,
        native_index: u32,
        native_id: String,
        kind: ToolKindV1,
        namespace: Option<String>,
        name: String,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<u32, ProtocolAdapterError> {
        let (index, fresh) = self.block_index(NativeBlockKind::Tool, native_index)?;
        if fresh {
            if self.accumulator.blocks.values().any(|block| {
                matches!(block, MutableResponseBlock::ToolCall { native_id: current, .. } if current == &native_id)
            }) {
                return Err(ModelIrError::MissingToolIdentity(native_id).into());
            }
            if self.accumulator.response_id.is_none() {
                return Err(ModelIrError::InvalidResponseLifecycle(
                    "Tool call arrived before native response identity".into(),
                )
                .into());
            }
            let logical_id = if let Some(projection) = &self.tool_id_projection {
                projection.project(&native_id, &self.owner)?
            } else {
                super::continuation::project_delivered_tool_id(&native_id, &self.owner)?
            };
            if self.accumulator.blocks.values().any(|block| {
                matches!(block, MutableResponseBlock::ToolCall { logical_id: current, .. } if current == &logical_id)
            }) {
                return Err(ModelIrError::ToolContinuationConflict.into());
            }
            let item_id = self.native_item_id(native_index);
            self.emit(
                ModelEvent::ToolCallStarted {
                    index,
                    logical_id,
                    native_id: native_id.clone(),
                    tool_kind: kind,
                    namespace,
                    name,
                    owner: Box::new(self.owner.clone()),
                    item_id,
                },
                output,
            )?;
        } else {
            match self.accumulator.blocks.get(&index) {
                Some(MutableResponseBlock::ToolCall {
                    native_id: current_native_id,
                    tool_kind: current_kind,
                    namespace: current_namespace,
                    name: current_name,
                    ..
                }) if current_native_id == &native_id
                    && current_kind == &kind
                    && current_namespace == &namespace
                    && current_name == &name => {}
                _ => return Err(invalid_block(index, "tool identity")),
            }
        }
        Ok(index)
    }

    // Native and canonical identity components stay explicit so delta events
    // cannot accidentally reuse a partial tool identity.
    #[allow(clippy::too_many_arguments)]
    fn tool_delta(
        &mut self,
        native_index: u32,
        native_id: String,
        kind: ToolKindV1,
        namespace: Option<String>,
        name: String,
        delta: String,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        let index = self.start_tool(native_index, native_id, kind, namespace, name, output)?;
        let logical_id = tool_logical_id(&self.accumulator, index)?;
        if !delta.is_empty() {
            self.emit(
                ModelEvent::ToolArgumentsDelta {
                    index,
                    logical_id,
                    delta,
                },
                output,
            )?;
        }
        Ok(())
    }

    fn finish_tool(
        &mut self,
        native_index: u32,
        native_id: String,
        kind: ToolKindV1,
        namespace: Option<String>,
        name: String,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        self.finish_tool_with_status(
            native_index,
            native_id,
            kind,
            namespace,
            name,
            ResponseItemStatus::Completed,
            output,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_tool_with_status(
        &mut self,
        native_index: u32,
        native_id: String,
        kind: ToolKindV1,
        namespace: Option<String>,
        name: String,
        status: ResponseItemStatus,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        let index = self.start_tool(
            native_index,
            native_id,
            kind,
            namespace.clone(),
            name.clone(),
            output,
        )?;
        let logical_id = tool_logical_id(&self.accumulator, index)?;
        let (arguments, finished) = match self.accumulator.blocks.get(&index) {
            Some(MutableResponseBlock::ToolCall {
                arguments,
                finished,
                ..
            }) => (
                canonical_tool_arguments(kind, arguments, &logical_id)?,
                *finished,
            ),
            _ => return Err(invalid_block(index, "tool completion")),
        };
        if finished {
            return self.validate_item_status(index, status);
        }
        self.emit(
            ModelEvent::ToolCallFinished {
                index,
                logical_id,
                tool_kind: kind,
                namespace,
                name,
                arguments,
                status,
            },
            output,
        )
    }

    // Completion validates the same complete identity tuple as start/delta.
    #[allow(clippy::too_many_arguments)]
    fn finish_tool_with_arguments(
        &mut self,
        native_index: u32,
        native_id: String,
        kind: ToolKindV1,
        namespace: Option<String>,
        name: String,
        expected: &str,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        self.finish_tool_with_arguments_status(
            native_index,
            native_id,
            kind,
            namespace,
            name,
            expected,
            ResponseItemStatus::Completed,
            output,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn reconcile_tool_with_arguments(
        &mut self,
        native_index: u32,
        native_id: String,
        kind: ToolKindV1,
        namespace: Option<String>,
        name: String,
        expected: &str,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        let index = self.start_tool(
            native_index,
            native_id.clone(),
            kind,
            namespace.clone(),
            name.clone(),
            output,
        )?;
        let current = match self.accumulator.blocks.get(&index) {
            Some(MutableResponseBlock::ToolCall { arguments, .. }) => arguments.clone(),
            _ => return Err(invalid_block(index, "tool completion")),
        };
        if let Some(suffix) = expected
            .strip_prefix(&current)
            .filter(|suffix| !suffix.is_empty())
        {
            self.tool_delta(
                native_index,
                native_id.clone(),
                kind,
                namespace.clone(),
                name.clone(),
                suffix.into(),
                output,
            )?;
        } else if current != expected {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "native final tool arguments disagree with deltas".into(),
            )
            .into());
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_tool_with_arguments_status(
        &mut self,
        native_index: u32,
        native_id: String,
        kind: ToolKindV1,
        namespace: Option<String>,
        name: String,
        expected: &str,
        status: ResponseItemStatus,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        self.reconcile_tool_with_arguments(
            native_index,
            native_id.clone(),
            kind,
            namespace.clone(),
            name.clone(),
            expected,
            output,
        )?;
        self.finish_tool_with_status(
            native_index,
            native_id,
            kind,
            namespace,
            name,
            status,
            output,
        )
    }

    fn provider_state(
        &mut self,
        protocol: IngressProtocol,
        kind: impl Into<String>,
        value: Value,
        native_block_index: Option<u32>,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        if protocol != self.owner.upstream_protocol {
            return Err(ModelIrError::ProviderStateNotPortable.into());
        }
        // `Never` is the compiled path's explicit instruction not to surface opaque provider
        // state in Model IR. Known wire fields such as Responses `encrypted_content` may still
        // be emitted by the provider and are safely consumed here rather than made portable.
        if self.provider_state_emission == NativeProviderStateEmission::Never {
            return Ok(());
        }
        if self.provider_state_emission != NativeProviderStateEmission::ExactOwnerAffine {
            return Err(ModelIrError::ProviderStateNotPortable.into());
        }
        let kind = kind.into();
        if kind == "encrypted_content" && self.tool_id_projection.is_none() {
            super::continuation::record_provider_state(&value, &self.owner)?;
        }
        if let Some(native_index) = native_block_index
            && (protocol == IngressProtocol::Responses
                || matches!(
                    kind.as_str(),
                    "thinking_signature" | "thinking_signature_delta"
                ))
        {
            self.reasoning_delta(native_index, String::new(), output)?;
        }
        let block_index = native_block_index.and_then(|native_index| {
            self.native_blocks
                .get(&(NativeBlockKind::Reasoning, native_index))
                .copied()
        });
        self.emit(
            ModelEvent::ProviderState {
                state: Box::new(OpaqueProviderState {
                    owner: self.owner.clone(),
                    block_index,
                    kind,
                    value,
                }),
            },
            output,
        )
    }

    fn responses_metadata(
        &mut self,
        response: &Map<String, Value>,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        let metadata: BTreeMap<_, _> = response
            .iter()
            .filter(|(key, _)| {
                !matches!(
                    key.as_str(),
                    "id" | "model" | "output" | "usage" | "status" | "error" | "incomplete_details"
                )
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        if !metadata.is_empty() {
            self.emit(ModelEvent::ResponsesMetadata { metadata }, output)?;
        }
        Ok(())
    }

    fn usage(
        &mut self,
        usage: ModelUsage,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        if !usage.is_empty() {
            self.emit(ModelEvent::UsageUpdated { usage }, output)?;
        }
        Ok(())
    }
}

fn tool_logical_id(
    accumulator: &ResponseAccumulator,
    index: u32,
) -> Result<String, ProtocolAdapterError> {
    match accumulator.blocks.get(&index) {
        Some(MutableResponseBlock::ToolCall { logical_id, .. }) => Ok(logical_id.clone()),
        _ => Err(invalid_block(index, "tool identity")),
    }
}

fn canonical_tool_arguments(
    kind: ToolKindV1,
    arguments: &str,
    logical_id: &str,
) -> Result<Value, ProtocolAdapterError> {
    match kind {
        ToolKindV1::Function => serde_json::from_str(arguments)
            .map_err(|_| ModelIrError::InvalidToolArguments(logical_id.into()).into()),
        ToolKindV1::Custom => Ok(Value::String(arguments.into())),
    }
}

fn invalid_block(index: u32, operation: &str) -> ProtocolAdapterError {
    ModelIrError::InvalidResponseLifecycle(format!(
        "{operation} does not match content block {index}"
    ))
    .into()
}
pub use client_stream::IncrementalClientSseRenderer;
