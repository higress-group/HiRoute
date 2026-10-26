//! Typed safe facts from the actual provider request and response boundaries.

use super::RequestObservation;
use crate::server::core_runtime::observation::wire_diagnostic;
use hiroute_diagnostics::{correlation::CorrelationDomain, event::DiagnosticEvent};

impl RequestObservation {
    /// The producer supplies only closed projection facts. Correlation and ingress
    /// are bound here; diagnostics never become captured conversation/score facts.
    pub(crate) fn request_projection(
        &self,
        stable_binding_id: &str,
        mut event: hiroute_diagnostics::event::RequestProjection,
    ) {
        event.request_token = self.inner.request_token;
        event.binding_token = self
            .inner
            .context
            .token(CorrelationDomain::Binding, stable_binding_id);
        event.ingress_protocol = self.ingress_protocol();
        self.emit_diagnostic(DiagnosticEvent::RequestProjection(event));
    }

    /// Main native projection evidence, not optional canonical capture status.
    pub(crate) fn native_completion_unknown(
        &self,
        upstream_protocol: hiroute_diagnostics::event::IngressProtocol,
        evidence: hiroute_diagnostics::event::NativeCompletionEvidence,
    ) {
        let state = self.lock_state();
        let attempt = state
            .accepted_attempt
            .as_ref()
            .or(state.current_attempt.as_ref())
            .or(state.pending_attempt.as_ref());
        let attempt_index = u64::from(
            attempt
                .map(|a| a.ordinal)
                .filter(|ordinal| *ordinal != 0)
                .unwrap_or(state.next_attempt_ordinal),
        );
        let attempt_token = attempt
            .filter(|a| !a.attempt_id.is_empty())
            .and_then(|a| self.attempt_token(&a.attempt_id));
        drop(state);
        self.emit_diagnostic(DiagnosticEvent::NativeCompletionUnknown(
            hiroute_diagnostics::event::NativeCompletionUnknown {
                request_token: self.inner.request_token,
                attempt_index,
                attempt_token,
                upstream_protocol,
                evidence,
            },
        ));
    }

    #[cfg(test)]
    pub(crate) fn response_failure(
        &self,
        stage: hiroute_diagnostics::event::ResponseFailureStage,
        reason: hiroute_diagnostics::event::ResponseFailureReason,
    ) {
        self.response_failure_detail(stage, reason, None, None);
    }

    #[cfg(all(unix, debug_assertions))]
    pub(crate) fn register_private_capture(
        &self,
        capture: crate::runtime::stream_capture::PendingCapture,
    ) {
        let mut state = self.lock_state();
        if let Some(pending) = state.pending_attempt.as_ref() {
            state.private_capture = Some((
                pending.stable_binding_id.clone(),
                pending.credential_ref.clone(),
                capture,
            ));
        }
    }

    #[cfg(all(unix, debug_assertions))]
    pub(crate) fn private_capture_correlation(
        &self,
    ) -> crate::runtime::stream_capture::CaptureCorrelation {
        let state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let attempt = state
            .accepted_attempt
            .as_ref()
            .or(state.current_attempt.as_ref())
            .or(state.pending_attempt.as_ref());
        crate::runtime::stream_capture::CaptureCorrelation {
            request_token: self.inner.request_token,
            request_id: self.inner.metadata.request_id.clone(),
            attempt_index: attempt
                .map(|a| a.ordinal)
                .filter(|n| *n != 0)
                .unwrap_or(state.next_attempt_ordinal),
            attempt_token: attempt
                .filter(|a| !a.attempt_id.is_empty())
                .and_then(|a| self.attempt_token(&a.attempt_id)),
        }
    }

    pub(crate) fn response_failure_detail(
        &self,
        stage: hiroute_diagnostics::event::ResponseFailureStage,
        reason: hiroute_diagnostics::event::ResponseFailureReason,
        field: Option<hiroute_diagnostics::event::ResponseFailureField>,
        position: Option<(u64, u64)>,
    ) {
        let state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let attempt = state
            .accepted_attempt
            .as_ref()
            .or(state.current_attempt.as_ref())
            .or(state.pending_attempt.as_ref());
        let ordinal = attempt
            .map(|a| a.ordinal)
            .filter(|n| *n != 0)
            .unwrap_or(state.next_attempt_ordinal);
        let attempt_token = attempt
            .filter(|a| !a.attempt_id.is_empty())
            .and_then(|a| self.attempt_token(&a.attempt_id));
        drop(state);
        self.emit_diagnostic(DiagnosticEvent::ResponseFailure(
            hiroute_diagnostics::event::ResponseFailure {
                request_token: self.inner.request_token,
                attempt_index: u64::from(ordinal),
                stage,
                reason,
                attempt_token,
                field,
                frame_index: position.map(|p| p.0).filter(|n| *n != 0),
                received_bytes: position.map(|p| p.1),
                capture: None,
            },
        ));
    }

