use serde::{Deserialize, Serialize};

use super::AgentTurnAttributionV1;

/// One bounded query over the latest persisted quality value for each execution segment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanQualitySamplesQuery {
    #[serde(default)]
    pub competence: Option<PlanCompetenceFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_configuration_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<PlanQualityExecutionIdentity>,
    #[serde(default)]
    pub unrated_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_gt: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_lt: Option<f64>,
    pub limit: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanQualityAssessment {
    pub event_id: String,
    pub trigger_request_id: String,
    pub assessed_at_ms: i64,
    pub target_from_turn_id: String,
    pub target_through_turn_id: String,
    pub target_from_ordinal: u64,
    pub target_through_ordinal: u64,
    pub score: f64,
    pub partial: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub evidence_available: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanQualitySample {
    /// The stage-opening selection; actual execution and later assessment stay separate.
    pub selection: Option<super::BranchDecisionV1>,
    pub branch_execution: Option<crate::BranchExecutionV1>,
    pub upgrade: Option<PlanQualityUpgrade>,
    pub segment_id: String,
    pub session_id: String,
    pub plan_id: String,
    pub plan_revision: u64,
    pub selected_branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executed_branch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_configuration_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_profile_id: Option<String>,
    pub attribution: AgentTurnAttributionV1,
    pub first_turn_id: String,
    pub first_turn_ordinal: u64,
    pub last_observed_turn_id: String,
    pub last_observed_turn_ordinal: u64,
    pub first_at_ms: i64,
    pub last_at_ms: i64,
    pub history_partial: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_request_id: Option<String>,
    pub execution_evidence_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment: Option<PlanQualityAssessment>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanQualitySamplesPage {
    pub samples: Vec<PlanQualitySample>,
    pub summary: PlanQualitySummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Full authorized scope, independent of the stage page and detail filters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanQualitySummary {
    pub models: Vec<PlanQualityModelSummary>,
    pub scored_stage_count: u64,
    pub unrated_stage_count: u64,
    pub session_count: u64,
    pub available_revisions: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanQualityModelSummary {
    pub branch_policy: Option<crate::BranchExecutionPolicyV1>,
    pub execution: PlanQualityExecutionIdentity,
    pub native_model: Option<String>,
    pub reasoning_profile_id: Option<String>,
    pub scored_stage_count: u64,
    pub unrated_stage_count: u64,
    pub average_score: Option<f64>,
}

/// The exact aggregate key, also used for stage drill-down (including nulls).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanQualityExecutionIdentity {
    pub group: Option<crate::ExecutionGroupV1>,
    pub candidate_index: Option<u16>,
    pub plan_revision: u64,
    /// Present only when no executed branch was recorded; never replaces it.
    pub selected_branch_id: Option<String>,
    pub executed_branch_id: Option<String>,
    pub model_configuration_id: Option<String>,
    pub profile_digest: Option<String>,
    pub attribution: AgentTurnAttributionV1,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanCompetenceFilter {
    BelowFloor,
    MeetsFloor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanQualityUpgrade {
    pub decision: crate::CompetenceProtectionV1,
    pub trigger_request_id: String,
}
