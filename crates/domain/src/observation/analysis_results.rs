//! Bounded, typed outputs for the opt-in analysis journey.
use super::{
    AnalysisCostBasisV1, AnalysisTimeRangeV1, ObservationMetricCoverageV2, ObservedRequestSummary,
    PlanQualitySample,
};
use crate::PriceValuationKindV1;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisStatusV1 {
    Ready,
    Partial,
    Pending,
    ChoicesRequired,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisChoiceV1 {
    pub field: String,
    pub values: Vec<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisProgressV1 {
    pub status: String,
    pub phase: String,
    pub target_ref: String,
    pub processed_requests: u64,
    pub total_requests: u64,
    pub started_at_ms: i64,
    pub last_progress_at_ms: i64,
    pub reason: Option<String>,
    pub retry_after_ms: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisMemberV1 {
    pub session_id: String,
    pub predecessor_session_id: Option<String>,
    pub gap_before: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisConversationV1 {
    pub analysis_session_id: String,
    pub members: Vec<AnalysisMemberV1>,
    pub member_count: u64,
    pub members_partial: bool,
    pub first_at_ms: i64,
    pub last_at_ms: i64,
    pub request_count: u64,
    pub lineage_partial: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisModelV1 {
    pub native_model: Option<String>,
    pub model_configuration_id: Option<String>,
    pub reasoning_profile_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisContentV1 {
    pub content_id: String,
    pub message_occurrence_id: String,
    pub direction: String,
    pub kind: String,
    pub media_type: String,
    pub text: Option<String>,
    pub next_cursor: Option<String>,
    pub completeness: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisRequestV1 {
    pub request: ObservedRequestSummary,
    pub model: AnalysisModelV1,
    pub finished_at_ms: Option<i64>,
    pub content: Vec<AnalysisContentV1>,
    pub content_partial: bool,
    pub content_state: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisTransitionKindV1 {
    InitialSelection,
    BetweenTurnChange,
    ContinuationBoundary,
    WithinRequestFallback,
    CredentialRelay,
    ReasoningChange,
    Unchanged,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisTransitionV1 {
    pub kind: AnalysisTransitionKindV1,
    pub session_id: String,
    pub request_id: String,
    pub previous_session_id: Option<String>,
    pub previous_request_id: Option<String>,
    pub at_ms: i64,
    pub previous_model: Option<AnalysisModelV1>,
    pub model: AnalysisModelV1,
    pub model_changed: Option<bool>,
    pub reasoning_changed: Option<bool>,
    pub completeness: String,
    pub receipt_id: Option<String>,
    pub reason: Option<PlanQualitySample>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_failure: Option<super::DecisionFallbackEvidenceV1>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisCostRowV1 {
    pub id: String,
    pub session_id: Option<String>,
    pub request_id: Option<String>,
    pub amount_micros: Option<u64>,
    pub known_sum_micros: Option<u64>,
    pub currency: String,
    pub valuation_kind: PriceValuationKindV1,
    pub coverage: ObservationMetricCoverageV2,
    pub request_count: u64,
    pub attempt_count: u64,
    pub missing_price_count: u64,
    pub missing_usage_count: u64,
    pub pending_request_count: u64,
    pub highest_known_only: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisSavingsV1 {
    pub currency: String,
    pub reference_binding_id: String,
    pub evaluated_at_ms: i64,
    pub request_count: u64,
    pub compared_request_count: u64,
    pub excluded_request_count: u64,
    pub compared_routed_subtotal_micros: Option<u64>,
    pub compared_reference_subtotal_micros: Option<u64>,
    pub estimated_savings_micros: Option<i64>,
    pub savings_percent: Option<f64>,
    pub coverage: ObservationMetricCoverageV2,
    pub decision_overhead: String,
    pub disclosure: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservationAnalysisPageV1 {
    /// Internal Application/Observation continuation. Never a public option or
    /// a source of price authority supplied by a client.
    #[serde(skip)]
    #[schemars(skip)]
    pub price_capture_request: Option<AnalysisPriceCaptureRequestV1>,
    pub schema: String,
    pub status: AnalysisStatusV1,
    pub analysis_scope: Option<String>,
    pub time: Option<AnalysisTimeRangeV1>,
    pub expires_at_ms: Option<i64>,
    pub relation_watermark: Option<u64>,
    pub numeric_watermark: Option<u64>,
    pub price_capture_ref: Option<String>,
    pub basis: Option<AnalysisCostBasisV1>,
    pub conversations: Vec<AnalysisConversationV1>,
    pub requests: Vec<AnalysisRequestV1>,
    pub transitions: Vec<AnalysisTransitionV1>,
    pub transition_counts: std::collections::BTreeMap<String, u64>,
    pub costs: Vec<AnalysisCostRowV1>,
    pub savings: Option<AnalysisSavingsV1>,
    pub choices: Vec<AnalysisChoiceV1>,
    pub progress: Option<AnalysisProgressV1>,
    pub gaps: Vec<String>,
    pub next_cursor: Option<String>,
}

impl Default for ObservationAnalysisPageV1 {
    fn default() -> Self {
        Self {
            price_capture_request: None,
            schema: "hiroute.observation.analysis/v1".into(),
            status: AnalysisStatusV1::Ready,
            analysis_scope: None,
            time: None,
            expires_at_ms: None,
            relation_watermark: None,
            numeric_watermark: None,
            price_capture_ref: None,
            basis: None,
            conversations: Vec::new(),
            requests: Vec::new(),
            transitions: Vec::new(),
            transition_counts: Default::default(),
            costs: Vec::new(),
            savings: None,
            choices: Vec::new(),
            progress: None,
            gaps: Vec::new(),
            next_cursor: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AnalysisPriceCaptureRequestV1 {
    pub currency: Option<String>,
    pub plan_id: Option<String>,
    pub reference_binding_id: Option<String>,
}
