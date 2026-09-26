//! Deterministic planner over immutable request and publication facts.
//!
//! This module has no ports: it cannot read credentials, runtime state,
//! network data, wall clock, cooldowns, observations or replay state. Its
//! frozen candidate ledger is the only order a later runtime may consume.

#[path = "planning/mod.rs"]
mod planning;

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

pub use planning::{
    BranchDecisionV1, CANDIDATE_FACTS_SCHEMA, CANDIDATE_LEDGER_SCHEMA, COMPLEXITY_SCHEMA,
    COMPLEXITY_STRATEGY_ID, COMPLEXITY_THRESHOLD, CacheCostFactV1, CandidateEvaluationV1,
    ClassifierFallbackReasonV1, CompiledClassifierKindV1, CompiledComplexPhraseV1,
    CompiledComplexityStrategyV1, CompiledPlannerPolicyV1, ComplexityBranchV1,
    ComplexityDecisionSourceV1, ComplexityReasonCodeV1, ComplexityV1, ContinuationKindV1,
    CorrelatedBranchDecisionV1, CostClassV1, ExclusionReasonCodeV1, FreeCandidateModeV1,
    FreeFirstExhaustionV1, FrozenCandidateLedgerV1, FrozenCandidateV1, GroupPlanV1, GroupPolicyV1,
    HoldPreferenceV1, LedgerReasonCodeV1, MaterializedModelGroupV1, MaterializedRouteV1,
    PLANNER_INPUT_SCHEMA, PLANNER_OUTPUT_SCHEMA, PLANNER_POLICY_SCHEMA, PaidBudgetQuoteFactV1,
    PlannedBranchV1, PlannerCandidateFactsV1, PlannerError, PlannerInputV1, PlannerOutcomeCodeV1,
    PlannerOutcomeV1, PlannerOutputV1, PlannerRouteIdentityV2, RankingReasonCodeV1,
    ReasonLedgerEntryV1, RequestOwnedLimitsV1, SanitizedStructuralFactsV1, StaticCostPolicyV1,
};

use planning::{
    EligibleForRanking, canonical_digest, evaluate_candidate, has_exact_reasoning_profile,
    rank_group, state_owners,
};

#[derive(Clone, Debug, Default)]
pub struct Planner;

