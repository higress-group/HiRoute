use hiroute_gateway_core::runtime::attempt::{
    AttemptGeneration, AttemptId, AttemptTimeoutKind, CommitFence, Disposition,
    PublishedDisposition,
};
use hiroute_gateway_core::runtime::driver::{
    AttemptCleanupOutcome, AttemptDownstreamOutcome, AttemptFailureFacts, AttemptStreamOutcome,
    AttemptTerminationReason, CompletedAttemptObservation, RetryabilityFact, UsageDimension,
    UsageFact, UsageProvenance,
};

use hiroute_diagnostics::event::{
    AttemptEnd, AttemptWireCommits, DiagnosticEvent, ModelStageKind, RequestCancel, RequestTimeout,
    WireCommitState, WireHttpProtocol,
};

use super::{
    AttemptObservation, RequestObservation, attempt_outcome, commit_state, duration_micros,
    duration_millis,
};
use crate::server::core_runtime::model_ir::ModelUsage;
use crate::server::core_runtime::observation::crypto::stable_id;
use crate::server::core_runtime::observation::otel::{OtelAttempt, emit_attempt_span};
use crate::server::core_runtime::observation::schema::{
    AttemptCommitFactV1, AttemptFinishedFactV1, AttemptTransportFactV1, ExecutionFactV1,
    LifecycleFactV1,
};

impl RequestObservation {
    pub(in crate::server::core_runtime::observation) fn usage_model(&self, usage: &ModelUsage) {
        if !self.is_enabled() {
            return;
        }
        let Some(attempt) = self.lock_state().accepted_attempt.clone() else {
            return;
        };
        let cache_status =
            if usage.cache_read_tokens.is_some() || usage.cache_write_tokens.is_some() {
                "confirmed_usage"
            } else {
                "unknown"
            };
        self.lock_state().accepted_wire_usage_recorded = true;
        self.emit_execution(
            ExecutionFactV1::UsageAndCache {
                ordinal: attempt.ordinal,
                source: "accepted_canonical_model_event".into(),
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                billable_tokens: None,
                cache_read_tokens: usage.cache_read_tokens,
                cache_write_tokens: usage.cache_write_tokens,
                reasoning_tokens: usage.reasoning_tokens,
                input_provenance: optional_provenance(usage.input_tokens),
                output_provenance: optional_provenance(usage.output_tokens),
                billable_provenance: "unknown".into(),
                cache_read_provenance: optional_provenance(usage.cache_read_tokens),
                cache_write_provenance: optional_provenance(usage.cache_write_tokens),
                reasoning_provenance: optional_provenance(usage.reasoning_tokens),
                effective_cost_micros: attempt.effective_cost_micros,
                cost_class: attempt.cost_class.clone(),
                cache_status: cache_status.into(),
            },
            Some(&attempt.attempt_id),
        );
    }

    pub(in crate::server::core_runtime::observation) fn disposition_published(
        &self,
        disposition: &PublishedDisposition,
    ) {
        if !self.tracks_attempts() {
            return;
        }
        {
            let mut state = self.lock_state();
            state.published_disposition = Some(disposition.disposition);
        }
        let promoted =
            self.promote_pending_attempt(disposition.attempt_id, disposition.generation, None);
        if let Some(attempt) = promoted {
            self.emit_attempt_started(&attempt);
        }
    }

    pub(in crate::server::core_runtime::observation) fn provisional_candidate_decided(
        &self,
        failure: &AttemptFailureFacts,
        disposition: Disposition,
    ) {
        if !self.tracks_attempts() {
            return;
        }
        let error_class = failure
            .provider
            .as_ref()
            .and_then(|facts| facts.error_class.as_ref())
            .map(|class| class.as_str())
            .unwrap_or_else(|| failure.termination_reason.as_str());
        let mut state = self.lock_state();
        let pending = state.pending_attempt.take();
        if disposition == Disposition::Continue {
            state.previous_attempt_id = pending.and_then(|attempt| attempt.previous_attempt_id);
            state.next_attempt_reason = Some(format!("precommit_fallback_after_{error_class}"));
        }
    }

