//! Non-secret projections for Desktop and CLI. None of these inputs grants write authority.
use crate::PrincipalKind;
use hiroute_domain::{
    AgentPlanAuthoringV2, AgentPlanId, CanonicalDigest, ModelAlias, PlanHeadV1, RevisionSetV1,
};
use serde::{Deserialize, Serialize};

#[derive(schemars::JsonSchema, Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClientEmptyRequestV1 {}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPlanLookupV1 {
    pub agent_plan_id: AgentPlanId,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationIdentityViewV1 {
    pub revision: u64,
    pub digest: CanonicalDigest,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientGatewayStateV1 {
    NotComposed,
    Empty,
    NoNewCalls,
    Ready,
    Unavailable,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClientServiceStatusV1 {
    pub schema: String,
    pub daemon_role: String,
    pub recovery_ready: bool,
    pub mutation_available: bool,
    pub gateway: ClientGatewayStateV1,
    pub active_publication: Option<PublicationIdentityViewV1>,
    pub revisions: RevisionSetV1,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPlanStatusV2 {
    pub protocol_advice: hiroute_domain::AgentProtocolAdviceV1,
    pub agent_plan_id: AgentPlanId,
    pub desired: AgentPlanAuthoringV2,
    pub head: PlanHeadV1,
    pub editable_route_digest: CanonicalDigest,
    pub agent_plan_revision: u64,
    pub model_alias: ModelAlias,
    pub materialized_route_digest: CanonicalDigest,
    pub publication: PublicationIdentityViewV1,
    pub execution: ClientGatewayStateV1,
    pub revisions: RevisionSetV1,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPlanCatalogViewV2 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<AgentPlanCatalogCursorV2>,
    pub schema: String,
    #[serde(default)]
    pub drafts: Vec<hiroute_domain::PlanDraftV1>,
    pub plans: Vec<AgentPlanStatusV2>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationIdempotencyLookupV1 {
    pub principal_kind: PrincipalKind,
    pub operation_kind: String,
    pub idempotency_key: String,
    pub accepted_digest: CanonicalDigest,
    /// Keep extensions last for released positional inputs; false preserves the response shape.
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_model_budget_conflict: bool,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClientOperationViewV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_budget_conflict: Option<hiroute_domain::AgentModelBudgetConflictV1>,
    pub operation_id: String,
    pub state: String,
    pub sequence: u64,
    pub cancellable: bool,
    pub accepted_digest: CanonicalDigest,
    pub safe_error_code: Option<String>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationIdempotencyResultV1 {
    pub operation: Option<ClientOperationViewV1>,
    pub digest_matches: bool,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPlanCatalogCursorV2 {
    pub snapshot_digest: CanonicalDigest,
    pub offset: usize,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPlanCatalogQueryV2 {
    #[serde(default = "default_plan_page_size")]
    pub limit: usize,
    #[serde(default)]
    pub cursor: Option<AgentPlanCatalogCursorV2>,
}
fn default_plan_page_size() -> usize {
    32
}
impl Default for AgentPlanCatalogQueryV2 {
    fn default() -> Self {
        Self {
            limit: default_plan_page_size(),
            cursor: None,
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn released_idempotency_lookup_keeps_positional_field_order() {
        let digest = format!("sha256:{}", "0".repeat(64));
        let original = json!(["interactive_user", "change", "test", digest]);
        for projection in [None, Some(false), Some(true)] {
            let mut request = original.clone();
            if let Some(projection) = projection {
                request.as_array_mut().unwrap().push(json!(projection));
            }
            let lookup: OperationIdempotencyLookupV1 =
                serde_json::from_value(request).expect("released V1 positional lookup");
            assert_eq!(lookup.principal_kind, PrincipalKind::InteractiveUser);
            assert_eq!(lookup.operation_kind, "change");
            assert_eq!(lookup.idempotency_key, "test");
            assert_eq!(lookup.accepted_digest.as_str(), digest);
            assert_eq!(
                lookup.include_model_budget_conflict,
                projection == Some(true)
            );
            let wire = serde_json::to_value(lookup).unwrap();
            assert_eq!(wire["accepted_digest"], digest);
            if projection == Some(true) {
                assert_eq!(wire["include_model_budget_conflict"], true);
            } else {
                assert!(wire.get("include_model_budget_conflict").is_none());
            }
        }
    }
}