    /// Called at materialization, before this attempt is promoted. Keep its
    /// ordinal even if the off-path capture worker runs after a later fallback.
    pub(in crate::server::core_runtime::observation) fn capture_response_failure_context(
        &self,
    ) -> hiroute_diagnostics::event::ResponseFailure {
        hiroute_diagnostics::event::ResponseFailure {
            request_token: self.inner.request_token,
            attempt_index: u64::from(self.lock_state().next_attempt_ordinal),
            stage: hiroute_diagnostics::event::ResponseFailureStage::Decode,
            reason: hiroute_diagnostics::event::ResponseFailureReason::OtherProtocol,
            attempt_token: None,
            field: None,
            frame_index: None,
            received_bytes: None,
            capture: None,
        }
    }

    pub(in crate::server::core_runtime::observation) fn capture_response_failure(
        &self,
        failure: hiroute_diagnostics::event::ResponseFailure,
    ) {
        self.emit_diagnostic(DiagnosticEvent::ResponseFailure(failure));
    }

    pub(in crate::server::core_runtime::observation) fn wire_diagnostic(
        &self,
        mut event: hiroute_diagnostics::event::UpstreamWire,
    ) {
        event.request_token = self.inner.context.token(
            CorrelationDomain::ModelRequest,
            &self.inner.metadata.request_id,
        );
        let state = self.lock_state();
        let attempt = state
            .accepted_attempt
            .as_ref()
            .or(state.current_attempt.as_ref())
            .or(state.pending_attempt.as_ref());
        event.attempt_index = Some(u64::from(
            attempt
                .map(|a| a.ordinal)
                .filter(|ordinal| *ordinal != 0)
                .unwrap_or(state.next_attempt_ordinal),
        ));
        event.attempt_token = attempt
            .filter(|a| !a.attempt_id.is_empty())
            .and_then(|a| self.attempt_token(&a.attempt_id));
        drop(state);
        self.emit_diagnostic(DiagnosticEvent::UpstreamWire(event));
    }

    pub(in crate::server::core_runtime::observation) fn response_head_diagnostic(
        &self,
        headers: &http::HeaderMap,
        status: u16,
    ) {
        let mut event = wire_diagnostic::response(headers, status, &self.inner.context);
        if self.lock_state().managed_cpa {
            event.cpa_execution = Some(super::super::provider::cpa_execution::from_headers(
                headers, status,
            ));
        }
        if let Some(error) = event.provider_error {
            self.lock_state().provider_error = Some(error);
        }
        self.lock_state().response_wire = Some(event.clone());
        self.wire_diagnostic(event);
    }

    pub(crate) fn provider_failure_diagnostic(
        &self,
        http_status: Option<u16>,
        error: hiroute_diagnostics::event::WireProviderError,
    ) {
        use hiroute_diagnostics::event::UpstreamWirePhase;
        self.lock_state().provider_error = Some(error);
        let mut event = wire_diagnostic::response(
            &http::HeaderMap::new(),
            http_status.unwrap_or(0),
            &self.inner.context,
        );
        event.phase = UpstreamWirePhase::Failure;
        event.http_status = http_status;
        event.provider_error = Some(error);
        let state = self.lock_state();
        if let Some(prepared) = &state.prepared_wire {
            event.native_model = prepared.native_model.clone();
            event.request_reasoning = prepared.request_reasoning.clone();
        }
        if state.managed_cpa {
            event.cpa_execution = Some(
                state
                    .response_wire
                    .as_ref()
                    .and_then(|wire| wire.cpa_execution.clone())
                    .unwrap_or(hiroute_diagnostics::event::CpaExecution::Unknown),
            );
        }
        event.upstream_request_token = state
            .response_wire
            .as_ref()
            .and_then(|wire| wire.upstream_request_token);
        drop(state);
        self.wire_diagnostic(event);
    }

    pub(crate) fn prepared_request_diagnostic(
        &self,
        headers: &http::HeaderMap,
        serialized_template: &[u8],
    ) {
        if self.inner.context.handle().level().is_some() {
            let event = wire_diagnostic::request(headers, serialized_template);
            self.lock_state().prepared_wire = Some(event.clone());
            self.wire_diagnostic(event);
        }
    }
}
