//! Safe evidence for a saved decision test and its upstream exchange.

use serde::{Deserialize, Serialize};

use super::ControlOperation;
use crate::{error::EventErrorCode, identity::CorrelationToken};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionConsoleEnd {
    pub operation: ControlOperation,
    pub command_returned: bool,
    pub http_status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<EventErrorCode>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionTestEnd {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_token: Option<CorrelationToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_token: Option<CorrelationToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<DecisionTestFailure>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionTestFailure {
    InvalidConfiguration,
    SourceNotFound,
    SourceLineageMismatch,
    SourceDisabled,
    SourceUnavailable,
    NoUsableCredentials,
    RevisionConflict,
    InUse,
    Timeout,
    AuthenticationRejected,
    RateLimited,
    InputRejected,
    MalformedOutput,
    TransportUnavailable,
    InvalidTestResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionExchangeFailure {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_token: Option<CorrelationToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_token: Option<CorrelationToken>,
    pub phase: DecisionExchangePhase,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionExchangePhase {
    RequestPreparation,
    Credential,
    NetworkRoute,
    Dns,
    ProxyDns,
    Connect,
    ConnectTimeout,
    ProxyConnect,
    ProxyTls,
    ProxyAuthentication,
    ProxyNegotiation,
    UpstreamTls,
    Budget,
    RequestWriteTimeout,
    FirstByteTimeout,
    ResponseIdleTimeout,
    Deadline,
    Cancelled,
    Transport,
    ResponseFraming,
    ResponseFinalize,
    HttpRejected,
    ResponseDecode,
}
