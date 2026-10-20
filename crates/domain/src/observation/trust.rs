//! Current read contracts over retained routing facts; no persisted trial state.
use super::PlanQualitySample;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(schemars::JsonSchema, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustRevisionNameV1 {
    #[default]
    Active,
    All,
}
#[derive(schemars::JsonSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TrustRevisionV1 {
    Number(u64),
    Name(TrustRevisionNameV1),
}
impl Default for TrustRevisionV1 {
    fn default() -> Self {
        Self::Name(TrustRevisionNameV1::Active)
    }
}
#[derive(
    schemars::JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum TrustGroupByV1 {
    #[default]
    Model,
    Executor,
}
#[derive(schemars::JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanTrustQueryV1 {
    pub plan_id: String,
    #[serde(default)]
    pub branch_id: Option<String>,
    #[serde(default)]
    pub revision: TrustRevisionV1,
    #[serde(default)]
    pub from_ms: Option<i64>,
    #[serde(default)]
    pub to_ms: Option<i64>,
    #[serde(default)]
    pub group_by: TrustGroupByV1,
}
#[derive(schemars::JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanTrustReplayQueryV1 {
    #[serde(flatten)]
    pub scope: PlanTrustQueryV1,
    pub thresholds_millis: Vec<u16>,
}
/// Internal admitted query; Application resolves the active revision.
#[derive(Clone, Debug)]
pub struct TrustPopulationQueryV1 {
    pub active_revision: u64,
    pub plan_id: String,
    pub branch_id: Option<String>,
    pub revision: Option<u64>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
}
#[derive(Clone, Debug)]
pub struct TrustObservedStageV1 {
    pub sample: PlanQualitySample,
    pub executor: Option<String>,
    pub has_later_turn: bool,
    pub request_durations_ms: Vec<u64>,
    pub output_tokens_per_second: Vec<f64>,
}
#[derive(Clone, Debug)]
pub struct TrustPopulationV1 {
    pub active_stages: Vec<TrustObservedStageV1>,
    pub retained_from_ms: i64,
    pub stages: Vec<TrustObservedStageV1>,
    pub generation: u64,
    pub from_ms: i64,
    pub to_ms: i64,
}

/// Latest status of each submitted task in a Plan/revision/creation-time scope.
/// Tasks that produced no model stages still belong to this population.
#[derive(Clone, Debug)]
pub struct TrustTaskOutcomeV1 {
    pub task_id: String,
    pub executor: String,
    pub ordinal: u64,
    pub state: crate::delegation::RunStateV1,
}

#[derive(Clone, Debug, Default)]
pub struct TrustTaskPopulationV1 {
    pub tasks: Vec<TrustTaskOutcomeV1>,
    pub over_limit: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustCountsV1 {
    pub executed: u64,
    pub complete: u64,
    pub meets_floor: u64,
    pub partial: u64,
    pub awaiting_assessment: u64,
    pub unscored: u64,
    pub sessions: u64,
}
impl TrustCountsV1 {
    pub fn competence_rate(&self) -> Option<f64> {
        (self.complete > 0).then(|| self.meets_floor as f64 / self.complete as f64)
    }
    pub fn scoring_coverage(&self) -> Option<f64> {
        (self.executed > 0).then(|| self.complete as f64 / self.executed as f64)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustReplayThresholdV1 {
    pub threshold_millis: u16,
    pub regular: u64,
    pub primary: u64,
    pub protected_as_primary: u64,
    pub regular_share: Option<f64>,
    pub primary_share: Option<f64>,
    pub protected_share: Option<f64>,
    pub newly_regular: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanTrustReplayV1 {
    pub included: u64,
    pub excluded_by_reason: BTreeMap<String, u64>,
    pub recorded_share_at_current: Option<f64>,
    pub thresholds: Vec<TrustReplayThresholdV1>,
    pub estimate_only: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TrustSuggestionV1 {
    Keep { missing: Vec<String> },
    StepUp { threshold_millis: u16 },
    StepDown { threshold_millis: u16 },
    ReturnToPreviousRouting,
    ChangeRegularModel,
    ExploreDivision,
}

/// Application-admitted estimate scope; never changes general value or settlement semantics.
#[derive(Clone, Debug)]
pub struct TrustCostScopeV1 {
    pub plan_id: String,
    pub revision: Option<u64>,
    pub branch_id: Option<String>,
    pub group: crate::ExecutionGroupV1,
    pub from_ms: i64,
    pub to_ms: i64,
    pub currency: String,
}
