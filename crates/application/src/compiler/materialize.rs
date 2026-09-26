#[cfg(test)]
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
use hiroute_domain::{
    AGENT_PLAN_COMPILED_SCHEMA_V1, AGENT_PLAN_COMPILER_REVISION_V1, AgentPlanDesiredV1,
    AgentPlanIdentityV1, AgentPlanStrategyV1, AttemptOwnedRouteV1, CompiledAgentPlanBodyV1,
    ComplexityClassifierV1, FreeFallbackPolicy, FreePoolMode, MaterializedAgentPlanV1,
    MaterializedCostPolicyV1, MaterializedGroupId, MaterializedModelGroupV1,
    MaterializedOrderingV1, MaterializedRatingFactV1, RequestOwnedRouteV1, RoutingStrategyError,
};
use hiroute_domain::{
    AttemptOwnedCandidateV1, BillingClass, CanonicalDigest, CapabilityRequirementsV1,
    CompiledAgentPlanV1, DiscreteReasoningDefault, FreeAccess, GatewayAccessGrantV1,
    GatewayPublicationRevision, GatewayPublicationV1, MaterializedFreeOfferRefV1,
    ReasoningContractError,
};
#[cfg(test)]
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(test)]
use super::AgentPlanCompilationFactsV1;
use super::{CandidateCompilationFactV1, CompilerFactError};

