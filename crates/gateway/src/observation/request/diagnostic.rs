//! Typed safe facts from the actual provider request and response boundaries.

use super::RequestObservation;
use crate::server::core_runtime::observation::wire_diagnostic;
use hiroute_diagnostics::{correlation::CorrelationDomain, event::DiagnosticEvent};

impl RequestObservation {
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
            },
        ));
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
        let event = wire_diagnostic::response(headers, status, &self.inner.context);
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
