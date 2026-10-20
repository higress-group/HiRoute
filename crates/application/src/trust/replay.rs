use hiroute_domain::{
    ExecutionGroupV1, ModelGroupReasonV1 as Reason, PlanTrustReplayV1, TrustObservedStageV1,
    TrustReplayThresholdV1,
};
use std::collections::BTreeMap;

pub fn replay(stages: &[TrustObservedStageV1], thresholds: &[u16]) -> PlanTrustReplayV1 {
    let mut excluded = BTreeMap::new();
    let mut included = Vec::new();
    for stage in stages {
        let sample = &stage.sample;
        let reason = sample.selection.as_ref().map(|s| s.selection_reason);
        let exclusion = match reason {
            Some(Reason::Heuristic) => Some("heuristic"),
            Some(Reason::DecisionFallback) => Some("decision_fallback"),
            Some(Reason::SingleGroup) => Some("single_group"),
            Some(Reason::AvailabilityRelay) => Some("availability_relay"),
            _ if sample
                .selection
                .as_ref()
                .zip(sample.branch_execution.as_ref())
                .is_some_and(|(s, e)| s.execution_group != e.group) =>
            {
                Some("availability_relay")
            }
            Some(Reason::DegreeUnavailable) | None => Some("degree_unavailable"),
            _ => None,
        };
        if let Some(reason) = exclusion {
            *excluded.entry(reason.into()).or_insert(0) += 1;
            continue;
        }
        let selection = sample.selection.as_ref().expect("checked selection");
        let protected = selection.selection_reason == Reason::LowCompetence
            || selection.competence_trigger.is_some();
        let probability = selection
            .simple_probability
            .as_ref()
            .and_then(|p| p.as_f64())
            .filter(|p| p.is_finite() && (0.0..=1.0).contains(p));
        if probability.is_none() && !protected {
            *excluded.entry("degree_unavailable".into()).or_insert(0) += 1;
            continue;
        }
        included.push((
            probability,
            protected,
            sample.branch_execution.as_ref().map(|e| e.group),
        ));
    }
    let total = included.len() as u64;
    let share = |count: u64| (total > 0).then(|| count as f64 / total as f64);
    PlanTrustReplayV1 {
        included: total,
        excluded_by_reason: excluded,
        recorded_share_at_current: share(
            included
                .iter()
                .filter(|(_, _, group)| *group == Some(ExecutionGroupV1::Regular))
                .count() as u64,
        ),
        thresholds: thresholds
            .iter()
            .map(|&threshold_millis| {
                let mut result = TrustReplayThresholdV1 {
                    threshold_millis,
                    regular: 0,
                    primary: 0,
                    protected_as_primary: 0,
                    newly_regular: 0,
                    regular_share: None,
                    primary_share: None,
                    protected_share: None,
                };
                for (probability, protected, group) in &included {
                    if *protected {
                        result.protected_as_primary += 1;
                        result.primary += 1;
                    } else if probability.is_some_and(|p| p >= f64::from(threshold_millis) / 1000.0)
                    {
                        result.regular += 1;
                        if *group == Some(ExecutionGroupV1::Primary) {
                            result.newly_regular += 1;
                        }
                    } else {
                        result.primary += 1;
                    }
                }
                result.regular_share = share(result.regular);
                result.primary_share = share(result.primary);
                result.protected_share = share(result.protected_as_primary);
                result
            })
            .collect(),
        estimate_only: true,
    }
}
