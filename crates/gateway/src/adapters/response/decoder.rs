use std::collections::VecDeque;

use hiroute_gateway_core::runtime::body::{BudgetTree, ChargedBytes, MemoryRole, StreamBudget};
use hiroute_gateway_core::runtime::sse::{
    BoundedEventEmitter, BoundedOutputSink, EofPolicy, SseComplexity, SseError, SseEventView,
    SseFeedOutcome, SseFramer, SseLimits, SseVisitor,
};

use crate::server::core_runtime::model_ir::{ModelIrError, ModelResponseIRV1, ModelStreamEventV1};
use crate::server::core_runtime::profiles::{
    CandidateProtocolProfile, CapabilityError, Fidelity, MAX_TERMINAL_CLASSIFIED_REFUSAL_BLOCKS,
    MAX_TERMINAL_CLASSIFIED_REFUSAL_BYTES, NativeProviderStateEmission, StateAffinity,
    StreamingRefusalSemantics,
};
use crate::server::request_plan::IngressProtocol;

use super::ProtocolAdapterError;
use super::protocols::ProtocolState;
use crate::server::core_runtime::adapters::ChatToolProjection;

const MAX_NATIVE_BODY_BYTES: usize = 16 * 1024 * 1024;
const MAX_SSE_EVENT_BYTES: usize = 256 * 1024;
const MAX_QUEUED_EVENTS: usize = 32;
const STREAM_MEMORY_BUDGET: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseDecodeStatus {
    Complete,
    NeedDrain,
    Terminal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedNativeResponse {
    pub response: ModelResponseIRV1,
    pub events: Vec<ModelStreamEventV1>,
    pub sse_complexity: Option<SseComplexity>,
}

pub struct NativeResponseDecoder {
    protocol: IngressProtocol,
    status: u16,
    streaming: bool,
    body: Vec<u8>,
    state: ProtocolState,
    events: VecDeque<ModelStreamEventV1>,
    budget: StreamBudget,
    framer: Option<SseFramer>,
    ended: bool,
    end_stream_pending: bool,
}

impl NativeResponseDecoder {
    pub fn new(
        profile: &CandidateProtocolProfile,
        status: u16,
        streaming: bool,
    ) -> Result<Self, ProtocolAdapterError> {
        Self::new_with_projections(
            profile,
            status,
            streaming,
            Some(super::super::continuation::ToolIdProjection::new(
                profile.ingress_protocol,
            )),
            None,
        )
    }

    pub(crate) fn new_for_attempt(
        profile: &CandidateProtocolProfile,
        status: u16,
        streaming: bool,
        chat_tool_projection: Option<ChatToolProjection>,
    ) -> Result<Self, ProtocolAdapterError> {
        Self::new_with_projections(profile, status, streaming, None, chat_tool_projection)
    }

    /// Builds an observation-only decoder with an explicit request-bound Tool
    /// identity projector. The projector has no continuation-authority side
    /// effects and replaces task-local lookup for this decoder.
    pub(crate) fn new_for_observation(
        profile: &CandidateProtocolProfile,
        status: u16,
        streaming: bool,
        tool_id_projection: super::super::continuation::ToolIdProjection,
        chat_tool_projection: Option<ChatToolProjection>,
    ) -> Result<Self, ProtocolAdapterError> {
        Self::new_with_projections(
            profile,
            status,
            streaming,
            Some(tool_id_projection),
            chat_tool_projection,
        )
    }

    fn new_with_projections(
        profile: &CandidateProtocolProfile,
        status: u16,
        streaming: bool,
        tool_id_projection: Option<super::super::continuation::ToolIdProjection>,
        chat_tool_projection: Option<ChatToolProjection>,
    ) -> Result<Self, ProtocolAdapterError> {
        if profile.capability.upstream_protocol != profile.connector.upstream_protocol
            || !profile.connector.critical_facts_are_exact()
            || profile.capability.native_provider_state == NativeProviderStateEmission::Unknown
            || profile.capability.response.refusal != Fidelity::Exact
            || (streaming && profile.capability.native_streaming.exact() != Some(&true))
            || (profile.capability.native_provider_state
                == NativeProviderStateEmission::ExactOwnerAffine
                && (profile.capability.request.provider_state != Fidelity::Exact
                    || profile.capability.request.state_affinity != StateAffinity::ExactOwner
                    || profile.capability.response.provider_state != Fidelity::Exact
                    || profile.capability.response.state_affinity != StateAffinity::ExactOwner))
        {
            return Err(
                crate::server::core_runtime::profiles::CapabilityError::ProfileUnknown.into(),
            );
        }
        let protocol = profile.capability.upstream_protocol;
        let (terminal_refusal_buffer, terminal_refusal_blocks) = if streaming {
            match (protocol, profile.capability.response.stream_refusal) {
                (
                    IngressProtocol::Messages,
                    StreamingRefusalSemantics::TerminalClassified {
                        max_buffered_bytes,
                        max_buffered_blocks,
                    },
                ) => {
                    let bytes = usize::try_from(max_buffered_bytes)
                        .ok()
                        .filter(|limit| {
                            *limit > 0
                                && *limit <= super::MAX_CANONICAL_SEMANTIC_BYTES
                                && max_buffered_bytes <= MAX_TERMINAL_CLASSIFIED_REFUSAL_BYTES
                        })
                        .ok_or(CapabilityError::StreamRefusalUnsupported)?;
                    if max_buffered_blocks == 0
                        || max_buffered_blocks > MAX_TERMINAL_CLASSIFIED_REFUSAL_BLOCKS
                    {
                        return Err(CapabilityError::StreamRefusalUnsupported.into());
                    }
                    (bytes, max_buffered_blocks)
                }
                (
                    IngressProtocol::Responses | IngressProtocol::ChatCompletions,
                    StreamingRefusalSemantics::ExactDelta,
                ) => (0, 0),
                _ => return Err(CapabilityError::StreamRefusalUnsupported.into()),
            }
        } else {
            (0, 0)
        };
        let owner = profile.exact_provider_path()?;
        let budget_tree = BudgetTree::new(STREAM_MEMORY_BUDGET, STREAM_MEMORY_BUDGET)
            .map_err(|error| ModelIrError::InvalidSse(error.to_string()))?;
        let budget = budget_tree
            .stream(STREAM_MEMORY_BUDGET)
            .map_err(|error| ModelIrError::InvalidSse(error.to_string()))?;
        let framer = if streaming {
            Some(
                SseFramer::new(
                    SseLimits {
                        max_event_bytes: MAX_SSE_EVENT_BYTES,
                        max_pending_bytes: MAX_SSE_EVENT_BYTES,
                        max_output_event_bytes: MAX_SSE_EVENT_BYTES,
                        expansion_ratio_numerator: 1,
                        expansion_ratio_denominator: 1,
                        expansion_slack_bytes: 0,
                        retained_capacity_threshold: MAX_SSE_EVENT_BYTES,
                        eof_policy: EofPolicy::Strict,
                    },
                    budget.clone(),
                )
                .map_err(|error| ModelIrError::InvalidSse(error.to_string()))?,
            )
        } else {
            None
        };
        Ok(Self {
            protocol,
            status,
            streaming,
            body: Vec::new(),
            state: ProtocolState::new(
                protocol,
                owner,
                profile.capability.native_provider_state,
                terminal_refusal_buffer,
                terminal_refusal_blocks,
                tool_id_projection,
                chat_tool_projection,
            ),
            events: VecDeque::new(),
            budget,
            framer,
            ended: false,
            end_stream_pending: false,
        })
    }

    /// Feeds one arbitrary transport fragment. No implicit terminal event is
    /// created at EOF; a native protocol terminal is mandatory.
    pub fn feed(
        &mut self,
        bytes: &[u8],
        end_stream: bool,
    ) -> Result<ResponseDecodeStatus, ProtocolAdapterError> {
        if self.ended {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "response decoder was fed after EOF".into(),
            )
            .into());
        }
        if self.events.len() >= MAX_QUEUED_EVENTS {
            return Ok(ResponseDecodeStatus::NeedDrain);
        }
        let need_drain = if self.streaming {
            if end_stream {
                self.end_stream_pending = true;
            }
            matches!(
                self.feed_sse(bytes, end_stream)?,
                SseFeedOutcome::NeedDrain { .. }
            )
        } else {
            self.feed_json(bytes, end_stream)?;
            false
        };
        if end_stream && !self.streaming {
            self.ended = true;
            if !self.state.core().accumulator.terminal {
                return Err(ModelIrError::MissingTerminalEvent.into());
            }
        }
        if self.streaming && self.end_stream_pending && !need_drain {
            self.finish_stream_eof()?;
        }
        if need_drain || self.events.len() >= MAX_QUEUED_EVENTS {
            return Ok(ResponseDecodeStatus::NeedDrain);
        }
        Ok(if self.state.core().accumulator.terminal {
            ResponseDecodeStatus::Terminal
        } else {
            ResponseDecodeStatus::Complete
        })
    }

    pub fn resume(&mut self) -> Result<ResponseDecodeStatus, ProtocolAdapterError> {
        if !self.streaming {
            return Ok(ResponseDecodeStatus::Complete);
        }
        if self.events.len() >= MAX_QUEUED_EVENTS {
            return Ok(ResponseDecodeStatus::NeedDrain);
        }
        let framer = self.framer.as_mut().expect("streaming decoder has framer");
        if !framer.needs_drain() {
            if self.end_stream_pending {
                self.finish_stream_eof()?;
            }
            return Ok(if self.state.core().accumulator.terminal {
                ResponseDecodeStatus::Terminal
            } else {
                ResponseDecodeStatus::Complete
            });
        }
        let outcome = run_sse(
            framer,
            None,
            &self.budget,
            &mut self.state,
            &mut self.events,
        )?;
        if matches!(outcome, SseFeedOutcome::Complete) && self.end_stream_pending {
            self.finish_stream_eof()?;
        }
        Ok(match outcome {
            SseFeedOutcome::NeedDrain { .. } => ResponseDecodeStatus::NeedDrain,
            SseFeedOutcome::Complete if self.state.core().accumulator.terminal => {
                ResponseDecodeStatus::Terminal
            }
            SseFeedOutcome::Complete => ResponseDecodeStatus::Complete,
        })
    }

    pub fn take_events(&mut self) -> Vec<ModelStreamEventV1> {
        self.events.drain(..).collect()
    }

    pub fn sse_complexity(&self) -> Option<SseComplexity> {
        self.framer.as_ref().map(SseFramer::complexity)
    }

    pub fn finish(self) -> Result<DecodedNativeResponse, ProtocolAdapterError> {
        if !self.ended || !self.state.core().accumulator.terminal {
            return Err(ModelIrError::MissingTerminalEvent.into());
        }
        let complexity = self.framer.as_ref().map(SseFramer::complexity);
        let response = self.state.into_core().finish(self.protocol)?;
        Ok(DecodedNativeResponse {
            response,
            events: self.events.into_iter().collect(),
            sse_complexity: complexity,
        })
    }

    fn feed_json(&mut self, bytes: &[u8], end_stream: bool) -> Result<(), ProtocolAdapterError> {
        let next = self
            .body
            .len()
            .checked_add(bytes.len())
            .ok_or(ModelIrError::BufferLimit(MAX_NATIVE_BODY_BYTES))?;
        if next > MAX_NATIVE_BODY_BYTES {
            return Err(ModelIrError::BufferLimit(MAX_NATIVE_BODY_BYTES).into());
        }
        self.body.extend_from_slice(bytes);
        if end_stream {
            self.state
                .decode_nonstream(self.status, &self.body, &mut self.events)?;
        }
        Ok(())
    }

    fn feed_sse(
        &mut self,
        bytes: &[u8],
        end_stream: bool,
    ) -> Result<SseFeedOutcome, ProtocolAdapterError> {
        let charged =
            ChargedBytes::copy_from_opaque(&self.budget, MemoryRole::TransportInflight, bytes)
                .map_err(|error| ModelIrError::InvalidSse(error.to_string()))?;
        let outcome = run_sse(
            self.framer.as_mut().expect("streaming decoder has framer"),
            Some((charged, end_stream)),
            &self.budget,
            &mut self.state,
            &mut self.events,
        )?;
        Ok(outcome)
    }

    fn finish_stream_eof(&mut self) -> Result<(), ProtocolAdapterError> {
        self.end_stream_pending = false;
        self.ended = true;
        if !self.state.core().accumulator.terminal {
            return Err(ModelIrError::MissingTerminalEvent.into());
        }
        Ok(())
    }
}

