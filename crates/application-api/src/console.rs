//! Owner credential operations. Credential exchanges never traverse Local Control.
use serde::{Deserialize, Serialize};
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConsoleLaunchRequestV1 {}
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
