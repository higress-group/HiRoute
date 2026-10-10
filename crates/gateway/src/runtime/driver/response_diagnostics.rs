use crate::server::core_runtime::{
    adapters::ProtocolAdapterError,
    model_ir::{ModelError, ModelIrError},
};
use hiroute_diagnostics::event::{
    ResponseFailureField as Field, ResponseFailureReason as Reason, ResponseFailureStage as Stage,
    WireProviderError,
};
use std::sync::Arc;

pub(super) fn note(stage: Stage, reason: Reason) {
    report(stage, reason, None, None);
}

fn report(stage: Stage, reason: Reason, field: Option<Field>, position: Option<(u64, u64)>) {
    if let Some(observation) = crate::server::core_runtime::observation::active_request() {
        observation.response_failure_detail(stage, reason, field, position);
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
    adapter_at(stage, error, None)
}

pub(super) fn adapter_at(
    stage: Stage,
    error: ProtocolAdapterError,
    position: Option<(u64, u64)>,
) -> Arc<str> {
    let (reason, field) = category(&error);
    report(stage, reason, field, position);
    Arc::from("provider response processing failed")
}

pub(crate) fn category(error: &ProtocolAdapterError) -> (Reason, Option<Field>) {
    let mut field = None;
    let reason = match error {
        ProtocolAdapterError::ModelIr(error) => match error {
            ModelIrError::BufferLimit(_) => Reason::ResourceLimit,
            ModelIrError::InvalidJson(_) => Reason::InvalidJson,
            ModelIrError::InvalidSse(_) => Reason::InvalidSse,
            ModelIrError::ExpectedObject => {
                field = Some(Field::Object);
                Reason::InvalidField
            }
            ModelIrError::InvalidField(name) => {
                field = Some(known_field(name));
                Reason::InvalidField
            }
            ModelIrError::UnsupportedField(name) => {
                field = Some(known_field(name));
                Reason::UnsupportedField
            }
            ModelIrError::UnsupportedValue(_) => Reason::UnsupportedValue,
            ModelIrError::InvalidToolArguments(_) => Reason::InvalidToolArguments,
            ModelIrError::MissingToolIdentity(_) => Reason::MissingToolIdentity,
            ModelIrError::ProviderStateNotPortable => Reason::ProviderStateNotPortable,
            ModelIrError::ToolIdBindingRequired(_) => Reason::ToolIdBindingRequired,
            ModelIrError::ToolContinuationConflict => Reason::ToolContinuationConflict,
            ModelIrError::InvalidResponseLifecycle(_) => Reason::InvalidLifecycle,
            ModelIrError::MissingTerminalEvent => Reason::MissingTerminal,
            ModelIrError::DuplicateTerminalEvent => Reason::DuplicateTerminal,
            ModelIrError::ResponsesPreviousResponseIdUnsupported
            | ModelIrError::ResponsesConversationUnsupported => Reason::UnsupportedValue,
        },
        ProtocolAdapterError::ClientUnrepresentable(_) => Reason::Unrepresentable,
        ProtocolAdapterError::Capability(_) => Reason::Capability,
        ProtocolAdapterError::Context(_) => Reason::ContextProjection,
        ProtocolAdapterError::Serialization(_) => Reason::Serialization,
    };
    (reason, field)
}

// Exact allowlist only: never parse an error message or echo an unknown field path.
fn known_field(name: &str) -> Field {
    match name {
        "choices" => Field::Choices,
        "delta" => Field::Delta,
        "content" => Field::Content,
        "tool_calls" => Field::ToolCalls,
        "tool id" | "tool_call.id" => Field::ToolId,
        "tool name" | "tool_call.function.name" => Field::ToolName,
        "arguments" | "tool_call.function.arguments" => Field::ToolArguments,
        "finish_reason" => Field::FinishReason,
        "usage" => Field::Usage,
        "system_fingerprint" => Field::SystemFingerprint,
        "service_tier" => Field::ServiceTier,
        "logprobs" => Field::Logprobs,
        "model" => Field::Model,
        "id" => Field::Id,
        "type" => Field::Type,
        "index" => Field::Index,
        "role" => Field::Role,
        "output" => Field::Output,
        "status" => Field::Status,
        _ => Field::Unknown,
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
    fn diagnostic_categories_are_closed_and_never_serialize_dynamic_messages() {
        for (error, expected) in [
            (ModelIrError::BufferLimit(100), Reason::ResourceLimit),
            (
                ModelIrError::InvalidJson("SECRET_BODY".into()),
                Reason::InvalidJson,
            ),
            (
                ModelIrError::InvalidSse("SECRET_BODY".into()),
                Reason::InvalidSse,
            ),
            (
                ModelIrError::UnsupportedField("SECRET_BODY".into()),
                Reason::UnsupportedField,
            ),
            (
                ModelIrError::UnsupportedValue("SECRET_BODY".into()),
                Reason::UnsupportedValue,
            ),
            (
                ModelIrError::MissingToolIdentity("SECRET_TOOL".into()),
                Reason::MissingToolIdentity,
            ),
            (
                ModelIrError::InvalidToolArguments("SECRET_KEY".into()),
                Reason::InvalidToolArguments,
            ),
            (
                ModelIrError::ProviderStateNotPortable,
                Reason::ProviderStateNotPortable,
            ),
            (
                ModelIrError::InvalidResponseLifecycle("SECRET_BODY".into()),
                Reason::InvalidLifecycle,
            ),
            (ModelIrError::MissingTerminalEvent, Reason::MissingTerminal),
            (
                ModelIrError::DuplicateTerminalEvent,
                Reason::DuplicateTerminal,
            ),
        ] {
            let result = category(&error.into());
            assert_eq!(result.0, expected);
            assert!(!serde_json::to_string(&result).unwrap().contains("SECRET"));
        }
        assert_eq!(known_field("system_fingerprint"), Field::SystemFingerprint);
        assert_eq!(known_field("tool_calls.SECRET_TOOL"), Field::Unknown);
    }
}
