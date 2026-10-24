//! Closed, non-authoritative facts observed before internal headers are consumed.
use crate::identity::CorrelationToken;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRoutingEvidence {
    pub request_token: Option<CorrelationToken>,
    pub context_token: Option<CorrelationToken>,
    pub header: NativeRoutingHeader,
    pub adapter: Option<NativeAdapterEvidence>,
    pub track_history: bool,
    pub origin: NativeRoutingOrigin,
    pub turn_applied: bool,
    pub checkpoint_applied: bool,
    pub compression_applied: bool,
    pub identity_conflict: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeRoutingHeader {
    Missing,
    Invalid,
    Present,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeAdapterEvidence {
    pub version: u8,
    pub agent: NativeRoutingAgent,
    pub purpose: NativeRoutingPurpose,
    pub disposition: NativeAdapterDisposition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeRoutingAgent {
    Pi,
    Dsh,
    OpenCode,
    Hermes,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeRoutingPurpose {
    Main,
    Unknown,
    Title,
    Summary,
    Compaction,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeAdapterDisposition {
    Applied,
    Auxiliary,
    AgentMismatch,
    UnsupportedPurpose,
    InvalidSession,
    SessionMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeRoutingOrigin {
    Wire,
    NativeTurn,
    CurrentUser,
    ExistingTurn,
    Observed,
}