    pub(in crate::server::core_runtime::observation) fn completed_attempt(
        &self,
        observation: &CompletedAttemptObservation,
        stable_binding_id: Option<&str>,
    ) {
        if !self.tracks_attempts() {
            return;
        }
        let promoted = stable_binding_id.and_then(|stable_binding_id| {
            self.promote_pending_attempt(
                observation.attempt_id,
                observation.generation,
                Some((stable_binding_id, observation.credential_ref.as_str())),
            )
        });
        if let Some(attempt) = promoted {
            self.emit_attempt_started(&attempt);
        }
        let provider = observation.provider.as_ref().or_else(|| {
            observation
                .failure
                .as_ref()
                .and_then(|failure| failure.provider.as_ref())
        });
        let error_class = provider
            .and_then(|facts| facts.error_class.as_ref())
            .map(|class| class.as_str())
            .or_else(|| {
                observation
                    .failure
                    .as_ref()
                    .map(|failure| failure.termination_reason.as_str())
            })
            .unwrap_or_else(|| termination_reason(observation.termination_reason));
        let retryable = provider.and_then(|facts| match facts.retryability {
            RetryabilityFact::Retryable => Some(true),
            RetryabilityFact::NonRetryable => Some(false),
            RetryabilityFact::Unknown => None,
        });
        let authoritative_duration = observation
            .ended_at
            .saturating_duration_since(observation.budget.issued_at);

        let (attempt, accepted, wire_usage_recorded) = {
            let state = self.lock_state();
            (
                state
                    .current_attempt
                    .clone()
                    .or_else(|| state.accepted_attempt.clone()),
                state.accepted_attempt.is_some(),
                state.accepted_wire_usage_recorded,
            )
        };
        if let (Some(attempt), Some(usage)) = (
            attempt.as_ref(),
            provider.and_then(|facts| facts.usage.as_ref()),
        ) && !(accepted && wire_usage_recorded)
        {
            self.usage_fact(attempt, usage);
        }

        let model_failed = provider
            .and_then(|facts| facts.model_event.as_ref())
            .is_some_and(|event| {
                matches!(
                    event.as_str(),
                    "response_failed" | "response_incomplete" | "response_unknown"
                )
            });
        if observation.disposition == Disposition::Accept && accepted {
            let attempt = {
                let mut state = self.lock_state();
                state.published_disposition = None;
                if state.accepted_attempt_finished {
                    None
                } else {
                    state.accepted_attempt_finished = true;
                    state.accepted_attempt_failed = model_failed;
                    state.accepted_attempt_cancelled =
                        observation.downstream == AttemptDownstreamOutcome::Cancelled;
                    state.accepted_attempt.clone()
                }
            };
            if let Some(attempt) = attempt {
                let (outcome, error_class, retryable) = match observation.downstream {
                    AttemptDownstreamOutcome::Completed if model_failed => {
                        ("rejected", Some("provider_rejected"), Some(false))
                    }
                    AttemptDownstreamOutcome::Completed => ("accepted", None, None),
                    AttemptDownstreamOutcome::Failed => (
                        "postcommit_transport_failed",
                        Some(error_class),
                        Some(false),
                    ),
                    AttemptDownstreamOutcome::Cancelled => {
                        ("postcommit_cancelled", Some(error_class), Some(false))
                    }
                    AttemptDownstreamOutcome::NotStarted => (
                        "failed_before_transport_acceptance",
                        Some(error_class),
                        retryable,
                    ),
                };
                self.finish_attempt_with_duration(
                    attempt,
                    outcome,
                    error_class,
                    retryable,
                    authoritative_duration,
                    Some(observation),
                );
            }
            return;
        }
        let attempt = {
            let mut state = self.lock_state();
            state.published_disposition = None;
            let attempt = state.current_attempt.take();
            if observation.disposition == Disposition::Continue
                && let Some(attempt) = &attempt
            {
                state.previous_attempt_id = Some(attempt.attempt_id.clone());
                state.next_attempt_reason = Some(format!("precommit_fallback_after_{error_class}"));
            }
            attempt
        };
        if let Some(attempt) = attempt {
            let outcome = match observation.disposition {
                Disposition::Continue | Disposition::Terminate => "rejected",
                Disposition::Accept
                    if matches!(
                        observation.downstream,
                        AttemptDownstreamOutcome::Failed | AttemptDownstreamOutcome::Cancelled
                    ) =>
                {
                    "postcommit_transport_failed"
                }
                Disposition::Accept => "failed_before_transport_acceptance",
            };
            self.finish_attempt_with_duration(
                attempt,
                outcome,
                Some(error_class),
                retryable,
                authoritative_duration,
                Some(observation),
            );
        }
    }

