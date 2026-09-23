//! Same-protocol response projection.
//!
//! This path deliberately does not build `ModelResponseIRV1`. It preserves the
//! provider's legal wire shape and owns only the fields HiRoute must rewrite or
//! validate for routing identity, continuation safety, terminal delivery and
//! usage observation.

use std::collections::BTreeMap;

use hiroute_gateway_core::runtime::body::{BudgetTree, MemoryRole, Reservation, StreamBudget};
use hiroute_gateway_core::runtime::sse::{EofPolicy, SseFramer, SseLimits};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::server::core_runtime::model_ir::{
    ExactProviderPathV1, ModelError, ModelIrError, ModelUsage, ToolKindV1,
};
use crate::server::core_runtime::profiles::CandidateProtocolProfile;
use crate::server::request_plan::IngressProtocol;

use super::super::continuation::{
    ToolIdProjection, project_delivered_tool_id, record_provider_state,
};
use super::super::{ChatToolIdentity, ChatToolProjection, ProtocolAdapterError};

#[path = "native_passthrough_helpers.rs"]
mod helpers;
use helpers::*;
#[path = "native_passthrough_evidence.rs"]
mod evidence;
#[path = "native_passthrough_state.rs"]
mod provider_state;

const RETAINED_SSE_CAPACITY: usize = 256 * 1024;
const OBSERVATION_STREAM_BUDGET: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NativeTerminalOutcome {
    Complete,
    Incomplete,
    Failed,
    Unknown,
}

#[derive(Clone, Debug)]
pub(crate) struct NativeProjectedUnit {
    pub(crate) bytes: Vec<u8>,
    pub(crate) source_bytes: usize,
    pub(crate) semantic: bool,
    pub(crate) terminal: Option<NativeTerminalOutcome>,
    pub(crate) failure: Option<ModelError>,
}

pub(crate) struct NativeResponseProjector {
    streaming: bool,
    body: super::body_buffer::BodyBuffer,
    budget: StreamBudget,
    framer: Option<SseFramer>,
    state: ProjectionState,
    pending_terminal_tail: bool,
    ended: bool,
}

impl NativeResponseProjector {
    pub(crate) fn new_for_attempt(
        profile: &CandidateProtocolProfile,
        streaming: bool,
        served_model_alias: String,
        chat_projection: Option<ChatToolProjection>,
        budget: StreamBudget,
    ) -> Result<Self, ProtocolAdapterError> {
        Self::new(
            profile,
            streaming,
            served_model_alias,
            chat_projection,
            ToolIdDelivery::Active,
            budget,
        )
    }

    pub(crate) fn new_for_observation(
        profile: &CandidateProtocolProfile,
        streaming: bool,
        served_model_alias: String,
        chat_projection: Option<ChatToolProjection>,
        tool_projection: ToolIdProjection,
    ) -> Result<Self, ProtocolAdapterError> {
        // Capture already reserves its own bounded input/output copies. This
        // local framer budget is off-path and cannot grant execution authority.
        let tree = BudgetTree::new(OBSERVATION_STREAM_BUDGET, OBSERVATION_STREAM_BUDGET)
            .map_err(|error| ModelIrError::InvalidSse(error.to_string()))?;
        let budget = tree
            .stream(OBSERVATION_STREAM_BUDGET)
            .map_err(|error| ModelIrError::InvalidSse(error.to_string()))?;
        Self::new(
            profile,
            streaming,
            served_model_alias,
            chat_projection,
            ToolIdDelivery::Projection(tool_projection),
            budget,
        )
    }

