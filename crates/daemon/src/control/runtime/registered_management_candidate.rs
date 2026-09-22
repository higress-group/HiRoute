use super::*;
use hiroute_application::compiler::{FreeCandidateEvidenceV1, OrderingPriceFactV1};
use hiroute_domain::{
    ComputeCredentialSelectionV2, ComputeManagementFactBasisV2, ComputeManagementProvenanceV2,
    ConnectionOrigin, MaterializationState,
};
use hiroute_integrations::{
    NativeCandidateFactBasisV1, NativeCandidateFactValueV1, NativeModelCapabilityDeclarationV1,
};

pub(in crate::control::runtime) fn materialize_registered_management_candidate(
    fact: &ComputeManagementCompilationFactV2,
    catalog: &TrustedReleaseCatalog,
) -> Option<CandidateCompilationFactV1> {
    let ComputeManagementProvenanceV2::Registered {
        connection_option_id,
        ..
    } = &fact.provenance
    else {
        return None;
    };
    if fact.eligibility != ComputeManagementEligibilityV2::CatalogMatched {
        return None;
    }
    let resolved = catalog
        .resolve_connection_option(connection_option_id)
        .ok()?;
    if resolved.connector.runtime_kind != ConnectorRuntimeKind::BuiltinNative
        || resolved.option.origin == ConnectionOrigin::AgentSubscription
    {
        return None;
    }
    let Some(model_id) = fact.catalog_configuration_id.as_deref() else {
        eprintln!("registered management routing candidate has no catalog model id");
        return None;
    };
    let data = catalog.model_data();
    let Some(model) = data.model(model_id).cloned() else {
        eprintln!("registered management routing candidate model is absent from current catalog");
        return None;
    };
    let mut capabilities = data
        .model_endpoint_capabilities
        .iter()
        .filter(|capability| {
            capability.model_configuration_id == model_id
                && capability.upstream_model_id == fact.upstream_model_id
                && capability.connector_id == resolved.connector.connector_id
                && capability.connector_revision == resolved.connector.revision
                && capability.endpoint_profile_id == resolved.endpoint_profile.endpoint_profile_id
                && capability.endpoint_profile_revision == resolved.endpoint_profile.revision
                && capability.upstream_protocol == fact.target.upstream_protocol
                && capability.required_adapter_ref == fact.target.protocol_profile_id
                && capability.required_adapter_revision == fact.target.protocol_profile_revision
        });
    let Some(capability) = capabilities.next().cloned() else {
        eprintln!("registered management routing candidate has no exact current capability");
        return None;
    };
    if capabilities.next().is_some() {
        return None;
    }
    let endpoint = resolved
        .endpoint_profile
        .protocol_endpoints
        .iter()
        .find(|endpoint| {
            endpoint.protocol_endpoint_id == capability.protocol_endpoint_id
                && endpoint.protocol == capability.upstream_protocol
                && endpoint.adapter_ref == capability.required_adapter_ref
                && endpoint.adapter_revision == capability.required_adapter_revision
        })
        .cloned();
    let Some(endpoint) = endpoint else {
        eprintln!("registered management routing candidate has no exact current endpoint");
        return None;
    };
    let authentication =
        native_endpoint_authentication(resolved.connector.authentication, &endpoint)?;
    if fact.authentication != authentication {
        return None;
    }
    if fact.target.scheme != "https"
        || fact.target.port != 443
        || endpoint.base_url.strip_prefix("https://") != Some(fact.target.authority.as_str())
        || endpoint.request_path != fact.target.request_path
        || fact.target.validate().is_err()
    {
        return None;
    }
    let reasoning = catalog
        .native_reasoning()
        .iter()
        .find(|reasoning| reasoning.model_configuration_id == model_id)?
        .clone();
    let declared_capabilities = NativeModelCapabilityDeclarationV1 {
        tool: catalog_fact(model.capabilities.tool),
        vision: catalog_fact(model.capabilities.vision),
        streaming: catalog_fact(model.capabilities.streaming),
        context_tokens: catalog_fact(model.capabilities.context_tokens),
        max_output_tokens: catalog_fact(model.capabilities.max_output_tokens),
        native_reasoning: catalog_fact(reasoning.capability.clone()),
    };
    let expected_capability_digest = CanonicalDigest::of(&(
        "declared-model-capabilities/v1",
        &fact.upstream_model_id,
        &declared_capabilities,
    ))
    .ok()?;
    if fact.capabilities.tool.value != Some(model.capabilities.tool)
        || fact.capabilities.vision.value != Some(model.capabilities.vision)
        || fact.capabilities.streaming.value != Some(model.capabilities.streaming)
        || fact.capabilities.context_tokens.value != Some(model.capabilities.context_tokens)
        || fact.capabilities.max_output_tokens.value != Some(model.capabilities.max_output_tokens)
        || fact.capabilities.native_reasoning.value.as_ref() != Some(&reasoning.capability)
        || fact.native_reasoning != reasoning.capability
        || [
            fact.capabilities.tool.basis,
            fact.capabilities.vision.basis,
            fact.capabilities.streaming.basis,
            fact.capabilities.context_tokens.basis,
            fact.capabilities.max_output_tokens.basis,
            fact.capabilities.native_reasoning.basis,
        ]
        .iter()
        .any(|basis| *basis != ComputeManagementFactBasisV2::RegisteredCatalog)
        || expected_capability_digest != fact.capability_evidence_digest
    {
        eprintln!("registered management routing candidate facts differ from current catalog");
        return None;
    }
    let mut offers = data.offers.iter().filter(|offer| {
        offer.endpoint_profile_id == resolved.endpoint_profile.endpoint_profile_id
            && offer.endpoint_profile_revision == resolved.endpoint_profile.revision
            && offer.service_offering_id == resolved.endpoint_profile.service_offering_id
            && offer.entitlement_id == resolved.endpoint_profile.entitlement_id
            && offer.usage_scope == resolved.endpoint_profile.usage_scope
            && offer.region_id == resolved.endpoint_profile.region_id
            && offer
                .model_configuration_ids
                .iter()
                .any(|id| id == model_id)
            && offer.billing_class == resolved.option.billing_class
    });
    let Some(offer) = offers.next() else {
        eprintln!("registered management routing candidate has no exact current offer");
        return None;
    };
    if offers.next().is_some() {
        return None;
    }
    let Ok(destination) = fact.target.credential_destination() else {
        eprintln!("registered management routing candidate has an invalid credential destination");
        return None;
    };
    let ComputeManagementCredentialCompilationV2::Native { ordered } = &fact.credential else {
        return None;
    };
    if ordered.is_empty() || ordered.len() > MAX_CREDENTIAL_REFS {
        return None;
    }
    let identity = CanonicalDigest::of(&(&fact.source_id, &fact.source_lineage_digest)).ok()?;
    let suffix = &identity.as_str()["sha256:".len()..][..24];
    let mut credential_refs = Vec::new();
    for selection in ordered {
        selection.validate_for(&authentication).ok()?;
        credential_refs.push(match selection {
            ComputeCredentialSelectionV2::Credential { credential_ref } => {
                if credential_ref.owner_scope() != format!("source/{}", fact.source_id)
                    || credential_ref.subject() != "hirouted"
                    || credential_ref.purpose() != "provider-auth"
                    || credential_ref.generation() == 0
                    || credential_ref.allowed_destinations()
                        != &std::collections::BTreeSet::from([destination.clone()])
                {
                    return None;
                }
                credential_ref.credential_id().to_owned()
            }
            ComputeCredentialSelectionV2::NoCredential => format!("credential/none/{suffix}"),
        });
    }
    let execution = materialize_candidate_execution(
        &resolved,
        &model,
        &capability,
        &endpoint,
        &reasoning,
        None,
        None,
    );
    let Some(mut execution) = execution else {
        eprintln!("registered management routing candidate protocol facts are not materializable");
        return None;
    };
    execution.protocol_profiles = registered_native_protocol_profiles(
        catalog,
        &resolved,
        &model,
        &capability,
        &endpoint,
        &reasoning,
        &execution.protocol_profiles,
    )?;
    let ordering_price =
        if let Some(price) = data.price_rates.iter().find(|price| {
            price.offer_ref == offer.offer_id && price.model_configuration_id == model_id
        }) {
            Some(OrderingPriceFactV1 {
                price_rate_id: price.price_rate_id.clone(),
                price_rate_revision: price.revision,
                offer_ref: price.offer_ref.clone(),
                model_configuration_id: price.model_configuration_id.clone(),
                currency: price.currency.clone(),
                input_micros_per_million: price.input_micros_per_million,
                output_micros_per_million: price.output_micros_per_million,
                frozen_digest: CanonicalDigest::of(price).ok()?,
            })
        } else {
            None
        };
    let free_evidence = if offer.billing_class == BillingClass::Free {
        let free = data.free_offers.iter().find(|free| {
            free.offer_ref == offer.offer_id
                && free.model_configuration_ids.iter().any(|id| id == model_id)
        })?;
        Some(FreeCandidateEvidenceV1 {
            free_offer_id: free.free_offer_id.clone(),
            free_offer_revision: free.revision,
            offer_ref: free.offer_ref.clone(),
            access: free.access,
            evidence_digest: free
                .direct_verification_evidence
                .as_ref()
                .map(|value| CanonicalDigest::of_bytes(value.as_bytes()))
                .unwrap_or_else(|| CanonicalDigest::of_bytes(b"api-key-required")),
        })
    } else {
        None
    };
    let candidate = CandidateCompilationFactV1 {
        authority: CandidateFactAuthorityV1::RegisteredCatalog,
        connection_option_id: connection_option_id.clone(),
        offer_revision: offer.revision,
        binding: SourceBindingV1 {
            binding_id: fact.binding_id.clone(),
            revision: fact.binding_revision,
            source_id: fact.source_id.clone(),
            source_revision: fact.source_revision,
            source_identity_digest: fact.source_lineage_digest.clone(),
            model_data_bundle_version: data.bundle_version.clone(),
            capability_slice_version: data.capability_slice_version.clone(),
            offer_ref: offer.offer_id.clone(),
            offer_evidence_digest: offer.evidence_digest.clone(),
            billing_class: offer.billing_class,
            model_configuration_id: model_id.into(),
            upstream_model_id: fact.upstream_model_id.clone(),
            capability_id: capability.capability_id.clone(),
            credential_pool_id: (authentication != GatewayAuthenticationSemanticsV1::None)
                .then(|| format!("pool/compute-management/{suffix}")),
        },
        model,
        capability,
        protocol_endpoint: endpoint,
        connector_runtime: execution.connector_runtime,
        operational_target: execution.operational_target,
        native_transport_model: execution.native_transport_model,
        protocol_profiles: execution.protocol_profiles,
        credential_refs,
        credential_destination_ref: Some(destination),
        source_state: MaterializationState::Ready,
        inventory_model_matched: true,
        reasoning: reasoning.capability,
        // Configuration-scoped ratings remain in the current rating snapshot and are not
        // collapsed into this legacy model-level compiler field.
        rating: None,
        ordering_price,
        free_evidence,
    };
    if let Err(error) = candidate.validate() {
        eprintln!("registered management routing candidate is invalid: {error}; {candidate:#?}");
        return None;
    }
    Some(candidate)
}

