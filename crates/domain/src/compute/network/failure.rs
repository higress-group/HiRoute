//! Safe observed network phases shared by checks and Gateway diagnostics.
use serde::{Deserialize, Serialize};
#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeNetworkFailurePhaseV1 {
    RouteUnavailable,
    ProxyConnect,
    ProxyTls,
    ProxyAuth,
    ProxyNegotiation,
    UpstreamConnect,
    UpstreamTls,
    UpstreamAuth,
    Directory,
    Inference,
    TransportUnknown,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeNetworkFailureV1 {
    pub phase: NativeNetworkFailurePhaseV1,
    pub code: String,
    pub retryable: bool,
}
