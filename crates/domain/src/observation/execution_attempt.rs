//! Typed credential, runtime, attempt, commit, usage, and finish vocabulary.

use serde::{Deserialize, Serialize};

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptTransportFactV1 {
    pub connect_micros: Option<u64>,
    pub request_write_micros: Option<u64>,
    pub upstream_ttfb_micros: Option<u64>,
    pub last_upstream_progress_micros_from_start: Option<u64>,
    pub local_read_suppressed_micros: u64,
    pub upstream_body_bytes: u64,
    pub timeout_kind: Option<AttemptTimeoutKindV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptCommitFactV1 {
    pub upstream_request: CommitFenceV1,
    pub downstream_headers: CommitFenceV1,
    pub downstream_semantic: CommitFenceV1,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialLeaseOutcomeV1 {
    Leased,
    Exhausted,
    AuthorityError,
    SourceDisabled,
    CredentialDisabled,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeOperationV1 {
    ReadExact,
    CompareAndSwapExact,
    AcquireProbeLeaseExact,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKeyScopeV1 {
    Binding,
    Credential,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeHealthV1 {
    Active,
    Disabled,
    CoolingDown,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeOperationOutcomeV1 {
    Ok,
    Applied,
    Acquired,
    Busy,
    Conflict,
    AuthorityError,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptOutcomeV1 {
    Accepted,
    Rejected,
    PostcommitTransportFailed,
    PostcommitCancelled,
    FailedBeforeTransportAcceptance,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptDispositionV1 {
    Accept,
    Continue,
    Terminate,
    Unknown,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptTimeoutKindV1 {
    Connect,
    RequestWrite,
    FirstByte,
    StreamIdle,
    AttemptDeadline,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitFenceV1 {
    Clear,
    WriteStartedMayHaveCommitted,
    WriteConfirmed,
    Unknown,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptStreamOutcomeV1 {
    NotStarted,
    CompletedEos,
    AbortedBeforeSemanticCommit,
    StreamStartedNoRetry,
    Unknown,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptDownstreamOutcomeV1 {
    NotStarted,
    Completed,
    Failed,
    Cancelled,
    Unknown,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptCleanupOutcomeV1 {
    Completed,
    Failed,
    Unknown,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptTerminationReasonV1 {
    FallbackComplete,
    AcceptedEos,
    TerminatedResponseComplete,
    PreexchangeFailure,
    AttemptFailure,
    StreamStartedNoRetry,
    Cancelled,
    DownstreamFailure,
    ProviderReleaseFailure,
    ProviderEncoderFailure,
    FilterFailure,
    CleanupFailure,
    RequestTerminal,
    Unknown,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticCommitBoundaryV1 {
    FullFrameTransportAccepted,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSourceV1 {
    AcceptedCanonicalModelEvent,
    ProviderCompletion,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageProvenanceV1 {
    Reported,
    Estimated,
    Unknown,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheStatusV1 {
    ConfirmedUsage,
    Unknown,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionRequestOutcomeV1 {
    Accepted,
    Failed,
    Cancelled,
    PostcommitPartial,
    PostcommitTransportFailed,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptFinishedFactV1 {
    pub ordinal: u32,
    pub stable_binding_id: String,
    pub outcome: AttemptOutcomeV1,
    pub error_class: Option<String>,
    pub retryable: Option<bool>,
    pub duration_micros: u64,
    pub disposition: AttemptDispositionV1,
    pub provider_http_status: Option<u16>,
    pub provider_code: Option<String>,
    pub provider_request_id: Option<String>,
    pub retry_after_millis: Option<u64>,
    pub reset_after_millis: Option<u64>,
    pub provider_readiness: Option<String>,
    pub provider_model_event: Option<String>,
    pub time_to_first_model_event_micros: Option<u64>,
    pub provider_ended_micros_from_start: Option<u64>,
    pub transport: AttemptTransportFactV1,
    pub commits: AttemptCommitFactV1,
    pub stream_outcome: AttemptStreamOutcomeV1,
    pub downstream_outcome: AttemptDownstreamOutcomeV1,
    pub cleanup_outcome: AttemptCleanupOutcomeV1,
    pub termination_reason: AttemptTerminationReasonV1,
}