    fn promote_pending_attempt(
        &self,
        attempt_id: AttemptId,
        generation: AttemptGeneration,
        authority: Option<(&str, &str)>,
    ) -> Option<AttemptObservation> {
        let mut state = self.lock_state();
        if attempt_id.0 == 0 || state.current_attempt.is_some() || state.accepted_attempt.is_some()
        {
            return None;
        }
        let pending = state.pending_attempt.as_ref()?;
        if authority.is_some_and(|(stable_binding_id, credential_ref)| {
            pending.stable_binding_id != stable_binding_id
                || pending.credential_ref != credential_ref
        }) {
            return None;
        }
        let mut attempt = state
            .pending_attempt
            .take()
            .expect("pending Attempt was validated before promotion");
        let ordinal = state.next_attempt_ordinal;
        state.next_attempt_ordinal = state.next_attempt_ordinal.saturating_add(1);
        attempt.ordinal = ordinal;
        attempt.attempt_id = stable_id(
            "attempt",
            &self.inner.key,
            b"core-attempt-id",
            &[
                self.inner.metadata.request_id.as_bytes(),
                &ordinal.to_be_bytes(),
                &attempt_id.0.to_be_bytes(),
                &generation.0.to_be_bytes(),
                attempt.stable_binding_id.as_bytes(),
                attempt.key_id.as_bytes(),
                &attempt.credential_generation.to_be_bytes(),
            ],
        );
        state.current_attempt = Some(attempt.clone());
        Some(attempt)
    }

    fn usage_fact(&self, attempt: &AttemptObservation, usage: &UsageFact) {
        self.emit_execution(
            ExecutionFactV1::UsageAndCache {
                ordinal: attempt.ordinal,
                source: "provider_completion".into(),
                input_tokens: usage.input.units,
                output_tokens: usage.output.units,
                billable_tokens: usage.billable.units,
                cache_read_tokens: usage.cache_read.units,
                cache_write_tokens: usage.cache_write.units,
                reasoning_tokens: usage.reasoning.units,
                input_provenance: usage_provenance(usage.input),
                output_provenance: usage_provenance(usage.output),
                billable_provenance: usage_provenance(usage.billable),
                cache_read_provenance: usage_provenance(usage.cache_read),
                cache_write_provenance: usage_provenance(usage.cache_write),
                reasoning_provenance: usage_provenance(usage.reasoning),
                effective_cost_micros: attempt.effective_cost_micros,
                cost_class: attempt.cost_class.clone(),
                cache_status: if usage.cache_read.units.is_some()
                    || usage.cache_write.units.is_some()
                {
                    "confirmed_usage".into()
                } else {
                    "unknown".into()
                },
            },
            Some(&attempt.attempt_id),
        );
    }

    pub(super) fn finish_attempt(
        &self,
        attempt: AttemptObservation,
        outcome: &str,
        error_class: Option<&str>,
        retryable: Option<bool>,
    ) {
        let duration = attempt.started_at.elapsed();
        self.finish_attempt_with_duration(attempt, outcome, error_class, retryable, duration, None);
    }

