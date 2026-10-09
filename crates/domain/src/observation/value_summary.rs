use crate::ObservationMetricCoverageV2;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationValueQueryV2 {
    pub from_ms: i64,
    pub to_ms: i64,
    pub session_id: Option<String>,
    pub plan_id: Option<String>,
    pub currency: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservationValueReportV2 {
    pub summary: ObservationValueSummaryV2,
    pub group_by: crate::ValueGroupByV1,
    pub day_timezone: Option<String>,
    pub days: Vec<ObservationValueSummaryV2>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservationValueTotalV2 {
    pub currency: String,
    pub valuation_kind: String,
    pub known_sum_micros: Option<u64>,
    pub coverage: ObservationMetricCoverageV2,
    pub missing_contribution_count: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservationUsageTotalV2 {
    pub metric: String,
    pub known_sum: Option<u64>,
    pub coverage: ObservationMetricCoverageV2,
    pub missing_attempt_count: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationCacheHitStateV2 {
    Available,
    NotApplicable,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservationMaintenanceStatusV2 {
    pub running: bool,
    pub index_running: bool,
    pub error_count: u64,
}
