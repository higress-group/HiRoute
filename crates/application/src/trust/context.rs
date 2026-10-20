use hiroute_domain::{AgentPlanAuthoringV2, CanonicalDigest, PlanExecutionRef};
use std::collections::BTreeMap;

/// One captured control/catalog view. It contains no mutable execution authority.
#[derive(Clone, Debug)]
pub struct TrustContextV1 {
    pub active: PlanExecutionRef,
    pub enabled: bool,
    pub configuration: AgentPlanAuthoringV2,
    pub binding_models: BTreeMap<String, String>,
    pub identity_stability: BTreeMap<String, String>,
    pub announcements: Vec<TrustAnnouncementV1>,
    pub generation: CanonicalDigest,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustAnnouncementV1 {
    pub model_configuration_id: String,
    pub included_on: String,
    pub cohort_from_ms: i64,
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct TrustRunV1 {
    pub run_id: String,
    pub task_id: String,
    pub state: hiroute_domain::delegation::RunStateV1,
    pub ordinal: u64,
}

/// The day after an editorial announcement (UTC), never a provider version fact.
pub fn announcement_cohort_start(date: &str) -> Option<i64> {
    if date.len() != 10 || !date.is_ascii() || &date[4..5] != "-" || &date[7..8] != "-" {
        return None;
    }
    let year: i64 = date[..4].parse().ok()?;
    let month: usize = date[5..7].parse().ok()?;
    let day: i64 = date[8..].parse().ok()?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let months = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if year < 1 || !(1..=12).contains(&month) || day < 1 || day > months[month - 1] {
        return None;
    }
    let prior = year - 1;
    let days = prior * 365 + prior / 4 - prior / 100
        + prior / 400
        + months[..month - 1].iter().sum::<i64>()
        + day
        - 719_163;
    Some((days + 1) * 86_400_000)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn announcement_day_is_excluded_even_for_fractional_day_execution_times() {
        assert_eq!(announcement_cohort_start("1970-01-01"), Some(86_400_000));
        assert_eq!(
            announcement_cohort_start("2024-02-29"),
            Some(1_709_251_200_000)
        );
        assert_eq!(announcement_cohort_start("2023-02-29"), None);
    }
}
