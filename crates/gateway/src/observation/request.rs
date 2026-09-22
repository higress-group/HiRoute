use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use hiroute_diagnostics::context::DiagnosticContext;
use hiroute_diagnostics::correlation::CorrelationDomain;
use hiroute_diagnostics::event::{
    AttemptBegin, AttemptOutcome, CommitState, DiagnosticEvent, Fallback, FallbackReason,
    IngressProtocol as DiagnosticIngressProtocol, ModelStage, ModelStageKind, RequestBegin,
    RequestEnd, RequestOutcome, RouteSelected, SemanticCommit as DiagnosticSemanticCommit,
};
use hiroute_diagnostics::identity::CorrelationToken;
use hiroute_diagnostics::runtime::DiagnosticsPort;
use hiroute_gateway_core::runtime::attempt::{CommitFence, Disposition};
use hiroute_gateway_core::runtime::driver::CompletedAttemptObservation;

use crate::agent_turn_history::{AgentTurnStatus, CompletedAgentTurn, ExecutionAttribution};
use crate::ports::{
    CasOutcome, ProbeLeaseOutcome, RuntimeHealth, RuntimeStateEntry, RuntimeStateKey,
};
use crate::server::core_runtime::adapters::ToolIdProjection;
use crate::server::core_runtime::classification::BoundAssessment;
use crate::server::core_runtime::profiles::CandidateProtocolProfile;

use super::ObservationChannels;
use super::crypto::WorkspaceHmac;
use super::otel::{OtelContentPolicy, emit_server_span};
use super::provider::CanonicalCaptureHandle;
use super::schema::{ExecutionFactV1, LifecycleFactV1};

#[path = "request/completion.rs"]
mod completion;
#[path = "request/emission.rs"]
mod emission;
#[path = "request/planner.rs"]
mod planner;
#[derive(Clone, Debug)]
pub struct RequestObservationMetadata {
    pub workspace_id: String,
    pub conversation_id: String,
    pub session_scope: String,
    pub correlation_provenance: String,
    pub turn_id: String,
    pub request_id: String,
    pub authority_id: String,
    pub authority_epoch: u64,
    pub publication_revision: u64,
    pub publication_digest: String,
    pub route: hiroute_domain::ModelRequestRouteV2,
    pub plan_display_name: Option<String>,
    pub served_model_id: String,
    pub grant_id: String,
    pub grant_generation: u64,
    pub ingress_protocol: String,
}

#[derive(Clone)]
pub struct RequestObservation {
    pub(super) inner: Arc<RequestObservationInner>,
}

pub(super) struct RequestObservationInner {
    pub(super) agent_turn_output: OnceLock<(
        Arc<crate::agent_turn_history::AgentTurnHistoryStore>,
        crate::agent_turn_history::AgentTurnTicket,
    )>,
    pub(super) prices: Arc<dyn super::RequestPriceSnapshot>,
    pub(super) enabled: bool,
    pub(super) capture_content: bool,
    pub(super) key: [u8; 32],
    pub(super) channels: ObservationChannels,
    pub(super) content_policy: OtelContentPolicy,
    pub(super) metadata: RequestObservationMetadata,
    pub(super) context: DiagnosticContext,
    pub(super) request_token: Option<CorrelationToken>,
    pub(super) started_at: Instant,
    pub(super) state: Mutex<RequestObservationState>,
}

#[derive(Default)]
pub(super) struct RequestObservationState {
    pub(super) agent_plan_id: Option<String>,
    pub(super) candidates: BTreeMap<String, CandidateObservation>,
    pub(super) pending_attempt: Option<AttemptObservation>,
    pub(super) current_attempt: Option<AttemptObservation>,
    pub(super) accepted_attempt: Option<AttemptObservation>,
    pub(super) next_attempt_ordinal: u32,
    pub(super) attempts_finished: u32,
    pub(super) response_frame_ordinal: u32,
    pub(super) request_content_ordinal: u32,
    pub(super) response_content_ordinal: u32,
    pub(super) request_content_started: bool,
    pub(super) request_content_terminal: bool,
    pub(super) response_content_started: bool,
    pub(super) response_content_terminal: bool,
    pub(super) request_transcript: Option<WorkspaceHmac>,
    pub(super) response_transcript: Option<WorkspaceHmac>,
    pub(super) request_content_stats: ContentCaptureStats,
    pub(super) response_content_stats: ContentCaptureStats,
    pub(super) request_result_transcript_root: Option<String>,
    pub(super) request_finished: bool,
    pub(super) previous_attempt_id: Option<String>,
    pub(super) next_attempt_reason: Option<String>,
    pub(super) published_disposition: Option<Disposition>,
    pub(super) accepted_wire_usage_recorded: bool,
    pub(super) accepted_attempt_finished: bool,
    pub(super) response_capture: Option<CanonicalCaptureHandle>,
    pub(super) tool_id_projection: Option<ToolIdProjection>,
    pub(super) response_part_ordinal: u32,
}

