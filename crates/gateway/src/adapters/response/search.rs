//! Responses hosted search lifecycle. The existing continuation issuer owns replay IDs.
use super::*;
use crate::server::core_runtime::model_ir::{
    UrlCitationV1, WebSearchCallV1, WebSearchPhase, WebSearchStatus,
};

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;

pub(super) fn apply(
    acc: &mut ResponseAccumulator,
    index: u32,
    phase: WebSearchPhase,
    item: &WebSearchCallV1,
    native_id: &str,
    owner: &ExactProviderPathV1,
    expected: &ExactProviderPathV1,
) -> Result<(), ProtocolAdapterError> {
    if owner != expected {
        return Err(ModelIrError::ProviderStateNotPortable.into());
    }
    if phase == WebSearchPhase::Added {
        if item.status != WebSearchStatus::InProgress || acc.blocks.contains_key(&index) {
            return Err(invalid_block(index, "search start"));
        }
        acc.blocks.insert(
            index,
            MutableResponseBlock::WebSearch {
                item: item.clone(),
                native_id: native_id.into(),
                owner: Box::new(owner.clone()),
                finished: false,
            },
        );
    } else {
        match acc.blocks.get_mut(&index) {
            Some(MutableResponseBlock::WebSearch {
                item: current,
                native_id: original,
                finished,
                ..
            }) if !*finished && current.id == item.id && original == native_id => {
                if current.status == WebSearchStatus::Searching
                    && item.status == WebSearchStatus::InProgress
                {
                    return Err(invalid_block(index, "search progress moved backwards"));
                }
                if matches!(
                    current.status,
                    WebSearchStatus::Completed | WebSearchStatus::Failed
                ) && phase != WebSearchPhase::Done
                {
                    return Err(invalid_block(index, "search after completion"));
                }
                if phase == WebSearchPhase::Done {
                    if !matches!(
                        item.status,
                        WebSearchStatus::Completed | WebSearchStatus::Failed
                    ) || (item.status == WebSearchStatus::Completed && item.action.is_none())
                    {
                        return Err(invalid_block(index, "incomplete search result"));
                    }
                    *finished = true;
                }
                *current = item.clone();
            }
            _ => return Err(invalid_block(index, "search lifecycle")),
        }
    }
    Ok(())
}

impl DecoderCore {
    pub(super) fn finish_search_snapshot(
        &mut self,
        native_index: u32,
        value: &Value,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        if let Some(index) = self
            .native_blocks
            .get(&(NativeBlockKind::Search, native_index))
            .copied()
        {
            let mut item = WebSearchCallV1::from_wire(value)?;
            let native_id = item.id.clone();
            let (current, original, finished) = match self.accumulator.blocks.get(&index) {
                Some(MutableResponseBlock::WebSearch {
                    item,
                    native_id,
                    finished,
                    ..
                }) => (item, native_id, *finished),
                _ => return Err(invalid_block(index, "search completion snapshot")),
            };
            item.id = current.id.clone();
            if original != &native_id {
                return Err(invalid_block(index, "search completion identity"));
            }
            if finished {
                return if current == &item {
                    Ok(())
                } else {
                    Err(invalid_block(index, "search completion snapshot"))
                };
            }
            return self.search_item(native_index, value, WebSearchPhase::Done, output);
        }

        let final_item = value.clone();
        let mut start = final_item.clone();
        start["status"] = "in_progress".into();
        start.as_object_mut().unwrap().remove("action");
        self.search_item(native_index, &start, WebSearchPhase::Added, output)?;
        self.search_item(native_index, &final_item, WebSearchPhase::Done, output)
    }

    pub(super) fn text_annotation(
        &mut self,
        native_index: u32,
        annotation_index: u32,
        value: &Value,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        let (index, fresh) = self.block_index(NativeBlockKind::Text, native_index)?;
        if fresh {
            return Err(invalid_block(index, "citation before text"));
        }
        let annotation = serde_json::from_value(value.clone())
            .map_err(|_| ModelIrError::InvalidField("url citation"))?;
        self.emit(
            ModelEvent::TextAnnotation {
                index,
                annotation_index,
                annotation,
            },
            output,
        )
    }

