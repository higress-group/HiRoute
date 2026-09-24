// Execution-only limits for an unknown text model; never persisted as provider facts.
const UNKNOWN_TEXT_CONTEXT_TOKENS: u64 = 4_096;
const UNKNOWN_TEXT_OUTPUT_TOKENS: u64 = 1_024;

use hiroute_application::compiler::{CandidateCompilationFactV1, CandidateFactAuthorityV1};
use hiroute_application::compute_management::{
    ComputeManagementCompilationFactV2, ComputeManagementCredentialCompilationV2,
    ComputeManagementEligibilityV2,
};
use hiroute_cpa_bridge::{CpaRoutingBatch, ExactCpaAttemptRequest, PreparedCpaTarget};
use hiroute_domain::{
    AuthenticationKind, BillingClass, CanonicalDigest, CapabilityFactsV1,
    ComputeProjectionExpectationV1, ConnectorRuntimeKind, CredentialPoolV1,
    GatewayAuthenticationSemanticsV1, GatewayCandidateProtocolProfileV1,
    GatewayNativeProfileTargetV2, GatewayOperationalTargetV1, InventoryDisposition,
    ModelDefinitionV1, ModelEndpointCapabilityV1, ModelNativeReasoningV1, PoolCredentialV1,
    ProtocolEndpointV1, ResolvedConnectionOptionV1, SourceBindingV1, UpstreamProtocol,
};
use hiroute_integrations::{CpaRegisteredSourceV1, TrustedReleaseCatalog};

#[path = "registered_management_candidate.rs"]
mod registered_management;
pub(super) use registered_management::materialize_registered_management_candidate;

#[path = "candidate_protocol_profiles.rs"]
mod protocol_profile;
use protocol_profile::protocol_profiles;
#[cfg(test)]
use protocol_profile::{cpa_reasoning_profiles, field, parameter_path, reasoning_profiles};

const MAX_CREDENTIAL_REFS: usize = 64;

pub(super) struct CandidateExecutionFactsV1 {
    pub connector_runtime: ConnectorRuntimeKind,
    pub operational_target: GatewayOperationalTargetV1,
    pub native_transport_model: String,
    pub protocol_profiles: Vec<GatewayCandidateProtocolProfileV1>,
    pub credential_refs: Vec<String>,
}

struct ProtocolConnectorFacts {
    provider_id: String,
    endpoint_id: String,
    entitlement_id: String,
    connector_id: String,
    connector_revision: String,
}

#[derive(Clone)]
struct ProtocolFace {
    protocol: UpstreamProtocol,
    request_path: String,
    authentication: GatewayAuthenticationSemanticsV1,
    required_headers: Vec<(String, String)>,
    native_target: Option<GatewayNativeProfileTargetV2>,
    adapter_ref: Option<String>,
    adapter_revision: Option<u64>,
}

