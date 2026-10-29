use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::server::request_plan::IngressProtocol;

use super::{ExactProviderPathV1, ModelIrError, OpaqueProviderState, ToolIdMapEntryV1, ToolKindV1};

pub const MODEL_STREAM_EVENT_SCHEMA: &str = "hiroute.model-stream-event/v1";
pub const SEMANTIC_LEDGER_SCHEMA: &str = "hiroute.semantic-ledger/v1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelStreamEventV1 {
    pub schema_version: String,
    pub sequence: u64,
    pub event: ModelEvent,
}

impl ModelStreamEventV1 {
    pub fn new(sequence: u64, event: ModelEvent) -> Self {
        Self {
            schema_version: MODEL_STREAM_EVENT_SCHEMA.into(),
            sequence,
            event,
        }
    }

    pub fn is_semantic_output(&self) -> bool {
        matches!(
            self.event,
            ModelEvent::TextDelta { .. }
                | ModelEvent::ReasoningDelta { .. }
                | ModelEvent::RefusalDelta { .. }
                | ModelEvent::ToolCallStarted { .. }
                | ModelEvent::ToolArgumentsDelta { .. }
                | ModelEvent::ToolCallFinished { .. }
                | ModelEvent::WebSearch { .. }
                | ModelEvent::TextAnnotation { .. }
                | ModelEvent::TextFinished { .. }
                | ModelEvent::ReasoningFinished { .. }
                | ModelEvent::RefusalFinished { .. }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelEvent {
    ResponsesMetadata {
        metadata: BTreeMap<String, Value>,
    },
    TextAnnotation {
        index: u32,
        annotation_index: u32,
        annotation: super::UrlCitationV1,
    },
    TextFinished {
        index: u32,
        text: String,
        annotations: Vec<super::UrlCitationV1>,
        status: ResponseItemStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        final_phase: Option<FinalMessagePhaseV1>,
    },
    ReasoningFinished {
        index: u32,
        text: String,
        status: ResponseItemStatus,
    },
    RefusalFinished {
        index: u32,
        text: String,
        status: ResponseItemStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        final_phase: Option<FinalMessagePhaseV1>,
    },
    WebSearch {
        index: u32,
        phase: super::WebSearchPhase,
        item: super::WebSearchCallV1,
        native_id: String,
        owner: Box<ExactProviderPathV1>,
    },
    ResponseStarted {
        response_id: String,
        provider_model: String,
    },
    ContentBlockStarted {
        index: u32,
        block_kind: ResponseBlockKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
    },
    TextDelta {
        index: u32,
        text: String,
    },
    ReasoningDelta {
        index: u32,
        text: String,
    },
    RefusalDelta {
        index: u32,
        text: String,
    },
    ToolCallStarted {
        index: u32,
        logical_id: String,
        native_id: String,
        tool_kind: ToolKindV1,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        namespace: Option<String>,
        name: String,
        owner: Box<ExactProviderPathV1>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item_id: Option<String>,
    },
    ToolArgumentsDelta {
        index: u32,
        logical_id: String,
        delta: String,
    },
    ToolCallFinished {
        index: u32,
        logical_id: String,
        tool_kind: ToolKindV1,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        namespace: Option<String>,
        name: String,
        arguments: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        encrypted_function_args: Option<Vec<String>>,
        status: ResponseItemStatus,
    },
    ProviderState {
        state: Box<OpaqueProviderState>,
    },
    UsageUpdated {
        usage: ModelUsage,
    },
    FinishReason {
        reason: FinishReason,
    },
    ResponseCompleted {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        output: Vec<ResponseBlock>,
    },
    ResponseFailed {
        error: ModelError,
    },
}

/// Authoritative message metadata at item completion. An absent envelope keeps
/// released events' start-phase semantics; an envelope with no value clears a
/// provisional phase. This never changes item identity or accepted content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinalMessagePhaseV1 {
    pub value: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseBlockKind {
    Text,
    Reasoning,
    Refusal,
    ToolCall,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCall,
    Refusal,
    Cancelled,
    Other(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseItemStatus {
    Completed,
    Incomplete,
}

impl ResponseItemStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Incomplete => "incomplete",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}

impl ModelUsage {
    pub fn merge_from(&mut self, update: &Self) {
        if update.input_tokens.is_some() {
            self.input_tokens = update.input_tokens;
        }
        if update.output_tokens.is_some() {
            self.output_tokens = update.output_tokens;
        }
        if update.cache_read_tokens.is_some() {
            self.cache_read_tokens = update.cache_read_tokens;
        }
        if update.cache_write_tokens.is_some() {
            self.cache_write_tokens = update.cache_write_tokens;
        }
        if update.reasoning_tokens.is_some() {
            self.reasoning_tokens = update.reasoning_tokens;
        }
    }

    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelError {
    pub status: Option<u16>,
    pub code: Option<String>,
    pub message: Option<String>,
    pub retryable: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResponseBlock {
    WebSearch {
        index: u32,
        item: super::WebSearchCallV1,
    },
    Text {
        index: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        annotations: Vec<super::UrlCitationV1>,
        status: ResponseItemStatus,
    },
    Reasoning {
        index: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item_id: Option<String>,
        text: String,
        status: ResponseItemStatus,
    },
    Refusal {
        index: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
        text: String,
        status: ResponseItemStatus,
    },
    ToolCall {
        index: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item_id: Option<String>,
        logical_id: String,
        tool_kind: ToolKindV1,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        namespace: Option<String>,
        name: String,
        arguments: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        encrypted_function_args: Option<Vec<String>>,
        status: ResponseItemStatus,
    },
}

impl ResponseBlock {
    pub fn index(&self) -> u32 {
        match self {
            Self::Text { index, .. }
            | Self::WebSearch { index, .. }
            | Self::Reasoning { index, .. }
            | Self::Refusal { index, .. }
            | Self::ToolCall { index, .. } => *index,
        }
    }

    fn without_transport_item_id(mut self) -> Self {
        match &mut self {
            Self::Text { item_id, .. }
            | Self::Reasoning { item_id, .. }
            | Self::Refusal { item_id, .. }
            | Self::ToolCall { item_id, .. } => *item_id = None,
            Self::WebSearch { .. } => {}
        }
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelResponseIRV1 {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub responses_metadata: BTreeMap<String, Value>,
    pub source_protocol: IngressProtocol,
    pub response_id: String,
    pub provider_model: String,
    pub blocks: Vec<ResponseBlock>,
    pub provider_state: Vec<OpaqueProviderState>,
    pub tool_id_map: Vec<ToolIdMapEntryV1>,
    pub usage: ModelUsage,
    pub finish_reason: Option<FinishReason>,
    pub error: Option<ModelError>,
    pub completed: bool,
}

impl ModelResponseIRV1 {
    pub fn semantic_ledger(&self) -> SemanticLedgerV1 {
        let mut blocks = self.blocks.clone();
        blocks.sort_by_key(ResponseBlock::index);
        let mut events = blocks
            .into_iter()
            .map(ResponseBlock::without_transport_item_id)
            .map(SemanticLedgerEvent::Block)
            .collect::<Vec<_>>();
        events.extend(
            self.provider_state
                .iter()
                .cloned()
                .map(Box::new)
                .map(SemanticLedgerEvent::ProviderState),
        );
        if !self.usage.is_empty() {
            events.push(SemanticLedgerEvent::Usage(self.usage.clone()));
        }
        if let Some(reason) = &self.finish_reason {
            events.push(SemanticLedgerEvent::Finish(reason.clone()));
        }
        if let Some(error) = &self.error {
            events.push(SemanticLedgerEvent::Failed(error.clone()));
        } else if self.completed {
            events.push(SemanticLedgerEvent::Completed);
        }
        SemanticLedgerV1 {
            schema_version: SEMANTIC_LEDGER_SCHEMA.into(),
            events,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticLedgerV1 {
    pub schema_version: String,
    pub events: Vec<SemanticLedgerEvent>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SemanticLedgerEvent {
    Block(ResponseBlock),
    ProviderState(Box<OpaqueProviderState>),
    Usage(ModelUsage),
    Finish(FinishReason),
    Completed,
    Failed(ModelError),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ResponseAccumulator {
    pub responses_metadata: BTreeMap<String, Value>,
    pub annotations: BTreeMap<u32, Vec<super::UrlCitationV1>>,
    pub finished_text: std::collections::BTreeSet<u32>,
    pub finished_reasoning: std::collections::BTreeSet<u32>,
    pub finished_refusal: std::collections::BTreeSet<u32>,
    pub item_statuses: BTreeMap<u32, ResponseItemStatus>,
    pub item_ids: BTreeMap<u32, String>,
    pub message_phases: BTreeMap<u32, String>,
    pub response_id: Option<String>,
    pub provider_model: Option<String>,
    pub blocks: BTreeMap<u32, MutableResponseBlock>,
    pub provider_state: Vec<OpaqueProviderState>,
    pub usage: ModelUsage,
    pub finish_reason: Option<FinishReason>,
    pub error: Option<ModelError>,
    pub terminal: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum MutableResponseBlock {
    WebSearch {
        item: super::WebSearchCallV1,
        native_id: String,
        owner: Box<ExactProviderPathV1>,
        finished: bool,
    },
    Text(String),
    Reasoning(String),
    Refusal(String),
    ToolCall {
        logical_id: String,
        native_id: String,
        tool_kind: ToolKindV1,
        namespace: Option<String>,
        name: String,
        owner: Box<ExactProviderPathV1>,
        arguments: String,
        encrypted_function_args: Option<Vec<String>>,
        finished: bool,
    },
}

impl ResponseAccumulator {
    pub fn finish(mut self, protocol: IngressProtocol) -> Result<ModelResponseIRV1, ModelIrError> {
        if !self.terminal {
            return Err(ModelIrError::MissingTerminalEvent);
        }
        if self.error.is_none()
            && (self.response_id.is_none()
                || self.provider_model.is_none()
                || self.finish_reason.is_none())
        {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "successful response lacks identity or finish reason".into(),
            ));
        }
        if self.error.is_some() {
            for index in self.blocks.keys() {
                self.item_statuses
                    .entry(*index)
                    .or_insert(ResponseItemStatus::Incomplete);
            }
        }
        let mut blocks = Vec::with_capacity(self.blocks.len());
        let mut tool_id_map = Vec::new();
        for (index, block) in self.blocks {
            blocks.push(match block {
                MutableResponseBlock::WebSearch {
                    item,
                    native_id,
                    owner,
                    finished,
                } => {
                    if !finished {
                        return Err(ModelIrError::MissingTerminalEvent);
                    }
                    tool_id_map.push(ToolIdMapEntryV1 {
                        logical_id: item.id.clone(),
                        native_id,
                        kind: ToolKindV1::Function,
                        name: "web_search".into(),
                        namespace: None,
                        owner: *owner,
                    });
                    ResponseBlock::WebSearch { index, item }
                }
                MutableResponseBlock::Text(text) => ResponseBlock::Text {
                    index,
                    item_id: self.item_ids.remove(&index),
                    phase: self.message_phases.remove(&index),
                    text,
                    annotations: self.annotations.remove(&index).unwrap_or_default(),
                    status: self.item_statuses.remove(&index).ok_or_else(|| {
                        ModelIrError::InvalidResponseLifecycle(format!(
                            "text block {index} lacks an item status"
                        ))
                    })?,
                },
                MutableResponseBlock::Reasoning(text) => ResponseBlock::Reasoning {
                    index,
                    item_id: self.item_ids.remove(&index),
                    text,
                    status: self.item_statuses.remove(&index).ok_or_else(|| {
                        ModelIrError::InvalidResponseLifecycle(format!(
                            "reasoning block {index} lacks an item status"
                        ))
                    })?,
                },
                MutableResponseBlock::Refusal(text) => ResponseBlock::Refusal {
                    index,
                    item_id: self.item_ids.remove(&index),
                    phase: self.message_phases.remove(&index),
                    text,
                    status: self.item_statuses.remove(&index).ok_or_else(|| {
                        ModelIrError::InvalidResponseLifecycle(format!(
                            "refusal block {index} lacks an item status"
                        ))
                    })?,
                },
                MutableResponseBlock::ToolCall {
                    logical_id,
                    native_id,
                    tool_kind,
                    namespace,
                    name,
                    owner,
                    arguments,
                    encrypted_function_args,
                    finished,
                } => {
                    if !finished {
                        return Err(ModelIrError::InvalidResponseLifecycle(format!(
                            "Tool call {logical_id} never finished"
                        )));
                    }
                    let arguments = match tool_kind {
                        ToolKindV1::Function => serde_json::from_str(&arguments)
                            .map_err(|_| ModelIrError::InvalidToolArguments(logical_id.clone()))?,
                        ToolKindV1::Custom => Value::String(arguments),
                    };
                    tool_id_map.push(ToolIdMapEntryV1 {
                        logical_id: logical_id.clone(),
                        native_id,
                        kind: tool_kind,
                        name: name.clone(),
                        namespace: namespace.clone(),
                        owner: *owner,
                    });
                    ResponseBlock::ToolCall {
                        index,
                        item_id: self.item_ids.remove(&index),
                        logical_id,
                        tool_kind,
                        namespace,
                        name,
                        arguments,
                        encrypted_function_args,
                        status: self.item_statuses.remove(&index).ok_or_else(|| {
                            ModelIrError::InvalidResponseLifecycle(format!(
                                "tool block {index} lacks an item status"
                            ))
                        })?,
                    }
                }
            });
        }
        if !self.item_ids.is_empty() {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "response item ID has no canonical block".into(),
            ));
        }
        if !self.message_phases.is_empty() {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "response message phase has no canonical message block".into(),
            ));
        }
        if !self.item_statuses.is_empty() {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "response item status has no canonical block".into(),
            ));
        }
        Ok(ModelResponseIRV1 {
            responses_metadata: self.responses_metadata,
            source_protocol: protocol,
            response_id: self.response_id.unwrap_or_else(|| "error".into()),
            provider_model: self.provider_model.unwrap_or_else(|| "error".into()),
            blocks,
            provider_state: self.provider_state,
            tool_id_map,
            usage: self.usage,
            finish_reason: self.finish_reason,
            completed: self.error.is_none(),
            error: self.error,
        })
    }
}
