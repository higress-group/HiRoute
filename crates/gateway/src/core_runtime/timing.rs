use std::time::Instant;

use hiroute_diagnostics::event::{IngressProtocol as DiagnosticProtocol, RequestOutcome};
use hiroute_gateway_core::runtime::timing::{WaitKind, wait};
use hiroute_gateway_core::transport::{
    GatewayResponseHead, GatewaySession, SessionReuse, TransportError,
};

use super::IngressProtocol;
use super::observation::latency::RequestLatency;

struct RequestGuard {
    latency: RequestLatency,
    outcome: RequestOutcome,
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.latency.finish(self.outcome, self.latency.status());
    }
}

pub(super) async fn process(
    runtime: &super::ProductionGatewayRuntime,
    session: &mut dyn GatewaySession,
) -> Result<SessionReuse, TransportError> {
    let started = Instant::now();
    let request = session.request_head()?;
    let path = request.path_and_query.split('?').next().unwrap_or_default();
    // Readiness and model discovery are control reads, not model requests.
    if request.method == http::Method::GET
        && (matches!(path, "/_hiroute/ready" | "/v1/models")
            || path.strip_prefix(hiroute_domain::QODER_MODEL_BASE_PATH) == Some("/models"))
    {
        return runtime.process_request(session, request).await;
    }
    let protocol = IngressProtocol::from_path(path).map(|p| match p {
        IngressProtocol::Responses => DiagnosticProtocol::OpenAiResponses,
        IngressProtocol::ChatCompletions => DiagnosticProtocol::OpenAiChat,
        IngressProtocol::Messages => DiagnosticProtocol::AnthropicMessages,
    });
    let latency = RequestLatency::new(runtime.observation.diagnostic_context(), protocol, started);
    let mut guard = RequestGuard {
        latency: latency.clone(),
        outcome: RequestOutcome::Cancelled,
    };
    let result = latency.run(runtime.process_request(session, request)).await;
    guard.outcome = match &result {
        Err(TransportError::Cancelled) => RequestOutcome::Cancelled,
        Err(TransportError::DeadlineExceeded) => RequestOutcome::Timeout,
        Err(_) => RequestOutcome::Failed,
        Ok(_) if matches!(latency.status(), Some(408 | 504)) => RequestOutcome::Timeout,
        Ok(_) if latency.status().is_some_and(|status| status >= 400) => RequestOutcome::Failed,
        Ok(_) => RequestOutcome::Completed,
    };
    drop(guard);
    result
}

pub(super) async fn write_head(
    session: &mut dyn GatewaySession,
    head: GatewayResponseHead,
) -> Result<(), TransportError> {
    if let Some(latency) = RequestLatency::current() {
        latency.response_status(head.status.as_u16());
    }
    wait(WaitKind::DownstreamWrite, session.write_response_head(head)).await
}
