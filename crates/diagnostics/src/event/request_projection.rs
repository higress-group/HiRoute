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
    MessagePhaseOmitted,
    MessagePhaseUnsupported,
    ItemLifecycleUnsupported,
    NativeSemanticsUnsupported,
}
