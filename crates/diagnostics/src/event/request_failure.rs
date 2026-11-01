//! Closed execution errors and numeric allocation evidence; no raw error messages.
use serde::{Deserialize, Serialize};

use crate::identity::CorrelationToken;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestExecutionFailure {
    pub request_token: Option<CorrelationToken>,
    pub source: ExecutionFailureSource,
    pub reason: ExecutionFailureReason,
    pub request: ExecutionRequestShape,
    pub attempt_started: bool,
    pub response_started: bool,
    /// The last refused reservation, which may also belong to optional work.
    pub last_budget_rejection: Option<ExecutionBudgetRejection>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRequestShape {
    pub raw_body_bytes: u64,
    pub canonical_messages: u64,
    pub tools: u64,
    pub replay_disk_backed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionFailureSource {
    Body,
    Attempt,
    Provider,
    Transport,
    Selection,
    Filter,
    Lifecycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionFailureReason {
    MemoryBudgetExceeded,
    BodyLimitExceeded,
    InvalidBodyState,
    InvalidMetadata,
    Unsupported,
    Deadline,
    Cancelled,
    CleanupTimeout,
    OperationPanic,
    ProviderProtocol,
    ProviderAuthority,
    ProviderRuntimeState,
    ProviderDns,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMemoryRole {
    RawRequest,
    ModelIrBacking,
    AttemptWire,
    Retry,
    ResponsePrefix,
    SseFrame,
    SemanticState,
    OutputQueue,
    TransportInflight,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionBudgetRejection {
    pub role: ExecutionMemoryRole,
    pub requested_bytes: u64,
    pub stream_live_bytes: u64,
    pub stream_limit_bytes: u64,
    pub worker_live_bytes: u64,
    pub worker_limit_bytes: u64,
    pub process_live_bytes: u64,
    pub process_limit_bytes: u64,
}
