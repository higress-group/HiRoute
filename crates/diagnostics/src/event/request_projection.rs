//! Safe evidence of a candidate-local request projection, never conversation facts.
use crate::identity::CorrelationToken;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestProjection {
    pub request_token: Option<CorrelationToken>,
    pub binding_token: Option<CorrelationToken>,
    pub ingress_protocol: super::IngressProtocol,
    pub upstream_protocol: super::IngressProtocol,
    pub stage: RequestProjectionStage,
    pub outcome: RequestProjectionOutcome,
    pub reason: RequestProjectionReason,
    /// Zero-based canonical request.messages coordinate; not a native input index.
    pub first_message_index: Option<u64>,
    /// Exactly known affected items. A first rejection reports one, not all blockers.
    pub affected_items: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestProjectionStage {
    CandidatePreflight,
    AttemptMaterialization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestProjectionOutcome {
    DeclaredLoss,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestProjectionReason {
    ResponsesToolSearchUnsupported,
    ResponsesIncludeUnsupported,
    ResponsesClientMetadataUnsupported,
    ResponsesReasoningSummaryUnsupported,
    ResponsesReasoningContextUnsupported,
    ResponsesStoreUnsupported,
    ResponsesPromptCacheKeyUnsupported,
    MessagePhaseOmitted,
    MessagePhaseUnsupported,
    ItemLifecycleUnsupported,
    NativeSemanticsUnsupported,
}

impl std::fmt::Display for RequestProjectionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let field = match self {
            Self::ResponsesToolSearchUnsupported => "tool_search",
            Self::ResponsesIncludeUnsupported => "include",
            Self::ResponsesClientMetadataUnsupported => "client_metadata",
            Self::ResponsesReasoningSummaryUnsupported => "reasoning.summary",
            Self::ResponsesReasoningContextUnsupported => "reasoning.context",
            Self::ResponsesStoreUnsupported => "store",
            Self::ResponsesPromptCacheKeyUnsupported => "prompt_cache_key",
            other => return write!(f, "{other:?}"),
        };
        write!(
            f,
            "Responses {field} has no equivalent on this upstream protocol"
        )
    }
}
