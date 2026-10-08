use super::*;

pub(super) fn recover_legacy_grant(
    legacy: LegacyGatewayExecutableGrantV2,
    plans: &[CompiledAgentPlanV1],
) -> Result<GatewayExecutableGrantV2, PublicationError> {
    let [protocol] = legacy.allowed_protocols.as_slice() else {
        return Err(PublicationError::InvalidGrant);
    };
    let mut routes = BTreeMap::new();
    for alias in &legacy.allowed_aliases {
        let plan = plans
            .iter()
            .find(|plan| plan.model_alias() == alias)
            .ok_or(PublicationError::InvalidGrant)?;
        if routes
            .insert(
                alias.as_str().to_owned(),
                crate::AgentModelRouteV2::Plan {
                    plan_id: plan.agent_plan_id().clone(),
                    alias: alias.clone(),
                    revision: plan.body.agent_plan_revision,
                    semantic_digest: plan.body.materialized_route_digest.clone(),
                },
            )
            .is_some()
        {
            return Err(PublicationError::InvalidGrant);
        }
    }
    let model_grant = crate::AgentModelGrantV2::seal(*protocol, routes)
        .map_err(|_| PublicationError::InvalidGrant)?;
    Ok(GatewayExecutableGrantV2 {
        grant_id: legacy.grant_id,
        generation: legacy.generation,
        bearer_token_sha256: legacy.bearer_token_sha256,
        model_grant,
    })
}

/// Rebinds a grant's plan routes to the upgraded compiled plans. A route bound to the plan
/// revision this publication carries follows the plan's current route digest; a route pinned
/// to an older revision keeps its sealed provenance digest unchanged.
pub(super) fn rebind_current_grant_routes(
    grant: GatewayExecutableGrantV2,
    plans: &[CompiledAgentPlanV1],
) -> Result<GatewayExecutableGrantV2, PublicationError> {
    let mut routes = BTreeMap::new();
    for (name, route) in grant.model_grant.routes {
        let route = match route {
            crate::AgentModelRouteV2::Plan {
                plan_id,
                alias,
                revision,
                semantic_digest,
            } => {
                let plan = plans
                    .iter()
                    .find(|plan| plan.agent_plan_id() == &plan_id && plan.model_alias() == &alias)
                    .ok_or(PublicationError::InvalidGrant)?;
                if revision == plan.body.agent_plan_revision {
                    crate::AgentModelRouteV2::Plan {
                        plan_id,
                        alias,
                        revision,
                        semantic_digest: plan.body.materialized_route_digest.clone(),
                    }
                } else {
                    crate::AgentModelRouteV2::Plan {
                        plan_id,
                        alias,
                        revision,
                        semantic_digest,
                    }
                }
            }
            fixed @ crate::AgentModelRouteV2::Fixed { .. } => fixed,
        };
        if routes.insert(name, route).is_some() {
            return Err(PublicationError::InvalidGrant);
        }
    }
    let model_grant = crate::AgentModelGrantV2::seal_routes(
        grant.model_grant.protocol,
        routes,
        grant.model_grant.route_protocols,
    )
    .map_err(|_| PublicationError::InvalidGrant)?;
    Ok(GatewayExecutableGrantV2 {
        grant_id: grant.grant_id,
        generation: grant.generation,
        bearer_token_sha256: grant.bearer_token_sha256,
        model_grant,
    })
}

pub(super) fn materialize_grants(
    access_grants: Vec<GatewayAccessGrantV1>,
) -> Result<Vec<GatewayExecutableGrantV2>, PublicationError> {
    let mut grants = access_grants
        .into_iter()
        .map(|grant| {
            grant
                .validate()
                .map_err(|_| PublicationError::InvalidGrant)?;
            Ok(GatewayExecutableGrantV2 {
                grant_id: grant.grant_id,
                generation: grant.generation,
                bearer_token_sha256: grant.bearer_token_sha256,
                model_grant: grant.model_grant,
            })
        })
        .collect::<Result<Vec<_>, PublicationError>>()?;
    grants.sort_by(|left, right| left.grant_id.cmp(&right.grant_id));
    Ok(grants)
}

pub(super) fn validate_grants(
    grants: &[GatewayExecutableGrantV2],
    published_aliases: &BTreeSet<&ModelAlias>,
) -> Result<(), PublicationError> {
    let mut ids = BTreeSet::new();
    let mut verifiers = BTreeSet::new();
    let mut previous = None;
    for grant in grants {
        if previous.is_some_and(|value: &str| value >= grant.grant_id.as_str())
            || !ids.insert(grant.grant_id.as_str())
            || !verifiers.insert(grant.bearer_token_sha256.as_str())
            || !valid_reference(&grant.grant_id)
            || grant.generation == 0
            || !valid_digest(&grant.bearer_token_sha256)
            || grant.model_grant.validate().is_err()
            || grant.model_grant.routes.values().any(|route| {
                matches!(route, crate::AgentModelRouteV2::Plan { alias, .. } if !published_aliases.contains(alias))
            })
        {
            return Err(PublicationError::InvalidGrant);
        }
        previous = Some(&grant.grant_id);
    }
    Ok(())
}