impl Planner {
    pub fn plan(&self, input: &PlannerInputV1) -> Result<PlannerOutputV1, PlannerError> {
        validate_input(input)?;
        let (branch, complexity, complexity_facts, mut group_ids, mut reason_ledger) =
            select_groups(input)?;
        // Deliberately digest only content-free planning facts. A Routing
        // Receipt may retain this value without creating a Prompt hash.
        let input_digest =
            planning_facts_digest(input, complexity.as_ref(), complexity_facts.as_ref())?;
        let candidate_by_id = input
            .candidates
            .iter()
            .map(|candidate| (candidate.candidate_id.as_str(), candidate))
            .collect::<BTreeMap<_, _>>();
        let group_by_id = input
            .policy
            .groups
            .iter()
            .map(|group| (group.group_id.as_str(), group))
            .collect::<BTreeMap<_, _>>();

        apply_provider_state_owner_continuation(
            input,
            &candidate_by_id,
            &group_by_id,
            &mut group_ids,
            &mut reason_ledger,
        )?;

        let mut seen = BTreeSet::new();
        let mut evaluations = Vec::new();
        let mut frozen = Vec::new();
        let mut groups = Vec::new();

        for (group_ordinal, group_id) in group_ids.iter().enumerate() {
            let group = group_by_id[group_id.as_str()];
            let mut group_evaluations = vec![None; group.candidate_ids.len()];
            let mut eligible = Vec::new();
            for (declared_index, candidate_id) in group.candidate_ids.iter().enumerate() {
                let candidate = candidate_by_id[candidate_id.as_str()];
                let declared_order =
                    u32::try_from(declared_index).map_err(|_| PlannerError::ArithmeticOverflow)?;
                if seen.contains(candidate_id.as_str()) {
                    group_evaluations[declared_index] = Some(excluded_evaluation(
                        candidate,
                        group_id,
                        declared_order,
                        ExclusionReasonCodeV1::DuplicateCandidate,
                    ));
                    continue;
                }
                match evaluate_candidate(
                    &input.request,
                    candidate,
                    input.policy.cost_policy,
                    &input.policy.limits,
                ) {
                    Ok(projection) => eligible.push(EligibleForRanking {
                        candidate,
                        projection,
                        declared_order,
                    }),
                    Err(reason) => {
                        group_evaluations[declared_index] = Some(excluded_evaluation(
                            candidate,
                            group_id,
                            declared_order,
                            reason,
                        ));
                    }
                }
            }

            let ranked = rank_group(group, eligible, guard_anchor_score(group, &candidate_by_id));
            for (candidate, declared_order, reason) in ranked.exclusions {
                group_evaluations[usize::try_from(declared_order)
                    .map_err(|_| PlannerError::ArithmeticOverflow)?] = Some(excluded_evaluation(
                    candidate,
                    group_id,
                    declared_order,
                    reason,
                ));
            }
            let mut ranked_ids = Vec::new();
            for ranked_candidate in ranked.ordered {
                seen.insert(ranked_candidate.candidate.candidate_id.as_str());
                let ordinal =
                    u32::try_from(frozen.len()).map_err(|_| PlannerError::ArithmeticOverflow)?;
                ranked_ids.push(ranked_candidate.candidate.candidate_id.clone());
                group_evaluations[usize::try_from(ranked_candidate.declared_order)
                    .map_err(|_| PlannerError::ArithmeticOverflow)?] =
                    Some(CandidateEvaluationV1 {
                        candidate_id: ranked_candidate.candidate.candidate_id.clone(),
                        stable_binding_id: ranked_candidate.candidate.stable_binding_id.clone(),
                        group_id: group_id.clone(),
                        declared_order: ranked_candidate.declared_order,
                        profile_digest: ranked_candidate.candidate.profile_digest.clone(),
                        eligible: true,
                        first_exclusion: None,
                        reasoning_profile_id: Some(
                            ranked_candidate.projection.reasoning_profile_id.clone(),
                        ),
                        context: Some(ranked_candidate.projection.context.clone()),
                        overall_score_tenths: ranked_candidate.candidate.overall_score_tenths,
                        effective_cost_micros: ranked_candidate.projection.effective_cost_micros,
                        cost_class: ranked_candidate.candidate.cost_class,
                    });
                frozen.push(FrozenCandidateV1 {
                    ordinal,
                    candidate_id: ranked_candidate.candidate.candidate_id.clone(),
                    stable_binding_id: ranked_candidate.candidate.stable_binding_id.clone(),
                    group_id: group_id.clone(),
                    profile_digest: ranked_candidate.candidate.profile_digest.clone(),
                    upstream_protocol: ranked_candidate
                        .candidate
                        .protocol_profile
                        .capability
                        .upstream_protocol,
                    reasoning_profile_id: ranked_candidate.projection.reasoning_profile_id,
                    context: ranked_candidate.projection.context,
                    overall_score_tenths: ranked_candidate.candidate.overall_score_tenths,
                    effective_cost_micros: ranked_candidate.projection.effective_cost_micros,
                    cost_class: ranked_candidate.candidate.cost_class,
                    ranking_reasons: ranked_candidate.reasons,
                });
            }
            evaluations.extend(group_evaluations.into_iter().flatten());
            groups.push(GroupPlanV1 {
                ordinal: u32::try_from(group_ordinal)
                    .map_err(|_| PlannerError::ArithmeticOverflow)?,
                group_id: group_id.clone(),
                ranked_candidate_ids: ranked_ids,
            });
        }

        apply_context_hold(
            input,
            &group_ids,
            &candidate_by_id,
            &group_by_id,
            &mut evaluations,
            &mut frozen,
            &mut reason_ledger,
        )?;
        apply_previous_success_fallback(
            input,
            &candidate_by_id,
            &mut groups,
            &mut evaluations,
            &mut frozen,
            &mut reason_ledger,
        )?;

        for (ordinal, entry) in reason_ledger.iter_mut().enumerate() {
            entry.ordinal = u32::try_from(ordinal).map_err(|_| PlannerError::ArithmeticOverflow)?;
        }
        PlannerOutputV1 {
            schema_version: PLANNER_OUTPUT_SCHEMA.into(),
            planner_version: "hiroute-deterministic-planner/v1".into(),
            input_digest,
            policy_digest: input.policy.policy_digest.clone(),
            output_digest: String::new(),
            served_model_id: input.policy.served_model_id.clone(),
            identity: input.policy.identity.clone(),
            branch,
            complexity,
            limits: input.policy.limits.clone(),
            outcome: if frozen.is_empty() {
                PlannerOutcomeV1::NoEligibleCandidates {
                    code: PlannerOutcomeCodeV1::NoEligibleCandidate,
                }
            } else {
                PlannerOutcomeV1::Ready
            },
            groups,
            ledger: FrozenCandidateLedgerV1 {
                schema_version: CANDIDATE_LEDGER_SCHEMA.into(),
                ordered_candidates: frozen,
                evaluations,
            },
            reason_ledger,
        }
        .seal()
    }
}

#[derive(Serialize)]
struct PlanningFactsDigestProjection<'a> {
    schema_version: &'static str,
    policy_digest: &'a str,
    request_requirements: crate::server::core_runtime::model_ir::RequestCapabilityRequirementsV1,
    branch_decision: Option<&'a BranchDecisionV1>,
    structural_facts: Option<&'a SanitizedStructuralFactsV1>,
    context_hold: Option<&'a HoldPreferenceV1>,
    previous_success_candidate_id: Option<&'a str>,
    candidates: Vec<&'a PlannerCandidateFactsV1>,
}

fn planning_facts_digest(
    input: &PlannerInputV1,
    branch_decision: Option<&BranchDecisionV1>,
    structural_facts: Option<&SanitizedStructuralFactsV1>,
) -> Result<String, PlannerError> {
    let mut candidates = input.candidates.iter().collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.candidate_id.cmp(&right.candidate_id));
    canonical_digest(&PlanningFactsDigestProjection {
        schema_version: "hiroute.planner-input-facts-digest/v1",
        policy_digest: &input.policy.policy_digest,
        request_requirements: input.request.requirements(),
        branch_decision,
        structural_facts,
        context_hold: input.context_hold.as_ref(),
        previous_success_candidate_id: input.previous_success_candidate_id.as_deref(),
        candidates,
    })
}