/// Aggregate counters for one content capture cycle. Only sizes and counts are kept, so a
/// burst of short reads produces one summary event instead of one event per read.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ContentCaptureStats {
    pub(super) bytes: u64,
    pub(super) chunks: u64,
    pub(super) read_calls: u64,
    pub(super) short_reads: u64,
    pub(super) max_chunk_bytes: u64,
}

impl ContentCaptureStats {
    pub(super) fn note_chunk(&mut self, len: usize) {
        self.bytes = self.bytes.saturating_add(len as u64);
        self.chunks = self.chunks.saturating_add(1);
        self.max_chunk_bytes = self.max_chunk_bytes.max(len as u64);
    }

    pub(super) fn note_reads(&mut self, read_calls: u64, short_reads: u64) {
        self.read_calls = self.read_calls.saturating_add(read_calls);
        self.short_reads = self.short_reads.saturating_add(short_reads);
    }
}

#[derive(Clone, Debug)]
pub(super) struct CandidateObservation {
    pub(super) candidate_id: String,
    pub(super) stable_binding_id: String,
    pub(super) declared_order: u32,
    pub(super) profile_digest: String,
    pub(super) provider_name: String,
    pub(super) request_model: String,
    pub(super) upstream_protocol: String,
    pub(super) model_configuration_id: String,
    pub(super) adapter_revision: String,
    pub(super) effective_cost_micros: Option<u64>,
    pub(super) cost_class: String,
    pub(super) protocol_profile: Option<Arc<CandidateProtocolProfile>>,
    pub(super) streaming: bool,
}

#[derive(Clone, Debug)]
pub(super) struct AttemptObservation {
    pub(super) ordinal: u32,
    pub(super) attempt_id: String,
    pub(super) stable_binding_id: String,
    pub(super) credential_ref: String,
    pub(super) key_id: String,
    pub(super) provider_name: String,
    pub(super) request_model: String,
    pub(super) upstream_protocol: String,
    pub(super) model_configuration_id: String,
    pub(super) adapter_revision: String,
    pub(super) candidate_id: String,
    pub(super) profile_digest: String,
    pub(super) start_reason: String,
    pub(super) previous_attempt_id: Option<String>,
    pub(super) started_at: Instant,
    pub(super) effective_cost_micros: Option<u64>,
    pub(super) cost_class: Option<String>,
    pub(super) credential_generation: u64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct RequestObservationCapture {
    pub(super) enabled: bool,
    pub(super) content: bool,
}

impl RequestObservation {
    pub(super) fn new(
        capture: RequestObservationCapture,
        key: [u8; 32],
        channels: ObservationChannels,
        content_policy: OtelContentPolicy,
        prices: Arc<dyn super::RequestPriceSnapshot>,
        metadata: RequestObservationMetadata,
        diagnostics: DiagnosticsPort,
    ) -> Self {
        let context = diagnostics.handle().context();
        let request_token = context.token(CorrelationDomain::ModelRequest, &metadata.request_id);
        let value = Self {
            inner: Arc::new(RequestObservationInner {
                agent_turn_output: OnceLock::new(),
                prices,
                enabled: capture.enabled,
                capture_content: capture.content,
                key,
                channels,
                content_policy,
                metadata,
                context,
                request_token,
                started_at: Instant::now(),
                state: Mutex::new(RequestObservationState {
                    next_attempt_ordinal: 1,
                    ..RequestObservationState::default()
                }),
            }),
        };
        if capture.enabled {
            value.emit_lifecycle(LifecycleFactV1::RequestAccepted {
                ingress_protocol: value.inner.metadata.ingress_protocol.clone(),
            });
        }
        value.emit_diagnostic(DiagnosticEvent::RequestBegin(RequestBegin {
            request_token,
        }));
        value
    }