    pub(super) fn reconcile_text(
        &mut self,
        native_index: u32,
        expected: &str,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(u32, String, Vec<UrlCitationV1>), ProtocolAdapterError> {
        let (index, fresh) = self.block_index(NativeBlockKind::Text, native_index)?;
        if fresh {
            // Some native streams send only the complete text. There is no prior delta
            // to contradict; emit that exact text rather than silently consuming it.
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
            if !expected.is_empty() {
                self.emit(
                    ModelEvent::TextDelta {
                        index,
                        text: expected.into(),
                    },
                    output,
                )?;
            }
        }
        let suffix = match self.accumulator.blocks.get(&index) {
            Some(MutableResponseBlock::Text(text)) => expected
                .strip_prefix(text)
                .filter(|suffix| !suffix.is_empty())
                .map(str::to_owned),
            _ => None,
        };
        if let Some(text) = suffix {
            self.emit(ModelEvent::TextDelta { index, text }, output)?;
        }
        let text = match self.accumulator.blocks.get(&index) {
            Some(MutableResponseBlock::Text(text)) if text == expected => text.clone(),
            _ => return Err(invalid_block(index, "final text disagrees with deltas")),
        };
        let annotations = self
            .accumulator
            .annotations
            .get(&index)
            .cloned()
            .unwrap_or_default();
        Ok((index, text, annotations))
    }

    pub(super) fn finish_text_with_status(
        &mut self,
        native_index: u32,
        expected: &str,
        status: ResponseItemStatus,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        let (index, text, annotations) = self.reconcile_text(native_index, expected, output)?;
        if self.accumulator.finished_text.contains(&index) {
            return self.validate_item_status(index, status);
        }
        self.emit(
            ModelEvent::TextFinished {
                index,
                text,
                annotations,
                status,
            },
            output,
        )
    }

    pub(super) fn search_item(
        &mut self,
        native_index: u32,
        value: &Value,
        phase: WebSearchPhase,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        let mut item = WebSearchCallV1::from_wire(value)?;
        let native_id = item.id.clone();
        let (index, fresh) = self.block_index(NativeBlockKind::Search, native_index)?;
        if phase == WebSearchPhase::Added {
            if !self.search_ids.insert(native_id.clone()) {
                return Err(invalid_block(index, "duplicate native search identity"));
            }
            if !fresh {
                return Err(invalid_block(index, "duplicate search"));
            }
            self.accumulator
                .response_id
                .as_deref()
                .ok_or_else(|| invalid_block(index, "search before response"))?;
            item.id = if let Some(projection) = &self.tool_id_projection {
                projection.project(&native_id, &self.owner)?
            } else {
                super::super::continuation::project_delivered_tool_id(&native_id, &self.owner)?
            };
        } else {
            match self.accumulator.blocks.get(&index) {
                Some(MutableResponseBlock::WebSearch {
                    item: previous,
                    native_id: original,
                    ..
                }) if original == &native_id => item.id = previous.id.clone(),
                _ => return Err(invalid_block(index, "unknown search")),
            }
        }
        self.emit(
            ModelEvent::WebSearch {
                index,
                phase,
                item,
                native_id,
                owner: Box::new(self.owner.clone()),
            },
            output,
        )
    }

    pub(super) fn search_progress(
        &mut self,
        native_index: u32,
        id: &str,
        phase: WebSearchPhase,
        output: &mut VecDeque<ModelStreamEventV1>,
    ) -> Result<(), ProtocolAdapterError> {
        let (index, _) = self.block_index(NativeBlockKind::Search, native_index)?;
        let mut item = match self.accumulator.blocks.get(&index) {
            Some(MutableResponseBlock::WebSearch {
                item, native_id, ..
            }) if native_id == id => item.clone(),
            _ => return Err(invalid_block(index, "search progress")),
        };
        item.id = id.into();
        item.status = match phase {
            WebSearchPhase::InProgress => WebSearchStatus::InProgress,
            WebSearchPhase::Searching => WebSearchStatus::Searching,
            WebSearchPhase::Completed => WebSearchStatus::Completed,
            _ => return Err(invalid_block(index, "search progress phase")),
        };
        self.search_item(native_index, &item.wire_value(), phase, output)
    }
}