fn guard_anchor_score(
    group: &MaterializedModelGroupV1,
    candidate_by_id: &BTreeMap<&str, &PlannerCandidateFactsV1>,
) -> Option<i32> {
    match &group.policy {
        GroupPolicyV1::CheapestWithRatingGuard {
            quality_anchor_ref, ..
        } => {
            let anchor = candidate_by_id[quality_anchor_ref.as_str()];
            anchor
                .overall_score_tenths
                .filter(|_| has_exact_reasoning_profile(anchor))
        }
        GroupPolicyV1::Manual | GroupPolicyV1::QualityFirst => None,
    }
}

fn group_has_eligible_state_owner(
    input: &PlannerInputV1,
    group: &MaterializedModelGroupV1,
    candidate_by_id: &BTreeMap<&str, &PlannerCandidateFactsV1>,
    owner: &crate::server::core_runtime::model_ir::ExactProviderPathV1,
) -> Result<bool, PlannerError> {
    let mut eligible = Vec::new();
    for (index, candidate_id) in group.candidate_ids.iter().enumerate() {
        let candidate = candidate_by_id[candidate_id.as_str()];
        if candidate
            .protocol_profile
            .exact_provider_path()
            .ok()
            .as_ref()
            != Some(owner)
        {
            continue;
        }
        if let Ok(projection) = evaluate_candidate(
            &input.request,
            candidate,
            input.policy.cost_policy,
            &input.policy.limits,
        ) {
            eligible.push(EligibleForRanking {
                candidate,
                projection,
                declared_order: u32::try_from(index)
                    .map_err(|_| PlannerError::ArithmeticOverflow)?,
            });
        }
    }
    Ok(
        !rank_group(group, eligible, guard_anchor_score(group, candidate_by_id))
            .ordered
            .is_empty(),
    )
}

fn group_has_eligible_candidate(
    input: &PlannerInputV1,
    group: &MaterializedModelGroupV1,
    candidate_by_id: &BTreeMap<&str, &PlannerCandidateFactsV1>,
) -> Result<bool, PlannerError> {
    let mut eligible = Vec::new();
    for (index, candidate_id) in group.candidate_ids.iter().enumerate() {
        let candidate = candidate_by_id[candidate_id.as_str()];
        if let Ok(projection) = evaluate_candidate(
            &input.request,
            candidate,
            input.policy.cost_policy,
            &input.policy.limits,
        ) {
            eligible.push(EligibleForRanking {
                candidate,
                projection,
                declared_order: u32::try_from(index)
                    .map_err(|_| PlannerError::ArithmeticOverflow)?,
            });
        }
    }
    Ok(
        !rank_group(group, eligible, guard_anchor_score(group, candidate_by_id))
            .ordered
            .is_empty(),
    )
}

fn apply_provider_state_owner_continuation(
    input: &PlannerInputV1,
    candidate_by_id: &BTreeMap<&str, &PlannerCandidateFactsV1>,
    group_by_id: &BTreeMap<&str, &MaterializedModelGroupV1>,
    group_ids: &mut Vec<String>,
    reason_ledger: &mut Vec<ReasonLedgerEntryV1>,
) -> Result<(), PlannerError> {
    let MaterializedRouteV1::SmartSaving {
        simple_group_id,
        simple_fallback_group_ids,
        complex_group_id,
        ..
    } = &input.policy.route
    else {
        return Ok(());
    };
    let mut owners = state_owners(&input.request);
    let Some(owner) = owners.next() else {
        return Ok(());
    };
    if owners.any(|other| other != owner) {
        return Ok(());
    }
    for group_id in group_ids.iter() {
        if group_has_eligible_candidate(input, group_by_id[group_id.as_str()], candidate_by_id)? {
            return Ok(());
        }
    }

    let alternatives = if group_ids.first() == Some(simple_group_id) {
        vec![complex_group_id.clone()]
    } else {
        std::iter::once(simple_group_id.clone())
            .chain(simple_fallback_group_ids.iter().cloned())
            .collect()
    };
    let mut continuation = Vec::new();
    for group_id in alternatives {
        if group_has_eligible_state_owner(
            input,
            group_by_id[group_id.as_str()],
            candidate_by_id,
            owner,
        )? {
            continuation.push(group_id);
        }
    }
    if !continuation.is_empty() {
        reason_ledger.retain(|entry| {
            !matches!(
                entry.code,
                LedgerReasonCodeV1::GroupExhaustedFallback | LedgerReasonCodeV1::ComplexNoDowngrade
            )
        });
        reason_ledger.push(reason(
            LedgerReasonCodeV1::ProviderStateOwnerContinuation,
            continuation.first().cloned(),
        ));
        *group_ids = continuation;
    }
    Ok(())
}

