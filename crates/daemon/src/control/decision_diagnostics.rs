use hiroute_application_api::{DecisionFailureCodeV2, DecisionTestOutcomeV2, MachineEnvelopeV2};
use hiroute_diagnostics::{
    context::DiagnosticHandle,
    event::{DecisionTestEnd, DecisionTestFailure, DiagnosticEvent},
    identity::CorrelationToken,
};
use serde::Deserialize;
use serde_json::Value;

pub(super) fn service_token(
    diagnostics: &DiagnosticHandle,
    input: &Value,
) -> Option<CorrelationToken> {
    diagnostics.decision_service_token(input.get("id")?.as_str()?, input.get("revision")?.as_u64()?)
}

/// A successful Local Control call can still report a failed decision test.
pub(super) fn record_test_end(
    diagnostics: &DiagnosticHandle,
    response: &MachineEnvelopeV2<Value>,
    request_token: Option<CorrelationToken>,
    service_token: Option<CorrelationToken>,
    elapsed_ms: u64,
) {
    if response.error.is_some() {
        return;
    }
    let result = response
        .data
        .as_ref()
        .and_then(|data| Check::deserialize(data.get("check").unwrap_or(data)).ok());
    let failure = match result.map(|check| (check.outcome, check.failure_code)) {
        Some((DecisionTestOutcomeV2::Passed, None)) => None,
        Some((DecisionTestOutcomeV2::Failed, Some(code))) => Some(failure_kind(code)),
        _ => Some(DecisionTestFailure::InvalidTestResult),
    };
    diagnostics.try_emit(DiagnosticEvent::DecisionTestEnd(DecisionTestEnd {
        request_token,
        service_token,
        failure,
        elapsed_ms,
    }));
}

#[derive(Deserialize)]
struct Check {
    outcome: DecisionTestOutcomeV2,
    failure_code: Option<DecisionFailureCodeV2>,
}

fn failure_kind(code: DecisionFailureCodeV2) -> DecisionTestFailure {
    use DecisionFailureCodeV2 as Code;
    use DecisionTestFailure as Failure;
    match code {
        Code::DecisionInvalidConfiguration => Failure::InvalidConfiguration,
        Code::DecisionSourceNotFound => Failure::SourceNotFound,
        Code::DecisionSourceLineageMismatch => Failure::SourceLineageMismatch,
        Code::DecisionSourceDisabled => Failure::SourceDisabled,
        Code::DecisionSourceUnavailable => Failure::SourceUnavailable,
        Code::DecisionNoUsableCredentials => Failure::NoUsableCredentials,
        Code::DecisionRevisionConflict => Failure::RevisionConflict,
        Code::DecisionInUse => Failure::InUse,
        Code::DecisionTimeout => Failure::Timeout,
        Code::DecisionAuthenticationRejected => Failure::AuthenticationRejected,
        Code::DecisionRateLimited => Failure::RateLimited,
        Code::DecisionInputRejected => Failure::InputRejected,
        Code::DecisionMalformedOutput => Failure::MalformedOutput,
        Code::DecisionTransportUnavailable => Failure::TransportUnavailable,
    }
}