    fn finish_attempt_with_duration(
        &self,
        attempt: AttemptObservation,
        outcome: &str,
        error_class: Option<&str>,
        retryable: Option<bool>,
        duration: std::time::Duration,
        observation: Option<&CompletedAttemptObservation>,
    ) {
        {
            let mut state = self.lock_state();
            state.attempts_finished = state.attempts_finished.saturating_add(1);
        }
        let provider = observation.and_then(provider_facts);
        let transport = observation.map(|value| &value.transport);
        self.emit_execution(
            ExecutionFactV1::AttemptFinished(Box::new(AttemptFinishedFactV1 {
                ordinal: attempt.ordinal,
                stable_binding_id: attempt.stable_binding_id.clone(),
                outcome: outcome.into(),
                error_class: error_class.map(Into::into),
                retryable,
                duration_micros: duration_micros(duration),
                disposition: observation
                    .map(|value| disposition(value.disposition))
                    .unwrap_or("unknown")
                    .into(),
                provider_http_status: provider
                    .and_then(|facts| facts.http_status)
                    .map(|status| status.as_u16()),
                provider_code: provider
                    .and_then(|facts| facts.provider_code.as_ref())
                    .map(|code| code.as_str().into()),
                provider_request_id: provider
                    .and_then(|facts| facts.provider_request_id.as_ref())
                    .map(|request_id| request_id.as_str().into()),
                retry_after_millis: provider
                    .and_then(|facts| facts.retry_after)
                    .map(super::duration_millis),
                reset_after_millis: observation.and_then(|value| {
                    provider.and_then(|facts| facts.reset_at).map(|reset| {
                        super::duration_millis(reset.saturating_duration_since(value.ended_at))
                    })
                }),
                provider_readiness: provider.map(|facts| facts.readiness.as_str().into()),
                provider_model_event: provider
                    .and_then(|facts| facts.model_event.as_ref())
                    .map(|event| event.as_str().into()),
                time_to_first_model_event_micros: provider
                    .and_then(|facts| facts.ttft)
                    .map(duration_micros),
                provider_ended_micros_from_start: observation.and_then(|value| {
                    provider.and_then(|facts| facts.ended_at).map(|ended| {
                        duration_micros(ended.saturating_duration_since(value.transport.started_at))
                    })
                }),
                transport: AttemptTransportFactV1 {
                    connect_micros: transport
                        .and_then(|facts| facts.connect_elapsed)
                        .map(duration_micros),
                    request_write_micros: transport
                        .and_then(|facts| facts.request_write_elapsed)
                        .map(duration_micros),
                    upstream_ttfb_micros: transport
                        .and_then(|facts| facts.upstream_ttfb)
                        .map(duration_micros),
                    last_upstream_progress_micros_from_start: transport.and_then(|facts| {
                        facts.last_upstream_progress_at.map(|progress| {
                            duration_micros(progress.saturating_duration_since(facts.started_at))
                        })
                    }),
                    local_read_suppressed_micros: transport
                        .map_or(0, |facts| duration_micros(facts.local_read_suppressed)),
                    upstream_body_bytes: transport.map_or(0, |facts| facts.upstream_body_bytes),
                    timeout_kind: transport
                        .and_then(|facts| facts.timeout)
                        .map(timeout_kind)
                        .map(Into::into),
                },
                commits: observation.map_or_else(
                    || AttemptCommitFactV1 {
                        upstream_request: "unknown".into(),
                        downstream_headers: "unknown".into(),
                        downstream_semantic: "unknown".into(),
                    },
                    |value| AttemptCommitFactV1 {
                        upstream_request: commit_fence(value.commits.upstream_request).into(),
                        downstream_headers: commit_fence(value.commits.downstream_headers).into(),
                        downstream_semantic: commit_fence(value.commits.downstream_semantic).into(),
                    },
                ),
                stream_outcome: observation
                    .map(|value| stream_outcome(value.stream))
                    .unwrap_or("unknown")
                    .into(),
                downstream_outcome: observation
                    .map(|value| downstream_outcome(value.downstream))
                    .unwrap_or("unknown")
                    .into(),
                cleanup_outcome: observation
                    .map(|value| cleanup_outcome(value.cleanup))
                    .unwrap_or("unknown")
                    .into(),
                termination_reason: observation
                    .map(|value| termination_reason(value.termination_reason))
                    .unwrap_or("unknown")
                    .into(),
            })),
            Some(&attempt.attempt_id),
        );
        emit_attempt_span(
            self,
            OtelAttempt::from_observation(&attempt),
            outcome,
            error_class,
        );
        self.emit_lifecycle(LifecycleFactV1::AttemptFinished {
            ordinal: attempt.ordinal,
            outcome: outcome.into(),
        });
        let timed_out = transport.and_then(|facts| facts.timeout).is_some();
        let commit = commit_state(observation);
        let (prepared_wire, provider_error, upstream_request_token) = {
            let mut state = self.lock_state();
            (
                state.prepared_wire.take(),
                state.provider_error,
                state
                    .response_wire
                    .as_ref()
                    .and_then(|wire| wire.upstream_request_token),
            )
        };
        if let Some(outcome) = attempt_outcome(outcome, timed_out) {
            let retain_controls = outcome != hiroute_diagnostics::event::AttemptOutcome::Completed
                || self.inner.context.handle().level()
                    == Some(hiroute_diagnostics::DiagnosticLevel::Debug);
            self.emit_diagnostic(DiagnosticEvent::AttemptEnd(AttemptEnd {
                reasoning_fields_removed: attempt.reasoning_fields_removed,
                attempt_token: self.attempt_token(&attempt.attempt_id),
                outcome,
                commit,
                elapsed_ms: duration_millis(duration),
                http_status: provider
                    .and_then(|facts| facts.http_status)
                    .map(|status| status.as_u16()),
                http_protocol: transport.and_then(|facts| facts.upstream_protocol).map(
                    |protocol| match protocol {
                        hiroute_gateway_core::transport::HttpProtocol::Http1 => {
                            WireHttpProtocol::Http1
                        }
                        hiroute_gateway_core::transport::HttpProtocol::Http2 => {
                            WireHttpProtocol::Http2
                        }
                    },
                ),
                provider_error,
                native_model: prepared_wire
                    .as_ref()
                    .filter(|_| retain_controls)
                    .and_then(|wire| wire.native_model.clone()),
                request_reasoning: prepared_wire
                    .as_ref()
                    .filter(|_| retain_controls)
                    .and_then(|wire| wire.request_reasoning.clone()),
                upstream_request_token,
                commits: observation.map(|value| AttemptWireCommits {
                    upstream_request: diagnostic_fence(value.commits.upstream_request),
                    downstream_headers: diagnostic_fence(value.commits.downstream_headers),
                    downstream_body: diagnostic_fence(value.commits.downstream_semantic),
                }),
            }));
        }
        if let Some(facts) = transport {
            if let Some(connect) = facts.connect_elapsed {
                self.emit_model_stage(ModelStageKind::Connect, connect);
            }
            if let Some(ttfb) = facts.upstream_ttfb {
                self.emit_model_stage(ModelStageKind::Ttfb, ttfb);
            }
        }
        let downstream_cancelled = matches!(
            observation.map(|value| value.downstream),
            Some(AttemptDownstreamOutcome::Cancelled)
        );
        if downstream_cancelled
            || matches!(
                observation.map(|value| value.termination_reason),
                Some(AttemptTerminationReason::Cancelled)
            )
        {
            self.emit_diagnostic(DiagnosticEvent::RequestCancel(RequestCancel {
                request_token: self.inner.request_token,
                phase: commit,
                accepted: downstream_cancelled,
            }));
        }
        if timed_out {
            self.emit_diagnostic(DiagnosticEvent::RequestTimeout(RequestTimeout {
                request_token: self.inner.request_token,
                phase: commit,
            }));
        }
    }
}