fn apply_context_hold(
    input: &PlannerInputV1,
    selected_group_ids: &[String],
    candidate_by_id: &BTreeMap<&str, &PlannerCandidateFactsV1>,
    group_by_id: &BTreeMap<&str, &MaterializedModelGroupV1>,
    evaluations: &mut Vec<CandidateEvaluationV1>,
    frozen: &mut Vec<FrozenCandidateV1>,
    reason_ledger: &mut Vec<ReasonLedgerEntryV1>,
) -> Result<(), PlannerError> {
    let Some(hold) = &input.context_hold else {
        return Ok(());
    };
    let Some(candidate) = candidate_by_id.get(hold.candidate_id.as_str()).copied() else {
        reason_ledger.push(reason(LedgerReasonCodeV1::ContextHoldInvalidated, None));
        return Ok(());
    };
    let Some(group) = group_by_id.get(hold.origin_group_id.as_str()).copied() else {
        reason_ledger.push(reason(LedgerReasonCodeV1::ContextHoldInvalidated, None));
        return Ok(());
    };
    if !selected_group_ids
        .iter()
        .any(|group_id| group_id == &hold.origin_group_id)
    {
        reason_ledger.push(reason(
            LedgerReasonCodeV1::ContextHoldInvalidated,
            Some(group.group_id.clone()),
        ));
        return Ok(());
    }
    let Some(declared_index) = group
        .candidate_ids
        .iter()
        .position(|candidate_id| candidate_id == &hold.candidate_id)
    else {
        reason_ledger.push(reason(LedgerReasonCodeV1::ContextHoldInvalidated, None));
        return Ok(());
    };
    if candidate.stable_binding_id != hold.stable_binding_id
        || candidate.profile_digest != hold.profile_digest
    {
        reason_ledger.push(reason(
            LedgerReasonCodeV1::ContextHoldInvalidated,
            Some(group.group_id.clone()),
        ));
        return Ok(());
    }
    let Ok(projection) = evaluate_candidate(
        &input.request,
        candidate,
        input.policy.cost_policy,
        &input.policy.limits,
    ) else {
        reason_ledger.push(reason(
            LedgerReasonCodeV1::ContextHoldInvalidated,
            Some(group.group_id.clone()),
        ));
        return Ok(());
    };
    if projection.reasoning_profile_id != hold.reasoning_profile_id {
        reason_ledger.push(reason(
            LedgerReasonCodeV1::ContextHoldInvalidated,
            Some(group.group_id.clone()),
        ));
        return Ok(());
    }

    let declared_order =
        u32::try_from(declared_index).map_err(|_| PlannerError::ArithmeticOverflow)?;
    if !evaluations.iter().any(|evaluation| {
        evaluation.candidate_id == hold.candidate_id && evaluation.group_id == hold.origin_group_id
    }) {
        evaluations.push(CandidateEvaluationV1 {
            candidate_id: candidate.candidate_id.clone(),
            stable_binding_id: candidate.stable_binding_id.clone(),
            group_id: group.group_id.clone(),
            declared_order,
            profile_digest: candidate.profile_digest.clone(),
            eligible: true,
            first_exclusion: None,
            reasoning_profile_id: Some(projection.reasoning_profile_id.clone()),
            context: Some(projection.context.clone()),
            overall_score_tenths: candidate.overall_score_tenths,
            effective_cost_micros: projection.effective_cost_micros,
            cost_class: candidate.cost_class,
        });
    }
    frozen.retain(|entry| entry.candidate_id != hold.candidate_id);
    frozen.insert(
        0,
        FrozenCandidateV1 {
            ordinal: 0,
            candidate_id: candidate.candidate_id.clone(),
            stable_binding_id: candidate.stable_binding_id.clone(),
            group_id: group.group_id.clone(),
            profile_digest: candidate.profile_digest.clone(),
            upstream_protocol: candidate.protocol_profile.capability.upstream_protocol,
            reasoning_profile_id: projection.reasoning_profile_id,
            context: projection.context,
            overall_score_tenths: candidate.overall_score_tenths,
            effective_cost_micros: projection.effective_cost_micros,
            cost_class: candidate.cost_class,
            ranking_reasons: vec![RankingReasonCodeV1::ContextModelHold],
        },
    );
    for (ordinal, candidate) in frozen.iter_mut().enumerate() {
        candidate.ordinal = u32::try_from(ordinal).map_err(|_| PlannerError::ArithmeticOverflow)?;
    }
    reason_ledger.push(reason(
        LedgerReasonCodeV1::ContextHoldApplied,
        Some(group.group_id.clone()),
    ));
    Ok(())
}