#[cfg(test)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CandidateExclusionReason {
    NotRoutable,
    NotFree,
    MissingRating,
    ReasoningSelectionRequired,
    ReasoningBudgetRequired,
    CapabilityUnqualified,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExcludedCandidateV1 {
    pub(crate) binding_id: String,
    pub(crate) reason: CandidateExclusionReason,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MaterializationResultV1 {
    pub(crate) materialized: MaterializedAgentPlanV1,
    #[serde(default)]
    pub(crate) excluded_candidates: Vec<ExcludedCandidateV1>,
}

#[cfg(test)]
pub(crate) fn materialize_agent_plan(
    desired: &AgentPlanDesiredV1,
    facts: &AgentPlanCompilationFactsV1,
) -> Result<MaterializationResultV1, AgentPlanCompilerError> {
    desired.validate().map_err(|error| match error {
        RoutingStrategyError::NoFreeCandidates => AgentPlanCompilerError::NoFreeCandidates,
        _ => AgentPlanCompilerError::InvalidDesiredPlan,
    })?;
    facts.validate()?;
    let by_binding = facts
        .candidates
        .iter()
        .map(|fact| (fact.binding.binding_id.as_str(), fact))
        .collect::<BTreeMap<_, _>>();

    let (request_owned, groups, mut excluded_candidates) = match &desired.strategy {
        AgentPlanStrategyV1::SmartSaving {
            economy_candidates,
            quality_anchor_binding_id,
            primary_candidates,
            quality_guard_score_gap_tenths,
            complex_keywords,
        } => {
            require_ratings(economy_candidates, &by_binding)?;
            let quality_anchor_score = by_binding
                .get(quality_anchor_binding_id.as_str())
                .and_then(|fact| fact.rating.as_ref())
                .map(|rating| rating.overall_score_tenths)
                .ok_or(AgentPlanCompilerError::QualityAnchorUnavailable)?;
            let mut economy = resolve_explicit(
                economy_candidates,
                &by_binding,
                &desired.requirements,
                DiscreteReasoningDefault::Lowest,
            )?;
            economy.retain(|candidate| {
                quality_anchor_score.saturating_sub(candidate.rating)
                    <= *quality_guard_score_gap_tenths
            });
            economy.sort_by(|left, right| {
                left.cost
                    .is_none()
                    .cmp(&right.cost.is_none())
                    .then_with(|| left.cost.cmp(&right.cost))
                    .then_with(|| right.rating.cmp(&left.rating))
                    .then_with(|| left.attempt.binding_id.cmp(&right.attempt.binding_id))
            });

            require_ratings(primary_candidates, &by_binding)?;
            let mut primary = resolve_explicit(
                primary_candidates,
                &by_binding,
                &desired.requirements,
                DiscreteReasoningDefault::Highest,
            )?;
            primary.sort_by(quality_order);
            let classifier = ComplexityClassifierV1::new(complex_keywords.clone())
                .map_err(|_| AgentPlanCompilerError::InvalidDesiredPlan)?;
            (
                RequestOwnedRouteV1::Classified {
                    classifier,
                    reselect_on_user_message: false,
                    simple_groups: vec![MaterializedGroupId::Economy, MaterializedGroupId::Primary],
                    complex_groups: vec![MaterializedGroupId::Primary],
                },
                vec![
                    build_group(
                        MaterializedGroupId::Economy,
                        MaterializedOrderingV1::CheapestWithRatingGuard {
                            quality_anchor_binding_id: quality_anchor_binding_id.clone(),
                            maximum_score_gap_tenths: *quality_guard_score_gap_tenths,
                        },
                        economy,
                    )?,
                    build_group(
                        MaterializedGroupId::Primary,
                        MaterializedOrderingV1::QualityFirst,
                        primary,
                    )?,
                ],
                Vec::new(),
            )
        }
        AgentPlanStrategyV1::FreeFirst {
            free_pool,
            fallback_policy,
            primary_candidates,
        } => {
            let (free, exclusions) = match free_pool.mode {
                FreePoolMode::AutomaticAllAvailable => automatic_free_candidates(
                    facts,
                    &free_pool.automatic_reasoning,
                    &desired.requirements,
                )?,
                FreePoolMode::Manual => {
                    let values = resolve_explicit(
                        &free_pool.candidates,
                        &by_binding,
                        &desired.requirements,
                        DiscreteReasoningDefault::RequireExplicit,
                    )?;
                    if values
                        .iter()
                        .any(|value| value.attempt.billing_class != BillingClass::Free)
                    {
                        return Err(AgentPlanCompilerError::NonFreeCandidate);
                    }
                    (values, Vec::new())
                }
            };
            if free.is_empty() {
                return Err(AgentPlanCompilerError::NoFreeCandidates);
            }
            let free_ordering = match free_pool.mode {
                FreePoolMode::AutomaticAllAvailable => MaterializedOrderingV1::FreeScoreDescending,
                FreePoolMode::Manual => MaterializedOrderingV1::ExplicitOrder,
            };
            let mut groups = vec![build_group(MaterializedGroupId::Free, free_ordering, free)?];
            let (cost_policy, ordered_groups) = match fallback_policy {
                FreeFallbackPolicy::FreeOnly => (
                    MaterializedCostPolicyV1::StrictFree,
                    vec![MaterializedGroupId::Free],
                ),
                FreeFallbackPolicy::PrimaryFallback => {
                    require_ratings(primary_candidates, &by_binding)?;
                    let mut primary = resolve_explicit(
                        primary_candidates,
                        &by_binding,
                        &desired.requirements,
                        DiscreteReasoningDefault::Highest,
                    )?;
                    primary.sort_by(quality_order);
                    groups.push(build_group(
                        MaterializedGroupId::Primary,
                        MaterializedOrderingV1::QualityFirst,
                        primary,
                    )?);
                    (
                        MaterializedCostPolicyV1::ApiEquivalent,
                        vec![MaterializedGroupId::Free, MaterializedGroupId::Primary],
                    )
                }
            };
            (
                RequestOwnedRouteV1::Ordered {
                    cost_policy,
                    ordered_groups,
                },
                groups,
                exclusions,
            )
        }
        AgentPlanStrategyV1::Custom { candidates } => {
            let values = resolve_explicit(
                candidates,
                &by_binding,
                &desired.requirements,
                DiscreteReasoningDefault::RequireExplicit,
            )?;
            (
                RequestOwnedRouteV1::Ordered {
                    cost_policy: MaterializedCostPolicyV1::ApiEquivalent,
                    ordered_groups: vec![MaterializedGroupId::Custom],
                },
                vec![build_group(
                    MaterializedGroupId::Custom,
                    MaterializedOrderingV1::ExplicitOrder,
                    values,
                )?],
                Vec::new(),
            )
        }
    };
    excluded_candidates.sort_by(|left, right| {
        left.binding_id
            .cmp(&right.binding_id)
            .then_with(|| left.reason.cmp(&right.reason))
    });
    let materialized = MaterializedAgentPlanV1 {
        fact_refs: facts.refs.clone(),
        request_owned,
        attempt_owned: AttemptOwnedRouteV1 {
            limits: desired.limits.clone(),
            groups,
        },
    };
    materialized
        .validate()
        .map_err(|_| AgentPlanCompilerError::InvalidCompiledPlan)?;
    Ok(MaterializationResultV1 {
        materialized,
        excluded_candidates,
    })
}

#[cfg(test)]
pub(crate) fn compile_agent_plan(
    identity: AgentPlanIdentityV1,
    agent_plan_revision: u64,
    desired: &AgentPlanDesiredV1,
    facts: &AgentPlanCompilationFactsV1,
) -> Result<CompiledAgentPlanV1, AgentPlanCompilerError> {
    if identity.display_name != desired.display_name || identity.purpose != desired.purpose {
        return Err(AgentPlanCompilerError::IdentityMismatch);
    }
    let materialized = materialize_agent_plan(desired, facts)?.materialized;
    let materialized_route_digest = materialized
        .legacy_route_digest()
        .map_err(|_| AgentPlanCompilerError::InvalidCompiledPlan)?;
    let body = CompiledAgentPlanBodyV1 {
        schema: AGENT_PLAN_COMPILED_SCHEMA_V1.to_owned(),
        compiler_revision: AGENT_PLAN_COMPILER_REVISION_V1.to_owned(),
        identity,
        agent_plan_revision,
        materialized_route_digest,
        materialized,
    };
    let digest =
        CanonicalDigest::of(&body).map_err(|_| AgentPlanCompilerError::InvalidCompiledPlan)?;
    CompiledAgentPlanV1::authenticate_persisted(body, digest)
        .map_err(|_| AgentPlanCompilerError::InvalidCompiledPlan)
}

#[allow(clippy::too_many_arguments)]
pub fn compile_publication(
    workspace_id: hiroute_domain::WorkspaceId,
    authority_id: impl Into<String>,
    authority_epoch: u64,
    revision: GatewayPublicationRevision,
    catalog_renderer_revision: impl Into<String>,
    alias_registry: hiroute_domain::AliasRegistryV1,
    plans: Vec<CompiledAgentPlanV1>,
    grants: Vec<GatewayAccessGrantV1>,
) -> Result<GatewayPublicationV1, AgentPlanCompilerError> {
    GatewayPublicationV1::seal(
        workspace_id,
        authority_id,
        authority_epoch,
        revision,
        catalog_renderer_revision,
        alias_registry,
        plans,
        grants,
    )
    .map_err(|_| AgentPlanCompilerError::InvalidPublication)
}

#[cfg(test)]
#[derive(Clone)]
struct ResolvedCandidate {
    attempt: AttemptOwnedCandidateV1,
    rating: u8,
    cost: Option<u64>,
    rating_fact: MaterializedRatingFactV1,
}

#[cfg(test)]
fn resolve_explicit(
    selections: &[hiroute_domain::CandidateSelectionV1],
    by_binding: &BTreeMap<&str, &CandidateCompilationFactV1>,
    requirements: &CapabilityRequirementsV1,
    default: DiscreteReasoningDefault,
) -> Result<Vec<ResolvedCandidate>, AgentPlanCompilerError> {
    selections
        .iter()
        .map(|selection| {
            let fact = by_binding
                .get(selection.binding_id.as_str())
                .copied()
                .ok_or_else(|| {
                    AgentPlanCompilerError::UnknownBinding(selection.binding_id.clone())
                })?;
            resolve_candidate(fact, selection.reasoning.as_ref(), requirements, default)
        })
        .collect()
}

#[cfg(test)]
fn require_ratings(
    selections: &[hiroute_domain::CandidateSelectionV1],
    by_binding: &BTreeMap<&str, &CandidateCompilationFactV1>,
) -> Result<(), AgentPlanCompilerError> {
    for selection in selections {
        let fact = by_binding
            .get(selection.binding_id.as_str())
            .copied()
            .ok_or_else(|| AgentPlanCompilerError::UnknownBinding(selection.binding_id.clone()))?;
        if fact.rating.is_none() {
            return Err(AgentPlanCompilerError::RatingRequired);
        }
    }
    Ok(())
}

#[cfg(test)]
fn resolve_candidate(
    fact: &CandidateCompilationFactV1,
    selection: Option<&hiroute_domain::ReasoningSelectionV1>,
    requirements: &CapabilityRequirementsV1,
    default: DiscreteReasoningDefault,
) -> Result<ResolvedCandidate, AgentPlanCompilerError> {
    let attempt = resolve_attempt(fact, selection, requirements, default)?;
    let rating_fact = fact
        .rating
        .as_ref()
        .ok_or(AgentPlanCompilerError::RatingRequired)?;
    let rating = rating_fact.overall_score_tenths;
    let cost = fact
        .ordering_price
        .as_ref()
        .map(|price| price.total_micros_per_million())
        .transpose()?;
    Ok(ResolvedCandidate {
        attempt,
        rating,
        cost,
        rating_fact: MaterializedRatingFactV1 {
            model_configuration_id: rating_fact.model_configuration_id.clone(),
            overall_score_tenths: rating_fact.overall_score_tenths,
            rating_count: rating_fact.rating_count,
        },
    })
}

/// Resolve only executable facts; display ratings and prices are not admission requirements.
pub(super) fn resolve_attempt(
    fact: &CandidateCompilationFactV1,
    selection: Option<&hiroute_domain::ReasoningSelectionV1>,
    requirements: &CapabilityRequirementsV1,
    default: DiscreteReasoningDefault,
) -> Result<AttemptOwnedCandidateV1, AgentPlanCompilerError> {
    if !fact.is_routable() {
        return Err(AgentPlanCompilerError::CandidateNotRoutable(
            fact.binding.binding_id.clone(),
        ));
    }
    qualify_capabilities(fact, requirements)?;
    if fact.binding.billing_class == BillingClass::Free
        && fact
            .free_evidence
            .as_ref()
            .is_some_and(|free| free.access == FreeAccess::ApiKeyRequired)
        && fact.binding.credential_pool_id.is_none()
    {
        return Err(AgentPlanCompilerError::CandidateNotRoutable(
            fact.binding.binding_id.clone(),
        ));
    }
    let exact_reasoning = fact.reasoning.resolve(selection, default)?;
    let mut protocol_profiles = fact.protocol_profiles.clone();
    for profile in &mut protocol_profiles {
        profile.capability.selected_reasoning_profile_id = profile
            .reasoning_profile_for(&exact_reasoning)
            .map_err(|_| AgentPlanCompilerError::InvalidCompiledPlan)?
            .profile_id
            .clone();
    }
    let attempt = AttemptOwnedCandidateV1 {
        binding_id: fact.binding.binding_id.clone(),
        binding_revision: fact.binding.revision,
        binding_digest: CanonicalDigest::of(&fact.binding)
            .map_err(|_| AgentPlanCompilerError::Encoding)?,
        source_id: fact.binding.source_id.clone(),
        source_revision: fact.binding.source_revision,
        source_identity_digest: fact.binding.source_identity_digest.clone(),
        connection_option_id: fact.connection_option_id.clone(),
        offer_ref: fact.binding.offer_ref.clone(),
        offer_revision: fact.offer_revision,
        offer_evidence_digest: fact.binding.offer_evidence_digest.clone(),
        billing_class: fact.binding.billing_class,
        model_configuration_id: fact.binding.model_configuration_id.clone(),
        model_configuration_revision: fact.model.revision,
        upstream_model_id: fact.binding.upstream_model_id.clone(),
        native_transport_model: fact.native_transport_model.clone(),
        capability_id: fact.capability.capability_id.clone(),
        capability_revision: fact.capability.revision,
        capability_evidence_digest: fact.capability.evidence_digest.clone(),
        connector_id: fact.capability.connector_id.clone(),
        connector_revision: fact.capability.connector_revision,
        endpoint_profile_id: fact.capability.endpoint_profile_id.clone(),
        endpoint_profile_revision: fact.capability.endpoint_profile_revision,
        protocol_endpoint_id: fact.capability.protocol_endpoint_id.clone(),
        endpoint: format!(
            "{}{}",
            fact.protocol_endpoint.base_url, fact.protocol_endpoint.request_path
        ),
        connector_runtime: fact.connector_runtime,
        operational_target: fact.operational_target.clone(),
        operational_target_digest: CanonicalDigest::of(&fact.operational_target)
            .map_err(|_| AgentPlanCompilerError::Encoding)?,
        protocol_profile_digest: CanonicalDigest::of(&protocol_profiles)
            .map_err(|_| AgentPlanCompilerError::Encoding)?,
        protocol_profiles,
        upstream_protocol: fact.capability.upstream_protocol,
        adapter_ref: fact.capability.required_adapter_ref.clone(),
        adapter_revision: fact.capability.required_adapter_revision,
        credential_refs: fact.credential_refs.clone(),
        credential_destination_ref: fact.credential_destination_ref.clone(),
        credential_pool_id: fact.binding.credential_pool_id.clone(),
        free_offer: fact
            .free_evidence
            .as_ref()
            .map(|free| MaterializedFreeOfferRefV1 {
                free_offer_id: free.free_offer_id.clone(),
                free_offer_revision: free.free_offer_revision,
                offer_ref: free.offer_ref.clone(),
                access: free.access,
                evidence_digest: free.evidence_digest.clone(),
            }),
        exact_reasoning,
    };
    attempt
        .validate()
        .map_err(|_| AgentPlanCompilerError::InvalidCompiledPlan)?;
    Ok(attempt)
}

#[cfg(test)]
fn build_group(
    group_id: MaterializedGroupId,
    ordering_evidence: MaterializedOrderingV1,
    values: Vec<ResolvedCandidate>,
) -> Result<MaterializedModelGroupV1, AgentPlanCompilerError> {
    let pinned_ratings = values
        .iter()
        .map(|value| (value.attempt.binding_id.clone(), value.rating_fact.clone()))
        .collect();
    Ok(MaterializedModelGroupV1 {
        group_id,
        ordering_evidence,
        pinned_ratings,
        candidates: values.into_iter().map(|value| value.attempt).collect(),
    })
}

#[cfg(test)]
fn automatic_free_candidates(
    facts: &AgentPlanCompilationFactsV1,
    selections: &BTreeMap<String, hiroute_domain::ReasoningSelectionV1>,
    requirements: &CapabilityRequirementsV1,
) -> Result<(Vec<ResolvedCandidate>, Vec<ExcludedCandidateV1>), AgentPlanCompilerError> {
    let mut included = Vec::new();
    let mut excluded = Vec::new();
    let known = facts
        .candidates
        .iter()
        .filter(|fact| {
            fact.binding.billing_class == BillingClass::Free && fact.free_evidence.is_some()
        })
        .map(|fact| fact.binding.binding_id.as_str())
        .collect::<BTreeSet<_>>();
    if selections
        .keys()
        .any(|binding| !known.contains(binding.as_str()))
    {
        return Err(AgentPlanCompilerError::UnknownAutomaticReasoningBinding);
    }
    for fact in &facts.candidates {
        let reason =
            if fact.binding.billing_class != BillingClass::Free || fact.free_evidence.is_none() {
                Some(CandidateExclusionReason::NotFree)
            } else if !fact.is_routable()
                || (fact
                    .free_evidence
                    .as_ref()
                    .is_some_and(|free| free.access == FreeAccess::ApiKeyRequired)
                    && fact.binding.credential_pool_id.is_none())
            {
                Some(CandidateExclusionReason::NotRoutable)
            } else if !capabilities_satisfied(fact, requirements) {
                Some(CandidateExclusionReason::CapabilityUnqualified)
            } else if fact.rating.is_none() {
                Some(CandidateExclusionReason::MissingRating)
            } else {
                match resolve_candidate(
                    fact,
                    selections.get(&fact.binding.binding_id),
                    requirements,
                    DiscreteReasoningDefault::Lowest,
                ) {
                    Ok(value) => {
                        included.push(value);
                        None
                    }
                    Err(AgentPlanCompilerError::Reasoning(
                        ReasoningContractError::SelectionRequired,
                    )) => Some(CandidateExclusionReason::ReasoningSelectionRequired),
                    Err(AgentPlanCompilerError::Reasoning(
                        ReasoningContractError::BudgetRequired,
                    )) => Some(CandidateExclusionReason::ReasoningBudgetRequired),
                    Err(error) => return Err(error),
                }
            };
        if let Some(reason) = reason {
            excluded.push(ExcludedCandidateV1 {
                binding_id: fact.binding.binding_id.clone(),
                reason,
            });
        }
    }
    included.sort_by(|left, right| {
        right
            .rating
            .cmp(&left.rating)
            .then_with(|| {
                left.attempt
                    .model_configuration_id
                    .cmp(&right.attempt.model_configuration_id)
            })
            .then_with(|| left.attempt.binding_id.cmp(&right.attempt.binding_id))
    });
    Ok((included, excluded))
}

fn qualify_capabilities(
    fact: &CandidateCompilationFactV1,
    requirements: &CapabilityRequirementsV1,
) -> Result<(), AgentPlanCompilerError> {
    if capabilities_satisfied(fact, requirements) {
        Ok(())
    } else {
        Err(AgentPlanCompilerError::CapabilityUnqualified(
            fact.binding.binding_id.clone(),
        ))
    }
}

fn capabilities_satisfied(
    fact: &CandidateCompilationFactV1,
    requirements: &CapabilityRequirementsV1,
) -> bool {
    (!requirements.tool || fact.model.capabilities.tool)
        && (!requirements.vision || fact.model.capabilities.vision)
        && (!requirements.streaming || fact.model.capabilities.streaming)
        && fact.model.capabilities.context_tokens >= requirements.minimum_context_tokens
        && fact.model.capabilities.max_output_tokens >= requirements.minimum_output_tokens
}

#[cfg(test)]
fn quality_order(left: &ResolvedCandidate, right: &ResolvedCandidate) -> std::cmp::Ordering {
    right
        .rating
        .cmp(&left.rating)
        .then_with(|| {
            left.attempt
                .model_configuration_id
                .cmp(&right.attempt.model_configuration_id)
        })
        .then_with(|| left.attempt.binding_id.cmp(&right.attempt.binding_id))
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum AgentPlanCompilerError {
    #[error("free suggestion exceeds the single-snapshot query limit")]
    SuggestionLimitExceeded,
    #[error("the requested rating snapshot is unavailable")]
    RatingSnapshotUnavailable,
    #[error(transparent)]
    Facts(#[from] CompilerFactError),
    #[error(transparent)]
    Reasoning(#[from] ReasoningContractError),
    #[error("desired AgentPlan is invalid")]
    InvalidDesiredPlan,
    #[error("AgentPlan identity does not match the desired display metadata")]
    IdentityMismatch,
    #[error("unknown Binding {0}")]
    UnknownBinding(String),
    #[error("automatic reasoning references an unknown Binding")]
    UnknownAutomaticReasoningBinding,
    #[error("candidate {0} is not materialized and routable")]
    CandidateNotRoutable(String),
    #[error("candidate {0} does not meet the exact capability requirements")]
    CapabilityUnqualified(String),
    #[error("quality-ordered group requires an exact rating")]
    RatingRequired,
    #[error("QUALITY_ANCHOR_UNAVAILABLE")]
    QualityAnchorUnavailable,
    #[error("free-first group contains a non-free candidate")]
    NonFreeCandidate,
    #[error("NO_FREE_CANDIDATES")]
    NoFreeCandidates,
    #[error("compiled AgentPlan failed its closed validation")]
    InvalidCompiledPlan,
    #[error("aggregate publication failed its closed validation")]
    InvalidPublication,
    #[error("compiler canonical encoding failed")]
    Encoding,
}
