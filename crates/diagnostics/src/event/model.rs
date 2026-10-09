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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_token: Option<CorrelationToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_model: Option<NativeModelId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_protocol: Option<IngressProtocol>,
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
    pub reasoning_fields_removed: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_protocol: Option<WireHttpProtocol>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_error: Option<WireProviderError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commits: Option<AttemptWireCommits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_model: Option<NativeModelId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_reasoning: Option<WireRequestReasoning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_request_token: Option<CorrelationToken>,
}

/// Counts only; never records history, signatures or provider error bodies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReasoningCleanup {
    pub request_token: Option<CorrelationToken>,
    pub attempt_token: Option<CorrelationToken>,
    pub attempt_index: u64,
    pub binding_token: Option<CorrelationToken>,
    pub ingress_protocol: super::IngressProtocol,
    pub upstream_protocol: super::IngressProtocol,
    pub reason: ReasoningCleanupReason,
    pub fields_removed_so_far: u64,
    pub cleaned_prefix_len: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningCleanupReason {
    RequestProtocolProjection,
    ContextBreakRetry,
    SuccessfulPrefixReuse,
    ResponseProtocolProjection,
}

/// Closed categories only: never serializes provider messages, payloads or field values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseFailure {
    pub request_token: Option<CorrelationToken>,
    pub attempt_index: u64,
    pub stage: ResponseFailureStage,
    pub reason: ResponseFailureReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFailureStage {
    NativeProjection,
    Decode,
    Render,
    PrefixBuffer,
    ProviderStream,
    EndOfStream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFailureReason {
    ResourceLimit,
    InvalidJson,
    InvalidSse,
    InvalidField,
    InvalidLifecycle,
    MissingTerminal,
    DuplicateTerminal,
    Unrepresentable,
    NoSemanticOutput,
    ProviderRejected,
    OtherProtocol,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_attempt_index: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_attempt_token: Option<CorrelationToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_binding_token: Option<CorrelationToken>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackReason {
    PolicyEligibleFailure,
    CandidateExcluded,
    ConnectorUnavailable,
    RateLimited,
    UpstreamFailure,
    AuthenticationRejected,
    InputRejected,
    Timeout,
    InvalidOutput,
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

/// Wire-shape facts and bounded model/control fields: never raw header values,
/// URLs, prompts, provider messages or response bodies.
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
    pub request_reasoning: Option<WireRequestReasoning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_kind: Option<WireRequestKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_token: Option<CorrelationToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_index: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_model: Option<NativeModelId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_request_token: Option<CorrelationToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_protocol: Option<WireHttpProtocol>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_error: Option<WireProviderError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireRequestReasoning {
    pub responses_effort: Option<WireReasoningEffort>,
    pub chat_effort: Option<WireReasoningEffort>,
    pub messages_effort: Option<WireReasoningEffort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages_thinking: Option<WireThinkingType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages_budget_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_thinking: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_enabled: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
    Ultra,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamWirePhase {
    Request,
    Response,
    Failure,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireRequestKind {
    ModelInference,
    DecisionService,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireHttpProtocol {
    Http1,
    Http2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireThinkingType {
    Enabled,
    Disabled,
    Adaptive,
    Other,
}

/// Closed attribution only. Provider error text and arbitrary codes never enter logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireProviderError {
    ThinkingBudgetRejected,
    AuthenticationRejected,
    RateLimited,
    EndpointRejected,
    InputRejected,
    UpstreamUnavailable,
    InvalidOutput,
    Timeout,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptWireCommits {
    pub upstream_request: WireCommitState,
    pub downstream_headers: WireCommitState,
    /// Any response body byte, including a failure event, closes transparent replay.
    pub downstream_body: WireCommitState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireCommitState {
    Clear,
    Committed,
    Poisoned,
}

/// Only a native identifier from the selected encoder may construct this field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct NativeModelId(String);

impl NativeModelId {
    pub fn new(value: &str) -> Option<Self> {
        (!value.is_empty()
            && value.len() <= 256
            && !value.contains("://")
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
            }))
        .then(|| Self(value.to_owned()))
    }
}

impl<'de> Deserialize<'de> for NativeModelId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Identifier;
        impl<'de> serde::de::Visitor<'de> for Identifier {
            type Value = NativeModelId;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a bounded native model identifier")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                NativeModelId::new(value)
                    .ok_or_else(|| E::custom("invalid native model identifier"))
            }
        }
        deserializer.deserialize_str(Identifier)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_wire_and_attempt_records_keep_missing_facts_unknown() {
        let old_wire = serde_json::json!({"request_token":null,"phase":"response",
            "bearer_present":false,"api_key_present":false,"authorization_bytes":0,
            "body_bytes":null,"http_status":400,"content_type":"json","request_reasoning":null});
        let wire: UpstreamWire = serde_json::from_value(old_wire).unwrap();
        assert!(wire.native_model.is_none() && wire.provider_error.is_none());
        assert!(wire.attempt_index.is_none() && wire.http_protocol.is_none());
        let old_attempt = serde_json::json!({"attempt_token":null,"outcome":"failed",
            "commit":"unknown","elapsed_ms":1,"http_status":400,"reasoning_fields_removed":0});
        let attempt: AttemptEnd = serde_json::from_value(old_attempt).unwrap();
        assert!(attempt.commits.is_none() && attempt.http_protocol.is_none());
        let mut encoded = serde_json::to_value(wire).unwrap();
        encoded["provider_error"] = "provider-secret-marker".into();
        assert!(serde_json::from_value::<UpstreamWire>(encoded).is_err());
    }

    #[test]
    fn native_model_field_rejects_content_urls_and_oversized_identifiers() {
        for value in [
            "",
            "prompt with spaces",
            "https://credential.invalid",
            "model\nsecret",
            &"x".repeat(257),
        ] {
            assert!(NativeModelId::new(value).is_none());
            assert!(serde_json::from_value::<NativeModelId>(serde_json::json!(value)).is_err());
        }
        assert_eq!(
            serde_json::to_value(NativeModelId::new("provider/qwen3.6-flash:revision").unwrap())
                .unwrap(),
            "provider/qwen3.6-flash:revision"
        );
    }
}
