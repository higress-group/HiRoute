use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use hiroute_domain::{
    CanonicalDigest, GatewayOperationalTargetV1, InputUsageMeaningV1, OutputUsageMeaningV1,
    PriceBillingContextV1, UsageFrameKindV1, UsageSemanticsV1,
};
use hiroute_gateway_core::core::execution_plan::{
    AdapterId, AtomicityGroupId, AttemptBodyPlans, AttemptPlanIndex, AttemptTimeouts, AuthorityId,
    CaPolicy, CompiledAcceptedResponsePlan, CompiledAttemptPlan, CompiledIngressPlan,
    CompiledLocalResponsePlan, CompiledLogicalRequestPlan, CompiledRequestPlan,
    CompiledRequestPlanHandle, CompiledRoute, ConfigBindingPolicy, ConfigBundle,
    ConfigCellDescriptor, ConfigCellGroup, ConfigCellId, ConfigGeneration, ConfigRevision,
    CredentialRef, ImmutableConfig, PlanRevision, PoolEpoch, ResolvedTargetBindingId,
    StableTargetKey, TransportReuseClassId, TransportScheme, TransportTarget,
    TransportTargetPolicy,
};
use hiroute_gateway_core::core::publication::{
    CompiledGatewayPublicationEnvelope, SUPPORTED_COMPILER_VERSION, SUPPORTED_SCHEMA_VERSION,
};
use hiroute_gateway_core::runtime::body::BodyPlan;
use http::Uri;
use sha2::{Digest, Sha256};

use super::schema::{
    AliasCostPolicyV1, AliasGroupIdV1, AliasRequestOwnedRouteV1, AliasRoutingV1,
    agent_plan_semantic_digest,
};
use super::{AliasPlanV1, GatewayPublicationSnapshotV3, GrantV1, PublicationInstallError};
use crate::server::core_runtime::profiles::{
    CompiledClassifierKindV1, CompiledComplexityStrategyV1, CompiledPlannerPolicyV1, ComplexityV1,
    FreeCandidateModeV1, FreeFirstExhaustionV1, GroupPolicyV1, MaterializedModelGroupV1,
    MaterializedRouteV1, PLANNER_POLICY_SCHEMA, RequestOwnedLimitsV1, StaticCostPolicyV1,
};
use crate::server::request_plan::{
    ClassifierAuthenticationAuthorityV1, ClassifierBranchAuthorityV1, IngressProtocol,
    ProviderCandidateAuthority, RequestPriceBindingV1, RestBranchClassifierAuthorityV1,
};