/// Resolve each same-protocol face from the current catalog, never from the saved target alone.
/// The saved target remains the credential and source identity; an alternate face must belong to
/// the same registered option, model and exact HTTPS authority, with matching authentication.
fn registered_native_protocol_profiles(
    catalog: &TrustedReleaseCatalog,
    resolved: &ResolvedConnectionOptionV1,
    model: &ModelDefinitionV1,
    primary_capability: &ModelEndpointCapabilityV1,
    primary_endpoint: &ProtocolEndpointV1,
    reasoning: &ModelNativeReasoningV1,
    primary_profiles: &[GatewayCandidateProtocolProfileV1],
) -> Option<Vec<GatewayCandidateProtocolProfileV1>> {
    let connector = ProtocolConnectorFacts {
        provider_id: resolved.endpoint_profile.provider_platform_id.clone(),
        endpoint_id: resolved.endpoint_profile.endpoint_profile_id.clone(),
        entitlement_id: resolved.endpoint_profile.entitlement_id.clone(),
        connector_id: resolved.connector.connector_id.clone(),
        connector_revision: resolved.connector.revision.to_string(),
    };
    let mut selected = Vec::new();
    for ingress in [
        UpstreamProtocol::Responses,
        UpstreamProtocol::ChatCompletions,
        UpstreamProtocol::Messages,
    ] {
        let mut matches = catalog
            .model_data()
            .model_endpoint_capabilities
            .iter()
            .filter(|face| {
                face.model_configuration_id == model.model_configuration_id
                    && face.upstream_model_id == primary_capability.upstream_model_id
                    && face.connector_id == resolved.connector.connector_id
                    && face.connector_revision == resolved.connector.revision
                    && face.endpoint_profile_id == resolved.endpoint_profile.endpoint_profile_id
                    && face.endpoint_profile_revision == resolved.endpoint_profile.revision
                    && face.upstream_protocol == ingress
            })
            .filter_map(|face| {
                resolved
                    .endpoint_profile
                    .protocol_endpoints
                    .iter()
                    .find(|endpoint| endpoint.protocol_endpoint_id == face.protocol_endpoint_id)
                    .filter(|endpoint| {
                        endpoint.protocol == face.upstream_protocol
                            && endpoint.adapter_ref == face.required_adapter_ref
                            && endpoint.adapter_revision == face.required_adapter_revision
                            && endpoint.base_url == primary_endpoint.base_url
                            && native_endpoint_authentication(
                                resolved.connector.authentication,
                                endpoint,
                            ) == primary_endpoint.authentication_semantics
                    })
                    .map(|endpoint| (face, endpoint))
            });
        let exact = matches.next();
        if matches.next().is_some() {
            return None;
        }
        if let Some((face, endpoint)) = exact {
            let faces = [ProtocolFace {
                protocol: face.upstream_protocol,
                request_path: endpoint.request_path.clone(),
                authentication: endpoint.authentication_semantics.clone()?,
                required_headers: endpoint.required_headers.clone(),
            }];
            if let Some(profile) = protocol_profiles(
                &connector,
                model,
                face,
                reasoning,
                &face.upstream_model_id,
                ConnectorRuntimeKind::BuiltinNative,
                &faces,
            )
            .and_then(|profiles| {
                profiles
                    .into_iter()
                    .find(|profile| profile.ingress_protocol == ingress)
            }) {
                selected.push(profile);
            }
        } else if let Some(profile) = primary_profiles
            .iter()
            .find(|profile| profile.ingress_protocol == ingress)
        {
            selected.push(profile.clone());
        }
    }
    (!selected.is_empty()).then_some(selected)
}

fn catalog_fact<T>(value: T) -> NativeCandidateFactValueV1<T> {
    NativeCandidateFactValueV1 {
        value: Some(value),
        basis: NativeCandidateFactBasisV1::RegisteredCatalog,
    }
}