    /// Emit one typed diagnostic event. A detached or degraded port records nothing and
    /// never changes the observation or business result.
    pub(super) fn emit_diagnostic(&self, event: DiagnosticEvent) {
        self.inner.context.emit(event);
    }

    pub(super) fn wire_diagnostic(&self, mut event: hiroute_diagnostics::event::UpstreamWire) {
        event.request_token = self.inner.context.token(
            CorrelationDomain::ModelRequest,
            &self.inner.metadata.request_id,
        );
        self.emit_diagnostic(DiagnosticEvent::UpstreamWire(event));
    }

    fn attempt_token(&self, attempt_id: &str) -> Option<CorrelationToken> {
        self.inner
            .context
            .token(CorrelationDomain::Attempt, attempt_id)
    }

    fn ingress_protocol(&self) -> DiagnosticIngressProtocol {
        match self.inner.metadata.ingress_protocol.as_str() {
            "chat_completions" => DiagnosticIngressProtocol::OpenAiChat,
            "messages" => DiagnosticIngressProtocol::AnthropicMessages,
            "responses" => DiagnosticIngressProtocol::OpenAiResponses,
            _ => DiagnosticIngressProtocol::Unknown,
        }
    }

    fn emit_model_stage(&self, stage: ModelStageKind, elapsed: Duration) {
        self.emit_diagnostic(DiagnosticEvent::ModelStage(ModelStage {
            stage,
            elapsed_ms: duration_millis(elapsed),
            protocol: Some(self.ingress_protocol()),
        }));
    }

    /// Request-owned pipeline timings measured at the real boundaries before the request
    /// token existed. They are attributed to this request once it starts.
    pub fn record_pipeline_stages(
        &self,
        parse: Duration,
        externalize: Duration,
        prevalidate: Duration,
    ) {
        self.emit_model_stage(ModelStageKind::Parse, parse);
        self.emit_model_stage(ModelStageKind::Externalize, externalize);
        self.emit_model_stage(ModelStageKind::Prevalidate, prevalidate);
    }

    pub fn record_plan_stage(&self, elapsed: Duration) {
        self.emit_model_stage(ModelStageKind::Plan, elapsed);
    }

    /// Route the plan's declared candidate ordinal selected by the real decision session.
    /// A binding outside this request's plan records nothing instead of guessing.
    pub(super) fn selected_candidate(&self, stable_binding_id: &str) {
        let ordinal = {
            let state = self.lock_state();
            state
                .candidates
                .get(stable_binding_id)
                .map(|candidate| u64::from(candidate.declared_order))
        };
        let Some(candidate_ordinal) = ordinal else {
            return;
        };
        self.emit_diagnostic(DiagnosticEvent::RouteSelected(RouteSelected {
            request_token: self.inner.request_token,
            candidate_ordinal,
        }));
    }

    pub fn metadata(&self) -> &RequestObservationMetadata {
        &self.inner.metadata
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.enabled || self.inner.agent_turn_output.get().is_some()
    }

    pub(crate) fn bind_agent_turn_output(
        &self,
        store: Arc<crate::agent_turn_history::AgentTurnHistoryStore>,
        ticket: crate::agent_turn_history::AgentTurnTicket,
    ) {
        let _ = self.inner.agent_turn_output.set((store, ticket));
    }

    pub(super) fn captures_content(&self) -> bool {
        self.inner.enabled && self.inner.capture_content
    }

    /// Installs the projection-only half of the execution-owned Tool
    /// continuation context. A second binding never replaces the first
    /// request scope, and observation remains unable to mutate authority.
    pub(in crate::server::core_runtime) fn bind_tool_id_projection(
        &self,
        projection: ToolIdProjection,
    ) {
        let mut state = self.lock_state();
        if state.tool_id_projection.is_none() {
            state.tool_id_projection = Some(projection);
        }
    }