pub(super) fn materialize_aliases(
    plans: &[CompiledAgentPlanV1],
    grants: &[GatewayExecutableGrantV2],
) -> Result<Vec<GatewayExecutableAliasV2>, PublicationError> {
    let mut local_ids = BTreeSet::new();
    plans
        .iter()
        .filter_map(|plan| {
            let protocols = grants
                .iter()
                .filter(|grant| grant.permits_plan(plan.model_alias()))
                .map(|grant| grant.model_grant.protocol_for(plan.model_alias().as_str()))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            if protocols.is_empty() {
                return None;
            }
            Some((|| {
                let limits = &plan.body.materialized.attempt_owned.limits;
                if limits.maximum_attempts > MAX_REQUEST_ATTEMPTS
                    || limits.request_timeout_ms > MAX_LOGICAL_REQUEST_DURATION_MS
                {
                    return Err(PublicationError::InvalidExecutionLimits);
                }
                let candidates = ordered_candidates(plan)?
                    .into_iter()
                    .map(|candidate| {
                        let key = candidate_execution_key(plan, candidate)?;
                        let local_id = candidate_local_id(plan.agent_plan_id().as_str(), &key)?;
                        if !local_ids.insert(local_id) {
                            return Err(PublicationError::CandidateIdCollision);
                        }
                        let mut projected =
                            snapshot::project_candidate(candidate, local_id, &protocols)?;
                        projected.stable_target_key = key;
                        Ok(projected)
                    })
                    .collect::<Result<Vec<_>, PublicationError>>()?;
                let local_id_by_binding = candidates
                    .iter()
                    .map(|candidate| (candidate.stable_target_key.as_str(), candidate.local_id))
                    .collect::<BTreeMap<_, _>>();
                let groups = plan
                    .body
                    .materialized
                    .attempt_owned
                    .groups
                    .iter()
                    .map(|group| {
                        let candidate_local_ids = group
                            .candidates
                            .iter()
                            .map(|candidate| {
                                local_id_by_binding
                                    .get(candidate_execution_key(plan, candidate)?.as_str())
                                    .copied()
                                    .ok_or(PublicationError::InvalidExecutableProjection)
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        Ok(GatewayExecutableGroupV2 {
                            group_id: group.group_id,
                            candidate_local_ids,
                        })
                    })
                    .collect::<Result<Vec<_>, PublicationError>>()?;
                Ok(GatewayExecutableAliasV2 {
                    served_model_id: plan.model_alias().clone(),
                    purpose: plan.body.identity.purpose.as_str().to_owned(),
                    agent_plan_revision: plan.body.agent_plan_revision,
                    protocols,
                    overall_timeout_ms: limits.request_timeout_ms,
                    max_attempts: u32::from(limits.maximum_attempts),
                    routing: Some(GatewayExecutableRoutingV2 {
                        agent_plan_id: plan.agent_plan_id().as_str().to_owned(),
                        plan_display_name: Some(
                            plan.body.identity.display_name.as_str().to_owned(),
                        ),
                        request_owned: plan.body.materialized.request_owned.clone(),
                        groups,
                    }),
                    candidates,
                })
            })())
        })
        .collect()
}

#[derive(Serialize)]
struct CandidateLocalIdSeed<'a> {
    schema: &'static str,
    agent_plan_id: &'a str,
    binding_id: &'a str,
}

fn candidate_local_id(agent_plan_id: &str, binding_id: &str) -> Result<u32, PublicationError> {
    let digest = CanonicalDigest::of(&CandidateLocalIdSeed {
        schema: "hiroute.gateway-candidate-local-id/v1",
        agent_plan_id,
        binding_id,
    })
    .map_err(|_| PublicationError::Encoding)?;
    let local_id = digest
        .as_str()
        .get(7..15)
        .and_then(|value| u32::from_str_radix(value, 16).ok())
        .ok_or(PublicationError::Encoding)?;
    if local_id == 0 {
        Err(PublicationError::CandidateIdCollision)
    } else {
        Ok(local_id)
    }
}

pub(super) fn ordered_candidates(
    plan: &CompiledAgentPlanV1,
) -> Result<Vec<&AttemptOwnedCandidateV1>, PublicationError> {
    let materialized = &plan.body.materialized;
    let group_ids: Vec<MaterializedGroupId> = match &materialized.request_owned {
        RequestOwnedRouteV1::Branches { branches, .. } => branches
            .iter()
            .flat_map(|b| [Some(b.group), b.primary_group])
            .flatten()
            .collect(),
        RequestOwnedRouteV1::Classified {
            simple_groups,
            complex_groups,
            ..
        } => simple_groups
            .iter()
            .chain(complex_groups)
            .copied()
            .collect(),
        RequestOwnedRouteV1::Ordered { ordered_groups, .. } => ordered_groups.clone(),
    };
    let groups = materialized
        .attempt_owned
        .groups
        .iter()
        .map(|group| (group.group_id, group))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();
    for group_id in group_ids {
        let group = groups
            .get(&group_id)
            .ok_or(PublicationError::InvalidExecutableProjection)?;
        for candidate in &group.candidates {
            if seen.insert(candidate_execution_key(plan, candidate)?) {
                ordered.push(candidate);
            }
        }
    }
    if ordered.is_empty() {
        Err(PublicationError::InvalidExecutableProjection)
    } else {
        Ok(ordered)
    }
}

pub(super) fn candidate_execution_key(
    plan: &CompiledAgentPlanV1,
    candidate: &AttemptOwnedCandidateV1,
) -> Result<String, PublicationError> {
    if matches!(
        plan.body.materialized.request_owned,
        RequestOwnedRouteV1::Branches { .. }
    ) {
        CanonicalDigest::of(&(&candidate.binding_id, &candidate.exact_reasoning))
            .map(|digest| format!("candidate/{digest}"))
            .map_err(|_| PublicationError::Encoding)
    } else {
        Ok(candidate.binding_id.clone())
    }
}

pub(super) fn valid_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains("//")
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
        })
}

pub(super) fn valid_digest(value: &CanonicalDigest) -> bool {
    matches!(CanonicalDigest::parse(value.as_str()), Ok(parsed) if &parsed == value)
}
