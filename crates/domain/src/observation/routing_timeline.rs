//! Read projections over recorded execution facts. A watermark is opaque and scope-bound.
use super::{BranchDecisionV1, PlanQualityAssessment};
use crate::{
    CanonicalDigest, ClassifierFallbackReasonV1, ComplexityDecisionSourceV1,
    ComplexityReasonCodeV1, DecisionProtocolV1,
};
use serde::{Deserialize, Serialize};

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingTimelineQueryV1 {
    pub session_id: String,
    #[serde(default)]
    pub after_watermark: Option<String>,
    pub limit: u16,
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_presentation: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingTimelinePageV1 {
    pub turns: Vec<RoutingTimelineTurnV1>,
    pub watermark: String,
    /// All changes through the captured watermark have been returned; future changes may follow.
    pub complete: bool,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingTurnBoundaryV1 {
    NewUserMessage,
    ContextRebuilt,
    Unknown,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Serialize)]
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
    pub selection: Option<RoutingTimelineSelectionV1>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<RoutingTimelinePresentationV1>,
    /// Lets a reconnecting consumer discard an identical projection without hiding late scores.
    pub content_digest: String,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingTimelineGroupStateV1 {
    Pending,
    Resolved,
    Mixed,
    Unavailable,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingTimelinePresentationV1 {
    pub group_state: RoutingTimelineGroupStateV1,
    pub execution_attribution: crate::AgentTurnAttributionV1,
    pub request_models: Vec<RoutingTimelineRequestModelV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingTimelineRequestModelV1 {
    pub request_id: String,
    pub model_configuration_id: Option<String>,
}

/// Opening-decision read view. Persisted BranchDecisionV1 remains unchanged.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingTimelineSelectionV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_protocol: Option<DecisionProtocolV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_observation: Option<crate::RoutingObservationV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<crate::BranchExecutionPolicyV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub competence_trigger: Option<crate::CompetenceProtectionV1>,
    pub execution_group: crate::ExecutionGroupV1,
    pub simple_probability: Option<serde_json::Number>,
    pub simple_threshold_millis: Option<u16>,
    pub selection_reason: crate::ModelGroupReasonV1,
    pub strategy_id: String,
    pub schema_version: String,
    pub payload_digest: CanonicalDigest,
    pub branch_id: String,
    pub complexity_score: Option<u8>,
    pub threshold: Option<u8>,
    pub decision_source: ComplexityDecisionSourceV1,
    pub reason_codes: Vec<ComplexityReasonCodeV1>,
    pub matched_user_phrase_ids: Vec<String>,
    pub fallback_used: bool,
    pub classification_duration_micros: Option<u64>,
    pub fallback_reason: Option<ClassifierFallbackReasonV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub historical_protection: Option<RoutingTimelineProtectionV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingTimelineProtectionV1 {
    /// Recorded historical protection action. Suppression removes only that constraint;
    /// the current difficulty can still select Primary.
    pub outcome: crate::HistoricalProtectionActionV1,
    pub previous_score: Option<serde_json::Number>,
    pub competence_floor_millis: u16,
    pub applicability_probability: Option<serde_json::Number>,
    pub applicability_boundary_millis: u16,
}

impl From<BranchDecisionV1> for RoutingTimelineSelectionV1 {
    fn from(value: BranchDecisionV1) -> Self {
        let historical_protection =
            value
                .routing_observation
                .as_ref()
                .map(|observation| RoutingTimelineProtectionV1 {
                    outcome: observation.protection_action,
                    previous_score: observation.score.clone(),
                    competence_floor_millis: observation.competence_floor_millis,
                    applicability_probability: observation.applicability_probability.clone(),
                    applicability_boundary_millis: observation.applicability_boundary_millis,
                });
        let BranchDecisionV1 {
            extension_reason,
            decision_protocol,
            routing_observation,
            policy,
            competence_trigger,
            execution_group,
            simple_probability,
            simple_threshold_millis,
            selection_reason,
            strategy_id,
            schema_version,
            payload_digest,
            branch_id,
            complexity_score,
            threshold,
            decision_source,
            reason_codes,
            matched_user_phrase_ids,
            fallback_used,
            classification_duration_micros,
            fallback_reason,
        } = value;
        Self {
            extension_reason,
            decision_protocol,
            routing_observation,
            policy,
            competence_trigger,
            execution_group,
            simple_probability,
            simple_threshold_millis,
            selection_reason,
            strategy_id,
            schema_version,
            payload_digest,
            branch_id,
            complexity_score,
            threshold,
            decision_source,
            reason_codes,
            matched_user_phrase_ids,
            fallback_used,
            classification_duration_micros,
            fallback_reason,
            historical_protection,
        }
    }
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingTimelineRelayV1 {
    pub request_id: String,
    pub from_model: Option<String>,
    pub to_model: Option<String>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Serialize)]
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

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingTimelineCostV1 {
    pub model_configuration_id: Option<String>,
    pub currency: Option<String>,
    pub valuation_kind: Option<String>,
    pub known_micros: Option<u64>,
    pub unknown_contributions: u64,
}
