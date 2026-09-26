//! A complete version cannot claim different authoring arrays/native choices than its executable.
use super::version::PlanVersionError;
use crate::{
    AgentPlanAuthoringV2, AgentPlanStrategyV2, CandidateSelectionV1, CompiledAgentPlanV1,
    ExactNativeReasoningV1, MaterializedCostPolicyV1, MaterializedGroupId as G,
    ReasoningSelectionV1, RequestOwnedRouteV1,
};

pub(super) fn validate_execution(
    configuration: &AgentPlanAuthoringV2,
    compiled: &CompiledAgentPlanV1,
) -> Result<(), PlanVersionError> {
    let materialized = &compiled.body.materialized;
    let desired: Vec<(G, &[CandidateSelectionV1])> =
        match (&configuration.strategy, &materialized.request_owned) {
            (
                AgentPlanStrategyV2::Custom { candidates },
                RequestOwnedRouteV1::Ordered {
                    cost_policy: MaterializedCostPolicyV1::ApiEquivalent,
                    ordered_groups,
                },
            ) if ordered_groups == &[G::Custom] => vec![(G::Custom, candidates)],
            (
                AgentPlanStrategyV2::SmartSaving {
                    economy,
                    primary,
                    primary_fallback,
                    reselect_on_user_message,
                    classifier: classifier_mode,
                    complex_keywords,
                },
                RequestOwnedRouteV1::Classified {
                    classifier,
                    reselect_on_user_message: materialized_reselect,
                    simple_groups,
                    complex_groups,
                },
            ) => {
                let expected = if *primary_fallback {
                    vec![G::Economy, G::Primary]
                } else {
                    vec![G::Economy]
                };
                if simple_groups != &expected
                    || reselect_on_user_message != materialized_reselect
                    || complex_groups != &[G::Primary]
                    || classifier
                        != &crate::ComplexityClassifierV1::with_mode(
                            complex_keywords.clone(),
                            classifier_mode.clone(),
                        )
                        .map_err(|_| PlanVersionError::Invalid)?
                {
                    return Err(PlanVersionError::Invalid);
                }
                vec![(G::Economy, economy), (G::Primary, primary)]
            }
            (
                AgentPlanStrategyV2::FreeFirst {
                    candidates,
                    primary_fallback,
                    primary,
                },
                RequestOwnedRouteV1::Ordered {
                    cost_policy,
                    ordered_groups,
                },
            ) => {
                let expected = if *primary_fallback {
                    vec![G::Free, G::Primary]
                } else {
                    vec![G::Free]
                };
                let expected_cost = if *primary_fallback {
                    MaterializedCostPolicyV1::ApiEquivalent
                } else {
                    MaterializedCostPolicyV1::StrictFree
                };
                if ordered_groups != &expected || cost_policy != &expected_cost {
                    return Err(PlanVersionError::Invalid);
                }
                let mut groups = vec![(G::Free, candidates.as_slice())];
                if *primary_fallback {
                    groups.push((G::Primary, primary.as_slice()));
                }
                groups
            }
            _ => return Err(PlanVersionError::Invalid),
        };
    if desired.len() != materialized.attempt_owned.groups.len() {
        return Err(PlanVersionError::Invalid);
    }
    for (id, selected) in desired {
        let group = materialized
            .attempt_owned
            .groups
            .iter()
            .find(|g| g.group_id == id)
            .ok_or(PlanVersionError::Invalid)?;
        if group.candidates.len() != selected.len() {
            return Err(PlanVersionError::Invalid);
        }
        for (candidate, selected) in group.candidates.iter().zip(selected) {
            if candidate.binding_id != selected.binding_id
                || !same_reasoning(&selected.reasoning, &candidate.exact_reasoning)
            {
                return Err(PlanVersionError::Invalid);
            }
        }
    }
    Ok(())
}

fn same_reasoning(
    selection: &Option<ReasoningSelectionV1>,
    exact: &ExactNativeReasoningV1,
) -> bool {
    match (selection, exact) {
        (None, ExactNativeReasoningV1::Fixed { .. }) => true,
        (
            Some(ReasoningSelectionV1::Toggle { enabled: a }),
            ExactNativeReasoningV1::Toggle { enabled: b, .. },
        ) => a == b,
        (
            Some(ReasoningSelectionV1::Profile { profile: a }),
            ExactNativeReasoningV1::Profile { profile: b, .. },
        ) => a == b,
        (
            Some(ReasoningSelectionV1::Budget { tokens: a }),
            ExactNativeReasoningV1::Budget { tokens: b, .. },
        ) => a == b,
        _ => false,
    }
}
