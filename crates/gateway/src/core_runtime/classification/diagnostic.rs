//! Safe diagnostics for the decision service's real transport path.

use std::sync::atomic::{AtomicU64, Ordering};

use hiroute_diagnostics::{
    context::DiagnosticContext,
    correlation::CorrelationDomain,
    event::{
        DiagnosticEvent, UpstreamWire, UpstreamWirePhase, WireHttpProtocol, WireProviderError,
        WireRequestKind,
    },
};
use hiroute_gateway_core::transport::HttpProtocol;
use http::{HeaderMap, StatusCode};

use super::{CLASSIFIER_DIAGNOSTIC_LATEST_USER, CallFailure, ClassifierDiagnosticError};
use crate::server::core_runtime::model_ir::{
    CanonicalMessage, ContentPart, MODEL_REQUEST_IR_SCHEMA, MessageRole, ModelRequestIRV1,
    RequestedReasoningControl, ToolChoice,
};
use crate::server::core_runtime::observation::wire_diagnostic;
use crate::server::request_plan::IngressProtocol;

static NEXT_CALL: AtomicU64 = AtomicU64::new(1);

pub(super) struct CallDiagnostic {
    context: DiagnosticContext,
    final_event: UpstreamWire,
    finished: bool,
}

impl CallDiagnostic {
    pub(super) fn new(context: DiagnosticContext) -> Self {
        let mut final_event = wire_diagnostic::response(&HeaderMap::new(), 0, &context);
        final_event.http_status = None;
        final_event.request_kind = Some(WireRequestKind::DecisionService);
        final_event.request_token = context.token(
            CorrelationDomain::Decision,
            &NEXT_CALL.fetch_add(1, Ordering::Relaxed).to_string(),
        );
        final_event.phase = UpstreamWirePhase::Failure;
        final_event.provider_error = Some(WireProviderError::Unknown);
        Self {
            context,
            final_event,
            finished: false,
        }
    }

    pub(super) fn prepared(&mut self, headers: &HeaderMap, template: &[u8]) {
        if self.context.handle().level().is_none() {
            return;
        }
        let mut event = wire_diagnostic::request(headers, template);
        event.request_token = self.final_event.request_token;
        event.request_kind = Some(WireRequestKind::DecisionService);
        self.final_event.native_model = event.native_model.clone();
        self.final_event.request_reasoning = event.request_reasoning.clone();
        self.context.emit(DiagnosticEvent::UpstreamWire(event));
    }

    pub(super) fn protocol(&mut self, protocol: Option<HttpProtocol>) {
        self.final_event.http_protocol = protocol.map(|protocol| match protocol {
            HttpProtocol::Http1 => WireHttpProtocol::Http1,
            HttpProtocol::Http2 => WireHttpProtocol::Http2,
        });
    }

    pub(super) fn response(&mut self, headers: &HeaderMap, status: StatusCode) {
        let mut event = wire_diagnostic::response(headers, status.as_u16(), &self.context);
        event.request_kind = Some(WireRequestKind::DecisionService);
        event.request_token = self.final_event.request_token;
        event.http_protocol = self.final_event.http_protocol;
        self.final_event.http_status = event.http_status;
        self.final_event.upstream_request_token = event.upstream_request_token;
        self.final_event.provider_error = event.provider_error.or(Some(WireProviderError::Unknown));
        self.context.emit(DiagnosticEvent::UpstreamWire(event));
    }

    pub(super) fn failure(&mut self, failure: CallFailure) {
        self.final_event.provider_error = Some(match failure {
            CallFailure::Timeout => WireProviderError::Timeout,
            CallFailure::Unavailable => WireProviderError::UpstreamUnavailable,
            CallFailure::RejectedInput => WireProviderError::InputRejected,
            CallFailure::AuthenticationRejected => WireProviderError::AuthenticationRejected,
            CallFailure::RateLimited => WireProviderError::RateLimited,
            CallFailure::EndpointRejected => WireProviderError::EndpointRejected,
            CallFailure::InvalidOutput => WireProviderError::InvalidOutput,
        });
    }

    pub(super) fn finish(&mut self, failure: Option<CallFailure>) {
        match failure {
            Some(failure) => self.failure(failure),
            None => {
                self.final_event.phase = UpstreamWirePhase::Completed;
                self.final_event.provider_error = None;
            }
        }
        self.context
            .emit(DiagnosticEvent::UpstreamWire(self.final_event.clone()));
        self.finished = true;
    }
}

impl Drop for CallDiagnostic {
    fn drop(&mut self) {
        if !self.finished {
            self.context
                .emit(DiagnosticEvent::UpstreamWire(self.final_event.clone()));
        }
    }
}