fn apply_previous_success_fallback(
    input: &PlannerInputV1,
    candidate_by_id: &BTreeMap<&str, &PlannerCandidateFactsV1>,
    groups: &mut Vec<GroupPlanV1>,
    evaluations: &mut Vec<CandidateEvaluationV1>,
    frozen: &mut Vec<FrozenCandidateV1>,
    reason_ledger: &mut Vec<ReasonLedgerEntryV1>,
) -> Result<(), PlannerError> {
    if !matches!(input.policy.route, MaterializedRouteV1::SmartSaving { .. }) || frozen.is_empty() {
        return Ok(());
    }
    let Some(previous_id) = input.previous_success_candidate_id.as_deref() else {
        return Ok(());
    };
    if frozen
        .first()
        .is_some_and(|candidate| candidate.candidate_id == previous_id)
    {
        return Ok(());
    }
    if let Some(index) = frozen
        .iter()
        .position(|candidate| candidate.candidate_id == previous_id)
    {
        let mut candidate = frozen.remove(index);
        candidate
            .ranking_reasons
            .push(RankingReasonCodeV1::PreviousSuccessFallback);
        let group_id = candidate.group_id.clone();
        frozen.insert(usize::from(!frozen.is_empty()), candidate);
        reason_ledger.push(reason(
            LedgerReasonCodeV1::PreviousSuccessFallback,
            Some(group_id),
        ));
    } else {
        // A candidate excluded from the selected groups cannot be revived by
        // a prior success. Only an exact, currently eligible plan member may
        // extend the frozen chain for this request.
        if evaluations
            .iter()
            .any(|entry| entry.candidate_id == previous_id)
        {
            return Ok(());
        }
        let Some(candidate) = candidate_by_id.get(previous_id).copied() else {
            return Ok(());
        };
        let mut matches = input
            .policy
            .groups
            .iter()
            .filter(|group| group.candidate_ids.iter().any(|id| id == previous_id));
        let Some(group) = matches.next() else {
            return Ok(());
        };
        if matches.next().is_some() {
            return Ok(());
        }
        let declared_order = u32::try_from(
            group
                .candidate_ids
                .iter()
                .position(|id| id == previous_id)
                .ok_or(PlannerError::InvalidPolicy(
                    "previous candidate is not in its group",
                ))?,
        )
        .map_err(|_| PlannerError::ArithmeticOverflow)?;
        let Ok(projection) = evaluate_candidate(
            &input.request,
            candidate,
            input.policy.cost_policy,
            &input.policy.limits,
        ) else {
            return Ok(());
        };
        if rank_group(
            group,
            vec![EligibleForRanking {
                candidate,
                projection: projection.clone(),
                declared_order,
            }],
            guard_anchor_score(group, candidate_by_id),
        )
        .ordered
        .is_empty()
        {
            return Ok(());
        }
        evaluations.push(CandidateEvaluationV1 {
            candidate_id: candidate.candidate_id.clone(),
            stable_binding_id: candidate.stable_binding_id.clone(),
            group_id: group.group_id.clone(),
            declared_order,
            profile_digest: candidate.profile_digest.clone(),
            eligible: true,
            first_exclusion: None,
            reasoning_profile_id: Some(projection.reasoning_profile_id.clone()),
            context: Some(projection.context.clone()),
            overall_score_tenths: candidate.overall_score_tenths,
            effective_cost_micros: projection.effective_cost_micros,
            cost_class: candidate.cost_class,
        });
        frozen.insert(
            usize::from(!frozen.is_empty()),
            FrozenCandidateV1 {
                ordinal: 0,
                candidate_id: candidate.candidate_id.clone(),
                stable_binding_id: candidate.stable_binding_id.clone(),
                group_id: group.group_id.clone(),
                profile_digest: candidate.profile_digest.clone(),
                upstream_protocol: candidate.protocol_profile.capability.upstream_protocol,
                reasoning_profile_id: projection.reasoning_profile_id,
                context: projection.context,
                overall_score_tenths: candidate.overall_score_tenths,
                effective_cost_micros: projection.effective_cost_micros,
                cost_class: candidate.cost_class,
                ranking_reasons: vec![RankingReasonCodeV1::PreviousSuccessFallback],
            },
        );
        groups.push(GroupPlanV1 {
            ordinal: u32::try_from(groups.len()).map_err(|_| PlannerError::ArithmeticOverflow)?,
            group_id: group.group_id.clone(),
            ranked_candidate_ids: vec![candidate.candidate_id.clone()],
        });
        reason_ledger.push(reason(
            LedgerReasonCodeV1::PreviousSuccessFallback,
            Some(group.group_id.clone()),
        ));
    }
    for (ordinal, candidate) in frozen.iter_mut().enumerate() {
        candidate.ordinal = u32::try_from(ordinal).map_err(|_| PlannerError::ArithmeticOverflow)?;
    }
    Ok(())
}