/// Joins client-bundled catalog facts, durable pool order, and (only for CPA) the live managed target.
/// Failure is candidate-local: callers exclude an unprovable candidate instead of inventing a
/// target, pool selector, model alias, or epoch.
pub(super) fn materialize_candidate_execution(
    resolved: &ResolvedConnectionOptionV1,
    model: &ModelDefinitionV1,
    capability: &ModelEndpointCapabilityV1,
    protocol_endpoint: &ProtocolEndpointV1,
    reasoning: &ModelNativeReasoningV1,
    pool: Option<&CredentialPoolV1>,
    cpa_targets: Option<&CpaRoutingBatch<'_>>,
) -> Option<CandidateExecutionFactsV1> {
    let credentials = pool
        .map(|pool| pool.credentials.as_slice())
        .unwrap_or_default()
        .iter()
        .filter(|entry| entry.enabled)
        .collect::<Vec<_>>();
    if credentials.len() > MAX_CREDENTIAL_REFS {
        return None;
    }
    let credential_refs = credentials
        .iter()
        .map(|entry| entry.credential.credential_id().to_owned())
        .collect::<Vec<_>>();
    let logical_endpoint = format!(
        "{}{}",
        protocol_endpoint.base_url, protocol_endpoint.request_path
    );

    let (operational_target, native_transport_model, protocol_faces) =
        match resolved.connector.runtime_kind {
            ConnectorRuntimeKind::BuiltinNative => (
                GatewayOperationalTargetV1::RegisteredHttps {
                    uri: logical_endpoint,
                },
                capability.upstream_model_id.clone(),
                vec![ProtocolFace {
                    protocol: capability.upstream_protocol,
                    request_path: protocol_endpoint.request_path.clone(),
                    authentication: native_endpoint_authentication(
                        resolved.connector.authentication,
                        protocol_endpoint,
                    )?,
                    required_headers: protocol_endpoint.required_headers.clone(),
                    native_target: None,
                    adapter_ref: None,
                    adapter_revision: None,
                }],
            ),
            ConnectorRuntimeKind::CpaBridge => {
                if credentials.is_empty() {
                    return None;
                }
                let authority = cpa_targets?;
                let base = prepare_cpa_face(
                    authority,
                    &credentials,
                    &capability.upstream_model_id,
                    capability.upstream_protocol,
                )?;
                let first = base.first()?;
                if first.connector_id() != resolved.connector.connector_id {
                    return None;
                }
                let mut protocol_faces = Vec::new();
                for protocol in [
                    UpstreamProtocol::Responses,
                    UpstreamProtocol::ChatCompletions,
                    UpstreamProtocol::Messages,
                ] {
                    let prepared = if protocol == capability.upstream_protocol {
                        Some(base.clone())
                    } else {
                        prepare_cpa_face(
                            authority,
                            &credentials,
                            &capability.upstream_model_id,
                            protocol,
                        )
                    };
                    let Some(prepared) = prepared else {
                        continue;
                    };
                    let face = prepared.first()?;
                    if !same_cpa_runtime_face(first, face) {
                        return None;
                    }
                    protocol_faces.push(ProtocolFace {
                        protocol,
                        request_path: face.request_path().to_owned(),
                        authentication: GatewayAuthenticationSemanticsV1::Bearer,
                        required_headers: if protocol == UpstreamProtocol::Messages {
                            vec![("anthropic-version".into(), "2023-06-01".into())]
                        } else {
                            Vec::new()
                        },
                        native_target: None,
                        adapter_ref: None,
                        adapter_revision: None,
                    });
                }
                (
                    GatewayOperationalTargetV1::ManagedCpaLoopback {
                        uri: format!("http://{}{}", first.address(), first.request_path()),
                        runtime_epoch: first.runtime_epoch(),
                        target_epoch: first.target_epoch(),
                    },
                    first.native_transport_model().to_owned(),
                    protocol_faces,
                )
            }
        };
    let connector = ProtocolConnectorFacts {
        provider_id: resolved.endpoint_profile.provider_platform_id.clone(),
        endpoint_id: resolved.endpoint_profile.endpoint_profile_id.clone(),
        entitlement_id: resolved.endpoint_profile.entitlement_id.clone(),
        connector_id: resolved.connector.connector_id.clone(),
        connector_revision: resolved.connector.revision.to_string(),
    };
    let protocol_profiles = protocol_profiles(
        &connector,
        model,
        capability,
        reasoning,
        &native_transport_model,
        resolved.connector.runtime_kind,
        &protocol_faces,
    )?;
    Some(CandidateExecutionFactsV1 {
        connector_runtime: resolved.connector.runtime_kind,
        operational_target,
        native_transport_model,
        protocol_profiles,
        credential_refs,
    })
}

fn native_endpoint_authentication(
    connector: AuthenticationKind,
    endpoint: &ProtocolEndpointV1,
) -> Option<GatewayAuthenticationSemanticsV1> {
    let authentication = endpoint.authentication_semantics.clone()?;
    match (connector, &authentication) {
        (AuthenticationKind::None, GatewayAuthenticationSemanticsV1::None)
        | (
            AuthenticationKind::ProviderApiKey,
            GatewayAuthenticationSemanticsV1::Bearer
            | GatewayAuthenticationSemanticsV1::ApiKeyHeader { .. },
        ) => Some(authentication),
        _ => None,
    }
}

fn prepare_cpa_face(
    authority: &CpaRoutingBatch<'_>,
    credentials: &[&hiroute_domain::PoolCredentialV1],
    upstream_model_id: &str,
    protocol: UpstreamProtocol,
) -> Option<Vec<PreparedCpaTarget>> {
    let prepared = credentials
        .iter()
        .map(|entry| {
            authority
                .prepare_target(ExactCpaAttemptRequest {
                    credential_ref: &entry.credential,
                    upstream_model_id,
                    protocol,
                })
                .ok()
        })
        .collect::<Option<Vec<_>>>()?;
    let first = prepared.first()?;
    if first.upstream_model_id() != upstream_model_id
        || first.protocol() != protocol
        || first.credential_ref() != &credentials[0].credential
        || prepared
            .iter()
            .enumerate()
            .skip(1)
            .any(|(index, candidate)| {
                candidate.connector_id() != first.connector_id()
                    || candidate.upstream_model_id() != first.upstream_model_id()
                    || candidate.protocol() != first.protocol()
                    || candidate.address() != first.address()
                    || candidate.request_path() != first.request_path()
                    || candidate.native_transport_model() != first.native_transport_model()
                    || candidate.runtime_epoch() != first.runtime_epoch()
                    || candidate.target_epoch() != first.target_epoch()
                    || candidate.credential_ref() != &credentials[index].credential
            })
    {
        return None;
    }
    Some(prepared)
}