fn diagnostic_fence(fence: CommitFence) -> WireCommitState {
    match fence {
        CommitFence::Clear => WireCommitState::Clear,
        CommitFence::WriteConfirmed => WireCommitState::Committed,
        CommitFence::WriteStartedMayHaveCommitted => WireCommitState::Poisoned,
    }
}

fn provider_facts(
    observation: &CompletedAttemptObservation,
) -> Option<&hiroute_gateway_core::runtime::driver::ProviderClassificationFacts> {
    observation.provider.as_ref().or_else(|| {
        observation
            .failure
            .as_ref()
            .and_then(|failure| failure.provider.as_ref())
    })
}

fn disposition(value: Disposition) -> &'static str {
    match value {
        Disposition::Accept => "accept",
        Disposition::Continue => "continue",
        Disposition::Terminate => "terminate",
    }
}

fn commit_fence(value: CommitFence) -> &'static str {
    match value {
        CommitFence::Clear => "clear",
        CommitFence::WriteStartedMayHaveCommitted => "write_started_may_have_committed",
        CommitFence::WriteConfirmed => "write_confirmed",
    }
}

fn timeout_kind(value: AttemptTimeoutKind) -> &'static str {
    match value {
        AttemptTimeoutKind::Connect => "connect",
        AttemptTimeoutKind::RequestWrite => "request_write",
        AttemptTimeoutKind::FirstByte => "first_byte",
        AttemptTimeoutKind::StreamIdle => "stream_idle",
        AttemptTimeoutKind::AttemptDeadline => "attempt_deadline",
    }
}