    pub(super) fn credential_lease(
        &self,
        stable_binding_id: &str,
        credential_ref: &str,
        excluded_key_count: usize,
        lease: Option<(&str, u64)>,
        outcome: &str,
    ) {
        if !self.is_enabled() {
            return;
        }
        self.emit_execution(
            ExecutionFactV1::CredentialLease {
                stable_binding_id: stable_binding_id.into(),
                credential_ref: credential_ref.into(),
                key_id: lease.map(|(key, _)| key.into()),
                credential_generation: lease.map(|(_, generation)| generation),
                excluded_key_count,
                outcome: outcome.into(),
            },
            None,
        );
    }

    /// Stages an explicitly unauthenticated candidate after Product
    /// materialization has verified the sealed `AuthenticationSemantics::None`
    /// profile. The frozen execution fact still requires a nonempty opaque key
    /// identity, so the reserved no-credential reference is reused as that
    /// observation-only identity; generation zero records that no credential
    /// authority or credential runtime state participated.
    pub(super) fn no_credential_materialized(&self, stable_binding_id: &str, credential_ref: &str) {
        if !self.is_enabled() || !credential_ref.starts_with("credential/none/") {
            return;
        }
        self.start_attempt(stable_binding_id, credential_ref, credential_ref, 0);
    }

    pub(super) fn runtime_state_read(
        &self,
        key: &RuntimeStateKey,
        result: Option<&RuntimeStateEntry>,
        outcome: &str,
    ) {
        if !self.is_enabled() {
            return;
        }
        let fields = state_key_fields(key);
        self.emit_execution(
            ExecutionFactV1::RuntimeState {
                operation: "read_exact".into(),
                key_scope: fields.scope.into(),
                stable_binding_id: fields.stable_binding_id.into(),
                credential_ref: fields.credential_ref.map(Into::into),
                key_id: fields.key_id.map(Into::into),
                expected_generation: None,
                observed_generation: result.map(|entry| entry.generation),
                health: result.map(|entry| health(&entry.health).into()),
                cooldown_remaining_millis: result
                    .and_then(|entry| cooldown_remaining_millis(&entry.health)),
                probe_lease_remaining_millis: result
                    .and_then(|entry| entry.probe_lease_until)
                    .map(remaining_millis),
                transient_backoff_step: result.map(|entry| entry.transient_backoff_step),
                outcome: outcome.into(),
            },
            None,
        );
        if matches!(
            result.map(|entry| &entry.health),
            Some(RuntimeHealth::Active)
        ) && let RuntimeStateKey::Credential {
            stable_binding_id,
            credential_ref,
            key_id,
            credential_generation,
        } = key
        {
            self.start_attempt(
                stable_binding_id,
                credential_ref,
                key_id,
                *credential_generation,
            );
        }
    }

    pub(super) fn runtime_state_cas(
        &self,
        key: &RuntimeStateKey,
        expected_generation: u64,
        next: &RuntimeStateEntry,
        result: Option<CasOutcome>,
        outcome: &str,
    ) {
        if !self.is_enabled() {
            return;
        }
        let fields = state_key_fields(key);
        let observed = result.and_then(|result| match result {
            CasOutcome::Applied { generation } => Some(generation),
            CasOutcome::Conflict => None,
        });
        self.emit_execution(
            ExecutionFactV1::RuntimeState {
                operation: "compare_and_swap_exact".into(),
                key_scope: fields.scope.into(),
                stable_binding_id: fields.stable_binding_id.into(),
                credential_ref: fields.credential_ref.map(Into::into),
                key_id: fields.key_id.map(Into::into),
                expected_generation: Some(expected_generation),
                observed_generation: observed,
                health: Some(health(&next.health).into()),
                cooldown_remaining_millis: cooldown_remaining_millis(&next.health),
                probe_lease_remaining_millis: next.probe_lease_until.map(remaining_millis),
                transient_backoff_step: Some(next.transient_backoff_step),
                outcome: outcome.into(),
            },
            None,
        );
    }