fn same_cpa_runtime_face(base: &PreparedCpaTarget, face: &PreparedCpaTarget) -> bool {
    face.connector_id() == base.connector_id()
        && face.upstream_model_id() == base.upstream_model_id()
        && face.address() == base.address()
        && face.native_transport_model() == base.native_transport_model()
        && face.runtime_epoch() == base.runtime_epoch()
        && face.target_epoch() == base.target_epoch()
}

pub(super) fn materialize_management_candidate(
    fact: &ComputeManagementCompilationFactV2,
    catalog: Option<&TrustedReleaseCatalog>,
) -> Option<CandidateCompilationFactV1> {
    let runtime_fallback = fact.eligibility == ComputeManagementEligibilityV2::RuntimeQualified;
    let registered = match (&fact.provenance, fact.eligibility) {
        (
            hiroute_domain::ComputeManagementProvenanceV2::UserConfigured { .. },
            ComputeManagementEligibilityV2::UserConfirmed
            | ComputeManagementEligibilityV2::RuntimeQualified,
        ) => None,
        (
            hiroute_domain::ComputeManagementProvenanceV2::Registered {
                connection_option_id,
                ..
            },
            ComputeManagementEligibilityV2::RuntimeQualified,
        ) => {
            let catalog = catalog?;
            let resolved = catalog
                .resolve_connection_option(connection_option_id)
                .ok()?;
            if resolved.connector.runtime_kind != ConnectorRuntimeKind::BuiltinNative {
                return None;
            }
            if fact.additional_native_endpoints.iter().any(|saved| {
                !resolved.endpoint_profile.protocol_endpoints.iter().any(|endpoint| {
                    crate::control::runtime::model_connections::registered_endpoint_matches_saved_extra(
                        saved, &resolved, endpoint,
                    )
                })
            }) {
                return None;
            }
            Some(resolved)
        }
        _ => return None,
    };
    let tool = fact.capabilities.tool.value.unwrap_or(false);
    let vision = fact.capabilities.vision.value.unwrap_or(false);
    let streaming = fact.capabilities.streaming.value.unwrap_or(false);
    let context_tokens = fact
        .capabilities
        .context_tokens
        .value
        .unwrap_or(UNKNOWN_TEXT_CONTEXT_TOKENS);
    let max_output_tokens = fact
        .capabilities
        .max_output_tokens
        .value
        .unwrap_or(UNKNOWN_TEXT_OUTPUT_TOKENS)
        .min(context_tokens);
    let identity = CanonicalDigest::of(&(
        if runtime_fallback {
            "hiroute.runtime-fallback-candidate/v1"
        } else {
            "hiroute.source-local-candidate/v2"
        },
        &fact.source_id,
        fact.source_revision,
        &fact.binding_id,
        fact.binding_revision,
    ))
    .ok()?;
    let suffix = &identity.as_str()["sha256:".len()..][..24];
    let identity_kind = if runtime_fallback {
        "runtime-fallback"
    } else {
        "source-local"
    };
    let model_configuration_id = format!("model/{identity_kind}/{suffix}");
    let capability_id = format!("capability/{identity_kind}/{suffix}");
    let offer_ref = format!("offer/{identity_kind}/{suffix}");
    let local_base_url = format!(
        "{}://{}",
        fact.target.scheme,
        target_authority(&fact.target)?
    );
    let local_adapter_ref = format!(
        "adapter/native/{}",
        protocol_label(fact.target.upstream_protocol)
    );
    let (connector, protocol_endpoint, billing_class, connection_option_id, operational_target) =
        if let Some(resolved) = registered.as_ref() {
            let mut endpoints =
                resolved
                    .endpoint_profile
                    .protocol_endpoints
                    .iter()
                    .filter(|endpoint| {
                        endpoint.protocol == fact.target.upstream_protocol
                            && endpoint.adapter_ref == fact.target.protocol_profile_id
                            && endpoint.adapter_revision == fact.target.protocol_profile_revision
                            && endpoint.request_path == fact.target.request_path
                            && endpoint.base_url.strip_prefix("https://")
                                == Some(fact.target.authority.as_str())
                            && fact.target.scheme == "https"
                            && fact.target.port == 443
                    });
            let endpoint = endpoints.next()?.clone();
            if endpoints.next().is_some() {
                return None;
            }
            let authentication =
                native_endpoint_authentication(resolved.connector.authentication, &endpoint)?;
            if fact.authentication != authentication {
                return None;
            }
            let logical_endpoint = format!("{}{}", endpoint.base_url, endpoint.request_path);
            (
                ProtocolConnectorFacts {
                    provider_id: resolved.endpoint_profile.provider_platform_id.clone(),
                    endpoint_id: resolved.endpoint_profile.endpoint_profile_id.clone(),
                    entitlement_id: resolved.endpoint_profile.entitlement_id.clone(),
                    connector_id: resolved.connector.connector_id.clone(),
                    connector_revision: resolved.connector.revision.to_string(),
                },
                endpoint,
                if resolved.option.billing_class == BillingClass::Free {
                    BillingClass::Unknown
                } else {
                    resolved.option.billing_class
                },
                resolved.option.connection_option_id.clone(),
                GatewayOperationalTargetV1::RegisteredHttps {
                    uri: logical_endpoint,
                },
            )
        } else {
            let endpoint = ProtocolEndpointV1 {
                protocol_endpoint_id: format!("endpoint/{identity_kind}/{suffix}"),
                protocol: fact.target.upstream_protocol,
                base_url: local_base_url.clone(),
                request_path: fact.target.request_path.clone(),
                adapter_ref: local_adapter_ref.clone(),
                adapter_revision: fact.target.protocol_profile_revision,
                stable_preference: 1,
                inventory_path: None,
                authentication_semantics: Some(fact.authentication.clone()),
                required_headers: if fact.target.upstream_protocol == UpstreamProtocol::Messages {
                    vec![("anthropic-version".into(), "2023-06-01".into())]
                } else {
                    Vec::new()
                },
            };
            (
                ProtocolConnectorFacts {
                    provider_id: format!("provider/{identity_kind}/{suffix}"),
                    endpoint_id: fact.target.protocol_profile_id.clone(),
                    entitlement_id: format!("entitlement/{identity_kind}/{suffix}"),
                    connector_id: format!("connector/{identity_kind}/{suffix}"),
                    connector_revision: fact.source_revision.to_string(),
                },
                endpoint,
                BillingClass::Paid,
                fact.source_id.clone(),
                GatewayOperationalTargetV1::UserConfiguredNative {
                    uri: format!("{}{}", local_base_url, fact.target.request_path),
                },
            )
        };
    let model = ModelDefinitionV1 {
        model_configuration_id: model_configuration_id.clone(),
        revision: fact.binding_revision,
        display_name: fact.display_name.clone(),
        publisher_id: format!("publisher/{identity_kind}/{suffix}"),
        capabilities: CapabilityFactsV1 {
            tool,
            vision,
            streaming,
            context_tokens,
            max_output_tokens,
        },
    };
    let capability = ModelEndpointCapabilityV1 {
        capability_id: capability_id.clone(),
        revision: fact.binding_revision,
        model_configuration_id: model_configuration_id.clone(),
        connector_id: connector.connector_id.clone(),
        connector_revision: connector.connector_revision.parse().ok()?,
        endpoint_profile_id: connector.endpoint_id.clone(),
        endpoint_profile_revision: fact.target.protocol_profile_revision,
        protocol_endpoint_id: protocol_endpoint.protocol_endpoint_id.clone(),
        upstream_protocol: fact.target.upstream_protocol,
        upstream_model_id: fact.upstream_model_id.clone(),
        required_adapter_ref: protocol_endpoint.adapter_ref.clone(),
        required_adapter_revision: protocol_endpoint.adapter_revision,
        evidence_digest: fact.capability_evidence_digest.clone(),
    };
    let reasoning = ModelNativeReasoningV1 {
        model_configuration_id: model_configuration_id.clone(),
        capability: fact.native_reasoning.clone(),
        native_render_convention: None,
    };
    let mut protocol_faces = vec![ProtocolFace {
        protocol: protocol_endpoint.protocol,
        request_path: protocol_endpoint.request_path.clone(),
        authentication: fact.authentication.clone(),
        required_headers: protocol_endpoint.required_headers.clone(),
        native_target: Some(GatewayNativeProfileTargetV2 {
            operational_target: operational_target.clone(),
            credential_destination_ref: fact.target.credential_destination().ok()?,
        }),
        adapter_ref: Some(fact.target.protocol_profile_id.clone()),
        adapter_revision: Some(fact.target.protocol_profile_revision),
    }];
    for endpoint in &fact.additional_native_endpoints {
        let target = &endpoint.target;
        let base = format!("{}://{}", target.scheme, target_authority(target)?);
        let uri = format!("{}{}", base, target.request_path);
        let operational_target = if registered.is_some() {
            GatewayOperationalTargetV1::RegisteredHttps { uri }
        } else {
            GatewayOperationalTargetV1::UserConfiguredNative { uri }
        };
        protocol_faces.push(ProtocolFace {
            protocol: target.upstream_protocol,
            request_path: target.request_path.clone(),
            authentication: endpoint.authentication.clone(),
            required_headers: endpoint.recheck.as_ref().map_or_else(Vec::new, |value| {
                value.protocol_header_semantics.required_headers.clone()
            }),
            native_target: Some(GatewayNativeProfileTargetV2 {
                operational_target,
                credential_destination_ref: target.credential_destination().ok()?,
            }),
            adapter_ref: Some(target.protocol_profile_id.clone()),
            adapter_revision: Some(target.protocol_profile_revision),
        });
    }
    let protocol_profiles = protocol_profiles(
        &connector,
        &model,
        &capability,
        &reasoning,
        &fact.upstream_model_id,
        ConnectorRuntimeKind::BuiltinNative,
        &protocol_faces,
    )?;
    let credential_refs = match &fact.credential {
        ComputeManagementCredentialCompilationV2::Native { ordered } => ordered
            .iter()
            .map(|selection| match selection {
                hiroute_domain::ComputeCredentialSelectionV2::Credential { credential_ref } => {
                    credential_ref.credential_id().to_owned()
                }
                hiroute_domain::ComputeCredentialSelectionV2::NoCredential => {
                    format!("credential/none/{suffix}")
                }
            })
            .collect::<Vec<_>>(),
        ComputeManagementCredentialCompilationV2::ConnectorOwned { .. } => return None,
    };
    if credential_refs.is_empty() || credential_refs.len() > MAX_CREDENTIAL_REFS {
        return None;
    }
    let credential_pool_id =
        (!matches!(&fact.authentication, GatewayAuthenticationSemanticsV1::None))
            .then(|| format!("pool/source-local/{suffix}"));
    let binding = SourceBindingV1 {
        binding_id: fact.binding_id.clone(),
        revision: fact.binding_revision,
        source_id: fact.source_id.clone(),
        source_revision: fact.source_revision,
        source_identity_digest: fact.source_lineage_digest.clone(),
        model_data_bundle_version: format!("{identity_kind}/v1"),
        capability_slice_version: format!("{identity_kind}/v1"),
        offer_ref,
        offer_evidence_digest: fact.source_lineage_digest.clone(),
        billing_class,
        model_configuration_id,
        upstream_model_id: fact.upstream_model_id.clone(),
        capability_id,
        credential_pool_id,
    };
    let candidate = CandidateCompilationFactV1 {
        authority: if runtime_fallback {
            CandidateFactAuthorityV1::RuntimeFallback
        } else {
            CandidateFactAuthorityV1::SourceLocalUser
        },
        connection_option_id,
        offer_revision: fact.source_revision,
        binding,
        model,
        capability,
        protocol_endpoint,
        connector_runtime: ConnectorRuntimeKind::BuiltinNative,
        operational_target,
        native_transport_model: fact.upstream_model_id.clone(),
        protocol_profiles,
        credential_refs,
        credential_destination_ref: Some(fact.target.credential_destination().ok()?),
        source_state: hiroute_domain::MaterializationState::Ready,
        inventory_model_matched: false,
        reasoning: fact.native_reasoning.clone(),
        rating: None,
        ordering_price: None,
        free_evidence: None,
    };
    candidate.validate().ok()?;
    Some(candidate)
}

