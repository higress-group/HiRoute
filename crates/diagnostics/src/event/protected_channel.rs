//! Closed, secret-free evidence for the native launcher exchange and loss transition.
use crate::identity::CorrelationToken;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtectedExchangeKind {
    Grant,
    InputRegister,
    InputRelease,
    Upgrade,
    Recovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtectedExchangePhase {
    RequestWrite,
    AcknowledgementRead,
    Register,
    AcknowledgementWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtectedExchangeError {
    Busy,
    Timeout,
    Closed,
    Oversized,
    Invalid,
    Mismatch,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectedExchange {
    pub kind: ProtectedExchangeKind,
    pub phase: ProtectedExchangePhase,
    pub registration_token: Option<CorrelationToken>,
    pub elapsed_ms: u64,
    pub error: Option<ProtectedExchangeError>,
    pub authority_lost: bool,
}
