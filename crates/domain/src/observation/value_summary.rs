use crate::ObservationMetricCoverageV2;
use serde::{Deserialize, Serialize};
#[derive(schemars::JsonSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationValueQueryV2 {
    pub from_ms: i64,
    pub to_ms: i64,
    pub session_id: Option<String>,
    pub plan_id: Option<String>,
    pub currency: Option<String>,
}
#[derive(schemars::JsonSchema, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservationValueSummaryV2 {
    pub from_ms: i64,
    pub to_ms: i64,
    pub pending_requests: u64,
    pub provisional_requests: u64,
    pub unknown_traffic_requests: u64,
    pub excluded_requests: u64,
    pub amounts: Vec<ObservationValueTotalV2>,
    pub archive_boundary_partial: bool,
    /// A session/run scope cannot recover details outside the retention window.
    pub retention_boundary_partial: bool,
    pub usage: Vec<ObservationUsageTotalV2>,
    pub input_cache_hit: ObservationCacheHitSummaryV2,
}

/// Summary and optional UTC day buckets from one database snapshot. Partial
/// first/last days retain their actual half-open query bounds.
#[derive(schemars::JsonSchema, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservationValueReportV2 {
    pub summary: ObservationValueSummaryV2,
    pub group_by: crate::ValueGroupByV1,
    pub day_timezone: Option<String>,
    pub days: Vec<ObservationValueSummaryV2>,
}
#[derive(schemars::JsonSchema, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservationValueTotalV2 {
    pub currency: String,
    pub valuation_kind: String,
    pub known_sum_micros: Option<u64>,
    pub coverage: ObservationMetricCoverageV2,
    pub missing_contribution_count: u64,
}
#[derive(schemars::JsonSchema, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservationUsageTotalV2 {
    pub metric: String,
    pub known_sum: Option<u64>,
    pub coverage: ObservationMetricCoverageV2,
    pub missing_attempt_count: u64,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationCacheHitStateV2 {
    Available,
    NotApplicable,
    Unknown,
}

#[derive(schemars::JsonSchema, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationCacheHitSummaryV2 {
    pub state: ObservationCacheHitStateV2,
    /// Percentage basis points: 1727 represents 17.27%.
    pub ratio_basis_points: Option<u32>,
    pub cache_read_tokens: Option<u64>,
    pub total_input_tokens: Option<u64>,
    pub eligible_attempt_count: u64,
    pub total_attempt_count: u64,
    pub zero_input_attempt_count: u64,
    pub missing_attempt_count: u64,
    pub invalid_attempt_count: u64,
    pub arithmetic_overflow: bool,
    pub archive_coverage_partial: bool,
    pub coverage: ObservationMetricCoverageV2,
}

#[derive(schemars::JsonSchema, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservationMaintenanceStatusV2 {
    pub running: bool,
    pub index_running: bool,
    pub error_count: u64,
}

/// Explicit read-time counterfactual, separate from immutable request settlement.
#[derive(schemars::JsonSchema, Clone, Debug, Serialize, Deserialize)]
pub struct ApiEquivalentPriceV1 {
    pub binding_id: String,
    /// Exact execution identity derived from this source/model's materialization facts.
    /// A local execution identity can differ from the canonical manual-price locator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_model_configuration_id: Option<String>,
    pub quote: crate::FrozenPriceQuoteV1,
}

/// Bounds for a same-usage scenario when the exclusive cache-write split was
/// not reported. They do not assert an observed write count or actual bill.
#[derive(schemars::JsonSchema, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApiEquivalentBoundsV1 {
    pub reference_min_micros: u64,
    pub reference_max_micros: u64,
    /// Routed amount minus reference; a negative value is cheaper routing.
    pub difference_min_micros: i64,
    pub difference_max_micros: i64,
}

#[derive(schemars::JsonSchema, Clone, Debug, Serialize, Deserialize)]
pub struct ApiEquivalentValueReportV1 {
    pub schema: String,
    pub from_ms: i64,
    pub to_ms: i64,
    pub evaluated_at_ms: i64,
    pub currency: String,
    pub reference: ApiEquivalentPriceV1,
    pub prices: Vec<ApiEquivalentPriceV1>,
    pub request_count: u64,
    pub compared_request_count: u64,
    pub arithmetic_overflow: bool,
    pub missing_request_count: u64,
    pub excluded_request_count: u64,
    pub retention_boundary_partial: bool,
    pub coverage: ObservationMetricCoverageV2,
    pub routed_estimate_micros: Option<u64>,
    pub reference_estimate_micros: Option<u64>,
    pub estimated_savings_micros: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub same_usage_bounds: Option<ApiEquivalentBoundsV1>,
    pub compared_routed_subtotal_micros: Option<u64>,
    pub compared_reference_subtotal_micros: Option<u64>,
    pub actual_cash_micros: Option<u64>,
    pub disclosure: String,
}
