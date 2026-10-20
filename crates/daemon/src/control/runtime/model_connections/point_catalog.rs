//! Exact product/protocol qualification for passive native points, without account claims.
use super::*;
use hiroute_application_api::ComputeCandidateTargetV2;
use hiroute_integrations::{NativeApiConnectionPointV2, NativeApiSource};

pub(in crate::control::runtime) fn point_evidence(
    catalog: &TrustedReleaseCatalog,
    point: &NativeApiConnectionPointV2,
) -> Result<hiroute_domain::CanonicalDigest, ComputeManagementControlError> {
    let provenance = catalog
        .compute_catalog_provenance()
        .map_err(|_| ComputeManagementControlError::Corrupt)?;
    hiroute_domain::CanonicalDigest::of(&(
        "native-point-preparation/v2",
        &point.snapshot_digest,
        provenance,
    ))
    .map_err(|_| ComputeManagementControlError::Corrupt)
}

pub(super) fn native_target(source: &NativeApiSource) -> Option<ComputeCandidateTargetV2> {
    let protocol = source.protocol?;
    let binding = source.point_binding.as_ref()?;
    hiroute_integrations::project_model_connection_target(
        hiroute_integrations::ModelConnectionTargetInputV1 {
            base_url: &binding.endpoint,
            base_kind: if protocol == UpstreamProtocol::Messages {
                ModelConnectionBaseKindV1::NativeMessagesBase
            } else {
                ModelConnectionBaseKindV1::NativeResponsesBase
            },
            protocol,
            request_path_override: None,
            inventory_path_override: None,
            protocol_profile_id: adapter(protocol),
            protocol_profile_revision: 1,
            protocol_header_semantics: &super::custom_endpoint_headers(protocol),
            authentication: &binding.authentication,
        },
    )
    .ok()
    .map(|value| value.candidate_target)
}

pub(super) fn authentication(protocol: UpstreamProtocol) -> GatewayAuthenticationSemanticsV1 {
    if protocol == UpstreamProtocol::Messages {
        GatewayAuthenticationSemanticsV1::ApiKeyHeader {
            header: "x-api-key".into(),
        }
    } else {
        GatewayAuthenticationSemanticsV1::Bearer
    }
}
pub(super) fn adapter(protocol: UpstreamProtocol) -> &'static str {
    match protocol {
        UpstreamProtocol::Responses => "adapter.openai-responses.v1",
        UpstreamProtocol::Messages => "adapter.anthropic-messages.v1",
        UpstreamProtocol::ChatCompletions => "adapter.openai-chat.v1",
    }
}

/// A registered product requires one unambiguous exact native address, protocol, auth and
/// validated connector binding. URL similarity, a provider label and metadata alone do not qualify.
pub(super) fn qualified_point(
    catalog: &TrustedReleaseCatalog,
    source: &NativeApiSource,
) -> Option<(
    hiroute_domain::ResolvedConnectionOptionV1,
    ProtocolEndpointV1,
)> {
    let binding = source.point_binding.as_ref()?;
    let protocol = source.protocol?;
    if !source.supported_auth {
        return None;
    }
    let suffix = match protocol {
        UpstreamProtocol::Responses => "/responses",
        UpstreamProtocol::ChatCompletions => "/chat/completions",
        UpstreamProtocol::Messages => "/v1/messages",
    };
    let mut matches = Vec::new();
    for option in &catalog.registry().connection_options {
        let Ok(resolved) = catalog.resolve_connection_option(&option.connection_option_id) else {
            continue;
        };
        if resolved.option.origin == hiroute_domain::ConnectionOrigin::AgentSubscription
            || resolved.connector.runtime_kind
                != hiroute_domain::ConnectorRuntimeKind::BuiltinNative
            || resolved.connector.authentication
                != hiroute_domain::AuthenticationKind::ProviderApiKey
        {
            continue;
        }
        for endpoint in &resolved.endpoint_profile.protocol_endpoints {
            if endpoint.protocol != protocol
                || endpoint.adapter_ref != adapter(protocol)
                || endpoint.adapter_revision != 1
                || endpoint.authentication_semantics.as_ref() != Some(&binding.authentication)
            {
                continue;
            }
            let destination = format!("{}{}", endpoint.base_url, endpoint.request_path);
            if destination.strip_suffix(suffix) == Some(binding.endpoint.as_str())
                && catalog_models(catalog, &resolved, endpoint)
                    .ok()
                    .is_some_and(|models| !models.is_empty())
            {
                matches.push((resolved.clone(), endpoint.clone()));
            }
        }
    }
    if matches.len() == 1 {
        matches.pop()
    } else {
        None
    }
}

