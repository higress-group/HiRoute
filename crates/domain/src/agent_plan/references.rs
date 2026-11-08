use serde::{Deserialize, Serialize};

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanReferenceProtectionV1 {
    CurrentHead,
    PreparedVersion,
    UnresolvedHold,
    Draft,
    ActivePublication,
    PreparedPublication,
    LkgPublication,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanReferenceBlockerV1 {
    pub plan_id: Option<String>,
    pub draft_id: Option<String>,
    pub revision: u64,
    pub protection: PlanReferenceProtectionV1,
}
