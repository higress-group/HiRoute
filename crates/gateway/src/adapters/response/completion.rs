//! Canonical response terminal reconciliation and snapshot emission.

use super::*;

impl DecoderCore {
    pub(super) fn finish_reason(
        &mut self,
        reason: FinishReason,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        if self.accumulator.finish_reason.is_none() {
            self.emit(ModelEvent::FinishReason { reason }, output)?;
        } else if self.accumulator.finish_reason.as_ref() != Some(&reason) {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "native finish reasons disagree".into(),
            )
            .into());
        }
        Ok(())
    }

    pub(super) fn complete(
        &mut self,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        self.validate_native_bindings()?;
        let item_status = match self.accumulator.finish_reason.as_ref() {
            Some(FinishReason::Stop | FinishReason::ToolCall) => ResponseItemStatus::Completed,
            Some(
                FinishReason::Length
                | FinishReason::Refusal
                | FinishReason::Cancelled
                | FinishReason::Other(_),
            ) => ResponseItemStatus::Incomplete,
            None => {
                return Err(ModelIrError::InvalidResponseLifecycle(
                    "response completion lacks a finish reason".into(),
                )
                .into());
            }
        };
        if self.accumulator.blocks.values().any(|block| {
            matches!(
                block,
                MutableResponseBlock::WebSearch {
                    finished: false,
                    ..
                }
            )
        }) {
            return Err(ModelIrError::MissingTerminalEvent.into());
        }
        let finished_text = &self.accumulator.finished_text;
        let finished_reasoning = &self.accumulator.finished_reasoning;
        let finished_refusal = &self.accumulator.finished_refusal;
        let mut completions = self
            .accumulator
            .blocks
            .iter()
            .filter_map(|(index, block)| match block {
                MutableResponseBlock::Text(text) if !finished_text.contains(index) => {
                    Some(ModelEvent::TextFinished {
                        index: *index,
                        text: text.clone(),
                        annotations: self
                            .accumulator
                            .annotations
                            .get(index)
                            .cloned()
                            .unwrap_or_default(),
                        status: item_status,
                        final_phase: None,
                    })
                }
                MutableResponseBlock::Reasoning(text) if !finished_reasoning.contains(index) => {
                    Some(ModelEvent::ReasoningFinished {
                        index: *index,
                        text: text.clone(),
                        status: item_status,
                    })
                }
                MutableResponseBlock::Refusal(text) if !finished_refusal.contains(index) => {
                    Some(ModelEvent::RefusalFinished {
                        index: *index,
                        text: text.clone(),
                        status: item_status,
                        final_phase: None,
                    })
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let tool_completions = self
            .accumulator
            .blocks
            .iter()
            .filter_map(|(index, block)| match block {
                MutableResponseBlock::ToolCall {
                    logical_id,
                    tool_kind,
                    namespace,
                    name,
                    arguments,
                    finished: false,
                    ..
                } => Some((*index, logical_id, *tool_kind, namespace, name, arguments)),
                _ => None,
            })
            .map(
                |(index, logical_id, tool_kind, namespace, name, arguments)| {
                    Ok(ModelEvent::ToolCallFinished {
                        encrypted_function_args: (tool_kind == ToolKindV1::Function
                            && self.function_encoding_projection
                                == super::super::FunctionEncodingProjection::Plaintext
                            && self.responses_client
                            && self.owner.upstream_protocol != IngressProtocol::Responses)
                            .then(Vec::new),
                        index,
                        logical_id: logical_id.clone(),
                        tool_kind,
                        namespace: namespace.clone(),
                        name: name.clone(),
                        arguments: canonical_tool_arguments(tool_kind, arguments, logical_id)?,
                        status: item_status,
                    })
                },
            )
            .collect::<Result<Vec<_>, ProtocolAdapterError>>()?;
        completions.extend(tool_completions);
        for completion in completions {
            self.emit(completion, output)?;
        }
        let mut snapshot = self.accumulator.clone();
        snapshot.terminal = true;
        let output_snapshot = snapshot.finish(self.owner.upstream_protocol)?.blocks;
        self.emit(
            ModelEvent::ResponseCompleted {
                output: output_snapshot,
            },
            output,
        )
    }

    pub(super) fn fail(
        &mut self,
        error: ModelError,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        self.emit(ModelEvent::ResponseFailed { error }, output)
    }

    pub(super) fn finish(
        self,
        protocol: IngressProtocol,
    ) -> Result<ModelResponseIRV1, ProtocolAdapterError> {
        self.validate_native_bindings()?;
        self.accumulator.finish(protocol).map_err(Into::into)
    }

    fn validate_native_bindings(&self) -> Result<(), ProtocolAdapterError> {
        if self.native_message_phases.iter().any(|(native_index, _)| {
            !self
                .native_blocks
                .contains_key(&(NativeBlockKind::Text, *native_index))
                && !self
                    .native_blocks
                    .contains_key(&(NativeBlockKind::Refusal, *native_index))
        }) {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "Responses message phase ended without a canonical message block".into(),
            )
            .into());
        }
        if self.native_item_ids.iter().any(|(_, item_id)| {
            !self
                .accumulator
                .item_ids
                .values()
                .any(|bound| bound == item_id)
        }) {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "Responses output item ended without a representable canonical block".into(),
            )
            .into());
        }
        if !self.responses_encrypted_fallback.is_empty() {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "Responses encrypted reasoning fallback was never finalized".into(),
            )
            .into());
        }
        Ok(())
    }
}