    pub(super) fn runtime_probe(
        &self,
        key: &RuntimeStateKey,
        expected_generation: u64,
        lease_duration: Duration,
        result: Option<ProbeLeaseOutcome>,
        outcome: &str,
    ) {
        if !self.is_enabled() {
            return;
        }
        let fields = state_key_fields(key);
        let observed = result.and_then(|result| match result {
            ProbeLeaseOutcome::Acquired { generation } => Some(generation),
            ProbeLeaseOutcome::Busy | ProbeLeaseOutcome::Conflict => None,
        });
        self.emit_execution(
            ExecutionFactV1::RuntimeState {
                operation: "acquire_probe_lease_exact".into(),
                key_scope: fields.scope.into(),
                stable_binding_id: fields.stable_binding_id.into(),
                credential_ref: fields.credential_ref.map(Into::into),
                key_id: fields.key_id.map(Into::into),
                expected_generation: Some(expected_generation),
                observed_generation: observed,
                health: None,
                cooldown_remaining_millis: None,
                probe_lease_remaining_millis: matches!(
                    result,
                    Some(ProbeLeaseOutcome::Acquired { .. })
                )
                .then(|| duration_millis(lease_duration)),
                transient_backoff_step: None,
                outcome: outcome.into(),
            },
            None,
        );
        if matches!(result, Some(ProbeLeaseOutcome::Acquired { .. }))
            && let RuntimeStateKey::Credential {
                stable_binding_id,
                credential_ref,
                key_id,
                credential_generation,
            } = key
        {
            self.start_attempt(
                stable_binding_id,
                credential_ref,
                key_id,
                *credential_generation,
            );
        }
    }

    pub(super) fn accept_current(
        &self,
        frame_id: &str,
        byte_count: usize,
    ) -> Option<AttemptObservation> {
        if !self.is_enabled() {
            return None;
        }
        let attempt = {
            let mut state = self.lock_state();
            if state.published_disposition != Some(Disposition::Accept) {
                return None;
            }
            if let Some(accepted) = &state.accepted_attempt {
                Some(accepted.clone())
            } else {
                let attempt = state.current_attempt.take()?;
                state.accepted_attempt = Some(attempt.clone());
                Some(attempt)
            }
        }?;
        let first_frame = {
            let mut state = self.lock_state();
            let first = state.response_frame_ordinal == 0;
            state.response_frame_ordinal = state.response_frame_ordinal.saturating_add(1);
            first
        };
        if first_frame {
            self.emit_execution(
                ExecutionFactV1::SemanticCommit {
                    ordinal: attempt.ordinal,
                    boundary: "full_frame_transport_accepted".into(),
                    frame_id: frame_id.into(),
                },
                Some(&attempt.attempt_id),
            );
            self.emit_diagnostic(DiagnosticEvent::SemanticCommit(DiagnosticSemanticCommit {
                attempt_token: self.attempt_token(&attempt.attempt_id),
                state: CommitState::SemanticCommitted,
            }));
        }
        self.emit_lifecycle(LifecycleFactV1::ResponseFrameAccepted {
            frame_id: frame_id.into(),
            byte_count: byte_count as u64,
            downstream_delivery: "full_frame_transport_accepted".into(),
        });
        Some(attempt)
    }

    pub fn finish(&self, outcome: &str) {
        if !self.is_enabled() {
            return;
        }
        let terminal_attempt = {
            let mut state = self.lock_state();
            if state.request_finished {
                return;
            }
            state.request_finished = true;
            state.pending_attempt.take();
            state.current_attempt.take()
        };
        if let Some(attempt) = terminal_attempt {
            self.finish_attempt(attempt, "rejected", Some("request_terminal"), Some(false));
        }
        let (started, finished, accepted) = {
            let state = self.lock_state();
            (
                state.next_attempt_ordinal.saturating_sub(1),
                state.attempts_finished,
                state
                    .accepted_attempt
                    .as_ref()
                    .map(|attempt| attempt.ordinal),
            )
        };
        self.emit_execution(
            ExecutionFactV1::RequestFinished {
                outcome: outcome.into(),
                attempts_started: started,
                attempts_finished: finished,
                accepted_attempt_ordinal: accepted,
                // A delivered terminal record closes the request's ordered
                // fact sequence. Any producer or sink loss is represented by
                // the independent gap/loss channel and downgrades the stored
                // receipt rather than making every healthy request unknown.
                facts_completeness: "complete".into(),
            },
            None,
        );
        self.emit_lifecycle(LifecycleFactV1::RequestFinished {
            outcome: outcome.into(),
        });
        if let Some(outcome) = request_outcome(outcome) {
            self.emit_diagnostic(DiagnosticEvent::RequestEnd(RequestEnd {
                request_token: self.inner.request_token,
                outcome,
                elapsed_ms: duration_millis(self.inner.started_at.elapsed()),
            }));
        }
        emit_server_span(self, outcome, None);
    }

