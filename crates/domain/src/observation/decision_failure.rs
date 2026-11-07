use serde::{Deserialize, Serialize};

/// Safe, closed classification shared by explicit tests and routing evidence.
#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionFailureKindV1 {
    Timeout,
    Unavailable,
    SourceDisabled,
    NoUsableCredentials,
    BudgetExhausted,
    BudgetUnavailable,
    AuthenticationRejected,
    ProxyAuthenticationRejected,
    RateLimited,
    EndpointRejected,
    InputRejected,
    ContextLimit,
    UpstreamError,
    InvalidOutput,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionFailureV1 {
    pub kind: DecisionFailureKindV1,
    pub http_status: Option<u16>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionServiceRevisionV1 {
    pub id: String,
    pub revision: u64,
}

/// One request's failed decision and the local decision that actually routed it.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionFallbackEvidenceV1 {
    pub service: Option<DecisionServiceRevisionV1>,
    pub failure: DecisionFailureV1,
    pub source: super::ComplexityDecisionSourceV1,
}
