use crate::server::core_runtime::{
    adapters::ProtocolAdapterError,
    model_ir::{ModelError, ModelIrError},
};
use hiroute_diagnostics::event::{
    ResponseFailureReason as Reason, ResponseFailureStage as Stage, WireProviderError,
};
use std::sync::Arc;

pub(super) fn note(stage: Stage, reason: Reason) {
    if let Some(observation) = crate::server::core_runtime::observation::active_request() {
        observation.response_failure(stage, reason);
    }
}

pub(super) fn model_error(error: &ModelError, response_status: Option<u16>) {
    if let Some(observation) = crate::server::core_runtime::observation::active_request() {
        observation.provider_failure_diagnostic(
            error.status.or(response_status),
            model_error_code(error, response_status),
        );
    }
}

fn model_error_code(error: &ModelError, response_status: Option<u16>) -> WireProviderError {
    if let Some(message) = error.message.as_deref() {
        const PREFIX: &str =
            "The thinking_budget parameter must be a positive integer and not greater than ";
        let message = message
            .strip_prefix("<400> InternalError.Algo.InvalidParameter: ")
            .unwrap_or(message);
        if message.strip_prefix(PREFIX).is_some_and(|limit| {
            !limit.is_empty()
                && limit.len() <= 10
                && limit.bytes().all(|byte| byte.is_ascii_digit())
        }) {
            return WireProviderError::ThinkingBudgetRejected;
        }
        if message == "Workspace endpoint is invalid." {
            return WireProviderError::EndpointRejected;
        }
    }
    match error.code.as_deref() {
        Some(
            "invalid_api_key" | "authentication_error" | "permission_denied" | "permission_error",
        ) => WireProviderError::AuthenticationRejected,
        Some(
            "rate_limit_exceeded" | "insufficient_quota" | "rate_limit_error" | "overloaded_error",
        ) => WireProviderError::RateLimited,
        Some("server_error" | "internal_error" | "api_error") => {
            WireProviderError::UpstreamUnavailable
        }
        Some("invalid_request_error" | "invalid_request" | "not_found_error") => {
            WireProviderError::InputRejected
        }
        _ => match error.status.or(response_status) {
            Some(401 | 403) => WireProviderError::AuthenticationRejected,
            Some(429) => WireProviderError::RateLimited,
            Some(400..=499) => WireProviderError::InputRejected,
            Some(500..=599) => WireProviderError::UpstreamUnavailable,
            _ => WireProviderError::Unknown,
        },
    }
}

pub(super) fn adapter(stage: Stage, error: ProtocolAdapterError) -> Arc<str> {
    note(stage, category(&error));
    Arc::from("provider response processing failed")
}

fn category(error: &ProtocolAdapterError) -> Reason {
    match error {
        ProtocolAdapterError::ModelIr(error) => match error {
            ModelIrError::BufferLimit(_) => Reason::ResourceLimit,
            ModelIrError::InvalidJson(_) => Reason::InvalidJson,
            ModelIrError::InvalidSse(_) => Reason::InvalidSse,
            ModelIrError::ExpectedObject | ModelIrError::InvalidField(_) => Reason::InvalidField,
            ModelIrError::InvalidResponseLifecycle(_) => Reason::InvalidLifecycle,
            ModelIrError::MissingTerminalEvent => Reason::MissingTerminal,
            ModelIrError::DuplicateTerminalEvent => Reason::DuplicateTerminal,
            _ => Reason::OtherProtocol,
        },
        ProtocolAdapterError::ClientUnrepresentable(_) => Reason::Unrepresentable,
        _ => Reason::OtherProtocol,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_errors_keep_safe_attribution_without_provider_text() {
        let known = ModelError { status: Some(400), code: Some("InvalidParameter".into()), message: Some("<400> InternalError.Algo.InvalidParameter: The thinking_budget parameter must be a positive integer and not greater than 81920".into()), retryable: None };
        assert_eq!(
            model_error_code(&known, Some(200)),
            WireProviderError::ThinkingBudgetRejected
        );
        let mut unknown = known;
        unknown.message = Some("provider-secret-marker thinking_budget arbitrary detail".into());
        assert_eq!(
            model_error_code(&unknown, Some(200)),
            WireProviderError::InputRejected
        );
        for (status, expected) in [
            (401, WireProviderError::AuthenticationRejected),
            (429, WireProviderError::RateLimited),
            (503, WireProviderError::UpstreamUnavailable),
        ] {
            unknown.status = Some(status);
            let actual = model_error_code(&unknown, Some(200));
            assert_eq!(actual, expected);
            assert!(!serde_json::to_string(&actual).unwrap().contains("secret"));
        }
    }

    #[test]
    fn diagnostic_categories_distinguish_local_limits_and_protocol_failures_without_payloads() {
        for (error, expected) in [
            (ModelIrError::BufferLimit(100), Reason::ResourceLimit),
            (
                ModelIrError::InvalidJson("secret provider body".into()),
                Reason::InvalidJson,
            ),
            (
                ModelIrError::InvalidSse("secret event".into()),
                Reason::InvalidSse,
            ),
            (ModelIrError::MissingTerminalEvent, Reason::MissingTerminal),
            (
                ModelIrError::DuplicateTerminalEvent,
                Reason::DuplicateTerminal,
            ),
        ] {
            let category = category(&error.into());
            assert_eq!(category, expected);
            assert!(!serde_json::to_string(&category).unwrap().contains("secret"));
        }
    }
}
