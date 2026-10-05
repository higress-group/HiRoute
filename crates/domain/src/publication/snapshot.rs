//! Versioned Gateway projection, including durable refusal of ordinary new calls.
use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GatewayModelRouteV2 {
    Plan {
        plan_id: crate::AgentPlanId,
        alias: ModelAlias,
        revision: u64,
        semantic_digest: CanonicalDigest,
    },
    Fixed {
        binding: Box<GatewayExecutableCandidateV2>,
        binding_digest: CanonicalDigest,
        overall_timeout_ms: u64,
        max_attempts: u32,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayProjectedGrantV2 {
    pub grant_id: String,
    pub generation: u64,
    pub bearer_token_sha256: CanonicalDigest,
    pub protocol: AgentIngressProtocolV1,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub route_protocols: BTreeMap<String, AgentIngressProtocolV1>,
    pub routes: BTreeMap<String, GatewayModelRouteV2>,
}

pub(super) fn project_grants(
    grants: Vec<GatewayExecutableGrantV2>,
    aliases: &[GatewayExecutableAliasV2],
    plans: &[CompiledAgentPlanV1],
) -> Result<Vec<GatewayProjectedGrantV2>, PublicationError> {
    let mut local_ids = aliases
        .iter()
        .flat_map(|alias| alias.candidates.iter().map(|c| c.local_id))
        .collect::<BTreeSet<_>>();
    grants
        .into_iter()
        .map(|grant| {
            let protocol = grant.model_grant.protocol;
            let routes = grant
                .model_grant
                .routes
                .into_iter()
                .map(|(name, route)| {
                    let route_protocol = grant
                        .model_grant
                        .route_protocols
                        .get(&name)
                        .copied()
                        .unwrap_or(protocol);
                    let route = match route {
                        crate::AgentModelRouteV2::Plan {
                            plan_id,
                            alias,
                            revision,
                            semantic_digest,
                        } => {
                            let plan = plans
                                .iter()
                                .find(|plan| {
                                    plan.agent_plan_id() == &plan_id && plan.model_alias() == &alias
                                })
                                .ok_or(PublicationError::InvalidGrant)?;
                            if revision > plan.body.agent_plan_revision
                                || (revision == plan.body.agent_plan_revision
                                    && semantic_digest != plan.body.materialized_route_digest)
                                || !aliases.iter().any(|projected| {
                                    projected.served_model_id == alias
                                        && projected.protocols.contains(&route_protocol)
                                })
                            {
                                return Err(PublicationError::InvalidGrant);
                            }
                            GatewayModelRouteV2::Plan {
                                plan_id,
                                alias,
                                revision: plan.body.agent_plan_revision,
                                semantic_digest: plan.body.materialized_route_digest.clone(),
                            }
                        }
                        crate::AgentModelRouteV2::Fixed { binding, .. } => {
                            let digest = CanonicalDigest::of(&(
                                "hiroute.fixed-local-id/v2",
                                &grant.grant_id,
                                &name,
                                &binding.binding_id,
                            ))
                            .map_err(|_| PublicationError::Encoding)?;
                            let local_id = digest
                                .as_str()
                                .get(7..15)
                                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                                .ok_or(PublicationError::Encoding)?;
                            if local_id == 0 || !local_ids.insert(local_id) {
                                return Err(PublicationError::CandidateIdCollision);
                            }
                            let mut projected = project_candidate(&binding, local_id, &[protocol])?;
                            projected.pricing_identity = Some(GatewayCandidatePricingIdentityV1 {
                                source_id: binding.source_id.clone(),
                                source_identity_digest: binding.source_identity_digest.clone(),
                                model_configuration_id: binding.model_configuration_id.clone(),
                                actual_offer_ref: binding.offer_ref.clone(),
                            });
                            let binding_digest = CanonicalDigest::of(&projected)
                                .map_err(|_| PublicationError::Encoding)?;
                            GatewayModelRouteV2::Fixed {
                                binding: Box::new(projected),
                                binding_digest,
                                overall_timeout_ms: MAX_LOGICAL_REQUEST_DURATION_MS,
                                max_attempts: u32::from(MAX_REQUEST_ATTEMPTS),
                            }
                        }
                    };
                    Ok((name, route))
                })
                .collect::<Result<BTreeMap<_, _>, PublicationError>>()?;
            Ok(GatewayProjectedGrantV2 {
                grant_id: grant.grant_id,
                generation: grant.generation,
                bearer_token_sha256: grant.bearer_token_sha256,
                protocol,
                route_protocols: grant.model_grant.route_protocols,
                routes,
            })
        })
        .collect()
}

pub(super) fn project_candidate(
    candidate: &AttemptOwnedCandidateV1,
    local_id: u32,
    protocols: &[AgentIngressProtocolV1],
) -> Result<GatewayExecutableCandidateV2, PublicationError> {
    candidate
        .validate()
        .map_err(|_| PublicationError::InvalidExecutableProjection)?;
    for protocol in protocols {
        let ingress = match protocol {
            AgentIngressProtocolV1::Responses => UpstreamProtocol::Responses,
            AgentIngressProtocolV1::Messages => UpstreamProtocol::Messages,
        };
        if candidate
            .protocol_profiles
            .iter()
            .filter(|profile| profile.ingress_protocol == ingress)
            .count()
            != 1
        {
            return Err(PublicationError::InvalidExecutableProjection);
        }
    }
    let authentication = candidate
        .protocol_profiles
        .first()
        .and_then(|profile| profile.connector.authentication.exact());
    if authentication.is_none()
        || candidate
            .protocol_profiles
            .iter()
            .any(|profile| profile.connector.authentication.exact() != authentication)
    {
        return Err(PublicationError::InvalidExecutableProjection);
    }
    Ok(GatewayExecutableCandidateV2 {
        local_id,
        stable_target_key: candidate.binding_id.clone(),
        adapter_id: format!("{}@{}", candidate.adapter_ref, candidate.adapter_revision),
        credential_refs: candidate.credential_refs.clone(),
        credential_destination_ref: candidate
            .credential_destination_ref
            .clone()
            .unwrap_or_else(|| format!("connection-option/{}", candidate.connection_option_id)),
        upstream_model_id: candidate.upstream_model_id.clone(),
        native_transport_model: candidate.native_transport_model.clone(),
        endpoint: candidate.endpoint.clone(),
        connector_runtime: candidate.connector_runtime,
        operational_target: candidate.operational_target.clone(),
        operational_target_digest: candidate.operational_target_digest.clone(),
        protocol_profiles: candidate.protocol_profiles.clone(),
        protocol_profile_digest: candidate.protocol_profile_digest.clone(),
        pricing_identity: None,
    })
}

pub(super) fn project_pricing_identities(
    plans: &[CompiledAgentPlanV1],
    aliases: &mut [GatewayExecutableAliasV2],
) -> Result<(), PublicationError> {
    for alias in aliases {
        let plan = plans
            .iter()
            .find(|plan| plan.model_alias() == &alias.served_model_id)
            .ok_or(PublicationError::InvalidExecutableProjection)?;
        let by_binding = ordered_candidates(plan)?
            .into_iter()
            .map(|candidate| (candidate.binding_id.as_str(), candidate))
            .collect::<BTreeMap<_, _>>();
        for projected in &mut alias.candidates {
            let candidate = by_binding
                .get(projected.stable_target_key.as_str())
                .copied()
                .ok_or(PublicationError::InvalidExecutableProjection)?;
            projected.pricing_identity = Some(GatewayCandidatePricingIdentityV1 {
                source_id: candidate.source_id.clone(),
                source_identity_digest: candidate.source_identity_digest.clone(),
                model_configuration_id: candidate.model_configuration_id.clone(),
                actual_offer_ref: candidate.offer_ref.clone(),
            });
        }
    }
    Ok(())
}

/// Explicit V3 serving state; an installed deny snapshot still has a durable authority identity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayAdmissionStateV1 {
    NewCallsAllowed,
    NoNewCalls,
}

/// Exact shape accepted by the frozen G0 publication contract.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayPublicationSnapshotProjectionV3 {
    pub schema_version: String,
    pub admission: GatewayAdmissionStateV1,
    pub workspace_id: String,
    pub authority_id: String,
    pub authority_epoch: u64,
    pub publication_revision: u64,
    pub payload_digest: String,
    pub catalog_renderer_revision: String,
    pub aliases: Vec<GatewayExecutableAliasV2>,
    pub grants: Vec<GatewayProjectedGrantV2>,
}

