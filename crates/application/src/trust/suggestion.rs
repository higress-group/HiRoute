use hiroute_domain::{TrustCountsV1, TrustLevelV1, TrustSuggestionV1};

pub const MIN_SCORED: u64 = 30;
pub const UPPER_PERCENT: u64 = 85;
pub const LOWER_PERCENT: u64 = 70;
pub const MIN_COVERAGE_PERCENT: u64 = 60;
pub const MIN_SESSIONS: u64 = 3;

/// Downward evidence always uses the complete active-revision history. Only
/// upward/division evidence uses the conservative post-announcement cohort.
pub fn suggest(
    threshold: u16,
    full: &TrustCountsV1,
    upward: &TrustCountsV1,
    previous_routing_available: bool,
) -> TrustSuggestionV1 {
    if full.complete >= MIN_SCORED && full.meets_floor * 100 < full.complete * LOWER_PERCENT {
        return match TrustLevelV1::step_down(threshold) {
            Some(level) => TrustSuggestionV1::StepDown {
                threshold_millis: level.threshold_millis,
            },
            None if previous_routing_available => TrustSuggestionV1::ReturnToPreviousRouting,
            None => TrustSuggestionV1::ChangeRegularModel,
        };
    }
    let mut missing = Vec::new();
    if upward.complete < MIN_SCORED {
        missing.push("complete_scores".into());
    }
    if upward.executed == 0 || upward.complete * 100 < upward.executed * MIN_COVERAGE_PERCENT {
        missing.push("scoring_coverage".into());
    }
    if upward.sessions < MIN_SESSIONS {
        missing.push("contributing_sessions".into());
    }
    if !missing.is_empty() {
        return TrustSuggestionV1::Keep { missing };
    }
    if upward.meets_floor * 100 >= upward.complete * UPPER_PERCENT {
        return match TrustLevelV1::step_up(threshold) {
            Some(level) => TrustSuggestionV1::StepUp {
                threshold_millis: level.threshold_millis,
            },
            None if threshold <= 500 => TrustSuggestionV1::ExploreDivision,
            None => TrustSuggestionV1::Keep {
                missing: Vec::new(),
            },
        };
    }
    TrustSuggestionV1::Keep {
        missing: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn counts(executed: u64, complete: u64, meets_floor: u64, sessions: u64) -> TrustCountsV1 {
        TrustCountsV1 {
            executed,
            complete,
            meets_floor,
            sessions,
            ..Default::default()
        }
    }
    #[test]
    fn sparse_success_does_not_raise_trust_but_sparse_failure_can_lower_it() {
        let sparse = counts(100, 30, 30, 3);
        assert!(matches!(
            suggest(800, &sparse, &sparse, false),
            TrustSuggestionV1::Keep { .. }
        ));
        let failed = counts(100, 30, 20, 1);
        assert_eq!(
            suggest(800, &failed, &sparse, false),
            TrustSuggestionV1::StepDown {
                threshold_millis: 900
            }
        );
    }
    #[test]
    fn announcement_requires_new_evidence_without_erasing_downward_history() {
        let good = counts(40, 40, 34, 3);
        assert!(matches!(
            suggest(800, &good, &TrustCountsV1::default(), false),
            TrustSuggestionV1::Keep { .. }
        ));
        assert_eq!(
            suggest(720, &good, &good, false),
            TrustSuggestionV1::StepUp {
                threshold_millis: 650
            }
        );
        let bad = counts(40, 40, 27, 3);
        assert_eq!(
            suggest(720, &bad, &good, false),
            TrustSuggestionV1::StepDown {
                threshold_millis: 800
            }
        );
    }
    #[test]
    fn exact_boundaries_and_extremes_do_not_round_custom_values() {
        let eligible = counts(50, 30, 26, 3);
        assert_eq!(
            suggest(500, &eligible, &eligible, false),
            TrustSuggestionV1::ExploreDivision
        );
        assert_eq!(
            suggest(400, &eligible, &eligible, false),
            TrustSuggestionV1::ExploreDivision
        );
        let at_floor = counts(30, 30, 21, 1);
        assert!(matches!(
            suggest(800, &at_floor, &at_floor, false),
            TrustSuggestionV1::Keep { .. }
        ));
        let below = counts(30, 30, 20, 1);
        assert_eq!(
            suggest(900, &below, &eligible, true),
            TrustSuggestionV1::ReturnToPreviousRouting
        );
        assert_eq!(
            suggest(900, &below, &eligible, false),
            TrustSuggestionV1::ChangeRegularModel
        );
    }
}