fn validate_input(input: &PlannerInputV1) -> Result<(), PlannerError> {
    if input.schema_version != PLANNER_INPUT_SCHEMA
        || input.request.schema_version
            != crate::server::core_runtime::model_ir::MODEL_REQUEST_IR_SCHEMA
    {
        return Err(PlannerError::InputSchema);
    }
    let policy = &input.policy;
    if policy.schema_version != PLANNER_POLICY_SCHEMA
        || policy.served_model_id.trim().is_empty()
        || match &policy.identity {
            PlannerRouteIdentityV2::Plan { plan_id, revision } => {
                plan_id.trim().is_empty() || *revision == 0
            }
            PlannerRouteIdentityV2::Fixed { binding_digest } => {
                hiroute_domain::CanonicalDigest::parse(binding_digest.as_str()).is_err()
                    || policy.groups.len() != 1
                    || policy.groups[0].candidate_ids.len() != 1
                    || policy.limits.max_candidate_bindings != 1
                    || !matches!(policy.route, MaterializedRouteV1::Custom { .. })
            }
        }
        || (matches!(policy.identity, PlannerRouteIdentityV2::Fixed { .. })
            != (policy.cost_policy == StaticCostPolicyV1::ExplicitFixed))
        || policy.limits.max_candidate_bindings == 0
        || policy.limits.max_attempts == 0
        || policy.limits.deadline_cap_ms == 0
        || input.request.served_model_id != policy.served_model_id
    {
        return Err(PlannerError::InvalidPolicy(
            "identity, request-owned limits, or served model is invalid",
        ));
    }
    if policy.cost_policy == StaticCostPolicyV1::BudgetedPaid
        && policy.limits.paid_budget_ceiling_micros.is_none()
    {
        return Err(PlannerError::InvalidPolicy(
            "budgeted paid policy requires a request-owned ceiling",
        ));
    }
    if policy.recompute_digest()? != policy.policy_digest {
        return Err(PlannerError::PolicyDigestMismatch);
    }

    let mut group_ids = BTreeSet::new();
    for group in &policy.groups {
        let mut candidate_ids = BTreeSet::new();
        if group.group_id.trim().is_empty()
            || group.candidate_ids.is_empty()
            || !group_ids.insert(group.group_id.as_str())
            || group
                .candidate_ids
                .iter()
                .any(|candidate| candidate.trim().is_empty() || !candidate_ids.insert(candidate))
        {
            return Err(PlannerError::InvalidPolicy(
                "materialized groups must be non-empty and unique",
            ));
        }
    }

    let group_by_id = policy
        .groups
        .iter()
        .map(|group| (group.group_id.as_str(), group))
        .collect::<BTreeMap<_, _>>();
    match &policy.route {
        MaterializedRouteV1::SmartSaving {
            simple_group_id,
            simple_fallback_group_ids,
            complex_group_id,
            ..
        } => {
            let strategy =
                policy
                    .complexity_strategy
                    .as_ref()
                    .ok_or(PlannerError::InvalidPolicy(
                        "smart saving requires complexity-v1",
                    ))?;
            ComplexityV1::validate(strategy)?;
            require_group(&group_by_id, simple_group_id)?;
            require_group(&group_by_id, complex_group_id)?;
            if simple_group_id == complex_group_id {
                return Err(PlannerError::InvalidPolicy(
                    "smart saving simple and complex groups must differ",
                ));
            }
            let mut transitions =
                BTreeSet::from([simple_group_id.as_str(), complex_group_id.as_str()]);
            let mut fallback_ids = BTreeSet::new();
            for fallback in simple_fallback_group_ids {
                require_group(&group_by_id, fallback)?;
                if fallback == simple_group_id || !fallback_ids.insert(fallback.as_str()) {
                    return Err(PlannerError::InvalidPolicy(
                        "smart saving group transitions must be unique",
                    ));
                }
                transitions.insert(fallback.as_str());
            }
            if transitions.len() != group_by_id.len() {
                return Err(PlannerError::InvalidPolicy(
                    "smart saving contains an unreferenced group",
                ));
            }
        }
        MaterializedRouteV1::FreeFirst {
            free_group_id,
            exhaustion,
            candidate_mode: _,
        } => {
            if policy.complexity_strategy.is_some() {
                return Err(PlannerError::InvalidPolicy(
                    "free first must not invoke complexity classification",
                ));
            }
            let free = require_group(&group_by_id, free_group_id)?;
            if !matches!(free.policy, GroupPolicyV1::Manual) {
                return Err(PlannerError::InvalidPolicy(
                    "free pool order must already be materialized",
                ));
            }
            match exhaustion {
                FreeFirstExhaustionV1::FreeOnly => {
                    if policy.cost_policy != StaticCostPolicyV1::StrictFree {
                        return Err(PlannerError::InvalidPolicy(
                            "free-only requires strict-free cost policy",
                        ));
                    }
                    if group_by_id.len() != 1 {
                        return Err(PlannerError::InvalidPolicy(
                            "free-only must contain exactly the free group",
                        ));
                    }
                }
                FreeFirstExhaustionV1::PrimaryFallback { primary_group_id } => {
                    require_group(&group_by_id, primary_group_id)?;
                    if primary_group_id == free_group_id {
                        return Err(PlannerError::InvalidPolicy(
                            "free and primary groups must differ",
                        ));
                    }
                    if group_by_id.len() != 2 {
                        return Err(PlannerError::InvalidPolicy(
                            "free-first primary fallback contains an unreferenced group",
                        ));
                    }
                }
            }
        }
        MaterializedRouteV1::Custom { group_id } => {
            if policy.complexity_strategy.is_some() {
                return Err(PlannerError::InvalidPolicy(
                    "custom route must not invoke complexity classification",
                ));
            }
            if !matches!(
                require_group(&group_by_id, group_id)?.policy,
                GroupPolicyV1::Manual
            ) {
                return Err(PlannerError::InvalidPolicy(
                    "custom route requires published manual order",
                ));
            }
            if group_by_id.len() != 1 {
                return Err(PlannerError::InvalidPolicy(
                    "custom route must contain exactly one group",
                ));
            }
        }
    }

    let mut candidate_ids = BTreeSet::new();
    let mut bindings = BTreeSet::new();
    for candidate in &input.candidates {
        if candidate.schema_version != CANDIDATE_FACTS_SCHEMA
            || candidate.candidate_id.trim().is_empty()
            || candidate.stable_binding_id.trim().is_empty()
            || candidate.target_serialized_bytes == 0
            || !candidate_ids.insert(candidate.candidate_id.as_str())
            || !bindings.insert(candidate.stable_binding_id.as_str())
            || candidate
                .overall_score_tenths
                .is_some_and(|score| !(5..=50).contains(&score))
        {
            return Err(PlannerError::InvalidCandidate(
                candidate.candidate_id.clone(),
            ));
        }
        if candidate.recompute_profile_digest()? != candidate.profile_digest {
            return Err(PlannerError::ProfileDigestMismatch(
                candidate.candidate_id.clone(),
            ));
        }
    }
    let mut referenced_candidates = BTreeSet::new();
    for group in &policy.groups {
        for candidate_id in &group.candidate_ids {
            if !candidate_ids.contains(candidate_id.as_str()) {
                return Err(PlannerError::UnknownCandidate(candidate_id.clone()));
            }
            referenced_candidates.insert(candidate_id.as_str());
        }
        if let GroupPolicyV1::CheapestWithRatingGuard {
            quality_anchor_ref, ..
        } = &group.policy
            && !group.candidate_ids.contains(quality_anchor_ref)
        {
            return Err(PlannerError::InvalidPolicy(
                "quality anchor must belong to its group",
            ));
        }
    }
    if referenced_candidates != candidate_ids {
        return Err(PlannerError::InvalidPolicy(
            "candidate facts must exactly match materialized group references",
        ));
    }
    if input
        .previous_success_candidate_id
        .as_ref()
        .is_some_and(|id| !candidate_ids.contains(id.as_str()))
    {
        return Err(PlannerError::InvalidPolicy(
            "previous success is not a published candidate",
        ));
    }
    if let MaterializedRouteV1::FreeFirst {
        free_group_id,
        candidate_mode,
        ..
    } = &policy.route
    {
        let candidates = input
            .candidates
            .iter()
            .map(|candidate| (candidate.candidate_id.as_str(), candidate))
            .collect::<BTreeMap<_, _>>();
        let free_group = group_by_id[free_group_id.as_str()];
        if free_group
            .candidate_ids
            .iter()
            .any(|candidate_id| candidates[candidate_id.as_str()].cost_class != CostClassV1::Free)
        {
            return Err(PlannerError::InvalidPolicy(
                "materialized free pool contains a non-free candidate",
            ));
        }
        if *candidate_mode == FreeCandidateModeV1::AutomaticAllAvailable
            && free_group.candidate_ids.iter().any(|candidate_id| {
                candidates[candidate_id.as_str()]
                    .overall_score_tenths
                    .is_none()
            })
        {
            return Err(PlannerError::InvalidPolicy(
                "automatic free pool contains an unrated candidate",
            ));
        }
    }
    Ok(())
}

