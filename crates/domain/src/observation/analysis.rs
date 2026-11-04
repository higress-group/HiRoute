//! Opt-in analysis queries. Physical session identity and monetary authority
//! remain with the existing observation and valuation owners.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ApiEquivalentPriceV1, ObservationRequestQuery};
use crate::PriceValuationKindV1;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisPeriodV1 {
    Today,
    Yesterday,
    #[default]
    #[serde(rename = "7d")]
    SevenDays,
    #[serde(rename = "30d")]
    ThirtyDays,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisTimeInputV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<AnalysisPeriodV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_ms: Option<i64>,
}

impl AnalysisTimeInputV1 {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisTimeRangeV1 {
    pub requested_from_ms: i64,
    pub requested_to_ms: i64,
    pub effective_from_ms: i64,
    pub effective_to_ms: i64,
    pub from_utc_offset_seconds: i32,
    pub to_utc_offset_seconds: i32,
    pub clipped_for_retention: bool,
}

/// Host-clock output. Each boundary has its own offset, including DST changes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisLocalDayV1 {
    pub from_ms: i64,
    pub to_ms: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisViewV1 {
    Conversations,
    Conversation,
    Routing,
    Switches,
    CostRanking,
    SavingsSummary,
}

impl AnalysisViewV1 {
    pub fn monetary(self) -> bool {
        matches!(self, Self::CostRanking | Self::SavingsSummary)
    }
    pub fn content(self) -> bool {
        self == Self::Conversation
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AnalysisCostBasisV1 {
    #[default]
    Recorded,
    ApiEquivalent,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisCostGroupV1 {
    #[default]
    Session,
    Request,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservationAnalysisQueryV1 {
    pub view: AnalysisViewV1,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_decision_failure: bool,
    #[serde(default)]
    pub time: AnalysisTimeInputV1,
    #[serde(default)]
    pub analysis_scope: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub plan_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub basis: Option<AnalysisCostBasisV1>,
    #[serde(default)]
    pub group_by: Option<AnalysisCostGroupV1>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub valuation_kind: Option<PriceValuationKindV1>,
    #[serde(default)]
    pub reference_binding_id: Option<String>,
    #[serde(default = "analysis_page_limit")]
    pub limit: u16,
    #[serde(default)]
    pub cursor: Option<String>,
}

fn analysis_page_limit() -> u16 {
    50
}

impl ObservationAnalysisQueryV1 {
    pub fn new(view: AnalysisViewV1) -> Self {
        Self {
            view,
            include_decision_failure: false,
            time: AnalysisTimeInputV1::default(),
            analysis_scope: None,
            session_id: None,
            request_id: None,
            plan_id: None,
            agent_id: None,
            query: None,
            basis: None,
            group_by: None,
            currency: None,
            valuation_kind: None,
            reference_binding_id: None,
            limit: analysis_page_limit(),
            cursor: None,
        }
    }
    pub fn request_query(&self, range: &AnalysisTimeRangeV1) -> ObservationRequestQuery {
        ObservationRequestQuery {
            from_ms: range.effective_from_ms,
            to_ms: range.effective_to_ms,
            session_id: self.session_id.clone(),
            request_id: self.request_id.clone(),
            agent_id: self.agent_id.clone(),
            plan_id: self.plan_id.clone(),
            limit: self.limit,
            cursor: self.cursor.clone(),
            native_model: None,
            outcome: None,
            only_model_switch: false,
        }
    }
}

/// Application-captured current prices. This is not a caller credential or a
/// new source of prices; Observation retains it for the selected report scope.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisPricesV1 {
    pub generation_ref: String,
    pub evaluated_at_ms: i64,
    pub prices: Vec<ApiEquivalentPriceV1>,
    pub reference_binding_id: Option<String>,
}