    fn start_attempt(
        &self,
        stable_binding_id: &str,
        credential_ref: &str,
        key_id: &str,
        generation: u64,
    ) {
        let mut state = self.lock_state();
        if state.current_attempt.is_some()
            || state.accepted_attempt.is_some()
            || state.pending_attempt.as_ref().is_some_and(|pending| {
                pending.stable_binding_id == stable_binding_id && pending.key_id == key_id
            })
        {
            return;
        }
        state.published_disposition = None;
        state.accepted_wire_usage_recorded = false;
        let candidate =
            state
                .candidates
                .get(stable_binding_id)
                .cloned()
                .unwrap_or(CandidateObservation {
                    candidate_id: "unknown".into(),
                    stable_binding_id: stable_binding_id.into(),
                    declared_order: 0,
                    profile_digest: "unknown".into(),
                    provider_name: "unknown".into(),
                    request_model: "unknown".into(),
                    upstream_protocol: "unknown".into(),
                    model_configuration_id: "unknown".into(),
                    adapter_revision: "unknown".into(),
                    effective_cost_micros: None,
                    cost_class: "unknown".into(),
                    protocol_profile: None,
                    streaming: false,
                });
        let attempt = AttemptObservation {
            ordinal: 0,
            attempt_id: String::new(),
            stable_binding_id: candidate.stable_binding_id,
            credential_ref: credential_ref.into(),
            key_id: key_id.into(),
            provider_name: candidate.provider_name,
            request_model: candidate.request_model,
            upstream_protocol: candidate.upstream_protocol,
            model_configuration_id: candidate.model_configuration_id,
            adapter_revision: candidate.adapter_revision,
            candidate_id: candidate.candidate_id,
            profile_digest: candidate.profile_digest,
            start_reason: state
                .next_attempt_reason
                .take()
                .unwrap_or_else(|| "initial_candidate".into()),
            previous_attempt_id: state.previous_attempt_id.take(),
            started_at: Instant::now(),
            effective_cost_micros: candidate.effective_cost_micros,
            cost_class: Some(candidate.cost_class),
            credential_generation: generation,
        };
        state.pending_attempt = Some(attempt);
    }

    pub(super) fn emit_attempt_started(&self, attempt: &AttemptObservation) {
        self.emit_execution(
            ExecutionFactV1::AttemptStarted {
                ordinal: attempt.ordinal,
                candidate_id: attempt.candidate_id.clone(),
                stable_binding_id: attempt.stable_binding_id.clone(),
                profile_digest: attempt.profile_digest.clone(),
                credential_ref: attempt.credential_ref.clone(),
                key_id: attempt.key_id.clone(),
                provider_name: attempt.provider_name.clone(),
                request_model: attempt.request_model.clone(),
                upstream_protocol: attempt.upstream_protocol.clone(),
                model_configuration_id: attempt.model_configuration_id.clone(),
                adapter_revision: attempt.adapter_revision.clone(),
                start_reason: attempt.start_reason.clone(),
                previous_attempt_id: attempt.previous_attempt_id.clone(),
            },
            Some(&attempt.attempt_id),
        );
        self.emit_lifecycle(LifecycleFactV1::AttemptStarted {
            ordinal: attempt.ordinal,
        });
        self.emit_diagnostic(DiagnosticEvent::AttemptBegin(AttemptBegin {
            request_token: self.inner.request_token,
            attempt_token: self.attempt_token(&attempt.attempt_id),
            attempt_index: u64::from(attempt.ordinal),
        }));
        if attempt.previous_attempt_id.is_some() {
            // Product attempts of one request are ordinal-sequential, so the attempt that
            // this fallback decision moved away from is the one before the current ordinal.
            self.emit_diagnostic(DiagnosticEvent::Fallback(Fallback {
                request_token: self.inner.request_token,
                from_attempt_index: u64::from(attempt.ordinal.saturating_sub(1)),
                reason: fallback_reason(&attempt.start_reason),
            }));
        }
    }