fn require_group<'a>(
    groups: &'a BTreeMap<&str, &'a MaterializedModelGroupV1>,
    group_id: &str,
) -> Result<&'a MaterializedModelGroupV1, PlannerError> {
    groups
        .get(group_id)
        .copied()
        .ok_or(PlannerError::InvalidPolicy(
            "route references an unknown group",
        ))
}

type GroupSelection = (
    PlannedBranchV1,
    Option<BranchDecisionV1>,
    Option<SanitizedStructuralFactsV1>,
    Vec<String>,
    Vec<ReasonLedgerEntryV1>,
);

fn validate_classification_decision(
    decision: &BranchDecisionV1,
    strategy: &CompiledComplexityStrategyV1,
) -> Result<(), PlannerError> {
    let identity_matches = decision.strategy_id == strategy.strategy_id
        && decision.schema_version == strategy.schema_version
        && decision.payload_digest == strategy.payload_digest;
    let branch_matches = matches!(
        decision.branch_id.as_str(),
        hiroute_domain::SMART_SAVING_SIMPLE_BRANCH_ID
            | hiroute_domain::SMART_SAVING_COMPLEX_BRANCH_ID
    );
    let facts_match = match (strategy.classifier_kind, decision.decision_source) {
        (CompiledClassifierKindV1::Rest, ComplexityDecisionSourceV1::ExternalClassifier) => {
            decision.complexity_score.is_none()
                && decision.threshold.is_none()
                && decision.classification_duration_micros.is_some()
                && decision.fallback_reason.is_none()
                && !decision.fallback_used
        }
        (_, ComplexityDecisionSourceV1::Inherited) => true,
        (
            CompiledClassifierKindV1::Rest,
            ComplexityDecisionSourceV1::UserPhrase
            | ComplexityDecisionSourceV1::BuiltinRules
            | ComplexityDecisionSourceV1::Unresolved,
        ) => {
            decision.complexity_score.is_some()
                && decision.threshold == Some(strategy.threshold)
                && decision.classification_duration_micros.is_some()
                && decision.fallback_reason.is_some()
                && decision.fallback_used
        }
        (
            CompiledClassifierKindV1::LocalRules,
            ComplexityDecisionSourceV1::UserPhrase
            | ComplexityDecisionSourceV1::BuiltinRules
            | ComplexityDecisionSourceV1::Unresolved,
        ) => {
            decision.complexity_score.is_some()
                && decision.threshold == Some(strategy.threshold)
                && decision.classification_duration_micros.is_none()
                && decision.fallback_reason.is_none()
        }
        _ => false,
    };
    if identity_matches && branch_matches && facts_match {
        Ok(())
    } else {
        Err(PlannerError::CorrelatedDecisionMismatch)
    }
}

