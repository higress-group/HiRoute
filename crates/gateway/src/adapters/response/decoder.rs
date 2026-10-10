use std::collections::VecDeque;

use hiroute_gateway_core::runtime::body::{BudgetTree, ChargedBytes, MemoryRole, StreamBudget};
use hiroute_gateway_core::runtime::sse::{
    BoundedEventEmitter, BoundedOutputSink, EofPolicy, SseComplexity, SseError, SseEventView,
    SseFeedOutcome, SseFramer, SseLimits, SseVisitor,
};

use crate::server::core_runtime::model_ir::{ModelIrError, ModelResponseIRV1, ModelStreamEventV1};
use crate::server::core_runtime::profiles::CandidateProtocolProfile;
use crate::server::request_plan::IngressProtocol;

use super::ProtocolAdapterError;
use super::protocols::ProtocolState;
use crate::server::core_runtime::adapters::ChatToolProjection;

const RETAINED_SSE_CAPACITY: usize = 256 * 1024;
const MAX_QUEUED_EVENTS: usize = 32;
const STREAM_MEMORY_BUDGET: usize = 256 * 1024 * 1024;

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
    body: super::body_buffer::BodyBuffer,
    state: ProtocolState,
    events: VecDeque<ModelStreamEventV1>,
    budget: StreamBudget,
    framer: Option<SseFramer>,
    ended: bool,
    end_stream_pending: bool,
    received_bytes: u64,
    frame_index: u64,
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
            None,
        )
    }

    pub(crate) fn new_for_attempt(
        profile: &CandidateProtocolProfile,
        status: u16,
        streaming: bool,
        chat_tool_projection: Option<ChatToolProjection>,
        budget: StreamBudget,
    ) -> Result<Self, ProtocolAdapterError> {
        Self::new_with_projections(
            profile,
            status,
            streaming,
            None,
            chat_tool_projection,
            Some(budget),
        )
    }

    /// Builds an observation-only decoder with an explicit request-bound Tool
    /// identity projector. The projector has no continuation-authority side
    /// effects and replaces task-local lookup for this decoder.
    #[cfg(test)]
    pub(crate) fn new_for_observation(
        profile: &CandidateProtocolProfile,
        status: u16,
        streaming: bool,
        tool_id_projection: super::super::continuation::ToolIdProjection,
        chat_tool_projection: Option<ChatToolProjection>,
    ) -> Result<Self, ProtocolAdapterError> {
        Self::new_for_observation_with_budget(
            profile,
            status,
            streaming,
            tool_id_projection,
            chat_tool_projection,
            super::body_buffer::standalone_budget(),
        )
    }

    pub(crate) fn new_for_observation_with_budget(
        profile: &CandidateProtocolProfile,
        status: u16,
        streaming: bool,
        tool_id_projection: super::super::continuation::ToolIdProjection,
        chat_tool_projection: Option<ChatToolProjection>,
        budget: StreamBudget,
    ) -> Result<Self, ProtocolAdapterError> {
        Self::new_with_projections(
            profile,
            status,
            streaming,
            Some(tool_id_projection),
            chat_tool_projection,
            Some(budget),
        )
    }

    fn new_with_projections(
        profile: &CandidateProtocolProfile,
        status: u16,
        streaming: bool,
        tool_id_projection: Option<super::super::continuation::ToolIdProjection>,
        chat_tool_projection: Option<ChatToolProjection>,
        request_budget: Option<StreamBudget>,
    ) -> Result<Self, ProtocolAdapterError> {
        if profile.capability.upstream_protocol != profile.connector.upstream_protocol
            || !profile.connector.critical_facts_are_exact()
        {
            return Err(
                crate::server::core_runtime::profiles::CapabilityError::ProfileUnknown.into(),
            );
        }
        let protocol = profile.capability.upstream_protocol;
        let owner = profile.exact_provider_path()?;
        let budget_tree = BudgetTree::new(STREAM_MEMORY_BUDGET, STREAM_MEMORY_BUDGET)
            .map_err(|error| ModelIrError::InvalidSse(error.to_string()))?;
        let budget = match request_budget {
            Some(budget) => budget,
            None => budget_tree
                .stream(STREAM_MEMORY_BUDGET)
                .map_err(|error| ModelIrError::InvalidSse(error.to_string()))?,
        };
        let framer = if streaming {
            Some(
                SseFramer::new(
                    SseLimits {
                        max_event_bytes: usize::MAX,
                        max_pending_bytes: usize::MAX,
                        max_output_event_bytes: usize::MAX,
                        expansion_ratio_numerator: 1,
                        expansion_ratio_denominator: 1,
                        expansion_slack_bytes: 0,
                        retained_capacity_threshold: RETAINED_SSE_CAPACITY,
                        eof_policy: EofPolicy::Strict,
                    },
                    budget.clone(),
                )
                .map_err(|error| ModelIrError::InvalidSse(error.to_string()))?,
            )
        } else {
            None
        };
        let mut state = ProtocolState::new(
            protocol,
            owner,
            profile.capability.native_provider_state,
            tool_id_projection,
            chat_tool_projection,
        );
        state.core_mut().retention = super::body_buffer::Retention::new(budget.clone());
        if let ProtocolState::Messages { state, .. } = &mut state {
            state.retention = super::body_buffer::Retention::new(budget.clone());
        }
        Ok(Self {
            protocol,
            status,
            streaming,
            body: super::body_buffer::BodyBuffer::default(),
            state,
            events: VecDeque::new(),
            budget,
            framer,
            ended: false,
            end_stream_pending: false,
            received_bytes: 0,
            frame_index: 0,
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
        self.received_bytes = self.received_bytes.saturating_add(bytes.len() as u64);
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
            &mut self.frame_index,
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

    /// Safe failure position; received bytes are an upper bound for a multi-event read.
    pub fn diagnostic_position(&self) -> (u64, u64) {
        (self.frame_index, self.received_bytes)
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
        self.body.append(bytes, &self.budget)?;
        if end_stream {
            self.state
                .decode_nonstream(self.status, self.body.bytes(), &mut self.events)?;
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
                .map_err(|_| ModelIrError::BufferLimit(bytes.len()))?;
        let outcome = run_sse(
            self.framer.as_mut().expect("streaming decoder has framer"),
            Some((charged, end_stream)),
            &self.budget,
            &mut self.state,
            &mut self.events,
            &mut self.frame_index,
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
    frame_index: &mut u64,
) -> Result<SseFeedOutcome, ProtocolAdapterError> {
    let mut visitor = NativeVisitor {
        budget,
        state,
        output,
        protocol_error: None,
        frame_index,
    };
    let mut sink = DiscardOutput;
    let framed = match input {
        Some((bytes, end_stream)) => framer.feed(bytes, end_stream, &mut visitor, &mut sink),
        None => framer.resume(&mut visitor, &mut sink),
    };
    if let Some(error) = visitor.protocol_error {
        return Err(error);
    }
    framed.map_err(|error| {
        match error {
            SseError::BudgetExceeded
            | SseError::EventLimit
            | SseError::PendingLimit
            | SseError::OutputLimit => ModelIrError::BufferLimit(usize::MAX),
            other => ModelIrError::InvalidSse(other.to_string()),
        }
        .into()
    })
}

struct NativeVisitor<'a> {
    budget: &'a StreamBudget,
    state: &'a mut ProtocolState,
    output: &'a mut VecDeque<ModelStreamEventV1>,
    protocol_error: Option<ProtocolAdapterError>,
    frame_index: &'a mut u64,
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
        *self.frame_index = self.frame_index.saturating_add(1);
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

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    use crate::server::core_runtime::profiles::fixed_reasoning;

    #[test]
    fn actual_sse_budget_rejection_is_distinct_from_malformed_sse() {
        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Messages,
            IngressProtocol::ChatCompletions,
            "physical",
            fixed_reasoning("fixed"),
        );
        for fragmented in [false, true] {
            let budget = BudgetTree::new(1024, 1024).unwrap().stream(1024).unwrap();
            let mut decoder =
                NativeResponseDecoder::new_for_attempt(&profile, 200, true, None, budget.clone())
                    .unwrap();
            let error = if fragmented {
                (0..16)
                    .find_map(|_| decoder.feed(&[b'x'; 256], false).err())
                    .expect("retained frame must exhaust the owner")
            } else {
                decoder.feed(&[b'x'; 2048], false).unwrap_err()
            };
            assert!(
                matches!(
                    error,
                    ProtocolAdapterError::ModelIr(ModelIrError::BufferLimit(_))
                ),
                "{error:?}"
            );
            drop(decoder);
            assert_eq!(budget.snapshot().unwrap().live, 0);
        }
        let mut decoder = NativeResponseDecoder::new(&profile, 200, true).unwrap();
        assert!(matches!(
            decoder.feed(b"data-invalid: {}\n\n", false),
            Err(ProtocolAdapterError::ModelIr(ModelIrError::InvalidSse(_)))
        ));
    }
}
