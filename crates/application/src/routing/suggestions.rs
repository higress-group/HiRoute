//! Read-only free suggestions. Adopting a suggestion is a separate editor action; neither this
//! query nor a rating refresh mutates a saved candidate array or publication.
use std::collections::BTreeMap;

use hiroute_application_api::{
    MAX_RATING_QUERY_ITEMS, ModelRatingQueryItemV1, RatingSnapshotSelectionV1,
    ResolveModelRatingsV1,
};
use hiroute_domain::*;

use crate::compiler::{AgentPlanCompilationFactsV1, AgentPlanCompilerError};
use crate::model_catalog::{ModelRatings, RatingQueryError};

pub use hiroute_application_api::{
    FreeSuggestionV1, FreeSuggestionsV1, SuggestionUnavailableReasonV1,
};

pub fn suggest_free(
    facts: &AgentPlanCompilationFactsV1,
    requirements: &CapabilityRequirementsV1,
    selections: &BTreeMap<String, ReasoningSelectionV1>,
    ratings: &ModelRatings,
    snapshot: RatingSnapshotSelectionV1,
) -> Result<FreeSuggestionsV1, AgentPlanCompilerError> {
    facts.validate_snapshot()?;
    requirements
        .validate()
        .map_err(|_| AgentPlanCompilerError::InvalidDesiredPlan)?;
    if selections
        .keys()
        .any(|id| !facts.candidates.iter().any(|f| &f.binding.binding_id == id))
    {
        return Err(AgentPlanCompilerError::UnknownAutomaticReasoningBinding);
    }
    let mut result = FreeSuggestionsV1 {
        snapshot_ref: None,
        candidates: vec![],
        unavailable: BTreeMap::new(),
    };
    for fact in &facts.candidates {
        let id = &fact.binding.binding_id;
        if fact.binding.billing_class != BillingClass::Free || fact.free_evidence.is_none() {
            result
                .unavailable
                .insert(id.clone(), SuggestionUnavailableReasonV1::NotFree);
            continue;
        }
        let attempt = match crate::compiler::resolve_suggestion_attempt(
            fact,
            selections.get(id),
            requirements,
        ) {
            Ok(attempt) => attempt,
            Err(error) => {
                let reason = match error {
                    AgentPlanCompilerError::Reasoning(
                        ReasoningContractError::SelectionRequired
                        | ReasoningContractError::BudgetRequired,
                    ) => SuggestionUnavailableReasonV1::NativeSelectionRequired,
                    AgentPlanCompilerError::Reasoning(_) => {
                        SuggestionUnavailableReasonV1::NativeSelectionInvalid
                    }
                    AgentPlanCompilerError::CapabilityUnqualified(_) => {
                        SuggestionUnavailableReasonV1::CapabilityUnqualified
                    }
                    _ => SuggestionUnavailableReasonV1::NotRoutable,
                };
                result.unavailable.insert(id.clone(), reason);
                continue;
            }
        };
        let reasoning = match &attempt.exact_reasoning {
            ExactNativeReasoningV1::Fixed { .. } => None,
            ExactNativeReasoningV1::Toggle { enabled, .. } => {
                Some(ReasoningSelectionV1::Toggle { enabled: *enabled })
            }
            ExactNativeReasoningV1::Profile { profile, .. } => {
                Some(ReasoningSelectionV1::Profile {
                    profile: profile.clone(),
                })
            }
            ExactNativeReasoningV1::Budget { tokens, .. } => {
                Some(ReasoningSelectionV1::Budget { tokens: *tokens })
            }
        };
        result.candidates.push(FreeSuggestionV1 {
            selection: CandidateSelectionV1 {
                binding_id: id.clone(),
                reasoning,
            },
            model_configuration_id: attempt.model_configuration_id,
            exact_native_reasoning: attempt.exact_reasoning,
            rating: None,
        });
    }
    if result.candidates.len() > MAX_RATING_QUERY_ITEMS {
        return Err(AgentPlanCompilerError::SuggestionLimitExceeded);
    }
    if !result.candidates.is_empty() {
        let query = ResolveModelRatingsV1 {
            snapshot: snapshot.clone(),
            items: result
                .candidates
                .iter()
                .enumerate()
                .map(|(index, c)| ModelRatingQueryItemV1 {
                    query_id: index.to_string(),
                    model_configuration_id: c.model_configuration_id.clone(),
                    exact_native_reasoning: c.exact_native_reasoning.clone(),
                })
                .collect(),
        };
        match ratings.resolve(&query) {
            Ok(resolved) => {
                result.snapshot_ref = Some(resolved.snapshot_ref);
                for (candidate, rating) in result.candidates.iter_mut().zip(resolved.items) {
                    candidate.rating = Some(rating.rating);
                }
            }
            Err(RatingQueryError::SnapshotUnavailable)
                if snapshot == RatingSnapshotSelectionV1::Latest => {}
            Err(RatingQueryError::SnapshotUnavailable) => {
                return Err(AgentPlanCompilerError::RatingSnapshotUnavailable);
            }
            Err(RatingQueryError::InvalidArguments) => {
                return Err(AgentPlanCompilerError::InvalidDesiredPlan);
            }
        }
    }
    result.candidates.sort_by(|a, b| {
        score(b)
            .cmp(&score(a))
            .then_with(|| a.selection.binding_id.cmp(&b.selection.binding_id))
    });
    Ok(result)
}
fn score(candidate: &FreeSuggestionV1) -> Option<u8> {
    match candidate.rating.as_ref().map(|r| &r.overall) {
        Some(
            RatingValueV1::Reference { score_tenths, .. }
            | RatingValueV1::Estimated { score_tenths, .. },
        ) => Some(*score_tenths),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
