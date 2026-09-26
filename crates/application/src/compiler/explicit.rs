//! V2 authoring compiles exactly the user's arrays and native choices. Suggestions never enter
//! this path: membership, ordering and fallback are already explicit before Preview.
use std::collections::BTreeMap;

use hiroute_domain::*;

use super::{AgentPlanCompilationFactsV1, AgentPlanCompilerError, materialize::resolve_attempt};

pub fn compile_agent_plan_v2(
    identity: AgentPlanIdentityV1,
    revision: u64,
    desired: &AgentPlanAuthoringV2,
    facts: &AgentPlanCompilationFactsV1,
) -> Result<CompiledAgentPlanV1, AgentPlanCompilerError> {
    desired
        .validate()
        .map_err(|_| AgentPlanCompilerError::InvalidDesiredPlan)?;
    if identity.display_name != desired.display_name || identity.purpose != desired.purpose {
        return Err(AgentPlanCompilerError::IdentityMismatch);
    }
    facts.validate()?;
    let by_binding = facts
        .candidates
        .iter()
        .map(|fact| (fact.binding.binding_id.as_str(), fact))
        .collect::<BTreeMap<_, _>>();
    let group = |id, selections: &[CandidateSelectionV1]| {
        let candidates = selections
            .iter()
            .map(|selection| {
                let fact = by_binding
                    .get(selection.binding_id.as_str())
                    .copied()
                    .ok_or_else(|| {
                        AgentPlanCompilerError::UnknownBinding(selection.binding_id.clone())
                    })?;
                let candidate = resolve_attempt(
                    fact,
                    selection.reasoning.as_ref(),
                    &desired.requirements,
                    DiscreteReasoningDefault::RequireExplicit,
                )?;
                if desired.delegation_enabled
                    && let Some(work) = &desired.work
                {
                    let protocol = match work.protocol {
                        AgentIngressProtocolV1::Responses => UpstreamProtocol::Responses,
                        AgentIngressProtocolV1::Messages => UpstreamProtocol::Messages,
                    };
                    if !candidate
                        .protocol_profiles
                        .iter()
                        .any(|p| p.ingress_protocol == protocol)
                    {
                        return Err(AgentPlanCompilerError::CapabilityUnqualified(
                            selection.binding_id.clone(),
                        ));
                    }
                }
                Ok(candidate)
            })
            .collect::<Result<Vec<_>, AgentPlanCompilerError>>()?;
        Ok::<_, AgentPlanCompilerError>(MaterializedModelGroupV1 {
            group_id: id,
            ordering_evidence: MaterializedOrderingV1::ExplicitOrder,
            pinned_ratings: BTreeMap::new(),
            candidates,
        })
    };
    let (request_owned, groups) = match &desired.strategy {
        AgentPlanStrategyV2::SmartSaving {
            economy,
            primary,
            primary_fallback,
            reselect_on_user_message,
            classifier,
            complex_keywords,
        } => {
            let mut simple_groups = vec![MaterializedGroupId::Economy];
            if *primary_fallback {
                simple_groups.push(MaterializedGroupId::Primary);
            }
            (
                RequestOwnedRouteV1::Classified {
                    reselect_on_user_message: *reselect_on_user_message,
                    classifier: ComplexityClassifierV1::with_mode(
                        complex_keywords.clone(),
                        classifier.clone(),
                    )
                    .map_err(|_| AgentPlanCompilerError::InvalidDesiredPlan)?,
                    simple_groups,
                    complex_groups: vec![MaterializedGroupId::Primary],
                },
                vec![
                    group(MaterializedGroupId::Economy, economy)?,
                    group(MaterializedGroupId::Primary, primary)?,
                ],
            )
        }
        AgentPlanStrategyV2::FreeFirst {
            candidates,
            primary_fallback,
            primary,
        } => {
            let free = group(MaterializedGroupId::Free, candidates)?;
            if free
                .candidates
                .iter()
                .any(|c| c.billing_class != BillingClass::Free)
            {
                return Err(AgentPlanCompilerError::NonFreeCandidate);
            }
            let mut groups = vec![free];
            let mut ordered_groups = vec![MaterializedGroupId::Free];
            if *primary_fallback {
                groups.push(group(MaterializedGroupId::Primary, primary)?);
                ordered_groups.push(MaterializedGroupId::Primary);
            }
            (
                RequestOwnedRouteV1::Ordered {
                    cost_policy: if *primary_fallback {
                        MaterializedCostPolicyV1::ApiEquivalent
                    } else {
                        MaterializedCostPolicyV1::StrictFree
                    },
                    ordered_groups,
                },
                groups,
            )
        }
        AgentPlanStrategyV2::Custom { candidates } => (
            RequestOwnedRouteV1::Ordered {
                cost_policy: MaterializedCostPolicyV1::ApiEquivalent,
                ordered_groups: vec![MaterializedGroupId::Custom],
            },
            vec![group(MaterializedGroupId::Custom, candidates)?],
        ),
    };
    let materialized = MaterializedAgentPlanV1 {
        fact_refs: facts.refs.clone(),
        request_owned,
        attempt_owned: AttemptOwnedRouteV1 {
            limits: desired.limits.clone(),
            groups,
        },
    };
    let materialized_route_digest = materialized
        .route_digest()
        .map_err(|_| AgentPlanCompilerError::InvalidCompiledPlan)?;
    CompiledAgentPlanV1::seal_current(CompiledAgentPlanBodyV1 {
        schema: AGENT_PLAN_COMPILED_SCHEMA_V2.into(),
        compiler_revision: AGENT_PLAN_COMPILER_REVISION_V2.into(),
        identity,
        agent_plan_revision: revision,
        materialized_route_digest,
        materialized,
    })
    .map_err(|_| AgentPlanCompilerError::InvalidCompiledPlan)
}

pub fn compile_fixed_model_bindings(
    selections: &[AgentFixedModelSelectionV2],
    facts: &[super::CandidateCompilationFactV1],
) -> Result<BTreeMap<String, AttemptOwnedCandidateV1>, AgentPlanCompilerError> {
    let mut bindings = BTreeMap::new();
    for selection in selections {
        if !valid_client_model_name(&selection.client_model_id) {
            return Err(AgentPlanCompilerError::InvalidDesiredPlan);
        }
        let mut matching = facts
            .iter()
            .filter(|fact| fact.binding.binding_id == selection.candidate.binding_id);
        let fact = matching.next().ok_or_else(|| {
            AgentPlanCompilerError::UnknownBinding(selection.candidate.binding_id.clone())
        })?;
        if matching.next().is_some() {
            return Err(AgentPlanCompilerError::InvalidCompiledPlan);
        }
        fact.validate()?;
        let binding = resolve_attempt(
            fact,
            selection.candidate.reasoning.as_ref(),
            &CapabilityRequirementsV1::default(),
            DiscreteReasoningDefault::RequireExplicit,
        )?;
        if bindings
            .insert(selection.client_model_id.clone(), binding)
            .is_some()
        {
            return Err(AgentPlanCompilerError::InvalidDesiredPlan);
        }
    }
    Ok(bindings)
}

#[cfg(test)]
mod tests;
