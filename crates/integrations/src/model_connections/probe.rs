use std::{
    collections::BTreeSet,
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use hiroute_application_api::{
    ModelConnectionAuthenticationStatusV1, ModelConnectionCheckIssueV1,
    ModelConnectionDirectoryStatusV1, ModelConnectionInferenceStatusV1,
    ModelConnectionProtocolStatusV1, ModelConnectionReachabilityV1,
};
use hiroute_domain::{GatewayAuthenticationSemanticsV1, ProtectedSecret};
use reqwest::{
    blocking::Client,
    header::{HOST, HeaderName, HeaderValue},
    redirect::Policy,
};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;
use zeroize::Zeroizing;

use super::NormalizedModelConnectionTargetV1;

pub const DEFAULT_MODEL_CONNECTION_TOTAL_TIMEOUT: Duration = Duration::from_secs(15);
pub const DEFAULT_MODEL_CONNECTION_RESPONSE_LIMIT: usize = 2 * 1024 * 1024;
pub const DEFAULT_MODEL_CONNECTION_PAGE_LIMIT: u8 = 3;
pub const DEFAULT_MODEL_CONNECTION_MODEL_LIMIT: usize = 1_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelConnectionProbeLimitsV1 {
    pub total_timeout: Duration,
    pub response_bytes: usize,
    pub pages: u8,
    pub models: usize,
}

impl Default for ModelConnectionProbeLimitsV1 {
    fn default() -> Self {
        Self {
            total_timeout: DEFAULT_MODEL_CONNECTION_TOTAL_TIMEOUT,
            response_bytes: DEFAULT_MODEL_CONNECTION_RESPONSE_LIMIT,
            pages: DEFAULT_MODEL_CONNECTION_PAGE_LIMIT,
            models: DEFAULT_MODEL_CONNECTION_MODEL_LIMIT,
        }
    }
}

impl ModelConnectionProbeLimitsV1 {
    pub(crate) fn valid(self) -> bool {
        !self.total_timeout.is_zero()
            && self.total_timeout <= DEFAULT_MODEL_CONNECTION_TOTAL_TIMEOUT
            && self.response_bytes > 0
            && self.response_bytes <= DEFAULT_MODEL_CONNECTION_RESPONSE_LIMIT
            && self.pages > 0
            && self.pages <= DEFAULT_MODEL_CONNECTION_PAGE_LIMIT
            && self.models > 0
            && self.models <= DEFAULT_MODEL_CONNECTION_MODEL_LIMIT
    }
}

#[derive(Clone, Default)]
pub struct ModelConnectionProbeCancellationV1(Arc<AtomicBool>);

impl ModelConnectionProbeCancellationV1 {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub struct ModelConnectionProbeCredentialV1<'a> {
    pub authentication: &'a GatewayAuthenticationSemanticsV1,
    pub secret: &'a ProtectedSecret,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelDirectoryHttpResponseV1 {
    pub status: u16,
    pub body: Vec<u8>,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ModelDirectoryTransportErrorV1 {
    #[error("model directory request timed out")]
    Timeout,
    #[error("model directory transport is unavailable")]
    Unavailable,
    #[error("model directory response exceeded its limit")]
    ResponseTooLarge,
    #[error("model directory authentication value is invalid")]
    InvalidAuthorization,
}

pub trait ModelDirectoryTransportV1: Send + Sync {
    fn infer(
        &self,
        _target: &NormalizedModelConnectionTargetV1,
        _model: &str,
        _credential: Option<ModelConnectionProbeCredentialV1<'_>>,
        _limits: ModelConnectionProbeLimitsV1,
    ) -> Result<ModelDirectoryHttpResponseV1, ModelDirectoryTransportErrorV1> {
        Err(ModelDirectoryTransportErrorV1::Unavailable)
    }

    fn get(
        &self,
        target: &NormalizedModelConnectionTargetV1,
        query: Option<&str>,
        credential: Option<ModelConnectionProbeCredentialV1<'_>>,
        timeout: Duration,
        response_limit: usize,
    ) -> Result<ModelDirectoryHttpResponseV1, ModelDirectoryTransportErrorV1>;
}

impl<T> ModelDirectoryTransportV1 for Arc<T>
where
    T: ModelDirectoryTransportV1 + ?Sized,
{
    fn infer(
        &self,
        target: &NormalizedModelConnectionTargetV1,
        model: &str,
        credential: Option<ModelConnectionProbeCredentialV1<'_>>,
        limits: ModelConnectionProbeLimitsV1,
    ) -> Result<ModelDirectoryHttpResponseV1, ModelDirectoryTransportErrorV1> {
        (**self).infer(target, model, credential, limits)
    }

    fn get(
        &self,
        target: &NormalizedModelConnectionTargetV1,
        query: Option<&str>,
        credential: Option<ModelConnectionProbeCredentialV1<'_>>,
        timeout: Duration,
        response_limit: usize,
    ) -> Result<ModelDirectoryHttpResponseV1, ModelDirectoryTransportErrorV1> {
        (**self).get(target, query, credential, timeout, response_limit)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ReqwestModelDirectoryTransportV1;

impl ModelDirectoryTransportV1 for ReqwestModelDirectoryTransportV1 {
    fn infer(
        &self,
        target: &NormalizedModelConnectionTargetV1,
        model: &str,
        credential: Option<ModelConnectionProbeCredentialV1<'_>>,
        limits: ModelConnectionProbeLimitsV1,
    ) -> Result<ModelDirectoryHttpResponseV1, ModelDirectoryTransportErrorV1> {
        super::inference::send(target, model, credential, limits)
    }

    fn get(
        &self,
        target: &NormalizedModelConnectionTargetV1,
        query: Option<&str>,
        credential: Option<ModelConnectionProbeCredentialV1<'_>>,
        timeout: Duration,
        response_limit: usize,
    ) -> Result<ModelDirectoryHttpResponseV1, ModelDirectoryTransportErrorV1> {
        let client = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .connect_timeout(timeout)
            .timeout(timeout)
            .build()
            .map_err(|_| ModelDirectoryTransportErrorV1::Unavailable)?;
        let mut request = client.get(target.inventory_url(query)).header(
            HOST,
            HeaderValue::from_str(target.host_header())
                .map_err(|_| ModelDirectoryTransportErrorV1::Unavailable)?,
        );
        for (name, value) in target.directory_headers() {
            request = request.header(name.clone(), value.clone());
        }
        if let Some(credential) = credential {
            request = apply_authorization(request, credential)?;
        }
        let response = request.send().map_err(|error| {
            if error.is_timeout() {
                ModelDirectoryTransportErrorV1::Timeout
            } else {
                ModelDirectoryTransportErrorV1::Unavailable
            }
        })?;
        let status = response.status().as_u16();
        if !(200..=299).contains(&status) {
            return Ok(ModelDirectoryHttpResponseV1 {
                status,
                body: Vec::new(),
                truncated: false,
            });
        }
        let mut body = Vec::new();
        response
            .take((response_limit as u64).saturating_add(1))
            .read_to_end(&mut body)
            .map_err(|_| ModelDirectoryTransportErrorV1::Unavailable)?;
        if body.len() > response_limit {
            body.clear();
            return Ok(ModelDirectoryHttpResponseV1 {
                status,
                body,
                truncated: true,
            });
        }
        Ok(ModelDirectoryHttpResponseV1 {
            status,
            body,
            truncated: false,
        })
    }
}

pub(super) fn apply_authorization(
    request: reqwest::blocking::RequestBuilder,
    credential: ModelConnectionProbeCredentialV1<'_>,
) -> Result<reqwest::blocking::RequestBuilder, ModelDirectoryTransportErrorV1> {
    match credential.authentication {
        GatewayAuthenticationSemanticsV1::None => {
            Err(ModelDirectoryTransportErrorV1::InvalidAuthorization)
        }
        GatewayAuthenticationSemanticsV1::Bearer => {
            let mut bytes = Zeroizing::new(Vec::with_capacity(
                "Bearer ".len() + credential.secret.expose().len(),
            ));
            bytes.extend_from_slice(b"Bearer ");
            bytes.extend_from_slice(credential.secret.expose());
            let value = HeaderValue::from_bytes(bytes.as_slice())
                .map_err(|_| ModelDirectoryTransportErrorV1::InvalidAuthorization)?;
            Ok(request.header(reqwest::header::AUTHORIZATION, value))
        }
        GatewayAuthenticationSemanticsV1::ApiKeyHeader { header } => {
            let name = HeaderName::from_bytes(header.as_bytes())
                .map_err(|_| ModelDirectoryTransportErrorV1::InvalidAuthorization)?;
            let value = HeaderValue::from_bytes(credential.secret.expose())
                .map_err(|_| ModelDirectoryTransportErrorV1::InvalidAuthorization)?;
            Ok(request.header(name, value))
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModelDirectoryProbeObservationV1 {
    pub reachability: ModelConnectionReachabilityV1,
    pub authentication: ModelConnectionAuthenticationStatusV1,
    pub directory: ModelConnectionDirectoryStatusV1,
    pub protocol: ModelConnectionProtocolStatusV1,
    pub inference: ModelConnectionInferenceStatusV1,
    pub model_ids: Vec<String>,
    pub invalid_model_count: u32,
    pub pages_read: u8,
    pub issues: Vec<ModelConnectionCheckIssueV1>,
}

impl ModelDirectoryProbeObservationV1 {
    pub fn missing_credential() -> Self {
        Self {
            reachability: ModelConnectionReachabilityV1::NotRun,
            authentication: ModelConnectionAuthenticationStatusV1::NotRun,
            directory: ModelConnectionDirectoryStatusV1::NotRun,
            protocol: ModelConnectionProtocolStatusV1::Selected,
            inference: ModelConnectionInferenceStatusV1::NotRun,
            model_ids: Vec::new(),
            invalid_model_count: 0,
            pages_read: 0,
            issues: vec![issue(
                "NEEDS_CREDENTIAL",
                "model_connections.needs_credential",
                true,
            )],
        }
    }

    pub fn timed_out() -> Self {
        transport_failure(
            ModelDirectoryTransportErrorV1::Timeout,
            BTreeSet::new(),
            0,
            0,
        )
    }
}

pub fn probe_model_directory<T: ModelDirectoryTransportV1>(
    transport: &T,
    target: &NormalizedModelConnectionTargetV1,
    credential: Option<ModelConnectionProbeCredentialV1<'_>>,
    cancellation: &ModelConnectionProbeCancellationV1,
    limits: ModelConnectionProbeLimitsV1,
) -> ModelDirectoryProbeObservationV1 {
    if !limits.valid() {
        return invalid_limits();
    }
    if cancellation.is_cancelled() {
        return cancelled(Vec::new(), 0, 0);
    }
    if target.inventory_path.is_none() {
        let mut observation = ModelDirectoryProbeObservationV1::missing_credential();
        observation.authentication = if credential.is_some() {
            ModelConnectionAuthenticationStatusV1::Unknown
        } else {
            ModelConnectionAuthenticationStatusV1::NotRequired
        };
        observation.issues.clear();
        return observation;
    }
    let deadline = Instant::now() + limits.total_timeout;
    let mut query = None;
    let mut models = BTreeSet::new();
    let mut invalid = 0_u32;
    let mut pages = 0_u8;
    let mut bytes_left = limits.response_bytes;
    let mut issues = Vec::new();
    let auth_on_success = if credential.is_some() {
        ModelConnectionAuthenticationStatusV1::Verified
    } else {
        ModelConnectionAuthenticationStatusV1::NotRequired
    };

    loop {
        if cancellation.is_cancelled() {
            return cancelled(models.into_iter().collect(), invalid, pages);
        }
        let timeout = deadline.saturating_duration_since(Instant::now());
        if timeout.is_zero() {
            return transport_failure(
                ModelDirectoryTransportErrorV1::Timeout,
                models,
                invalid,
                pages,
            );
        }
        let response = match transport.get(
            target,
            query.as_deref(),
            credential
                .as_ref()
                .map(|value| ModelConnectionProbeCredentialV1 {
                    authentication: value.authentication,
                    secret: value.secret,
                }),
            timeout,
            bytes_left,
        ) {
            Ok(response) => response,
            Err(error) => return transport_failure(error, models, invalid, pages),
        };
        pages = pages.saturating_add(1);
        bytes_left = bytes_left.saturating_sub(response.body.len());
        match response.status {
            200..=299 if response.truncated => {
                return ModelDirectoryProbeObservationV1 {
                    reachability: ModelConnectionReachabilityV1::Reachable,
                    authentication: auth_on_success,
                    directory: ModelConnectionDirectoryStatusV1::Partial,
                    protocol: ModelConnectionProtocolStatusV1::Selected,
                    inference: ModelConnectionInferenceStatusV1::NotRun,
                    model_ids: models.into_iter().collect(),
                    invalid_model_count: invalid,
                    pages_read: pages,
                    issues: vec![issue(
                        "RESPONSE_TOO_LARGE",
                        "model_connections.response_too_large",
                        false,
                    )],
                };
            }
            200..=299 => {}
            401 => {
                return http_failure(
                    ModelConnectionAuthenticationStatusV1::Rejected,
                    "AUTH_REJECTED",
                    "model_connections.auth_rejected",
                    false,
                    pages,
                );
            }
            403 => {
                return http_failure(
                    ModelConnectionAuthenticationStatusV1::Unknown,
                    "DIRECTORY_FORBIDDEN",
                    "model_connections.directory_forbidden",
                    false,
                    pages,
                );
            }
            404 => {
                return http_failure(
                    if credential.is_some() {
                        ModelConnectionAuthenticationStatusV1::Unknown
                    } else {
                        ModelConnectionAuthenticationStatusV1::NotRequired
                    },
                    "DIRECTORY_UNAVAILABLE",
                    "model_connections.directory_unavailable",
                    false,
                    pages,
                );
            }
            429 => {
                return http_failure(
                    ModelConnectionAuthenticationStatusV1::Unknown,
                    "RATE_LIMITED",
                    "model_connections.rate_limited",
                    true,
                    pages,
                );
            }
            300..=399 => {
                return http_failure(
                    ModelConnectionAuthenticationStatusV1::Unknown,
                    "REDIRECT_REJECTED",
                    "model_connections.redirect_rejected",
                    false,
                    pages,
                );
            }
            _ => {
                return http_failure(
                    ModelConnectionAuthenticationStatusV1::Unknown,
                    "DIRECTORY_FAILED",
                    "model_connections.directory_failed",
                    true,
                    pages,
                );
            }
        }

        let page = match parse_page(&response.body) {
            Ok(page) => page,
            Err(()) => {
                return ModelDirectoryProbeObservationV1 {
                    reachability: ModelConnectionReachabilityV1::Reachable,
                    authentication: auth_on_success,
                    directory: ModelConnectionDirectoryStatusV1::Invalid,
                    protocol: ModelConnectionProtocolStatusV1::Selected,
                    inference: ModelConnectionInferenceStatusV1::NotRun,
                    model_ids: models.into_iter().collect(),
                    invalid_model_count: invalid,
                    pages_read: pages,
                    issues: vec![issue(
                        "DIRECTORY_INVALID",
                        "model_connections.directory_invalid",
                        true,
                    )],
                };
            }
        };
        invalid = invalid.saturating_add(page.invalid);
        let mut limit_reached = false;
        for model in page.models {
            if models.len() == limits.models {
                limit_reached = true;
                break;
            }
            models.insert(model);
        }
        if models.len() == limits.models && page.next_query.is_some() {
            limit_reached = true;
        }
        if limit_reached {
            issues.push(issue(
                "MODEL_LIMIT_REACHED",
                "model_connections.model_limit_reached",
                false,
            ));
        }
        if invalid > 0 && !issues.iter().any(|item| item.code == "DIRECTORY_PARTIAL") {
            issues.push(issue(
                "DIRECTORY_PARTIAL",
                "model_connections.directory_partial",
                true,
            ));
        }
        let has_more = page.next_query.is_some() && !limit_reached;
        if !has_more {
            let directory = if limit_reached || invalid > 0 {
                ModelConnectionDirectoryStatusV1::Partial
            } else if models.is_empty() {
                ModelConnectionDirectoryStatusV1::Empty
            } else {
                ModelConnectionDirectoryStatusV1::Available
            };
            return ModelDirectoryProbeObservationV1 {
                reachability: ModelConnectionReachabilityV1::Reachable,
                authentication: auth_on_success,
                directory,
                protocol: ModelConnectionProtocolStatusV1::Selected,
                inference: ModelConnectionInferenceStatusV1::NotRun,
                model_ids: models.into_iter().collect(),
                invalid_model_count: invalid,
                pages_read: pages,
                issues,
            };
        }
        if pages == limits.pages || bytes_left == 0 {
            issues.push(issue(
                "DIRECTORY_TRUNCATED",
                "model_connections.directory_truncated",
                true,
            ));
            return ModelDirectoryProbeObservationV1 {
                reachability: ModelConnectionReachabilityV1::Reachable,
                authentication: auth_on_success,
                directory: ModelConnectionDirectoryStatusV1::Partial,
                protocol: ModelConnectionProtocolStatusV1::Selected,
                inference: ModelConnectionInferenceStatusV1::NotRun,
                model_ids: models.into_iter().collect(),
                invalid_model_count: invalid,
                pages_read: pages,
                issues,
            };
        }
        query = page.next_query;
    }
}

struct ParsedPage {
    models: Vec<String>,
    invalid: u32,
    next_query: Option<String>,
}

fn parse_page(bytes: &[u8]) -> Result<ParsedPage, ()> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| ())?;
    let object = value.as_object().ok_or(())?;
    let bailian = object.get("output").and_then(Value::as_object);
    let items = match bailian {
        Some(output) => output.get("models").and_then(Value::as_array).ok_or(())?,
        None => object
            .get("data")
            .or_else(|| object.get("models"))
            .and_then(Value::as_array)
            .ok_or(())?,
    };
    let mut models = Vec::new();
    let mut invalid = 0_u32;
    for item in items {
        let id = item
            .as_object()
            .and_then(|entry| entry.get(if bailian.is_some() { "model" } else { "id" }))
            .and_then(Value::as_str);
        if let Some(id) = id.filter(|id| valid_model_id(id)) {
            models.push(id.to_owned());
        } else {
            invalid = invalid.saturating_add(1);
        }
    }
    let next_query = if let Some(output) = bailian {
        let page_no = output.get("page_no").and_then(Value::as_u64).ok_or(())?;
        let page_size = output.get("page_size").and_then(Value::as_u64).ok_or(())?;
        let total = output.get("total").and_then(Value::as_u64).ok_or(())?;
        if page_no == 0 || page_size == 0 {
            return Err(());
        }
        let covered = page_no.checked_mul(page_size).ok_or(())?;
        if covered < total {
            let next_page = page_no.checked_add(1).ok_or(())?;
            Some(format!("page_no={next_page}&page_size={page_size}"))
        } else {
            None
        }
    } else if object
        .get("has_more")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let cursor = object
            .get("last_id")
            .and_then(Value::as_str)
            .filter(|id| valid_model_id(id))
            .map(str::to_owned)
            .or_else(|| models.last().cloned())
            .ok_or(())?;
        Some(format!("after={}", percent_encode_query(&cursor)))
    } else {
        None
    };
    Ok(ParsedPage {
        models,
        invalid,
        next_query,
    })
}

fn valid_model_id(value: &str) -> bool {
    hiroute_domain::valid_upstream_model_id(value)
}

fn percent_encode_query(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn issue(code: &str, message_key: &str, retryable: bool) -> ModelConnectionCheckIssueV1 {
    ModelConnectionCheckIssueV1 {
        code: code.into(),
        message_key: message_key.into(),
        retryable,
    }
}

fn invalid_limits() -> ModelDirectoryProbeObservationV1 {
    ModelDirectoryProbeObservationV1 {
        reachability: ModelConnectionReachabilityV1::NotRun,
        authentication: ModelConnectionAuthenticationStatusV1::NotRun,
        directory: ModelConnectionDirectoryStatusV1::NotRun,
        protocol: ModelConnectionProtocolStatusV1::Selected,
        inference: ModelConnectionInferenceStatusV1::NotRun,
        model_ids: Vec::new(),
        invalid_model_count: 0,
        pages_read: 0,
        issues: vec![issue(
            "INVALID_CHECK_LIMITS",
            "model_connections.invalid_check_limits",
            false,
        )],
    }
}

fn cancelled(models: Vec<String>, invalid: u32, pages: u8) -> ModelDirectoryProbeObservationV1 {
    ModelDirectoryProbeObservationV1 {
        reachability: if pages == 0 {
            ModelConnectionReachabilityV1::NotRun
        } else {
            ModelConnectionReachabilityV1::Reachable
        },
        authentication: ModelConnectionAuthenticationStatusV1::NotRun,
        directory: if pages == 0 {
            ModelConnectionDirectoryStatusV1::NotRun
        } else {
            ModelConnectionDirectoryStatusV1::Partial
        },
        protocol: ModelConnectionProtocolStatusV1::Selected,
        inference: ModelConnectionInferenceStatusV1::NotRun,
        model_ids: models,
        invalid_model_count: invalid,
        pages_read: pages,
        issues: vec![issue(
            "CHECK_CANCELLED",
            "model_connections.check_cancelled",
            true,
        )],
    }
}

fn transport_failure(
    error: ModelDirectoryTransportErrorV1,
    models: BTreeSet<String>,
    invalid: u32,
    pages: u8,
) -> ModelDirectoryProbeObservationV1 {
    let (code, key, retryable) = match error {
        ModelDirectoryTransportErrorV1::Timeout => {
            ("CHECK_TIMEOUT", "model_connections.check_timeout", true)
        }
        ModelDirectoryTransportErrorV1::ResponseTooLarge => (
            "RESPONSE_TOO_LARGE",
            "model_connections.response_too_large",
            false,
        ),
        ModelDirectoryTransportErrorV1::InvalidAuthorization => (
            "AUTH_INPUT_INVALID",
            "model_connections.auth_input_invalid",
            true,
        ),
        ModelDirectoryTransportErrorV1::Unavailable => (
            "TRANSPORT_FAILED",
            "model_connections.transport_failed",
            true,
        ),
    };
    ModelDirectoryProbeObservationV1 {
        reachability: ModelConnectionReachabilityV1::TransportFailed,
        authentication: ModelConnectionAuthenticationStatusV1::NotRun,
        directory: ModelConnectionDirectoryStatusV1::NotRun,
        protocol: ModelConnectionProtocolStatusV1::Selected,
        inference: ModelConnectionInferenceStatusV1::NotRun,
        model_ids: models.into_iter().collect(),
        invalid_model_count: invalid,
        pages_read: pages,
        issues: vec![issue(code, key, retryable)],
    }
}

fn http_failure(
    authentication: ModelConnectionAuthenticationStatusV1,
    code: &str,
    key: &str,
    retryable: bool,
    pages: u8,
) -> ModelDirectoryProbeObservationV1 {
    ModelDirectoryProbeObservationV1 {
        reachability: ModelConnectionReachabilityV1::Reachable,
        authentication,
        directory: ModelConnectionDirectoryStatusV1::Unavailable,
        protocol: ModelConnectionProtocolStatusV1::Selected,
        inference: ModelConnectionInferenceStatusV1::NotRun,
        model_ids: Vec::new(),
        invalid_model_count: 0,
        pages_read: pages,
        issues: vec![issue(code, key, retryable)],
    }
}

#[cfg(test)]
mod page_tests {
    use super::*;

    #[test]
    fn parses_bailian_model_directory_and_page_number() {
        let page = parse_page(
            br#"{"success":true,"output":{"total":21,"page_no":1,"page_size":20,"models":[{"model":"qwen3-max-2026-01-23"}]}}"#,
        )
        .unwrap();
        assert_eq!(page.models, ["qwen3-max-2026-01-23"]);
        assert_eq!(page.invalid, 0);
        assert_eq!(page.next_query.as_deref(), Some("page_no=2&page_size=20"));
    }

    #[test]
    fn openai_cursor_shape_remains_supported() {
        let page =
            parse_page(br#"{"data":[{"id":"model-1"}],"has_more":true,"last_id":"model-1"}"#)
                .unwrap();
        assert_eq!(page.models, ["model-1"]);
        assert_eq!(page.next_query.as_deref(), Some("after=model-1"));
    }

    #[test]
    fn pagination_without_a_safe_continuation_is_invalid() {
        assert!(parse_page(br#"{"data":[],"has_more":true}"#).is_err());
        assert!(
            parse_page(br#"{"output":{"total":2,"page_no":0,"page_size":1,"models":[]}}"#).is_err()
        );
        assert!(
            parse_page(
                br#"{"output":{"total":1,"page_no":1,"page_size":1},"data":[{"id":"not-a-bailian-model"}]}"#
            )
            .is_err()
        );
    }
}
