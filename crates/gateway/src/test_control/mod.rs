//! Explicitly enabled, local-only production E2E control surface.
//!
//! This is a test control plane around the real production installer, never a
//! fixture or alternate publication path. Without sealed options, neither its
//! handle nor its listener is constructed.

mod contract;
mod dial;
mod fixture;
#[cfg(all(test, target_os = "linux"))]
mod registered_feed_tls;
mod runtime_state;
mod security;

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use hiroute_gateway_core::transport::{
    GatewayLifecycle, GatewayResponseHead, GatewaySession, SessionReuse, TransportError,
};
use http::header::{CONNECTION, CONTENT_LENGTH, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue, Method, StatusCode};

pub use contract::E2eControlHandle;
pub use fixture::{
    E2E_DIAL_CONFIG_ENV, E2E_DIAL_CONFIG_FILE, TestTlsListener, TestTlsStream,
    sealed_native_candidate, write_dial_config, write_dial_config_with_dns_failures,
};
pub use security::{E2eControlError, E2eControlOptions};

use contract::{ControlRequestEnvelope, ControlResponse};
pub(crate) use dial::E2eDialMap;
use security::E2eControlConfig;

pub const CONTROL_PATH: &str = "/_hiroute/e2e-control/v1";
pub const NONCE_HEADER: &str = "x-hiroute-e2e-control-nonce";
pub const E2E_ATTEMPT_TIMEOUT_MS_ENV: &str = "HIROUTE_E2E_ATTEMPT_TIMEOUT_MS";

const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
const REQUEST_DEADLINE: Duration = Duration::from_secs(5);

pub(crate) fn attempt_timeout_override() -> Option<Duration> {
    let value = std::env::var(E2E_ATTEMPT_TIMEOUT_MS_ENV).ok()?;
    let milliseconds = value
        .parse::<u64>()
        .unwrap_or_else(|_| panic!("{E2E_ATTEMPT_TIMEOUT_MS_ENV} must be a positive integer"));
    assert!(
        milliseconds > 0,
        "{E2E_ATTEMPT_TIMEOUT_MS_ENV} must be a positive integer"
    );
    Some(Duration::from_millis(milliseconds))
}

pub struct E2eControlEndpoint {
    listen: std::net::SocketAddr,
    lifecycle: Arc<E2eControlLifecycle>,
}

impl E2eControlEndpoint {
    pub fn new(
        options: E2eControlOptions,
        dispatcher: E2eControlHandle,
    ) -> Result<Self, E2eControlError> {
        let config = E2eControlConfig::load(options)?;
        let listen = config.listen;
        Ok(Self {
            listen,
            lifecycle: Arc::new(E2eControlLifecycle { config, dispatcher }),
        })
    }

    pub fn listen(&self) -> std::net::SocketAddr {
        self.listen
    }

    pub fn command_handle(&self) -> E2eControlHandle {
        self.lifecycle.dispatcher.clone()
    }

    pub fn listener(&self) -> Arc<impl GatewayLifecycle> {
        Arc::clone(&self.lifecycle)
    }
}

struct E2eControlLifecycle {
    config: E2eControlConfig,
    dispatcher: E2eControlHandle,
}

#[async_trait]
impl GatewayLifecycle for E2eControlLifecycle {
    async fn process(
        &self,
        session: &mut dyn GatewaySession,
    ) -> Result<SessionReuse, TransportError> {
        let request = session.request_head()?;
        if request.method != Method::POST || request.path_and_query.as_ref() != CONTROL_PATH {
            return write_response(
                session,
                ControlResponse::malformed("E2E_CONTROL_ROUTE_NOT_FOUND", StatusCode::NOT_FOUND),
            )
            .await;
        }
        let nonce = request
            .headers
            .get(NONCE_HEADER)
            .and_then(|value| value.to_str().ok());
        if !self.config.authenticates(nonce) {
            return write_response(session, ControlResponse::unauthorized()).await;
        }
        if request
            .headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            != Some("application/json")
        {
            return write_response(
                session,
                ControlResponse::malformed(
                    "E2E_CONTROL_CONTENT_TYPE_INVALID",
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                ),
            )
            .await;
        }
        let Some(content_length) = declared_body_length(&request.headers) else {
            return write_response(
                session,
                ControlResponse::malformed(
                    "E2E_CONTROL_BODY_LENGTH_INVALID",
                    StatusCode::BAD_REQUEST,
                ),
            )
            .await;
        };
        let deadline = Instant::now() + REQUEST_DEADLINE;
        let body = match read_body(session, deadline, content_length).await {
            Ok(body) => body,
            Err(response) => return write_response(session, response).await,
        };
        let request = match serde_json::from_slice::<ControlRequestEnvelope>(&body) {
            Ok(request) => request,
            Err(_) => {
                return write_response(
                    session,
                    ControlResponse::malformed(
                        "E2E_CONTROL_REQUEST_INVALID",
                        StatusCode::BAD_REQUEST,
                    ),
                )
                .await;
            }
        };
        write_response(session, self.dispatcher.execute(request)).await
    }
}

fn declared_body_length(headers: &HeaderMap) -> Option<usize> {
    let length = headers.get(CONTENT_LENGTH)?.to_str().ok()?.parse().ok()?;
    (length > 0 && length <= MAX_BODY_BYTES).then_some(length)
}

async fn read_body(
    session: &mut dyn GatewaySession,
    deadline: Instant,
    declared: usize,
) -> Result<Vec<u8>, ControlResponse> {
    let mut body = Vec::with_capacity(declared);
    loop {
        let chunk =
            session
                .read_request_body_before(deadline)
                .await
                .map_err(|error| match error {
                    TransportError::DeadlineExceeded => ControlResponse::malformed(
                        "E2E_CONTROL_REQUEST_TIMEOUT",
                        StatusCode::REQUEST_TIMEOUT,
                    ),
                    _ => ControlResponse::malformed(
                        "E2E_CONTROL_BODY_READ_FAILED",
                        StatusCode::BAD_REQUEST,
                    ),
                })?;
        let Some(chunk) = chunk else {
            break;
        };
        if body.len().saturating_add(chunk.len()) > declared {
            return Err(ControlResponse::malformed(
                "E2E_CONTROL_BODY_LENGTH_INVALID",
                StatusCode::BAD_REQUEST,
            ));
        }
        body.extend_from_slice(&chunk);
    }
    if body.len() != declared {
        return Err(ControlResponse::malformed(
            "E2E_CONTROL_BODY_LENGTH_INVALID",
            StatusCode::BAD_REQUEST,
        ));
    }
    Ok(body)
}

async fn write_response(
    session: &mut dyn GatewaySession,
    response: ControlResponse,
) -> Result<SessionReuse, TransportError> {
    let status = response.status;
    let body = serde_json::to_vec(&response)
        .map(Bytes::from)
        .map_err(|error| TransportError::Io(error.to_string().into()))?;
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(CONTENT_LENGTH, HeaderValue::from(body.len() as u64));
    headers.insert(CONNECTION, HeaderValue::from_static("close"));
    session
        .write_response_head(GatewayResponseHead { status, headers })
        .await?;
    session.write_response_body(body, true).await?;
    Ok(SessionReuse::Close)
}