/// Re-intersects a saved connector-owned V2 source with the current client-bundled catalog and
/// live CPA current binding. The saved row contributes selection/state only; it cannot invent a
/// destination, protocol, capability, model, or credential selector. CPA credential generations
/// remain internal to the authority and never identify this durable management candidate.
pub(super) fn materialize_connector_management_candidate(
    fact: &ComputeManagementCompilationFactV2,
    registered: &CpaRegisteredSourceV1,
    catalog: &TrustedReleaseCatalog,
    cpa_targets: Option<&CpaRoutingBatch<'_>>,
) -> Option<CandidateCompilationFactV1> {
    if fact.eligibility == ComputeManagementEligibilityV2::RuntimeQualified {
        return materialize_connector_runtime_fallback(fact, registered, catalog, cpa_targets);
    }
    let (
        ComputeManagementEligibilityV2::ConnectorVerified,
        ComputeManagementCredentialCompilationV2::ConnectorOwned {
            connector_id,
            account_ref,
            ..
        },
    ) = (fact.eligibility, &fact.credential)
    else {
        return None;
    };
    let model_configuration_id = fact.catalog_configuration_id.as_deref()?;
    if &registered.source.connector_id != connector_id
        || &registered.source.identity.account_subject_ref != account_ref
        || fact.authentication != GatewayAuthenticationSemanticsV1::Bearer
    {
        return None;
    }
    let prepared = catalog
        .prepare_cpa_compute_projection(
            registered,
            model_configuration_id,
            ComputeProjectionExpectationV1 {
                source_revision: 0,
                source_digest: None,
                binding_revision: 0,
                binding_digest: None,
                inventory_revision: 0,
                inventory_digest: None,
            },
            true,
        )
        .ok()?;
    let desired = prepared.desired;
    let resolved = catalog
        .resolve_connection_option(&desired.source.connection_option_id)
        .ok()?;
    let model = catalog
        .model_data()
        .model(model_configuration_id)
        .cloned()?;
    let capability = catalog
        .model_data()
        .model_endpoint_capabilities
        .iter()
        .find(|value| value.capability_id == desired.binding.capability_id)
        .cloned()?;
    let protocol_endpoint = resolved
        .endpoint_profile
        .protocol_endpoints
        .iter()
        .find(|value| value.protocol_endpoint_id == capability.protocol_endpoint_id)
        .cloned()?;
    let target_authority = target_authority(&fact.target)?;
    if fact.target.scheme != "https"
        || target_authority
            != resolved
                .endpoint_profile
                .protocol_endpoints
                .iter()
                .find(|value| value.protocol_endpoint_id == capability.protocol_endpoint_id)?
                .base_url
                .strip_prefix("https://")?
        || fact.target.request_path != protocol_endpoint.request_path
        || fact.target.upstream_protocol != capability.upstream_protocol
        || fact.target.protocol_profile_id != resolved.endpoint_profile.endpoint_profile_id
        || fact.target.protocol_profile_revision != resolved.endpoint_profile.revision
        || fact.upstream_model_id != capability.upstream_model_id
        || fact.capabilities.tool.value != Some(model.capabilities.tool)
        || fact.capabilities.vision.value != Some(model.capabilities.vision)
        || fact.capabilities.streaming.value != Some(model.capabilities.streaming)
        || fact.capabilities.context_tokens.value != Some(model.capabilities.context_tokens)
        || fact.capabilities.max_output_tokens.value != Some(model.capabilities.max_output_tokens)
    {
        return None;
    }
    let reasoning = catalog
        .native_reasoning()
        .iter()
        .find(|value| value.model_configuration_id == model_configuration_id)
        .cloned()?;
    if fact.native_reasoning != reasoning.capability
        || fact.capabilities.native_reasoning.value.as_ref() != Some(&reasoning.capability)
    {
        return None;
    }
    let pool = catalog
        .transient_cpa_credential_pool(registered, &desired.source, &desired.binding)
        .ok()?;
    let execution = materialize_candidate_execution(
        &resolved,
        &model,
        &capability,
        &protocol_endpoint,
        &reasoning,
        Some(&pool),
        cpa_targets,
    )?;
    let offer = catalog.model_data().offer(&desired.binding.offer_ref)?;
    let mut binding = desired.binding;
    binding.binding_id = fact.binding_id.clone();
    binding.revision = fact.binding_revision;
    binding.source_id = fact.source_id.clone();
    binding.source_revision = fact.source_revision;
    binding.source_identity_digest = fact.source_lineage_digest.clone();
    let candidate = CandidateCompilationFactV1 {
        authority: CandidateFactAuthorityV1::RegisteredCatalog,
        connection_option_id: desired.source.connection_option_id,
        offer_revision: offer.revision,
        binding,
        model,
        capability,
        protocol_endpoint,
        connector_runtime: execution.connector_runtime,
        operational_target: execution.operational_target,
        native_transport_model: execution.native_transport_model,
        protocol_profiles: execution.protocol_profiles,
        credential_refs: execution.credential_refs,
        credential_destination_ref: None,
        source_state: hiroute_domain::MaterializationState::Ready,
        inventory_model_matched: execution.connector_runtime != ConnectorRuntimeKind::BuiltinNative,
        reasoning: reasoning.capability,
        // Configuration-scoped ratings remain in the current rating snapshot and are not
        // collapsed into this legacy model-level compiler field.
        rating: None,
        ordering_price: None,
        free_evidence: None,
    };
    candidate.validate().ok()?;
    Some(candidate)
}