    /// Count the reads performed for one content direction. A read that returned fewer
    /// bytes than requested is a short read; a burst of them is summarized once per cycle.
    pub(super) fn note_content_reads(&self, direction: &str, read_calls: u64, short_reads: u64) {
        let mut state = self.lock_state();
        let stats = if direction == "request_input" {
            &mut state.request_content_stats
        } else {
            &mut state.response_content_stats
        };
        stats.note_reads(read_calls, short_reads);
    }

    pub(super) fn take_content_stats(&self, direction: &str) -> ContentCaptureStats {
        let mut state = self.lock_state();
        if direction == "request_input" {
            std::mem::take(&mut state.request_content_stats)
        } else {
            std::mem::take(&mut state.response_content_stats)
        }
    }

    pub(super) fn lock_state(&self) -> std::sync::MutexGuard<'_, RequestObservationState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn has_accepted_attempt(&self) -> bool {
        self.lock_state().accepted_attempt.is_some()
    }

    pub fn accepted_attempt_identity(&self) -> Option<(String, String, String)> {
        self.lock_state().accepted_attempt.as_ref().map(|attempt| {
            (
                attempt.candidate_id.clone(),
                attempt.model_configuration_id.clone(),
                attempt.profile_digest.clone(),
            )
        })
    }

    pub(crate) fn agent_turn_finished(&self, turn: &CompletedAgentTurn) {
        let (executed_branch_id, model_configuration_id, profile_digest, attribution) =
            match &turn.attribution {
                ExecutionAttribution::Single {
                    executed_branch_id,
                    model_configuration_id,
                    profile_digest,
                    ..
                } => (
                    Some(executed_branch_id.clone()),
                    Some(model_configuration_id.clone()),
                    Some(profile_digest.clone()),
                    "single",
                ),
                ExecutionAttribution::Mixed => (None, None, None, "mixed"),
                ExecutionAttribution::Unknown => (None, None, None, "unknown"),
            };
        self.emit_execution(
            ExecutionFactV1::AgentTurnFinished {
                agent_turn_id: turn.agent_turn_id.clone(),
                segment_id: turn.segment_id.clone(),
                ordinal: turn.ordinal,
                plan_id: turn.plan.plan_id.clone(),
                plan_revision: turn.plan.plan_revision,
                selected_branch_id: turn.selected_branch_id.clone(),
                executed_branch_id,
                model_configuration_id,
                profile_digest,
                attribution: attribution.into(),
                started_at_ms: turn.started_at_ms,
                finished_at_ms: turn.finished_at_ms,
                status: match turn.status {
                    AgentTurnStatus::Completed => "completed",
                    AgentTurnStatus::Failed => "failed",
                    AgentTurnStatus::Interrupted => "interrupted",
                    AgentTurnStatus::Unknown => "unknown",
                }
                .into(),
                history_partial: turn.history_partial,
                first_request_id: turn.first_request_id.clone(),
                last_request_id: turn.last_request_id.clone(),
            },
            None,
        );
    }

    pub(crate) fn branch_assessment_recorded(&self, assessment: &BoundAssessment) {
        let ExecutionAttribution::Single {
            model_configuration_id,
            profile_digest,
            ..
        } = &assessment.target.attribution
        else {
            return;
        };
        self.emit_execution(
            ExecutionFactV1::BranchAssessmentRecorded {
                segment_id: assessment.target.segment_id.clone(),
                plan_id: assessment.target.plan.plan_id.clone(),
                plan_revision: assessment.target.plan.plan_revision,
                model_configuration_id: model_configuration_id.clone(),
                profile_digest: profile_digest.clone(),
                trigger_request_id: self.inner.metadata.request_id.clone(),
                target_from_turn_id: assessment.target.first_turn_id.clone(),
                target_through_turn_id: assessment.target.through_turn_id.clone(),
                target_from_ordinal: assessment.target.first_ordinal,
                target_through_ordinal: assessment.target.through_ordinal,
                assessed_at_ms: super::unix_nanos() / 1_000_000,
                score: assessment.score,
                partial: assessment.partial,
                reason: assessment.reason.clone(),
            },
            None,
        );
    }
}