impl GatewayPublicationSnapshotProjectionV3 {
    /// Called only after the product aggregate proves there are no ordinary callable aliases.
    pub fn no_new_calls(
        workspace_id: String,
        authority_id: String,
        authority_epoch: u64,
        publication_revision: u64,
        catalog_renderer_revision: String,
    ) -> Result<Self, PublicationError> {
        WorkspaceId::parse(&workspace_id).map_err(|_| PublicationError::InvalidWorkspace)?;
        GatewayPublicationRevision::new(publication_revision)?;
        if !valid_reference(&authority_id)
            || authority_epoch == 0
            || !valid_reference(&catalog_renderer_revision)
        {
            return Err(PublicationError::InvalidAuthority);
        }
        let mut value = Self {
            schema_version: GATEWAY_SNAPSHOT_SCHEMA_V3.to_owned(),
            admission: GatewayAdmissionStateV1::NoNewCalls,
            workspace_id,
            authority_id,
            authority_epoch,
            publication_revision,
            payload_digest: String::new(),
            catalog_renderer_revision,
            aliases: Vec::new(),
            grants: Vec::new(),
        };
        value.payload_digest = value.canonical_digest()?.to_string();
        Ok(value)
    }

    pub(super) fn seal(
        workspace_id: String,
        authority_id: String,
        authority_epoch: u64,
        publication_revision: u64,
        catalog_renderer_revision: String,
        aliases: Vec<GatewayExecutableAliasV2>,
        grants: Vec<GatewayProjectedGrantV2>,
    ) -> Result<Self, PublicationError> {
        let mut value = Self {
            schema_version: GATEWAY_SNAPSHOT_SCHEMA_V3.to_owned(),
            admission: GatewayAdmissionStateV1::NewCallsAllowed,
            workspace_id,
            authority_id,
            authority_epoch,
            publication_revision,
            payload_digest: String::new(),
            catalog_renderer_revision,
            aliases,
            grants,
        };
        value.payload_digest = value.canonical_digest()?.to_string();
        Ok(value)
    }

    pub fn canonical_digest(&self) -> Result<CanonicalDigest, PublicationError> {
        let mut digestless = self.clone();
        digestless.payload_digest.clear();
        // Pricing identities are an additive observation projection derived
        // from the already-digested Product Plan. They must not rewrite an
        // existing Gateway execution revision during an upgrade.
        for alias in &mut digestless.aliases {
            for candidate in &mut alias.candidates {
                candidate.pricing_identity = None;
            }
        }
        let bytes = serde_json::to_vec(&digestless).map_err(|_| PublicationError::Encoding)?;
        Ok(CanonicalDigest::of_bytes(&bytes))
    }
}