// Replay spills; allocation is governed by the request memory budget.
// The body plan carries no additional protocol byte limit.
const LOGICAL_BODY_LIMIT: usize = usize::MAX;
const FRAME_LIMIT: usize = 64 * 1024;
const DEFAULT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug)]
pub(crate) struct CompiledAlias {
    pub served_model_id: Arc<str>,
    pub purpose: Arc<str>,
    pub plan_display_name: Option<Arc<str>>,
    pub agent_plan_revision: u64,
    pub agent_plan_semantic_digest: Arc<str>,
    pub execution: CompiledModelExecution,
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledModelExecution {
    pub protocols: Arc<[IngressProtocol]>,
    pub overall_timeout: Duration,
    pub max_attempts: u32,
    pub planner_policy: Arc<CompiledPlannerPolicyV1>,
    pub classifier: Option<Arc<RestBranchClassifierAuthorityV1>>,
    pub route: CompiledRoute,
    pub request_plan: CompiledRequestPlanHandle,
    pub candidates: Arc<[ProviderCandidateAuthority]>,
    pub pricing_bindings: Arc<[RequestPriceBindingV1]>,
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledGrant {
    pub grant_id: Arc<str>,
    pub generation: u64,
    pub bearer_token_sha256: Arc<str>,
    pub protocol: IngressProtocol,
    pub routes: BTreeMap<Arc<str>, CompiledGrantRoute>,
}

#[derive(Clone, Debug)]
pub(crate) enum CompiledGrantRoute {
    Plan {
        alias: Arc<str>,
    },
    Fixed {
        binding_digest: CanonicalDigest,
        execution: CompiledModelExecution,
    },
}

#[derive(Debug)]
pub(crate) struct CompiledAggregate {
    pub envelope: CompiledGatewayPublicationEnvelope,
    pub aliases: BTreeMap<Arc<str>, CompiledAlias>,
    pub grants: Arc<[CompiledGrant]>,
}

pub(crate) fn compile(
    snapshot: &GatewayPublicationSnapshotV3,
) -> Result<CompiledAggregate, PublicationInstallError> {
    snapshot.validate()?;
    let plan_revision = PlanRevision(snapshot.publication_revision);
    let attempt_timeouts = attempt_timeouts();
    let mut routes = Vec::with_capacity(snapshot.aliases.len());
    let mut attempts = HashMap::new();
    let mut config_cells = HashMap::new();
    let mut aliases = BTreeMap::new();

    for alias in &snapshot.aliases {
        let compiled = compile_alias(alias, plan_revision, snapshot.publication_revision)?;
        routes.push(compiled.execution.route.clone());
        aliases.insert(Arc::clone(&compiled.served_model_id), compiled);
    }
    let fixed_candidates = snapshot
        .grants
        .iter()
        .flat_map(|grant| grant.routes.values())
        .filter_map(|route| match route {
            super::schema::ModelRouteV2::Fixed { binding, .. } => Some(binding.as_ref()),
            super::schema::ModelRouteV2::Plan { .. } => None,
        });
    for candidate in snapshot
        .aliases
        .iter()
        .flat_map(|alias| &alias.candidates)
        .chain(fixed_candidates)
    {
        let binding = ResolvedTargetBindingId::new(plan_revision, candidate.local_id);
        let endpoint = parse_endpoint(candidate.operational_target.uri())?;
        let config_id = ConfigCellId(u64::from(candidate.local_id));
        let group_id = AtomicityGroupId(u64::from(candidate.local_id));
        // Request-pricing identity is observation-only metadata. Keep it
        // out of core config compatibility so an additive upgrade cannot
        // mutate an existing execution revision.
        let mut execution_candidate = candidate.clone();
        execution_candidate.pricing_identity = None;
        let config_bytes =
            serde_json::to_vec(&execution_candidate).map_err(PublicationInstallError::Json)?;
        let compatibility_hash: [u8; 32] = Sha256::digest(&config_bytes).into();
        let descriptor = ConfigCellDescriptor {
            id: config_id,
            compatibility_hash,
            atomicity_group: group_id,
            binding_policy: ConfigBindingPolicy::AttemptPinned,
        };
        let bundle = Arc::new(ConfigBundle::new(
            group_id,
            HashMap::from([(
                config_id,
                ImmutableConfig {
                    generation: ConfigGeneration(snapshot.publication_revision),
                    compatibility_hash,
                    bytes: config_bytes.into(),
                },
            )]),
        ));
        let group = ConfigCellGroup::new([descriptor], bundle)?;
        config_cells.extend(group.handles());
        let authority = if endpoint.resolution_required {
            TransportTarget::mark_resolution_required(&endpoint.authority)
        } else {
            endpoint.authority
        };
        let target = TransportTarget {
            scheme: endpoint.scheme,
            authority,
            addresses: endpoint.addresses,
            sni: endpoint.sni,
            ca: CaPolicy::System,
            alpn: endpoint.alpn,
            connect_timeout: Duration::from_secs(2),
            transport_read_buffer_bytes: FRAME_LIMIT,
            h2_stream_window_bytes: FRAME_LIMIT as u32,
            h2_connection_window_bytes: (FRAME_LIMIT * 4) as u32,
            h2_max_concurrent_streams: 16,
            reuse_class: TransportReuseClassId(u64::from(candidate.local_id)),
            pool_epoch: PoolEpoch(snapshot.publication_revision),
            connection_fingerprint: [0; 32],
        }
        .with_derived_connection_fingerprint();
        attempts.insert(
            binding,
            Arc::new(CompiledAttemptPlan {
                binding,
                stable_target_key: StableTargetKey::new(candidate.stable_target_key.clone())?,
                adapter_id: AdapterId::new(candidate.adapter_id.clone())?,
                credential_refs: candidate
                    .credential_refs
                    .iter()
                    .cloned()
                    .map(CredentialRef::new)
                    .collect::<Result<Vec<_>, _>>()?
                    .into(),
                transport_target: target,
                transport_target_policy: match candidate.operational_target {
                    GatewayOperationalTargetV1::RegisteredHttps { .. }
                    | GatewayOperationalTargetV1::UserConfiguredNative { .. } => {
                        TransportTargetPolicy::Exact
                    }
                    GatewayOperationalTargetV1::ManagedCpaLoopback { .. } => {
                        TransportTargetPolicy::ManagedLoopback
                    }
                },
                timeouts: attempt_timeouts,
                config_cell_ids: Arc::new([config_id]),
                attempt_request_filters: Arc::new([]),
                attempt_response_filters: Arc::new([]),
                body_plans: AttemptBodyPlans {
                    attempt_request: BodyPlan::StreamingReplay {
                        max_chunk_bytes: FRAME_LIMIT,
                        max_replay_bytes: LOGICAL_BODY_LIMIT,
                    },
                    attempt_response_precommit: BodyPlan::PassThrough {
                        max_chunk_bytes: FRAME_LIMIT,
                    },
                },
                attempt_request_chunk_capacity: 64,
                precommit_event_capacity: 64,
            }),
        );
    }

    let attempt_index = AttemptPlanIndex::new(plan_revision, attempts)?;
    let connection_epoch_fingerprints = attempt_index
        .plans()
        .map(|(_, plan)| plan.transport_target.connection_epoch_fingerprint())
        .collect::<Vec<_>>()
        .into();
    let local_accepted = Arc::new(CompiledAcceptedResponsePlan {
        filters: Arc::new([]),
        body_plan: BodyPlan::PassThrough {
            max_chunk_bytes: FRAME_LIMIT,
        },
        semantic_replacement_authorized: false,
        config_cell_ids: Arc::new([]),
    });
    let grants = snapshot
        .grants
        .iter()
        .map(|grant| compile_grant(grant, snapshot.publication_revision))
        .collect::<Result<Vec<_>, _>>()?;
    for grant in &grants {
        for route in grant.routes.values() {
            if let CompiledGrantRoute::Fixed { execution, .. } = route {
                routes.push(execution.route.clone());
            }
        }
    }
    Ok(CompiledAggregate {
        envelope: CompiledGatewayPublicationEnvelope {
            authority_id: AuthorityId::new(snapshot.authority_id.clone())?,
            authority_epoch: snapshot.authority_epoch,
            config_revision: ConfigRevision(snapshot.publication_revision),
            plan_revision,
            schema_version: SUPPORTED_SCHEMA_VERSION,
            compiler_version: SUPPORTED_COMPILER_VERSION,
            payload_digest: snapshot.digest_bytes()?,
            ingress_plan_handle: Arc::new(CompiledIngressPlan {
                plan_revision,
                routes: routes.into(),
                local_response_plan: Arc::new(CompiledLocalResponsePlan {
                    accepted_response: local_accepted,
                    overall_request_timeout: Duration::from_secs(60),
                }),
            }),
            attempt_plan_index_handle: Arc::new(attempt_index),
            config_cells_handle: Arc::new(config_cells),
            connection_epoch_fingerprints,
            // The aggregate feed is ordered. A live revision gap cannot be
            // treated as a resync because doing so would conceal missed grant
            // or alias transitions. A cold restore has no active revision and
            // remains accepted by the core installer.
            full_snapshot: false,
            rollback_authorized: false,
        },
        aliases,
        grants: grants.into(),
    })
}

fn attempt_timeouts() -> AttemptTimeouts {
    #[cfg(feature = "e2e-test-control")]
    let timeout =
        crate::server::test_control::attempt_timeout_override().unwrap_or(DEFAULT_ATTEMPT_TIMEOUT);
    #[cfg(not(feature = "e2e-test-control"))]
    let timeout = DEFAULT_ATTEMPT_TIMEOUT;
    AttemptTimeouts {
        request_write: timeout,
        first_byte: timeout,
        stream_idle: timeout,
    }
}

fn compile_alias(
    alias: &AliasPlanV1,
    plan_revision: PlanRevision,
    publication_revision: u64,
) -> Result<CompiledAlias, PublicationInstallError> {
    let classifier = compile_classifier_authority(alias, publication_revision)?;
    Ok(CompiledAlias {
        served_model_id: alias.served_model_id.clone().into(),
        purpose: alias.purpose.clone().into(),
        plan_display_name: alias
            .routing
            .as_ref()
            .and_then(|routing| routing.plan_display_name.clone())
            .map(Arc::from),
        agent_plan_revision: alias.agent_plan_revision,
        agent_plan_semantic_digest: agent_plan_semantic_digest(alias)?.into(),
        execution: compile_execution(
            &alias.candidates,
            &alias.protocols,
            alias.overall_timeout_ms,
            alias.max_attempts,
            compile_planner_policy(alias)?,
            classifier,
            plan_revision,
            publication_revision,
        )?,
    })
}

fn compile_classifier_authority(
    alias: &AliasPlanV1,
    publication_revision: u64,
) -> Result<Option<Arc<RestBranchClassifierAuthorityV1>>, PublicationInstallError> {
    let Some(AliasRoutingV1 {
        request_owned: AliasRequestOwnedRouteV1::Classified { classifier, .. },
        ..
    }) = alias.routing.as_ref()
    else {
        return Ok(None);
    };
    if !matches!(
        &classifier.mode,
        hiroute_domain::ComplexityClassifierModeV1::Rest { .. }
    ) {
        return Ok(None);
    }
    compile_classifier_mode_authority(&classifier.mode, publication_revision).map(Some)
}

pub(crate) fn compile_classifier_mode_authority(
    mode: &hiroute_domain::ComplexityClassifierModeV1,
    publication_revision: u64,
) -> Result<Arc<RestBranchClassifierAuthorityV1>, PublicationInstallError> {
    mode.validate()
        .map_err(|_| PublicationInstallError::InvalidPlannerPolicy)?;
    let hiroute_domain::ComplexityClassifierModeV1::Rest {
        endpoint,
        timeout_ms,
        auth_header,
    } = mode
    else {
        return Err(PublicationInstallError::InvalidPlannerPolicy);
    };
    let uri = endpoint
        .parse::<Uri>()
        .map_err(|_| PublicationInstallError::InvalidEndpoint(endpoint.clone()))?;
    let path_and_query = uri
        .path_and_query()
        .map(http::uri::PathAndQuery::as_str)
        .unwrap_or("/");
    if !path_and_query.starts_with('/') || path_and_query.contains('#') {
        return Err(PublicationInstallError::InvalidEndpoint(endpoint.clone()));
    }
    let parsed = parse_endpoint(endpoint)?;
    if parsed.authority.contains('@') {
        return Err(PublicationInstallError::InvalidEndpoint(endpoint.clone()));
    }
    let authority = uri
        .authority()
        .ok_or_else(|| PublicationInstallError::InvalidEndpoint(endpoint.clone()))?;
    let host = authority.host();
    let bare_host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host)
        .to_ascii_lowercase();
    let rendered_host = if bare_host.contains(':') {
        format!("[{bare_host}]")
    } else {
        bare_host.clone()
    };
    let default_port = if parsed.scheme == TransportScheme::Https {
        443
    } else {
        80
    };
    let port = authority.port_u16().unwrap_or(default_port);
    let normalized_authority =
        if authority.port_u16().is_some() || bare_host.parse::<IpAddr>().is_ok() {
            format!("{rendered_host}:{port}")
        } else {
            rendered_host
        };
    let scheme = if parsed.scheme == TransportScheme::Https {
        "https"
    } else {
        "http"
    };
    let normalized_endpoint = format!("{scheme}://{normalized_authority}{path_and_query}");
    let timeout = Duration::from_millis(*timeout_ms);
    let digest: [u8; 32] = Sha256::digest(normalized_endpoint.as_bytes()).into();
    let reuse_class = TransportReuseClassId(u64::from_be_bytes(
        digest[..8]
            .try_into()
            .map_err(|_| PublicationInstallError::InvalidPlannerPolicy)?,
    ));
    let target_authority = if parsed.resolution_required {
        TransportTarget::mark_resolution_required(&normalized_authority)
    } else {
        normalized_authority.clone().into()
    };
    let target = TransportTarget {
        scheme: parsed.scheme,
        authority: target_authority,
        addresses: parsed.addresses,
        sni: parsed.sni.map(|_| Arc::from(bare_host.as_str())),
        ca: CaPolicy::System,
        alpn: parsed.alpn,
        connect_timeout: timeout,
        transport_read_buffer_bytes: FRAME_LIMIT,
        h2_stream_window_bytes: FRAME_LIMIT as u32,
        h2_connection_window_bytes: (FRAME_LIMIT * 4) as u32,
        h2_max_concurrent_streams: 16,
        reuse_class,
        pool_epoch: PoolEpoch(publication_revision),
        connection_fingerprint: [0; 32],
    }
    .with_derived_connection_fingerprint();
    target
        .validate()
        .map_err(|_| PublicationInstallError::InvalidEndpoint(endpoint.clone()))?;
    let authentication = match auth_header {
        None => ClassifierAuthenticationAuthorityV1::None,
        Some(header) => ClassifierAuthenticationAuthorityV1::Header {
            name: Arc::from(header.name.as_str()),
            value_secret_ref: Arc::from(header.value_secret_ref.as_str()),
        },
    };
    let branches: Arc<[ClassifierBranchAuthorityV1]> = Arc::from([
        ClassifierBranchAuthorityV1 {
            id: Arc::from(hiroute_domain::SMART_SAVING_SIMPLE_BRANCH_ID),
            description: Arc::from(hiroute_domain::SMART_SAVING_SIMPLE_BRANCH_DESCRIPTION),
        },
        ClassifierBranchAuthorityV1 {
            id: Arc::from(hiroute_domain::SMART_SAVING_COMPLEX_BRANCH_ID),
            description: Arc::from(hiroute_domain::SMART_SAVING_COMPLEX_BRANCH_DESCRIPTION),
        },
    ]);
    Ok(Arc::new(RestBranchClassifierAuthorityV1 {
        endpoint: normalized_endpoint.into(),
        http_authority: normalized_authority.into(),
        request_path: Arc::from(path_and_query),
        timeout,
        transport_target: target,
        authentication,
        branches,
    }))
}

#[allow(clippy::too_many_arguments)]
fn compile_execution(
    bindings: &[super::CandidateBindingV1],
    protocols: &[IngressProtocol],
    overall_timeout_ms: u64,
    max_attempts: u32,
    planner_policy: CompiledPlannerPolicyV1,
    classifier: Option<Arc<RestBranchClassifierAuthorityV1>>,
    plan_revision: PlanRevision,
    publication_revision: u64,
) -> Result<CompiledModelExecution, PublicationInstallError> {
    let candidates = bindings
        .iter()
        .map(|candidate| ResolvedTargetBindingId::new(plan_revision, candidate.local_id))
        .collect::<Vec<_>>();
    let primary = candidates[0];
    let logical = Arc::new(CompiledLogicalRequestPlan {
        filters: Arc::new([]),
        body_plan: BodyPlan::BufferedTransform {
            max_body_bytes: LOGICAL_BODY_LIMIT,
        },
        config_cell_ids: Arc::new([]),
        chunk_capacity: 64,
    });
    let accepted = Arc::new(CompiledAcceptedResponsePlan {
        filters: Arc::new([]),
        body_plan: BodyPlan::PassThrough {
            max_chunk_bytes: FRAME_LIMIT,
        },
        semantic_replacement_authorized: false,
        config_cell_ids: Arc::new([]),
    });
    let request_plan = Arc::new(CompiledRequestPlan {
        candidate_bindings: candidates.into(),
        logical_request: logical,
        accepted_response: accepted,
        overall_request_timeout: Duration::from_millis(overall_timeout_ms),
        max_attempts,
    });
    let candidate_authorities = bindings
        .iter()
        .map(|candidate| ProviderCandidateAuthority {
            binding_local_id: candidate.local_id,
            endpoint: Arc::from(candidate.operational_target.uri()),
            credential_refs: candidate
                .credential_refs
                .iter()
                .cloned()
                .map(Arc::from)
                .collect::<Vec<_>>()
                .into(),
        })
        .collect::<Vec<_>>()
        .into();
    let pricing_bindings = compile_candidate_pricing(bindings, protocols)?;
    let planner_policy = Arc::new(planner_policy);
    let route = CompiledRoute {
        normalized_host: Arc::from("gateway.invalid"),
        path_prefix: format!(
            "/_hiroute/compiled/{publication_revision}/{}",
            bindings[0].local_id
        )
        .into(),
        binding: primary,
        request_plan: Arc::clone(&request_plan),
    };
    Ok(CompiledModelExecution {
        protocols: protocols.to_vec().into(),
        overall_timeout: Duration::from_millis(overall_timeout_ms),
        max_attempts,
        planner_policy,
        classifier,
        route,
        request_plan,
        candidates: candidate_authorities,
        pricing_bindings,
    })
}

pub(super) fn compile_pricing_bindings(
    alias: &AliasPlanV1,
) -> Result<Arc<[RequestPriceBindingV1]>, PublicationInstallError> {
    compile_candidate_pricing(&alias.candidates, &alias.protocols)
}

fn compile_candidate_pricing(
    candidates: &[super::CandidateBindingV1],
    protocols: &[IngressProtocol],
) -> Result<Arc<[RequestPriceBindingV1]>, PublicationInstallError> {
    let mut bindings = Vec::new();
    for candidate in candidates {
        let Some(identity) = &candidate.pricing_identity else {
            continue;
        };
        for product_profile in &candidate.protocol_profiles {
            let profile: crate::server::core_runtime::profiles::CandidateProtocolProfile =
                serde_json::from_value(
                    serde_json::to_value(product_profile).map_err(PublicationInstallError::Json)?,
                )
                .map_err(PublicationInstallError::Json)?;
            if !protocols.contains(&profile.ingress_protocol) {
                continue;
            }
            let profile_digest = CanonicalDigest::of(&profile).map_err(|_| {
                PublicationInstallError::Schema(super::PublicationSchemaError::InvalidCandidate(
                    candidate.local_id,
                ))
            })?;
            bindings.push(RequestPriceBindingV1 {
                stable_binding_id: candidate.stable_target_key.clone().into(),
                model_configuration_id: identity.model_configuration_id.clone().into(),
                profile_digest: profile_digest.as_str().into(),
                source_id: identity.source_id.clone().into(),
                source_identity_digest: identity.source_identity_digest.clone(),
                actual_offer_ref: identity.actual_offer_ref.clone().into(),
                usage_semantics: usage_semantics(profile.capability.upstream_protocol),
                billing_context: PriceBillingContextV1::StandardTokens,
            });
        }
    }
    bindings.sort_by(|left, right| {
        (
            left.stable_binding_id.as_ref(),
            left.model_configuration_id.as_ref(),
            left.profile_digest.as_ref(),
        )
            .cmp(&(
                right.stable_binding_id.as_ref(),
                right.model_configuration_id.as_ref(),
                right.profile_digest.as_ref(),
            ))
    });
    if bindings.windows(2).any(|pair| {
        pair[0].stable_binding_id == pair[1].stable_binding_id
            && pair[0].model_configuration_id == pair[1].model_configuration_id
            && pair[0].profile_digest == pair[1].profile_digest
    }) {
        return Err(PublicationInstallError::Schema(
            super::PublicationSchemaError::InvalidCandidate(candidates[0].local_id),
        ));
    }
    Ok(bindings.into())
}

fn usage_semantics(protocol: IngressProtocol) -> UsageSemanticsV1 {
    UsageSemanticsV1 {
        frame_kind: UsageFrameKindV1::Cumulative,
        input: match protocol {
            IngressProtocol::Responses | IngressProtocol::ChatCompletions => {
                InputUsageMeaningV1::IncludesExclusiveCache
            }
            IngressProtocol::Messages => InputUsageMeaningV1::UncachedOnly,
        },
        output: OutputUsageMeaningV1::IncludesReasoning,
        cache_buckets_exclusive: true,
    }
}

fn compile_planner_policy(
    alias: &AliasPlanV1,
) -> Result<CompiledPlannerPolicyV1, PublicationInstallError> {
    let candidate_key_by_local_id = alias
        .candidates
        .iter()
        .map(|candidate| (candidate.local_id, candidate.stable_target_key.as_str()))
        .collect::<BTreeMap<_, _>>();
    let limits = RequestOwnedLimitsV1 {
        max_candidate_bindings: u32::try_from(alias.candidates.len())
            .map_err(|_| PublicationInstallError::InvalidPlannerPolicy)?,
        max_attempts: alias.max_attempts,
        deadline_cap_ms: alias.overall_timeout_ms,
        paid_budget_ceiling_micros: None,
    };
    let (plan_id, route, groups, complexity_strategy, cost_policy) = match &alias.routing {
        None => (
            format!("legacy/{}", alias.served_model_id),
            MaterializedRouteV1::Custom {
                group_id: "custom".into(),
            },
            vec![MaterializedModelGroupV1 {
                group_id: "custom".into(),
                policy: GroupPolicyV1::Manual,
                candidate_ids: alias
                    .candidates
                    .iter()
                    .map(|candidate| candidate.stable_target_key.clone())
                    .collect(),
            }],
            None,
            StaticCostPolicyV1::SubscriptionAndFree,
        ),
        Some(routing) => compile_routing(routing, &candidate_key_by_local_id)?,
    };
    CompiledPlannerPolicyV1 {
        schema_version: PLANNER_POLICY_SCHEMA.into(),
        served_model_id: alias.served_model_id.clone(),
        identity: crate::server::core_runtime::profiles::PlannerRouteIdentityV2::Plan {
            plan_id,
            revision: alias.agent_plan_revision,
        },
        route,
        groups,
        complexity_strategy,
        cost_policy,
        limits,
        policy_digest: String::new(),
    }
    .seal()
    .map_err(|_| PublicationInstallError::InvalidPlannerPolicy)
}

type CompiledRoutingParts = (
    String,
    MaterializedRouteV1,
    Vec<MaterializedModelGroupV1>,
    Option<CompiledComplexityStrategyV1>,
    StaticCostPolicyV1,
);

fn compile_routing(
    routing: &AliasRoutingV1,
    candidate_key_by_local_id: &BTreeMap<u32, &str>,
) -> Result<CompiledRoutingParts, PublicationInstallError> {
    let groups = routing
        .groups
        .iter()
        .map(|group| {
            let candidate_ids = group
                .candidate_local_ids
                .iter()
                .map(|local_id| {
                    candidate_key_by_local_id
                        .get(local_id)
                        .map(|value| (*value).to_owned())
                        .ok_or(PublicationInstallError::InvalidPlannerPolicy)
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(MaterializedModelGroupV1 {
                group_id: group.group_id.as_str().into(),
                policy: GroupPolicyV1::Manual,
                candidate_ids,
            })
        })
        .collect::<Result<Vec<_>, PublicationInstallError>>()?;
    let group_names = |values: &[AliasGroupIdV1]| {
        values
            .iter()
            .copied()
            .map(AliasGroupIdV1::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let (route, complexity, cost_policy) = match &routing.request_owned {
        AliasRequestOwnedRouteV1::Classified {
            classifier,
            simple_groups,
            complex_groups,
        } => {
            let simple = group_names(simple_groups);
            let complex = group_names(complex_groups);
            let simple_group_id = simple
                .first()
                .cloned()
                .ok_or(PublicationInstallError::InvalidPlannerPolicy)?;
            let complex_group_id = complex
                .first()
                .cloned()
                .ok_or(PublicationInstallError::InvalidPlannerPolicy)?;
            let phrases = classifier
                .user_keywords
                .iter()
                .enumerate()
                .map(|(index, phrase)| (format!("user-phrase-{index}"), phrase.clone()));
            let (classifier_kind, classifier_config_digest) = match &classifier.mode {
                hiroute_domain::ComplexityClassifierModeV1::LocalRules => {
                    (CompiledClassifierKindV1::LocalRules, None)
                }
                hiroute_domain::ComplexityClassifierModeV1::Rest { .. } => (
                    CompiledClassifierKindV1::Rest,
                    Some(
                        CanonicalDigest::of(&classifier.mode)
                            .map_err(|_| PublicationInstallError::InvalidPlannerPolicy)?
                            .as_str()
                            .to_owned(),
                    ),
                ),
            };
            (
                MaterializedRouteV1::SmartSaving {
                    simple_group_id,
                    simple_fallback_group_ids: simple.into_iter().skip(1).collect(),
                    complex_group_id,
                },
                Some(
                    ComplexityV1::compile_with_classifier(
                        phrases,
                        classifier_kind,
                        classifier_config_digest,
                    )
                    .map_err(|_| PublicationInstallError::InvalidPlannerPolicy)?,
                ),
                StaticCostPolicyV1::SubscriptionAndFree,
            )
        }
        AliasRequestOwnedRouteV1::Ordered {
            cost_policy,
            ordered_groups,
        } => match ordered_groups.as_slice() {
            [AliasGroupIdV1::Custom] => (
                MaterializedRouteV1::Custom {
                    group_id: "custom".into(),
                },
                None,
                StaticCostPolicyV1::SubscriptionAndFree,
            ),
            [AliasGroupIdV1::Free] => (
                MaterializedRouteV1::FreeFirst {
                    free_group_id: "free".into(),
                    exhaustion: FreeFirstExhaustionV1::FreeOnly,
                    candidate_mode: FreeCandidateModeV1::Manual,
                },
                None,
                StaticCostPolicyV1::StrictFree,
            ),
            [AliasGroupIdV1::Free, AliasGroupIdV1::Primary] => (
                MaterializedRouteV1::FreeFirst {
                    free_group_id: "free".into(),
                    exhaustion: FreeFirstExhaustionV1::PrimaryFallback {
                        primary_group_id: "primary".into(),
                    },
                    candidate_mode: FreeCandidateModeV1::Manual,
                },
                None,
                match cost_policy {
                    AliasCostPolicyV1::StrictFree => StaticCostPolicyV1::StrictFree,
                    AliasCostPolicyV1::ApiEquivalent => StaticCostPolicyV1::SubscriptionAndFree,
                },
            ),
            _ => return Err(PublicationInstallError::InvalidPlannerPolicy),
        },
    };
    Ok((
        routing.agent_plan_id.clone(),
        route,
        groups,
        complexity,
        cost_policy,
    ))
}

struct ParsedEndpoint {
    scheme: TransportScheme,
    authority: Arc<str>,
    resolution_required: bool,
    addresses: Arc<[SocketAddr]>,
    sni: Option<Arc<str>>,
    alpn: Arc<[Arc<str>]>,
}

/// Parse only. Network DNS is a Provider-side effect performed by PROCESS-
/// 22004 after request authority, credential grant and exact candidate choice.
fn parse_endpoint(value: &str) -> Result<ParsedEndpoint, PublicationInstallError> {
    let uri: Uri = value
        .parse()
        .map_err(|_| PublicationInstallError::InvalidEndpoint(value.into()))?;
    let scheme = match uri.scheme_str() {
        Some("http") => TransportScheme::Http,
        Some("https") => TransportScheme::Https,
        _ => return Err(PublicationInstallError::InvalidEndpoint(value.into())),
    };
    let authority = uri
        .authority()
        .ok_or_else(|| PublicationInstallError::InvalidEndpoint(value.into()))?;
    let host = authority.host();
    let numeric_host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    let port = authority
        .port_u16()
        .unwrap_or(if scheme == TransportScheme::Https {
            443
        } else {
            80
        });
    let (resolution_required, addresses) = match numeric_host.parse::<IpAddr>() {
        Ok(address) => (false, Arc::from([SocketAddr::new(address, port)])),
        Err(_) => (true, Arc::from([])),
    };
    Ok(ParsedEndpoint {
        scheme,
        authority: Arc::from(authority.as_str()),
        resolution_required,
        addresses,
        sni: (scheme == TransportScheme::Https).then(|| Arc::from(numeric_host)),
        alpn: if scheme == TransportScheme::Https {
            Arc::new([Arc::from("h2"), Arc::from("http/1.1")])
        } else {
            Arc::new([Arc::from("http/1.1")])
        },
    })
}

fn compile_grant(grant: &GrantV1, revision: u64) -> Result<CompiledGrant, PublicationInstallError> {
    use super::schema::ModelRouteV2;
    use crate::server::core_runtime::profiles::PlannerRouteIdentityV2;
    let routes = grant
        .routes
        .iter()
        .map(|(name, route)| {
            let compiled = match route {
                ModelRouteV2::Plan { alias, .. } => CompiledGrantRoute::Plan {
                    alias: Arc::from(alias.as_str()),
                },
                ModelRouteV2::Fixed {
                    binding,
                    binding_digest,
                    overall_timeout_ms,
                    max_attempts,
                } => {
                    let policy = CompiledPlannerPolicyV1 {
                        schema_version: PLANNER_POLICY_SCHEMA.into(),
                        served_model_id: name.clone(),
                        identity: PlannerRouteIdentityV2::Fixed {
                            binding_digest: binding_digest.clone(),
                        },
                        route: MaterializedRouteV1::Custom {
                            group_id: "fixed".into(),
                        },
                        groups: vec![MaterializedModelGroupV1 {
                            group_id: "fixed".into(),
                            policy: GroupPolicyV1::Manual,
                            candidate_ids: vec![binding.stable_target_key.clone()],
                        }],
                        complexity_strategy: None,
                        cost_policy: StaticCostPolicyV1::ExplicitFixed,
                        limits: RequestOwnedLimitsV1 {
                            max_candidate_bindings: 1,
                            max_attempts: *max_attempts,
                            deadline_cap_ms: *overall_timeout_ms,
                            paid_budget_ceiling_micros: None,
                        },
                        policy_digest: String::new(),
                    }
                    .seal()
                    .map_err(|_| PublicationInstallError::InvalidPlannerPolicy)?;
                    CompiledGrantRoute::Fixed {
                        binding_digest: binding_digest.clone(),
                        execution: compile_execution(
                            std::slice::from_ref(binding.as_ref()),
                            std::slice::from_ref(&grant.protocol),
                            *overall_timeout_ms,
                            *max_attempts,
                            policy,
                            None,
                            PlanRevision(revision),
                            revision,
                        )?,
                    }
                }
            };
            Ok((Arc::from(name.as_str()), compiled))
        })
        .collect::<Result<BTreeMap<_, _>, PublicationInstallError>>()?;
    Ok(CompiledGrant {
        grant_id: grant.grant_id.clone().into(),
        generation: grant.generation,
        bearer_token_sha256: grant.bearer_token_sha256.clone().into(),
        protocol: grant.protocol,
        routes,
    })
}