fn run_sse(
    framer: &mut SseFramer,
    input: Option<(ChargedBytes, bool)>,
    budget: &StreamBudget,
    state: &mut ProtocolState,
    output: &mut VecDeque<ModelStreamEventV1>,
) -> Result<SseFeedOutcome, ProtocolAdapterError> {
    let mut visitor = NativeVisitor {
        budget,
        state,
        output,
        protocol_error: None,
    };
    let mut sink = DiscardOutput;
    let framed = match input {
        Some((bytes, end_stream)) => framer.feed(bytes, end_stream, &mut visitor, &mut sink),
        None => framer.resume(&mut visitor, &mut sink),
    };
    if let Some(error) = visitor.protocol_error {
        return Err(error);
    }
    framed.map_err(|error| ModelIrError::InvalidSse(error.to_string()).into())
}

struct NativeVisitor<'a> {
    budget: &'a StreamBudget,
    state: &'a mut ProtocolState,
    output: &'a mut VecDeque<ModelStreamEventV1>,
    protocol_error: Option<ProtocolAdapterError>,
}

impl SseVisitor for NativeVisitor<'_> {
    fn on_event(
        &mut self,
        event: SseEventView<'_>,
        emitter: &mut BoundedEventEmitter<'_, '_>,
    ) -> Result<(), SseError> {
        if self.output.len() >= MAX_QUEUED_EVENTS {
            return Err(SseError::NeedDrain);
        }
        if event
            .fields()
            .any(|field| !field.comment && field.name.starts_with(b"data") && field.name != b"data")
        {
            self.protocol_error =
                Some(ModelIrError::InvalidSse("malformed data-prefixed field".into()).into());
            return Err(SseError::FramerFailed);
        }
        let data = event.data(self.budget)?;
        if data.as_ref().is_empty() {
            return emitter.drop_event();
        }
        let event_type = match event.event_type() {
            Some(value) => match std::str::from_utf8(value) {
                Ok(value) => Some(value),
                Err(error) => {
                    self.protocol_error = Some(ModelIrError::InvalidSse(error.to_string()).into());
                    return Err(SseError::FramerFailed);
                }
            },
            None => None,
        };
        if let Err(error) = self
            .state
            .decode_sse(event_type, data.as_ref(), self.output)
        {
            self.protocol_error = Some(error);
            return Err(SseError::FramerFailed);
        }
        emitter.drop_event()
    }
}

struct DiscardOutput;

impl BoundedOutputSink for DiscardOutput {
    fn emit_borrowed(&mut self, _bytes: &[u8]) -> Result<(), SseError> {
        Ok(())
    }

    fn emit_owned(&mut self, _bytes: ChargedBytes) -> Result<(), SseError> {
        Ok(())
    }
}