fn stream_outcome(value: AttemptStreamOutcome) -> &'static str {
    match value {
        AttemptStreamOutcome::NotStarted => "not_started",
        AttemptStreamOutcome::CompletedEos => "completed_eos",
        AttemptStreamOutcome::AbortedBeforeSemanticCommit => "aborted_before_semantic_commit",
        AttemptStreamOutcome::StreamStartedNoRetry => "stream_started_no_retry",
    }
}

fn downstream_outcome(value: AttemptDownstreamOutcome) -> &'static str {
    match value {
        AttemptDownstreamOutcome::NotStarted => "not_started",
        AttemptDownstreamOutcome::Completed => "completed",
        AttemptDownstreamOutcome::Failed => "failed",
        AttemptDownstreamOutcome::Cancelled => "cancelled",
    }
}

fn cleanup_outcome(value: AttemptCleanupOutcome) -> &'static str {
    match value {
        AttemptCleanupOutcome::Completed => "completed",
        AttemptCleanupOutcome::Failed => "failed",
    }
}

fn optional_provenance(value: Option<u64>) -> String {
    if value.is_some() {
        "reported".into()
    } else {
        "unknown".into()
    }
}

fn usage_provenance(dimension: UsageDimension) -> String {
    match dimension.provenance {
        UsageProvenance::Reported => "reported",
        UsageProvenance::Estimated => "estimated",
        UsageProvenance::Unknown => "unknown",
    }
    .into()
}

fn termination_reason(reason: AttemptTerminationReason) -> &'static str {
    match reason {
        AttemptTerminationReason::FallbackComplete => "fallback_complete",
        AttemptTerminationReason::AcceptedEos => "accepted_eos",
        AttemptTerminationReason::TerminatedResponseComplete => "terminated_response_complete",
        AttemptTerminationReason::PreexchangeFailure => "preexchange_failure",
        AttemptTerminationReason::AttemptFailure => "attempt_failure",
        AttemptTerminationReason::StreamStartedNoRetry => "stream_started_no_retry",
        AttemptTerminationReason::Cancelled => "cancelled",
        AttemptTerminationReason::DownstreamFailure => "downstream_failure",
        AttemptTerminationReason::ProviderReleaseFailure => "provider_release_failure",
        AttemptTerminationReason::ProviderEncoderFailure => "provider_encoder_failure",
        AttemptTerminationReason::FilterFailure => "filter_failure",
        AttemptTerminationReason::CleanupFailure => "cleanup_failure",
    }
}