    fn new(
        profile: &CandidateProtocolProfile,
        streaming: bool,
        served_model_alias: String,
        chat_projection: Option<ChatToolProjection>,
        authority: ToolIdDelivery,
        budget: StreamBudget,
    ) -> Result<Self, ProtocolAdapterError> {
        if profile.ingress_protocol != profile.capability.upstream_protocol
            || profile.capability.upstream_protocol != profile.connector.upstream_protocol
            || !profile.connector.critical_facts_are_exact()
            || (streaming && profile.capability.native_streaming.exact() != Some(&true))
        {
            return Err(
                crate::server::core_runtime::profiles::CapabilityError::ProfileUnknown.into(),
            );
        }
        let framer = streaming
            .then(|| {
                SseFramer::new(
                    SseLimits {
                        max_event_bytes: usize::MAX,
                        max_pending_bytes: usize::MAX,
                        max_output_event_bytes: usize::MAX,
                        expansion_ratio_numerator: 2,
                        expansion_ratio_denominator: 1,
                        // Owned ID rewrites may expand many small native IDs.
                        // Charge their actual allocation instead of imposing a wire ratio.
                        expansion_slack_bytes: usize::MAX,
                        retained_capacity_threshold: RETAINED_SSE_CAPACITY,
                        eof_policy: EofPolicy::Strict,
                    },
                    budget.clone(),
                )
                .map_err(|error| {
                    ProtocolAdapterError::from(ModelIrError::InvalidSse(error.to_string()))
                })
            })
            .transpose()?;
        Ok(Self {
            streaming,
            body: super::body_buffer::BodyBuffer::default(),
            budget: budget.clone(),
            framer,
            state: ProjectionState {
                protocol: profile.capability.upstream_protocol,
                alias: served_model_alias,
                owner: profile.exact_provider_path()?,
                authority,
                chat_projection,
                response_id: None,
                native_model: None,
                tools: BTreeMap::new(),
                retained: Vec::new(),
                response_items: BTreeMap::new(),
                response_deltas: BTreeMap::new(),
                response_items_uncertain: false,
                reasoning_state: BTreeMap::new(),
                messages_signatures: BTreeMap::new(),
                responses_done_seen: false,
                usage: ModelUsage::default(),
                usage_input_overflow: false,
                terminal: None,
                messages_stop: None,
                messages_stop_digest: None,
                chat_seen: false,
                chat_finish: None,
                chat_finish_digest: None,
                chat_uncertain: false,
                budget,
            },
            pending_terminal_tail: false,
            ended: false,
        })
    }

    pub(crate) fn feed(
        &mut self,
        bytes: &[u8],
        end_stream: bool,
    ) -> Result<Vec<NativeProjectedUnit>, ProtocolAdapterError> {
        if self.ended {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "native projector was fed after transport EOF".into(),
            )
            .into());
        }
        if !self.streaming {
            self.body.append(bytes, &self.budget)?;
            if !end_stream {
                return Ok(Vec::new());
            }
            self.ended = true;
            let mut value: Value = serde_json::from_slice(self.body.bytes())
                .map_err(|error| ModelIrError::InvalidJson(error.to_string()))?;
            let metadata = self.state.project_nonstream(&mut value)?;
            let bytes = serde_json::to_vec(&value)
                .map_err(|error| ProtocolAdapterError::Serialization(error.to_string()))?;
            return Ok(vec![NativeProjectedUnit {
                bytes,
                source_bytes: self.body.len(),
                semantic: metadata.semantic,
                terminal: Some(
                    metadata
                        .terminal
                        .ok_or(ModelIrError::MissingTerminalEvent)?,
                ),
                failure: metadata.failure,
            }]);
        }

        let output = super::native_passthrough_sse::feed_projected_sse(
            self.framer
                .as_mut()
                .expect("streaming projector owns an SSE framer"),
            &self.budget,
            &mut self.state,
            &mut self.pending_terminal_tail,
            bytes,
            end_stream,
        )?;
        if end_stream {
            self.ended = true;
            if self.state.terminal.is_none() {
                return Err(ModelIrError::MissingTerminalEvent.into());
            }
        }
        Ok(output)
    }

    pub(crate) fn usage(&self) -> &ModelUsage {
        &self.state.usage
    }
}

#[derive(Clone)]
enum ToolIdDelivery {
    Active,
    Projection(ToolIdProjection),
}

impl ToolIdDelivery {
    fn project(
        &self,
        native_id: &str,
        owner: &ExactProviderPathV1,
    ) -> Result<String, ProtocolAdapterError> {
        match self {
            Self::Active => project_delivered_tool_id(native_id, owner),
            Self::Projection(projection) => projection.project(native_id, owner),
        }
    }

