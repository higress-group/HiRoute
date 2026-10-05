//! Non-secret projections for Desktop and CLI. None of these inputs grants write authority.
use crate::PrincipalKind;
use hiroute_domain::{
    AgentPlanAuthoringV2, AgentPlanId, CanonicalDigest, ModelAlias, PlanHeadV1, RevisionSetV1,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClientEmptyRequestV1 {}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPlanLookupV1 {
    pub agent_plan_id: AgentPlanId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationIdentityViewV1 {
    pub revision: u64,
    pub digest: CanonicalDigest,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientGatewayStateV1 {
    NotComposed,
    Empty,
    NoNewCalls,
    Ready,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPlanCatalogViewV2 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<AgentPlanCatalogCursorV2>,
    pub schema: String,
    #[serde(default)]
    pub drafts: Vec<hiroute_domain::PlanDraftV1>,
    pub plans: Vec<AgentPlanStatusV2>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationIdempotencyLookupV1 {
    pub principal_kind: PrincipalKind,
    pub operation_kind: String,
    pub idempotency_key: String,
    pub accepted_digest: CanonicalDigest,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClientOperationViewV1 {
    pub operation_id: String,
    pub state: String,
    pub sequence: u64,
    pub cancellable: bool,
    pub accepted_digest: CanonicalDigest,
    pub safe_error_code: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationIdempotencyResultV1 {
    pub operation: Option<ClientOperationViewV1>,
    pub digest_matches: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPlanCatalogCursorV2 {
    pub snapshot_digest: CanonicalDigest,
    pub offset: usize,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
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