pub(super) fn catalog_models(
    catalog: &TrustedReleaseCatalog,
    resolved: &hiroute_domain::ResolvedConnectionOptionV1,
    endpoint: &ProtocolEndpointV1,
) -> Result<Vec<NativeModelDeclarationV1>, ComputeManagementControlError> {
    catalog
        .model_data()
        .model_endpoint_capabilities
        .iter()
        .filter(|cap| {
            cap.endpoint_profile_id == resolved.endpoint_profile.endpoint_profile_id
                && cap.endpoint_profile_revision == resolved.endpoint_profile.revision
                && cap.connector_id == resolved.connector.connector_id
                && cap.connector_revision == resolved.connector.revision
                && cap.protocol_endpoint_id == endpoint.protocol_endpoint_id
                && cap.upstream_protocol == endpoint.protocol
                && cap.required_adapter_ref == endpoint.adapter_ref
                && cap.required_adapter_revision == endpoint.adapter_revision
                && cap.evidence_digest != hiroute_domain::CanonicalDigest::of_bytes(&[])
        })
        .map(|cap| {
            let model = catalog
                .model_data()
                .model(&cap.model_configuration_id)
                .ok_or(ComputeManagementControlError::Corrupt)?;
            let reasoning = catalog
                .native_reasoning()
                .iter()
                .find(|r| r.model_configuration_id == cap.model_configuration_id)
                .ok_or(ComputeManagementControlError::Corrupt)?;
            Ok(NativeModelDeclarationV1 {
                upstream_model_id: cap.upstream_model_id.clone(),
                display_name: model.display_name.clone(),
                catalog_configuration_id: Some(model.model_configuration_id.clone()),
                membership: ComputeModelMembershipV2::Catalog,
                capabilities: super::registered::capability_declaration(
                    catalog, cap, model, reasoning,
                ),
            })
        })
        .collect()
}
fn registered<T>(value: T) -> NativeCandidateFactValueV1<T> {
    NativeCandidateFactValueV1 {
        value: Some(value),
        basis: NativeCandidateFactBasisV1::RegisteredCatalog,
    }
}

pub(super) fn expand_models(
    catalog: &TrustedReleaseCatalog,
    sources: &[NativeApiSource],
    mut models: Vec<NativeModelDeclarationV1>,
) -> Result<Vec<NativeModelDeclarationV1>, ComputeManagementControlError> {
    let Some((resolved, endpoint)) = sources
        .first()
        .and_then(|source| qualified_point(catalog, source))
    else {
        return Ok(models);
    };
    for registered in catalog_models(catalog, &resolved, &endpoint)? {
        if let Some(observed) = models
            .iter_mut()
            .find(|m| m.upstream_model_id == registered.upstream_model_id)
        {
            observed.catalog_configuration_id = registered.catalog_configuration_id;
            fill(
                &mut observed.capabilities.tool,
                registered.capabilities.tool,
            );
            fill(
                &mut observed.capabilities.vision,
                registered.capabilities.vision,
            );
            fill(
                &mut observed.capabilities.streaming,
                registered.capabilities.streaming,
            );
            fill(
                &mut observed.capabilities.context_tokens,
                registered.capabilities.context_tokens,
            );
            fill(
                &mut observed.capabilities.max_output_tokens,
                registered.capabilities.max_output_tokens,
            );
            fill(
                &mut observed.capabilities.native_reasoning,
                registered.capabilities.native_reasoning,
            );
        } else {
            models.push(registered);
        }
    }
    models.sort_by(|a, b| a.upstream_model_id.cmp(&b.upstream_model_id));
    Ok(models)
}
fn fill<T>(
    observed: &mut NativeCandidateFactValueV1<T>,
    registered: NativeCandidateFactValueV1<T>,
) {
    if observed.value.is_none() && observed.basis == NativeCandidateFactBasisV1::Unknown {
        *observed = registered;
    }
}
