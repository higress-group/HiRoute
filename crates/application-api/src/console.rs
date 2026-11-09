//! Owner credential operations. Credential exchanges remain on the HTTP console.
use serde::{Deserialize, Serialize};
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConsoleLaunchRequestV1 {
    #[serde(default)]
    pub loopback_only: bool,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConsoleListTokensRequestV1 {}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConsoleCreateTokenRequestV1 {
    pub name: String,
    #[serde(default)]
    pub expires_days: Option<u32>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConsoleRevokeTokenRequestV1 {
    pub id: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConsoleLaunchV1 {
    pub url: String,
    pub expires_in: u32,
}

pub const QUALIFY_NATIVE_CONSOLE_V1: &str = "QualifyNativeConsoleV1";

/// Private Local Control admission for a matching native console host.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QualifyNativeConsoleRequestV1 {
    pub origin: String,
    pub assets_sha256: String,
    pub session: Option<NativeConsoleSessionV1>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeConsoleSessionV1 {
    pub bearer_sha256: String,
    pub launch_sha256: String,
}
