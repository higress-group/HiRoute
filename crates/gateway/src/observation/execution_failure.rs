//! Diagnose a terminal local error without retaining its free-form message.
use hiroute_diagnostics::event::{
    DiagnosticEvent, ExecutionBudgetRejection, ExecutionFailureReason as Reason,
    ExecutionFailureSource as Source, ExecutionMemoryRole, ExecutionRequestShape,
    RequestExecutionFailure,
};
use hiroute_gateway_core::runtime::attempt::AttemptError;
use hiroute_gateway_core::runtime::body::{BodyError, MemoryRole, StreamBudget};
use hiroute_gateway_core::runtime::driver::GatewayExecutionError;
use hiroute_gateway_core::transport::TransportError;

use super::RequestObservation;

impl RequestObservation {
    pub(crate) fn execution_failure(
        &self,
        error: &GatewayExecutionError,
        budget: &StreamBudget,
        request: ExecutionRequestShape,
        response_started: bool,
    ) {
        self.emit_execution_failure(error, budget, request, response_started, true);
    }

    pub(crate) fn attempt_preparation_failure(
        &self,
        error: &std::sync::Arc<str>,
        budget: &StreamBudget,
        request: ExecutionRequestShape,
    ) {
        self.emit_execution_failure(
            &GatewayExecutionError::Provider(std::sync::Arc::clone(error)),
            budget,
            request,
            false,
            false,
        );
    }

    fn emit_execution_failure(
        &self,
        error: &GatewayExecutionError,
        budget: &StreamBudget,
        request: ExecutionRequestShape,
        response_started: bool,
        terminal: bool,
    ) {
        let (source, reason) = classification(error);
        let last_budget_rejection = budget.last_rejection().map(|r| ExecutionBudgetRejection {
            role: match r.role {
                MemoryRole::RawRequest => ExecutionMemoryRole::RawRequest,
                MemoryRole::ModelIrBacking => ExecutionMemoryRole::ModelIrBacking,
                MemoryRole::AttemptWire => ExecutionMemoryRole::AttemptWire,
                MemoryRole::Retry => ExecutionMemoryRole::Retry,
                MemoryRole::ResponsePrefix => ExecutionMemoryRole::ResponsePrefix,
                MemoryRole::SseFrame => ExecutionMemoryRole::SseFrame,
                MemoryRole::SemanticState => ExecutionMemoryRole::SemanticState,
                MemoryRole::OutputQueue => ExecutionMemoryRole::OutputQueue,
                MemoryRole::TransportInflight => ExecutionMemoryRole::TransportInflight,
            },
            requested_bytes: r.requested_bytes as u64,
            stream_live_bytes: r.stream_live_bytes as u64,
            stream_limit_bytes: r.stream_limit_bytes as u64,
            worker_live_bytes: r.worker_live_bytes as u64,
            worker_limit_bytes: r.worker_limit_bytes as u64,
            process_live_bytes: r.process_live_bytes as u64,
            process_limit_bytes: r.process_limit_bytes as u64,
        });
        let attempt_started = self.lock_state().next_attempt_ordinal > 1;
        let failure = RequestExecutionFailure {
            request_token: self.inner.request_token,
            source,
            reason,
            request,
            attempt_started,
            response_started,
            last_budget_rejection,
        };
        self.emit_diagnostic(if terminal {
            DiagnosticEvent::RequestExecutionFailure(failure)
        } else {
            DiagnosticEvent::AttemptPreparationFailure(failure)
        });
    }
}

fn body_reason(error: &BodyError) -> Reason {
    match error {
        BodyError::BudgetExceeded => Reason::MemoryBudgetExceeded,
        BodyError::BodyLimitExceeded => Reason::BodyLimitExceeded,
        _ => Reason::InvalidBodyState,
    }
}

fn provider_reason(error: &str) -> Reason {
    match error {
        "body budget exceeded" => Reason::MemoryBudgetExceeded,
        "protocol_failed" => Reason::ProviderProtocol,
        "authority_failed" => Reason::ProviderAuthority,
        "runtime_state_authority_failed" => Reason::ProviderRuntimeState,
        "dns_failed" => Reason::ProviderDns,
        _ => Reason::Other,
    }
}

fn classification(error: &GatewayExecutionError) -> (Source, Reason) {
    use GatewayExecutionError as E;
    match error {
        E::Body(error) => (Source::Body, body_reason(error)),
        E::Attempt(AttemptError::Body(error)) => (Source::Attempt, body_reason(error)),
        E::Attempt(AttemptError::Cancelled) => (Source::Attempt, Reason::Cancelled),
        E::Attempt(AttemptError::DeadlineExceeded)
        | E::Attempt(AttemptError::ConnectTimeout)
        | E::Attempt(AttemptError::RequestWriteTimeout)
        | E::Attempt(AttemptError::FirstByteTimeout)
        | E::Attempt(AttemptError::StreamIdleTimeout) => (Source::Attempt, Reason::Deadline),
        E::Attempt(AttemptError::CleanupTimeout) => (Source::Attempt, Reason::CleanupTimeout),
        E::Attempt(_) => (Source::Attempt, Reason::Other),
        E::Provider(error) | E::ProviderTerminalRelease(error) => {
            (Source::Provider, provider_reason(error))
        }
        E::Transport(TransportError::Cancelled) => (Source::Transport, Reason::Cancelled),
        E::Transport(TransportError::DeadlineExceeded) => (Source::Transport, Reason::Deadline),
        E::Transport(TransportError::InvalidMetadata(_)) => {
            (Source::Transport, Reason::InvalidMetadata)
        }
        E::Transport(TransportError::Unsupported(_)) => (Source::Transport, Reason::Unsupported),
        E::Transport(_) => (Source::Transport, Reason::Other),
        E::Selection(_) => (Source::Selection, Reason::Other),
        E::Filter(_) => (Source::Filter, Reason::Other),
        E::Cancelled => (Source::Lifecycle, Reason::Cancelled),
        E::Deadline | E::PreexchangeAttemptDeadline => (Source::Lifecycle, Reason::Deadline),
        E::CleanupTimeout => (Source::Lifecycle, Reason::CleanupTimeout),
        E::OperationPanic => (Source::Lifecycle, Reason::OperationPanic),
        _ => (Source::Lifecycle, Reason::Other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_errors_are_only_classified_by_closed_values() {
        let error = GatewayExecutionError::Provider("private request and credential".into());
        assert_eq!(classification(&error), (Source::Provider, Reason::Other));
        let encoded = serde_json::to_string(&classification(&error)).unwrap();
        assert!(!encoded.contains("private"));
        assert_eq!(
            classification(&GatewayExecutionError::Provider(
                "body budget exceeded".into()
            )),
            (Source::Provider, Reason::MemoryBudgetExceeded)
        );
        assert_eq!(
            classification(&GatewayExecutionError::Body(BodyError::BudgetExceeded)),
            (Source::Body, Reason::MemoryBudgetExceeded)
        );
    }
}