struct StateKeyFields<'a> {
    scope: &'static str,
    stable_binding_id: &'a str,
    credential_ref: Option<&'a str>,
    key_id: Option<&'a str>,
}

fn state_key_fields(key: &RuntimeStateKey) -> StateKeyFields<'_> {
    match key {
        RuntimeStateKey::Binding { stable_binding_id } => StateKeyFields {
            scope: "binding",
            stable_binding_id,
            credential_ref: None,
            key_id: None,
        },
        RuntimeStateKey::Credential {
            stable_binding_id,
            credential_ref,
            key_id,
            ..
        } => StateKeyFields {
            scope: "credential",
            stable_binding_id,
            credential_ref: Some(credential_ref),
            key_id: Some(key_id),
        },
    }
}

fn health(health: &RuntimeHealth) -> &'static str {
    match health {
        RuntimeHealth::Active => "active",
        RuntimeHealth::Disabled => "disabled",
        RuntimeHealth::CoolingDown { .. } => "cooling_down",
    }
}

fn cooldown_remaining_millis(health: &RuntimeHealth) -> Option<u64> {
    match health {
        RuntimeHealth::CoolingDown { until } => Some(remaining_millis(*until)),
        RuntimeHealth::Active | RuntimeHealth::Disabled => None,
    }
}

fn remaining_millis(until: Instant) -> u64 {
    duration_millis(until.saturating_duration_since(Instant::now()))
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn duration_micros(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

fn request_outcome(outcome: &str) -> Option<RequestOutcome> {
    match outcome {
        "accepted" => Some(RequestOutcome::Completed),
        "failed" | "gateway_error" | "postcommit_transport_failed" => Some(RequestOutcome::Failed),
        _ => None,
    }
}

fn attempt_outcome(outcome: &str, timed_out: bool) -> Option<AttemptOutcome> {
    match outcome {
        "accepted" => Some(AttemptOutcome::Completed),
        "postcommit_cancelled" => Some(AttemptOutcome::Cancelled),
        "rejected" | "postcommit_transport_failed" | "failed_before_transport_acceptance" => {
            Some(if timed_out {
                AttemptOutcome::Timeout
            } else {
                AttemptOutcome::Failed
            })
        }
        _ => None,
    }
}

/// Project the real commit fences onto the diagnostic commit state. An attempt without
/// observed fences reports `unknown` instead of claiming it never reached the upstream.
fn commit_state(observation: Option<&CompletedAttemptObservation>) -> CommitState {
    let Some(commits) = observation.map(|value| &value.commits) else {
        return CommitState::Unknown;
    };
    match (
        commits.upstream_request,
        commits.downstream_headers,
        commits.downstream_semantic,
    ) {
        (_, _, CommitFence::WriteConfirmed) => CommitState::SemanticCommitted,
        (CommitFence::WriteConfirmed, _, _) | (_, CommitFence::WriteConfirmed, _) => {
            CommitState::TransportCommitted
        }
        (CommitFence::WriteStartedMayHaveCommitted, _, _)
        | (_, CommitFence::WriteStartedMayHaveCommitted, _)
        | (_, _, CommitFence::WriteStartedMayHaveCommitted) => CommitState::Unknown,
        (CommitFence::Clear, CommitFence::Clear, CommitFence::Clear) => {
            CommitState::BeforeTransport
        }
    }
}

/// The attempt start reason is `initial_candidate` or the product's
/// `precommit_fallback_after_<stable error class>`. Only the stable error class selects a
/// diagnostic reason; anything else stays `other_stable_reason`.
fn fallback_reason(start_reason: &str) -> FallbackReason {
    let Some(class) = start_reason.strip_prefix("precommit_fallback_after_") else {
        return FallbackReason::OtherStableReason;
    };
    match class {
        "rate_limited" | "rate_limit_exceeded" | "too_many_requests" => FallbackReason::RateLimited,
        "connector_unavailable" | "connector_not_ready" => FallbackReason::ConnectorUnavailable,
        "upstream_unavailable" | "upstream_error" | "provider_error" | "upstream_rejected" => {
            FallbackReason::UpstreamFailure
        }
        "candidate_excluded" => FallbackReason::CandidateExcluded,
        _ => FallbackReason::OtherStableReason,
    }
}