    fn records_state(&self) -> bool {
        matches!(self, Self::Active)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NativeToolIdentity {
    native_id: String,
    logical_id: String,
    kind: ToolKindV1,
    namespace: Option<String>,
    name: String,
}

#[derive(Default)]
struct ResponseItemEvidence {
    identity: Option<[u8; 32]>,
    done: Option<[u8; 32]>,
    added_text: Vec<evidence::AddedTextPrefix>,
}

pub(super) struct ProjectionState {
    pub(super) protocol: IngressProtocol,
    alias: String,
    owner: ExactProviderPathV1,
    authority: ToolIdDelivery,
    chat_projection: Option<ChatToolProjection>,
    response_id: Option<String>,
    native_model: Option<String>,
    tools: BTreeMap<u32, NativeToolIdentity>,
    retained: Vec<Reservation>,
    response_items: BTreeMap<u32, ResponseItemEvidence>,
    response_deltas: BTreeMap<evidence::ResponseDeltaKey, evidence::ResponseDeltaEvidence>,
    response_items_uncertain: bool,
    reasoning_state: BTreeMap<u32, [u8; 32]>,
    messages_signatures: BTreeMap<u32, String>,
    responses_done_seen: bool,
    usage: ModelUsage,
    usage_input_overflow: bool,
    pub(super) terminal: Option<NativeTerminalOutcome>,
    messages_stop: Option<NativeTerminalOutcome>,
    messages_stop_digest: Option<[u8; 32]>,
    chat_seen: bool,
    chat_finish: Option<NativeTerminalOutcome>,
    chat_finish_digest: Option<[u8; 32]>,
    chat_uncertain: bool,
    budget: StreamBudget,
}

pub(super) struct ProjectionMetadata {
    pub(super) semantic: bool,
    pub(super) terminal: Option<NativeTerminalOutcome>,
    pub(super) failure: Option<ModelError>,
    pub(super) source_bytes: usize,
}

impl ProjectionState {
    fn project_nonstream(
        &mut self,
        value: &mut Value,
    ) -> Result<ProjectionMetadata, ProtocolAdapterError> {
        let object = value.as_object_mut().ok_or(ModelIrError::ExpectedObject)?;
        match self.protocol {
            IngressProtocol::Responses => self.project_responses_nonstream(object),
            IngressProtocol::ChatCompletions => self.project_chat_nonstream(object),
            IngressProtocol::Messages => self.project_messages_nonstream(object),
        }
    }

    pub(super) fn project_sse(
        &mut self,
        event_type: Option<&str>,
        data: &[u8],
        raw_event: &[u8],
    ) -> Result<(Option<Vec<u8>>, ProjectionMetadata), ProtocolAdapterError> {
        if self.protocol == IngressProtocol::Responses && data == b"[DONE]" {
            if event_type.is_some() || self.terminal.is_none() || self.responses_done_seen {
                return Err(ModelIrError::InvalidResponseLifecycle(
                    "Responses [DONE] must follow one terminal event".into(),
                )
                .into());
            }
            self.responses_done_seen = true;
            return Ok((None, ProjectionMetadata::opaque()));
        }
        let after_terminal = self.terminal.take().is_some();
        if after_terminal {
            // Reuse the same owned identity projection, but a second provider
            // description can no longer certify one unambiguous completion.
            self.mark_unowned_uncertain();
        }
        if self.protocol == IngressProtocol::ChatCompletions && data == b"[DONE]" {
            if after_terminal {
                self.terminal = Some(NativeTerminalOutcome::Unknown);
                return Ok((None, ProjectionMetadata::opaque()));
            }
            let terminal = self.chat_terminal();
            self.set_terminal(terminal)?;
            return Ok((
                None,
                ProjectionMetadata {
                    semantic: false,
                    terminal: Some(terminal),
                    failure: None,
                    source_bytes: 0,
                },
            ));
        }
        let mut value: Value = match serde_json::from_slice(data) {
            Ok(Value::Object(object)) => Value::Object(object),
            Ok(_) | Err(_)
                if event_type.is_some_and(|event| unowned_extension(self.protocol, event)) =>
            {
                self.mark_unowned_uncertain();
                if after_terminal {
                    self.terminal = Some(NativeTerminalOutcome::Unknown);
                }
                return Ok((None, ProjectionMetadata::opaque()));
            }
            Ok(_) => return Err(ModelIrError::ExpectedObject.into()),
            Err(error) => return Err(ModelIrError::InvalidJson(error.to_string()).into()),
        };
        let object = value.as_object_mut().expect("checked JSON object");
        let tools_before = self.tools.len();
        let mut metadata = match self.protocol {
            IngressProtocol::Responses => self.project_responses_sse(event_type, object)?,
            IngressProtocol::Messages => {
                let metadata = self.project_messages_sse(event_type, object)?;
                self.observe_messages_state(object, raw_event)?;
                metadata
            }
            IngressProtocol::ChatCompletions => self.project_chat_sse(object)?,
        };
        if after_terminal {
            self.terminal = Some(NativeTerminalOutcome::Unknown);
            metadata.terminal = None;
            metadata.failure = None;
        }
        let changed = value
            != serde_json::from_slice::<Value>(data)
                .map_err(|error| ModelIrError::InvalidJson(error.to_string()))?;
        // Tool delivery bookkeeping matches the serialized ID field only after
        // transport acceptance. Normalize this known frame's JSON syntax even
        // when the native ID itself is unchanged (including escaped IDs).
        let rewritten = (changed || self.tools.len() != tools_before)
            .then(|| serde_json::to_vec(&value))
            .transpose()
            .map_err(|error| ProtocolAdapterError::Serialization(error.to_string()))?;
        Ok((rewritten, metadata))
    }

    fn project_responses_sse(
        &mut self,
        event_type: Option<&str>,
        object: &mut Map<String, Value>,
    ) -> Result<ProjectionMetadata, ProtocolAdapterError> {
        let Some(native_type) = object
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            self.response_items_uncertain = true;
            return Ok(ProjectionMetadata::opaque());
        };
        if event_type.is_some_and(|event| event != native_type) {
            self.response_items_uncertain = true;
        }
        let mut terminal = None;
        let mut failure = None;
        let semantic = match native_type.as_str() {
            "response.created" | "response.in_progress" => {
                let response = object_field_mut(object, "response")?;
                self.project_response_identity(response)?;
                self.observe_usage(IngressProtocol::Responses, response.get("usage"));
                false
            }
            "response.output_item.added" | "response.output_item.done" => {
                let index = u32_field(object, "output_index")?;
                let item = object_field_mut(object, "item")?;
                self.project_responses_item(index, item)?;
                self.observe_response_item(index, item, native_type == "response.output_item.done");
                response_item_is_semantic(item)
            }
            "response.web_search_call.in_progress"
            | "response.web_search_call.searching"
            | "response.web_search_call.completed" => {
                let index = u32_field(object, "output_index")?;
                let native_id = string_field(object, "item_id")?.to_owned();
                if !self.tools.contains_key(&index) {
                    self.response_items_uncertain = true;
                    return Ok(ProjectionMetadata::opaque());
                }
                let logical =
                    self.project_tool(index, &native_id, ToolKindV1::Function, None, "web_search")?;
                object.insert("item_id".into(), Value::String(logical));
                false
            }
            "response.completed" | "response.incomplete" | "response.failed" => {
                let response = object_field_mut(object, "response")?;
                self.project_response_identity(response)?;
                self.project_responses_output(response)?;
                let semantic = responses_output_is_semantic(response);
                self.reconcile_response_items(response);
                self.reconcile_response_deltas(response);
                self.observe_usage(IngressProtocol::Responses, response.get("usage"));
                let expected = match native_type.as_str() {
                    "response.completed" => ("completed", NativeTerminalOutcome::Complete),
                    "response.incomplete" => ("incomplete", NativeTerminalOutcome::Incomplete),
                    _ => ("failed", NativeTerminalOutcome::Failed),
                };
                let outcome = if expected.1 == NativeTerminalOutcome::Complete
                    && (self.response_items_uncertain
                        || response.get("status").and_then(Value::as_str) != Some(expected.0))
                {
                    NativeTerminalOutcome::Unknown
                } else {
                    expected.1
                };
                self.set_terminal(outcome)?;
                terminal = Some(outcome);
                if expected.1 == NativeTerminalOutcome::Failed {
                    failure = Some(loose_error(response.get("error").unwrap_or(&Value::Null)));
                }
                semantic
            }
            "error" => {
                let outcome = NativeTerminalOutcome::Failed;
                self.set_terminal(outcome)?;
                terminal = Some(outcome);
                failure = Some(loose_error(&Value::Object(object.clone())));
                false
            }
            event if responses_semantic_event(event) => {
                self.observe_response_payload(event, object)
            }
            _ => false,
        };
        Ok(ProjectionMetadata {
            semantic,
            terminal,
            failure,
            source_bytes: 0,
        })
    }

    fn project_messages_sse(
        &mut self,
        event_type: Option<&str>,
        object: &mut Map<String, Value>,
    ) -> Result<ProjectionMetadata, ProtocolAdapterError> {
        let Some(native_type) = object
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            self.messages_stop = Some(NativeTerminalOutcome::Unknown);
            return Ok(ProjectionMetadata::opaque());
        };
        if event_type.is_some_and(|event| event != native_type) {
            self.messages_stop = Some(NativeTerminalOutcome::Unknown);
        }
        let mut terminal = None;
        let mut failure = None;
        let semantic = match native_type.as_str() {
            "message_start" => {
                let message = object_field_mut(object, "message")?;
                self.project_identity(message)?;
                self.observe_usage(IngressProtocol::Messages, message.get("usage"));
                false
            }
            "content_block_start" => {
                let index = u32_field(object, "index")?;
                let block = object_field_mut(object, "content_block")?;
                if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                    self.project_messages_tool(index, block)?;
                }
                let semantic = message_block_is_semantic(block);
                if semantic && self.messages_stop.is_some() {
                    self.messages_stop = Some(NativeTerminalOutcome::Unknown);
                }
                semantic
            }
            "content_block_delta" => {
                let semantic = object
                    .get("delta")
                    .and_then(Value::as_object)
                    .is_some_and(message_delta_is_semantic);
                if semantic && self.messages_stop.is_some() {
                    self.messages_stop = Some(NativeTerminalOutcome::Unknown);
                }
                semantic
            }
            "content_block_stop" => false,
            "message_delta" => {
                let delta = object
                    .get("delta")
                    .and_then(Value::as_object)
                    .ok_or(ModelIrError::InvalidField("delta"))?;
                if let Some(reason) = delta.get("stop_reason").and_then(Value::as_str) {
                    self.messages_stop = Some(merge_terminal_description(
                        self.messages_stop,
                        &mut self.messages_stop_digest,
                        reason,
                        classify_messages_finish(reason),
                    ));
                }
                self.observe_usage(IngressProtocol::Messages, object.get("usage"));
                false
            }
            "message_stop" => {
                let outcome = self.messages_stop.unwrap_or(NativeTerminalOutcome::Unknown);
                self.set_terminal(outcome)?;
                terminal = Some(outcome);
                false
            }
            "error" => {
                let outcome = NativeTerminalOutcome::Failed;
                self.set_terminal(outcome)?;
                terminal = Some(outcome);
                failure = Some(loose_error(&Value::Object(object.clone())));
                false
            }
            "ping" => false,
            _ => false,
        };
        Ok(ProjectionMetadata {
            semantic,
            terminal,
            failure,
            source_bytes: 0,
        })
    }

    fn project_chat_sse(
        &mut self,
        object: &mut Map<String, Value>,
    ) -> Result<ProjectionMetadata, ProtocolAdapterError> {
        if object.contains_key("error") {
            let outcome = NativeTerminalOutcome::Failed;
            self.set_terminal(outcome)?;
            return Ok(ProjectionMetadata {
                semantic: false,
                terminal: Some(outcome),
                failure: Some(loose_error(&Value::Object(object.clone()))),
                source_bytes: 0,
            });
        }
        self.project_identity(object)?;
        self.observe_usage(IngressProtocol::ChatCompletions, object.get("usage"));
        let mut semantic = false;
        if let Some(choices) = object.get_mut("choices").and_then(Value::as_array_mut) {
            for choice in choices {
                let choice = choice
                    .as_object_mut()
                    .ok_or(ModelIrError::InvalidField("choices"))?;
                let choice_index = choice.get("index").and_then(Value::as_u64);
                if choice_index != Some(0) {
                    self.chat_uncertain = true;
                    if let Some(delta) = choice.get("delta").and_then(Value::as_object) {
                        semantic |= chat_delta_is_semantic(delta);
                    }
                    continue;
                }
                self.chat_seen = true;
                let finish_preceded_content = self.chat_finish.is_some();
                if let Some(delta) = choice.get_mut("delta").and_then(Value::as_object_mut) {
                    let content = chat_delta_is_semantic(delta);
                    if content && finish_preceded_content {
                        self.chat_uncertain = true;
                    }
                    semantic |= content;
                    self.project_chat_tools(delta, None)?;
                }
                if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                    self.chat_finish = Some(merge_terminal_description(
                        self.chat_finish,
                        &mut self.chat_finish_digest,
                        reason,
                        classify_chat_finish(reason),
                    ));
                }
            }
        }
        Ok(ProjectionMetadata {
            semantic,
            terminal: None,
            failure: None,
            source_bytes: 0,
        })
    }

    fn project_responses_nonstream(
        &mut self,
        object: &mut Map<String, Value>,
    ) -> Result<ProjectionMetadata, ProtocolAdapterError> {
        let semantic = responses_output_is_semantic(object);
        self.project_identity(object)?;
        self.project_responses_output(object)?;
        self.observe_usage(IngressProtocol::Responses, object.get("usage"));
        let terminal = match string_field(object, "status")? {
            "completed" => NativeTerminalOutcome::Complete,
            "incomplete" => NativeTerminalOutcome::Incomplete,
            "failed" => NativeTerminalOutcome::Failed,
            _ => NativeTerminalOutcome::Unknown,
        };
        self.set_terminal(terminal)?;
        Ok(ProjectionMetadata {
            semantic,
            terminal: Some(terminal),
            failure: (terminal == NativeTerminalOutcome::Failed)
                .then(|| loose_error(object.get("error").unwrap_or(&Value::Null))),
            source_bytes: 0,
        })
    }

    fn project_messages_nonstream(
        &mut self,
        object: &mut Map<String, Value>,
    ) -> Result<ProjectionMetadata, ProtocolAdapterError> {
        self.project_identity(object)?;
        self.observe_usage(IngressProtocol::Messages, object.get("usage"));
        if let Some(content) = object.get_mut("content").and_then(Value::as_array_mut) {
            for (index, block) in content.iter_mut().enumerate() {
                let block = block
                    .as_object_mut()
                    .ok_or(ModelIrError::InvalidField("content"))?;
                if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                    self.project_messages_tool(
                        u32::try_from(index).map_err(|_| ModelIrError::InvalidField("content"))?,
                        block,
                    )?;
                }
                if block.get("type").and_then(Value::as_str) == Some("thinking")
                    && let Some(signature) = block
                        .get("signature")
                        .filter(|value| value.as_str().is_some_and(|s| !s.is_empty()))
                    && self.authority.records_state()
                {
                    record_provider_state(signature, &self.owner)?;
                }
            }
        }
        let terminal = object
            .get("stop_reason")
            .and_then(Value::as_str)
            .map(classify_messages_finish)
            .unwrap_or(NativeTerminalOutcome::Unknown);
        self.set_terminal(terminal)?;
        Ok(ProjectionMetadata {
            semantic: object
                .get("content")
                .and_then(Value::as_array)
                .is_some_and(|blocks| {
                    blocks
                        .iter()
                        .any(|block| block.as_object().is_some_and(message_block_is_semantic))
                }),
            terminal: Some(terminal),
            failure: None,
            source_bytes: 0,
        })
    }

    fn project_chat_nonstream(
        &mut self,
        object: &mut Map<String, Value>,
    ) -> Result<ProjectionMetadata, ProtocolAdapterError> {
        self.project_identity(object)?;
        self.observe_usage(IngressProtocol::ChatCompletions, object.get("usage"));
        if let Some(choices) = object.get_mut("choices").and_then(Value::as_array_mut) {
            for choice in choices {
                let choice = choice
                    .as_object_mut()
                    .ok_or(ModelIrError::InvalidField("choices"))?;
                let index = choice.get("index").and_then(Value::as_u64);
                if index != Some(0) {
                    self.chat_uncertain = true;
                    continue;
                }
                self.chat_seen = true;
                let message = choice
                    .get_mut("message")
                    .and_then(Value::as_object_mut)
                    .ok_or(ModelIrError::InvalidField("message"))?;
                self.project_chat_tools(message, Some(0))?;
                if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                    self.chat_finish = Some(merge_terminal_description(
                        self.chat_finish,
                        &mut self.chat_finish_digest,
                        reason,
                        classify_chat_finish(reason),
                    ));
                }
            }
        }
        let terminal = self.chat_terminal();
        self.set_terminal(terminal)?;
        Ok(ProjectionMetadata {
            semantic: object
                .get("choices")
                .and_then(Value::as_array)
                .is_some_and(|choices| {
                    choices.iter().any(|choice| {
                        choice
                            .get("message")
                            .and_then(Value::as_object)
                            .is_some_and(chat_delta_is_semantic)
                    })
                }),
            terminal: Some(terminal),
            failure: None,
            source_bytes: 0,
        })
    }

    fn project_response_identity(
        &mut self,
        object: &mut Map<String, Value>,
    ) -> Result<(), ProtocolAdapterError> {
        self.project_identity(object)
    }

    fn project_identity(
        &mut self,
        object: &mut Map<String, Value>,
    ) -> Result<(), ProtocolAdapterError> {
        let id = string_field(object, "id")?.to_owned();
        let model = string_field(object, "model")?.to_owned();
        if self.response_id.as_ref().is_some_and(|known| known != &id)
            || self
                .native_model
                .as_ref()
                .is_some_and(|known| known != &model)
        {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "native response identity changed".into(),
            )
            .into());
        }
        self.response_id.get_or_insert(id);
        self.native_model.get_or_insert(model);
        object.insert("model".into(), Value::String(self.alias.clone()));
        Ok(())
    }

    fn project_responses_output(
        &mut self,
        response: &mut Map<String, Value>,
    ) -> Result<(), ProtocolAdapterError> {
        let Some(output) = response.get_mut("output").and_then(Value::as_array_mut) else {
            return Ok(());
        };
        for (index, item) in output.iter_mut().enumerate() {
            self.project_responses_item(
                u32::try_from(index).map_err(|_| ModelIrError::InvalidField("output"))?,
                item.as_object_mut()
                    .ok_or(ModelIrError::InvalidField("output"))?,
            )?;
        }
        Ok(())
    }

    fn project_responses_item(
        &mut self,
        index: u32,
        item: &mut Map<String, Value>,
    ) -> Result<(), ProtocolAdapterError> {
        match item.get("type").and_then(Value::as_str) {
            Some("function_call") | Some("custom_tool_call") => {
                let kind = if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    ToolKindV1::Function
                } else {
                    ToolKindV1::Custom
                };
                let native_id = string_field(item, "call_id")?.to_owned();
                let name = string_field(item, "name")?.to_owned();
                let namespace = item
                    .get("namespace")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let logical =
                    self.project_tool(index, &native_id, kind, namespace.as_deref(), &name)?;
                item.insert("call_id".into(), Value::String(logical));
            }
            Some("web_search_call") => {
                let native_id = string_field(item, "id")?.to_owned();
                let logical =
                    self.project_tool(index, &native_id, ToolKindV1::Function, None, "web_search")?;
                item.insert("id".into(), Value::String(logical));
            }
            Some("reasoning") => {
                if let Some(value) = item
                    .get("encrypted_content")
                    .filter(|value| !value.is_null())
                {
                    self.observe_reasoning_state(index, value)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn project_messages_tool(
        &mut self,
        index: u32,
        block: &mut Map<String, Value>,
    ) -> Result<(), ProtocolAdapterError> {
        let native_id = string_field(block, "id")?.to_owned();
        let name = string_field(block, "name")?.to_owned();
        let logical = self.project_tool(index, &native_id, ToolKindV1::Function, None, &name)?;
        block.insert("id".into(), Value::String(logical));
        Ok(())
    }

    fn project_chat_tools(
        &mut self,
        container: &mut Map<String, Value>,
        nonstream_index_base: Option<u32>,
    ) -> Result<(), ProtocolAdapterError> {
        let Some(calls) = container
            .get_mut("tool_calls")
            .and_then(Value::as_array_mut)
        else {
            return Ok(());
        };
        for (position, call) in calls.iter_mut().enumerate() {
            let call = call
                .as_object_mut()
                .ok_or(ModelIrError::InvalidField("tool_calls"))?;
            let index = match nonstream_index_base {
                Some(base) => base
                    .checked_add(
                        u32::try_from(position)
                            .map_err(|_| ModelIrError::InvalidField("tool_calls"))?,
                    )
                    .ok_or(ModelIrError::InvalidField("tool_calls"))?,
                None => u32_field(call, "index")?,
            };
            let existing = self.tools.get(&index).cloned();
            let native_id = call
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| existing.as_ref().map(|tool| tool.native_id.clone()))
                .ok_or_else(|| ModelIrError::MissingToolIdentity(format!("chat index {index}")))?;
            let emitted_name = call
                .get("function")
                .and_then(Value::as_object)
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let identity = match emitted_name {
                Some(emitted_name) => self.chat_identity(&emitted_name)?,
                None => existing
                    .as_ref()
                    .map(|tool| ChatToolIdentity {
                        emitted_name: tool.name.clone(),
                        kind: tool.kind,
                        namespace: tool.namespace.clone(),
                        local_name: tool.name.clone(),
                    })
                    .ok_or_else(|| {
                        ModelIrError::MissingToolIdentity(format!("chat index {index}"))
                    })?,
            };
            let logical = self.project_tool(
                index,
                &native_id,
                identity.kind,
                identity.namespace.as_deref(),
                &identity.local_name,
            )?;
            if call.contains_key("id") {
                call.insert("id".into(), Value::String(logical));
            }
        }
        Ok(())
    }

    fn chat_identity(&self, emitted: &str) -> Result<ChatToolIdentity, ProtocolAdapterError> {
        self.chat_projection
            .as_ref()
            .map(|projection| {
                projection.resolve_emitted(emitted).cloned().ok_or_else(|| {
                    ModelIrError::MissingToolIdentity(format!(
                        "unknown projected Chat tool {emitted}"
                    ))
                    .into()
                })
            })
            .unwrap_or_else(|| {
                Ok(ChatToolIdentity {
                    emitted_name: emitted.into(),
                    kind: ToolKindV1::Function,
                    namespace: None,
                    local_name: emitted.into(),
                })
            })
    }

    fn project_tool(
        &mut self,
        index: u32,
        native_id: &str,
        kind: ToolKindV1,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<String, ProtocolAdapterError> {
        if let Some(existing) = self.tools.get(&index) {
            if existing.native_id != native_id
                || existing.kind != kind
                || existing.namespace.as_deref() != namespace
                || existing.name != name
            {
                return Err(ModelIrError::InvalidResponseLifecycle(
                    "native Tool identity changed".into(),
                )
                .into());
            }
            return Ok(existing.logical_id.clone());
        }
        let retained = std::mem::size_of::<NativeToolIdentity>()
            .saturating_add(native_id.len())
            .saturating_add(name.len())
            .saturating_add(namespace.map_or(0, str::len))
            .saturating_add(128);
        let charge = self
            .budget
            .reserve(MemoryRole::SemanticState, retained)
            .map_err(|_| ModelIrError::BufferLimit(retained))?;
        self.response_id
            .as_deref()
            .ok_or_else(|| ModelIrError::MissingToolIdentity("response id".into()))?;
        let logical_id = self.authority.project(native_id, &self.owner)?;
        if self
            .tools
            .values()
            .any(|tool| tool.logical_id == logical_id)
        {
            return Err(ModelIrError::ToolContinuationConflict.into());
        }
        self.tools.insert(
            index,
            NativeToolIdentity {
                native_id: native_id.into(),
                logical_id: logical_id.clone(),
                kind,
                namespace: namespace.map(str::to_owned),
                name: name.into(),
            },
        );
        self.retained.push(charge);
        Ok(logical_id)
    }

    fn observe_usage(&mut self, protocol: IngressProtocol, value: Option<&Value>) {
        let Some(object) = value.and_then(Value::as_object) else {
            return;
        };
        let update = match protocol {
            IngressProtocol::Responses => ModelUsage {
                input_tokens: u64_value(object, "input_tokens"),
                output_tokens: u64_value(object, "output_tokens"),
                cache_read_tokens: nested_u64_value(
                    object,
                    "input_tokens_details",
                    "cached_tokens",
                ),
                cache_write_tokens: None,
                reasoning_tokens: nested_u64_value(
                    object,
                    "output_tokens_details",
                    "reasoning_tokens",
                ),
            },
            IngressProtocol::ChatCompletions => ModelUsage {
                input_tokens: u64_value(object, "prompt_tokens"),
                output_tokens: u64_value(object, "completion_tokens"),
                cache_read_tokens: nested_u64_value(
                    object,
                    "prompt_tokens_details",
                    "cached_tokens",
                ),
                cache_write_tokens: None,
                reasoning_tokens: nested_u64_value(
                    object,
                    "completion_tokens_details",
                    "reasoning_tokens",
                ),
            },
            IngressProtocol::Messages => {
                let base = u64_value(object, "input_tokens");
                let cache_read = u64_value(object, "cache_read_input_tokens");
                let cache_write = u64_value(object, "cache_creation_input_tokens");
                let input = base.and_then(|input| {
                    input
                        .checked_add(cache_read.unwrap_or_default())?
                        .checked_add(cache_write.unwrap_or_default())
                });
                self.usage_input_overflow |= base.is_some() && input.is_none();
                ModelUsage {
                    input_tokens: input,
                    output_tokens: u64_value(object, "output_tokens"),
                    cache_read_tokens: cache_read,
                    cache_write_tokens: cache_write,
                    reasoning_tokens: None,
                }
            }
        };
        self.usage.merge_from(&update);
        if self.usage_input_overflow {
            self.usage.input_tokens = None;
        }
    }
}
