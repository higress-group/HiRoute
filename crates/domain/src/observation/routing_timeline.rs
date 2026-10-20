//! Read projections over recorded execution facts. A watermark is opaque and scope-bound.
use super::{BranchDecisionV1, PlanQualityAssessment};
use serde::{Deserialize, Serialize};

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingTimelineQueryV1 {
    pub session_id: String,
    #[serde(default)]
    pub after_watermark: Option<String>,
    pub limit: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingTimelinePageV1 {
    pub turns: Vec<RoutingTimelineTurnV1>,
    pub watermark: String,
    /// All changes through the captured watermark have been returned; future changes may follow.
    pub complete: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingTurnBoundaryV1 {
    NewUserMessage,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingTimelineTurnV1 {
    /// Stable opening-request identity, including while the turn is still running.
    pub turn_id: String,
    pub agent_turn_id: Option<String>,
    pub segment_id: Option<String>,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub boundary: RoutingTurnBoundaryV1,
    pub plan_id: Option<String>,
    pub plan_revision: Option<u64>,
    pub branch_id: Option<String>,
    pub model_configuration_id: Option<String>,
    pub reasoning_profile_id: Option<String>,
    pub group: Option<crate::ExecutionGroupV1>,
    pub selection: Option<BranchDecisionV1>,
    pub tool_continuations: u64,
    pub relays: Vec<RoutingTimelineRelayV1>,
    #[serde(default)]
    pub transitions: Vec<crate::AnalysisTransitionV1>,
    pub assessment_state: String,
    pub assessment: Option<PlanQualityAssessment>,
    pub competence_floor_millis: Option<u16>,
    pub protected_turn_ids: Vec<String>,
    pub status: String,
    pub requests: Vec<RoutingTimelineRequestV1>,
    pub cost_by_model: Vec<RoutingTimelineCostV1>,
    pub evidence_partial: bool,
    /// Lets a reconnecting consumer discard an identical projection without hiding late scores.
    pub content_digest: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingTimelineRelayV1 {
    pub request_id: String,
    pub from_model: Option<String>,
    pub to_model: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingTimelineRequestV1 {
    pub request_id: String,
    pub started_at_ms: i64,
    pub duration_ms: Option<u64>,
    pub native_model: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_read_share: Option<f64>,
    pub costs: Vec<RoutingTimelineCostV1>,
    pub outcome: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingTimelineCostV1 {
    pub model_configuration_id: Option<String>,
    pub currency: Option<String>,
    pub valuation_kind: Option<String>,
    pub known_micros: Option<u64>,
    pub unknown_contributions: u64,
}
