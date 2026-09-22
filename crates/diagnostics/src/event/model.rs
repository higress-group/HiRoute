//! Model request, attempt, fallback and commit-fence events.

use serde::{Deserialize, Serialize};

use crate::identity::CorrelationToken;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestBegin {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_token: Option<CorrelationToken>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestEnd {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_token: Option<CorrelationToken>,
    pub outcome: RequestOutcome,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestOutcome {
    Completed,
    Failed,
    Cancelled,
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteSelected {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_token: Option<CorrelationToken>,
    /// Ordinal of the selected candidate in the plan, not a display name or endpoint.
    pub candidate_ordinal: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptBegin {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_token: Option<CorrelationToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_token: Option<CorrelationToken>,
    pub attempt_index: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptEnd {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_token: Option<CorrelationToken>,
    pub outcome: AttemptOutcome,
    pub commit: CommitState,
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptOutcome {
    Completed,
    Failed,
    Cancelled,
    Timeout,
}

/// How far a request got before it ended. Distinguishes a pre-send failure from an
/// uncertain post-transport result and a semantically committed result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitState {
    BeforeTransport,
    TransportCommitted,
    SemanticCommitted,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fallback {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_token: Option<CorrelationToken>,
    pub from_attempt_index: u64,
    pub reason: FallbackReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackReason {
    PolicyEligibleFailure,
    CandidateExcluded,
    ConnectorUnavailable,
    RateLimited,
    UpstreamFailure,
    OtherStableReason,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticCommit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_token: Option<CorrelationToken>,
    pub state: CommitState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestCancel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_token: Option<CorrelationToken>,
    pub phase: CommitState,
    pub accepted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestTimeout {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_token: Option<CorrelationToken>,
    pub phase: CommitState,
}

/// Bounded stage timings inside request handling. Only stable kinds and durations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelStage {
    pub stage: ModelStageKind,
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<IngressProtocol>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStageKind {
    IngressProtocol,
    Parse,
    Externalize,
    Prevalidate,
    Plan,
    RuntimeExclusion,
    Connect,
    Ttfb,
    CommitFence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngressProtocol {
    OpenAiChat,
    OpenAiResponses,
    AnthropicMessages,
    Unknown,
}

/// Stable reason a runtime candidate was excluded, never a free-form message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExclusionReason {
    AuthUnavailable,
    QuotaExhausted,
    DisabledByUser,
    UnsupportedProtocol,
    UnavailableConnector,
    Other,
}

/// Wire-shape facts only: never header values, URLs, model names or response bodies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamWire {
    pub request_token: Option<CorrelationToken>,
    pub phase: UpstreamWirePhase,
    pub bearer_present: bool,
    pub api_key_present: bool,
    pub authorization_bytes: u64,
    pub body_bytes: Option<u64>,
    pub http_status: Option<u16>,
    pub content_type: WireContentType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamWirePhase {
    Request,
    Response,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireContentType {
    Json,
    EventStream,
    Html,
    Other,
    Missing,
}