fn materialize_connector_runtime_fallback(
    fact: &ComputeManagementCompilationFactV2,
    registered: &CpaRegisteredSourceV1,
    catalog: &TrustedReleaseCatalog,
    cpa_targets: Option<&CpaRoutingBatch<'_>>,
) -> Option<CandidateCompilationFactV1> {
    let ComputeManagementCredentialCompilationV2::ConnectorOwned {
        connector_id,
        account_ref,
        ..
    } = &fact.credential
    else {
        return None;
    };
    if fact.catalog_configuration_id.is_some()
        || fact.authentication != GatewayAuthenticationSemanticsV1::Bearer
        || &registered.source.connector_id != connector_id
        || &registered.source.identity.account_subject_ref != account_ref
        || !registered.inventory.iter().any(|model| {
            model.disposition == InventoryDisposition::InventoryOnly
                && model.model_configuration_id.is_none()
                && model.upstream_model_id == fact.upstream_model_id
        })
        || !catalog.runtime_fallback_allows_observed_text(&fact.upstream_model_id)
    {
        return None;
    }
    let resolved = catalog
        .resolve_connection_option(&registered.source.connection_option_id)
        .ok()?;
    if resolved.option.origin != hiroute_domain::ConnectionOrigin::AgentSubscription
        || resolved.connector.runtime_kind != ConnectorRuntimeKind::CpaBridge
        || resolved.connector.connector_id != *connector_id
        || resolved.endpoint_profile.endpoint_profile_id
            != registered.source.identity.endpoint_profile_id
    {
        return None;
    }
    let mut endpoints = resolved
        .endpoint_profile
        .protocol_endpoints
        .iter()
        .filter(|endpoint| {
            endpoint.protocol == fact.target.upstream_protocol
                && endpoint.request_path == fact.target.request_path
                && endpoint.base_url.strip_prefix("https://")
                    == Some(fact.target.authority.as_str())
                && fact.target.scheme == "https"
                && fact.target.port == 443
        });
    let protocol_endpoint = endpoints.next()?.clone();
    if endpoints.next().is_some() {
        return None;
    }
    let identity = CanonicalDigest::of(&(
        "hiroute.connector-runtime-fallback/v1",
        &fact.source_id,
        &fact.binding_id,
        &fact.upstream_model_id,
    ))
    .ok()?;
    let suffix = &identity.as_str()["sha256:".len()..][..24];
    let model_configuration_id = format!("model/runtime-fallback/{suffix}");
    let capability_id = format!("capability/runtime-fallback/{suffix}");
    let offer_ref = format!("offer/runtime-fallback/{suffix}");
    let model = ModelDefinitionV1 {
        model_configuration_id: model_configuration_id.clone(),
        revision: fact.binding_revision,
        display_name: fact.display_name.clone(),
        publisher_id: format!("publisher/runtime-fallback/{suffix}"),
        capabilities: CapabilityFactsV1 {
            tool: fact.capabilities.tool.value?,
            vision: fact.capabilities.vision.value?,
            streaming: fact.capabilities.streaming.value?,
            context_tokens: fact.capabilities.context_tokens.value?,
            max_output_tokens: fact.capabilities.max_output_tokens.value?,
        },
    };
    let capability = ModelEndpointCapabilityV1 {
        capability_id: capability_id.clone(),
        revision: fact.binding_revision,
        model_configuration_id: model_configuration_id.clone(),
        connector_id: resolved.connector.connector_id.clone(),
        connector_revision: resolved.connector.revision,
        endpoint_profile_id: resolved.endpoint_profile.endpoint_profile_id.clone(),
        endpoint_profile_revision: resolved.endpoint_profile.revision,
        protocol_endpoint_id: protocol_endpoint.protocol_endpoint_id.clone(),
        upstream_protocol: protocol_endpoint.protocol,
        upstream_model_id: fact.upstream_model_id.clone(),
        required_adapter_ref: protocol_endpoint.adapter_ref.clone(),
        required_adapter_revision: protocol_endpoint.adapter_revision,
        evidence_digest: fact.capability_evidence_digest.clone(),
    };
    let reasoning = ModelNativeReasoningV1 {
        model_configuration_id: model_configuration_id.clone(),
        capability: fact.native_reasoning.clone(),
        native_render_convention: None,
    };
    let pool_id = format!("pool/runtime-fallback/{suffix}");
    let pool = CredentialPoolV1 {
        pool_id: pool_id.clone(),
        binding_id: fact.binding_id.clone(),
        binding_revision: fact.binding_revision,
        binding_digest: fact.capability_evidence_digest.clone(),
        source_id: registered.source.source_id.clone(),
        source_revision: registered.source.revision,
        connection_option_id: registered.source.connection_option_id.clone(),
        source_identity_digest: registered.source.identity_digest.clone(),
        offer_ref: offer_ref.clone(),
        offer_revision: fact.source_revision,
        offer_evidence_digest: fact.capability_evidence_digest.clone(),
        billing_class: resolved.option.billing_class,
        model_configuration_id: model_configuration_id.clone(),
        authentication: AuthenticationKind::ConnectorOwnedOpaque,
        revision: 1,
        credentials: vec![PoolCredentialV1 {
            credential: registered.credential_ref.clone(),
            fingerprint: CanonicalDigest::of(&(
                "hiroute.connector-owned-runtime-fallback/v1",
                &registered.credential_ref,
            ))
            .ok()?,
            ordinal: 0,
            enabled: true,
        }],
    };
    let execution = materialize_candidate_execution(
        &resolved,
        &model,
        &capability,
        &protocol_endpoint,
        &reasoning,
        Some(&pool),
        cpa_targets,
    )?;
    let binding = SourceBindingV1 {
        binding_id: fact.binding_id.clone(),
        revision: fact.binding_revision,
        source_id: fact.source_id.clone(),
        source_revision: fact.source_revision,
        source_identity_digest: fact.source_lineage_digest.clone(),
        model_data_bundle_version: "runtime-fallback/v1".into(),
        capability_slice_version: "runtime-fallback/v1".into(),
        offer_ref,
        offer_evidence_digest: fact.capability_evidence_digest.clone(),
        billing_class: resolved.option.billing_class,
        model_configuration_id,
        upstream_model_id: fact.upstream_model_id.clone(),
        capability_id,
        credential_pool_id: Some(pool_id),
    };
    let candidate = CandidateCompilationFactV1 {
        authority: CandidateFactAuthorityV1::RuntimeFallback,
        connection_option_id: resolved.option.connection_option_id,
        offer_revision: fact.source_revision,
        binding,
        model,
        capability,
        protocol_endpoint,
        connector_runtime: execution.connector_runtime,
        operational_target: execution.operational_target,
        native_transport_model: execution.native_transport_model,
        protocol_profiles: execution.protocol_profiles,
        credential_refs: execution.credential_refs,
        credential_destination_ref: None,
        source_state: hiroute_domain::MaterializationState::Ready,
        inventory_model_matched: execution.connector_runtime != ConnectorRuntimeKind::BuiltinNative,
        reasoning: reasoning.capability,
        rating: None,
        ordering_price: None,
        free_evidence: None,
    };
    candidate.validate().ok()?;
    Some(candidate)
}

fn protocol_label(protocol: UpstreamProtocol) -> &'static str {
    match protocol {
        UpstreamProtocol::Responses => "responses",
        UpstreamProtocol::ChatCompletions => "chat-completions",
        UpstreamProtocol::Messages => "messages",
    }
}

fn target_authority(target: &hiroute_domain::ComputeManagementTargetV2) -> Option<String> {
    let host = if target.authority.contains(':') {
        format!(
            "[{}]",
            target
                .authority
                .trim_start_matches('[')
                .trim_end_matches(']')
        )
    } else {
        target.authority.clone()
    };
    let default_port = match target.scheme.as_str() {
        "https" => 443,
        "http" => 80,
        _ => return None,
    };
    Some(if target.port == default_port {
        host
    } else {
        format!("{host}:{}", target.port)
    })
}

#[cfg(test)]
#[path = "candidate_execution_tests.rs"]
mod tests;
