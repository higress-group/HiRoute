//! Closed, bounded facts observed inside CPA; never upstream error text.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CpaModelDiscovery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_ordinal: Option<u32>,
    pub observation: CpaDiscoveryObservation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum CpaDiscoveryObservation {
    Observed {
        process_id: u32,
        evicted_accounts: u64,
        /// Retained by CPA from an earlier attempt, possibly before this span.
        retained_first_failure: bool,
        attempt: CpaDiscoveryAttempt,
    },
    Unavailable {
        reason: CpaDiscoveryUnavailable,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CpaDiscoveryUnavailable {
    MetadataAbsent,
    InvalidMetadata,
    NoAttempt,
    ObservationLimit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CpaDiscoveryAttempt {
    pub attempt_id: u64,
    pub phase: CpaDiscoveryPhase,
    pub outcome: CpaDiscoveryOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<CpaDiscoveryReason>,
    pub elapsed_ms: u64,
    pub phase_elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_matches: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registration_epoch_matches: Option<bool>,
    pub timings: Vec<CpaDiscoveryTiming>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CpaDiscoveryTiming {
    pub phase: CpaDiscoveryPhase,
    pub outcome: CpaDiscoveryOutcome,
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<CpaDiscoveryReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_count: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CpaDiscoveryPhase {
    Credentials,
    ClientVersion,
    Request,
    Response,
    Catalog,
    Registration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CpaDiscoveryOutcome {
    Entered,
    Completed,
    Failed,
    Discarded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CpaDiscoveryReason {
    MissingAccessToken,
    ClientVersionUnavailable,
    Cancelled,
    Timeout,
    DnsFailure,
    ConnectFailure,
    ProxyFailure,
    ProxyRejected,
    TlsFailure,
    TransportFailure,
    HttpRejected,
    BodyReadFailure,
    ResponseTooLarge,
    InvalidCatalog,
    InvalidModelId,
    DuplicateModelId,
    EmptyCatalog,
    EmptyAfterFiltering,
    AccountMissing,
    AccountDisabled,
    AccountChanged,
}