pub(super) fn classifier_response_failure(status: StatusCode, body: &[u8]) -> CallFailure {
    if status == StatusCode::BAD_REQUEST
        && let Ok(value) = serde_json::from_slice::<serde_json::Value>(body)
        && value
            .get("error")
            .unwrap_or(&value)
            .get("message")
            .and_then(serde_json::Value::as_str)
            == Some("Workspace endpoint is invalid.")
    {
        return CallFailure::EndpointRejected;
    }
    classifier_status_failure(status)
}

pub(super) fn classifier_status_failure(status: StatusCode) -> CallFailure {
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return CallFailure::AuthenticationRejected;
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return CallFailure::RateLimited;
    }
    if status.as_u16() == 529 || status.is_server_error() {
        CallFailure::Unavailable
    } else if status.is_client_error() {
        CallFailure::RejectedInput
    } else {
        CallFailure::InvalidOutput
    }
}

pub(super) fn diagnostic_request() -> ModelRequestIRV1 {
    ModelRequestIRV1 {
        native_body: None,
        native_only: false,
        schema_version: MODEL_REQUEST_IR_SCHEMA.into(),
        ingress_protocol: IngressProtocol::Responses,
        responses_options: None,
        responses_item_ids: Default::default(),
        responses_item_statuses: Default::default(),
        served_model_id: "hiroute-classifier-diagnostic".into(),
        stream: false,
        instructions: Vec::new(),
        messages: vec![CanonicalMessage {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: CLASSIFIER_DIAGNOSTIC_LATEST_USER.into(),
            }],
            name: None,
        }],
        tools: Vec::new(),
        tool_namespaces: Vec::new(),
        responses_tool_order: Vec::new(),
        web_search: None,
        responses_search_history: Default::default(),
        responses_annotations: Default::default(),
        responses_message_phases: Default::default(),
        responses_internal_chat_message_metadata: Default::default(),
        responses_reasoning_history: Default::default(),
        tool_choice: ToolChoice::None,
        parallel_tool_calls: false,
        requested_reasoning: RequestedReasoningControl::absent(),
        requested_max_output_tokens: None,
        provider_state: Vec::new(),
    }
}

impl CallFailure {
    pub(super) fn diagnostic(self) -> ClassifierDiagnosticError {
        match self {
            Self::Timeout => ClassifierDiagnosticError::Timeout,
            Self::Unavailable => ClassifierDiagnosticError::Unavailable,
            Self::RejectedInput => ClassifierDiagnosticError::RejectedInput,
            Self::InvalidOutput => ClassifierDiagnosticError::InvalidOutput,
            Self::AuthenticationRejected => ClassifierDiagnosticError::AuthenticationRejected,
            Self::RateLimited => ClassifierDiagnosticError::RateLimited,
            Self::EndpointRejected => ClassifierDiagnosticError::EndpointRejected,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_decision_failures_keep_closed_authentication_and_endpoint_causes() {
        let root = std::env::temp_dir().join(format!(
            "hiroute-info-decision-{}-{}",
            std::process::id(),
            crate::server::core_runtime::observation::unix_nanos()
        ));
        let runtime = hiroute_diagnostics::runtime::DiagnosticRuntime::start(
            hiroute_diagnostics::runtime::RuntimeConfig {
                root: root.clone(),
                role: hiroute_diagnostics::event::ProcessRole::Daemon,
                component: hiroute_diagnostics::record::Component::Gateway,
                parent_session_id: None,
                level_override: Some(hiroute_diagnostics::DiagnosticLevel::Info),
            },
        );
        for (status, failure) in [
            (
                StatusCode::UNAUTHORIZED,
                CallFailure::AuthenticationRejected,
            ),
            (StatusCode::BAD_REQUEST, CallFailure::EndpointRejected),
        ] {
            let mut diagnostic = CallDiagnostic::new(runtime.port().handle().context());
            diagnostic.prepared(
                &HeaderMap::new(),
                br#"{"model":"decision-model-preview","input":"private-decision-prompt"}"#,
            );
            diagnostic.protocol(Some(HttpProtocol::Http2));
            diagnostic.response(&HeaderMap::new(), status);
            diagnostic.finish(Some(failure));
        }
        runtime.shutdown();
        let log = std::fs::read_to_string(root.join("daemon/current.jsonl")).unwrap();
        for expected in [
            "authentication_rejected",
            "endpoint_rejected",
            "decision-model-preview",
            "http2",
            "decision_service",
        ] {
            assert!(log.contains(expected), "{expected}: {log}");
        }
        assert!(!log.contains("private-decision-prompt"), "{log}");
        assert!(!log.contains("\"phase\":\"request\""), "{log}");
        std::fs::remove_dir_all(root).unwrap();
    }
}
