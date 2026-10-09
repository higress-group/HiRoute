//! Typed safe facts from the actual provider request and response boundaries.

use super::RequestObservation;
use hiroute_diagnostics::{correlation::CorrelationDomain, event::DiagnosticEvent};

impl RequestObservation {
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
        let event =
            super::provider::wire_diagnostic::response(headers, status, &self.inner.context);
        if let Some(error) = event.provider_error {
            self.lock_state().provider_error = Some(error);
        }
        self.wire_diagnostic(event);
    }

    pub(crate) fn provider_failure_diagnostic(
        &self,
        http_status: Option<u16>,
        error: hiroute_diagnostics::event::WireProviderError,
    ) {
        use hiroute_diagnostics::event::UpstreamWirePhase;
        self.lock_state().provider_error = Some(error);
        let mut event = super::provider::wire_diagnostic::response(
            &http::HeaderMap::new(),
            http_status.unwrap_or(0),
            &self.inner.context,
        );
        event.phase = UpstreamWirePhase::Failure;
        event.http_status = http_status;
        event.provider_error = Some(error);
        self.wire_diagnostic(event);
    }

    pub(crate) fn prepared_request_diagnostic(
        &self,
        headers: &http::HeaderMap,
        serialized_template: &[u8],
    ) {
        if self.is_enabled()
            && self.inner.context.handle().level()
                == Some(hiroute_diagnostics::DiagnosticLevel::Debug)
        {
            self.wire_diagnostic(super::provider::wire_diagnostic::request(
                headers,
                serialized_template,
            ));
        }
    }
}