fn select_groups(input: &PlannerInputV1) -> Result<GroupSelection, PlannerError> {
    match &input.policy.route {
        MaterializedRouteV1::SmartSaving {
            simple_group_id,
            simple_fallback_group_ids,
            complex_group_id,
            ..
        } => {
            let strategy = input
                .policy
                .complexity_strategy
                .as_ref()
                .ok_or(PlannerError::InvalidPolicy("missing complexity strategy"))?;
            let decision =
                input
                    .classification_decision
                    .clone()
                    .ok_or(PlannerError::InvalidPolicy(
                        "classification decision is missing",
                    ))?;
            let facts = input
                .classification_facts
                .clone()
                .ok_or(PlannerError::InvalidPolicy(
                    "classification facts are missing",
                ))?;
            validate_classification_decision(&decision, strategy)?;
            match decision.branch_id.as_str() {
                hiroute_domain::SMART_SAVING_SIMPLE_BRANCH_ID => {
                    let mut groups = vec![simple_group_id.clone()];
                    groups.extend(simple_fallback_group_ids.iter().cloned());
                    let mut reasons = vec![reason(LedgerReasonCodeV1::SmartSavingSimple, None)];
                    reasons.extend(simple_fallback_group_ids.iter().map(|group| {
                        reason(
                            LedgerReasonCodeV1::GroupExhaustedFallback,
                            Some(group.clone()),
                        )
                    }));
                    Ok((
                        PlannedBranchV1::SmartSavingSimple,
                        Some(decision),
                        Some(facts),
                        groups,
                        reasons,
                    ))
                }
                hiroute_domain::SMART_SAVING_COMPLEX_BRANCH_ID => Ok((
                    PlannedBranchV1::SmartSavingComplex,
                    Some(decision),
                    Some(facts),
                    vec![complex_group_id.clone()],
                    vec![
                        reason(LedgerReasonCodeV1::SmartSavingComplex, None),
                        reason(LedgerReasonCodeV1::ComplexNoDowngrade, None),
                    ],
                )),
                _ => Err(PlannerError::CorrelatedDecisionMismatch),
            }
        }
        MaterializedRouteV1::FreeFirst {
            free_group_id,
            exhaustion,
            candidate_mode: _,
        } => match exhaustion {
            FreeFirstExhaustionV1::FreeOnly => Ok((
                PlannedBranchV1::FreeFirstFreeOnly,
                None,
                None,
                vec![free_group_id.clone()],
                vec![
                    reason(LedgerReasonCodeV1::FreeFirstNoClassification, None),
                    reason(LedgerReasonCodeV1::FreeOnlyBoundary, None),
                ],
            )),
            FreeFirstExhaustionV1::PrimaryFallback { primary_group_id } => Ok((
                PlannedBranchV1::FreeFirstPrimaryFallback,
                None,
                None,
                vec![free_group_id.clone(), primary_group_id.clone()],
                vec![
                    reason(LedgerReasonCodeV1::FreeFirstNoClassification, None),
                    reason(
                        LedgerReasonCodeV1::GroupExhaustedFallback,
                        Some(primary_group_id.clone()),
                    ),
                ],
            )),
        },
        MaterializedRouteV1::Custom { group_id } => Ok((
            PlannedBranchV1::CustomExactOrder,
            None,
            None,
            vec![group_id.clone()],
            vec![
                reason(LedgerReasonCodeV1::CustomNoClassification, None),
                reason(LedgerReasonCodeV1::CustomExactOrder, None),
            ],
        )),
    }
}

fn reason(code: LedgerReasonCodeV1, group_id: Option<String>) -> ReasonLedgerEntryV1 {
    ReasonLedgerEntryV1 {
        ordinal: 0,
        code,
        group_id,
    }
}

fn excluded_evaluation(
    candidate: &PlannerCandidateFactsV1,
    group_id: &str,
    declared_order: u32,
    reason: ExclusionReasonCodeV1,
) -> CandidateEvaluationV1 {
    CandidateEvaluationV1 {
        candidate_id: candidate.candidate_id.clone(),
        stable_binding_id: candidate.stable_binding_id.clone(),
        group_id: group_id.into(),
        declared_order,
        profile_digest: candidate.profile_digest.clone(),
        eligible: false,
        first_exclusion: Some(reason),
        reasoning_profile_id: None,
        context: None,
        overall_score_tenths: candidate.overall_score_tenths,
        effective_cost_micros: candidate.effective_cost_micros(),
        cost_class: candidate.cost_class,
    }
}

#[cfg(test)]
#[path = "planning/tests.rs"]
mod tests;
