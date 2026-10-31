//! Match API metadata only to the installed official native endpoint, never a compatible proxy.
use super::*;

pub(super) fn api_endpoint(
    catalog: &TrustedReleaseCatalog,
    provenance: &ComputeManagementProvenanceV2,
    target: &ComputeManagementTargetV2,
    authentication: &GatewayAuthenticationSemanticsV1,
) -> Option<ProtocolEndpointV1> {
    if target.scheme != "https" || target.port != 443 {
        return None;
    }
    let ComputeManagementProvenanceV2::Registered {
        connection_option_id,
        ..
    } = provenance
    else {
        return None;
    };
    let resolved = catalog
        .resolve_connection_option(connection_option_id)
        .ok()?;
    if resolved.connector.runtime_kind != ConnectorRuntimeKind::BuiltinNative
        || resolved.connector.authentication != AuthenticationKind::ProviderApiKey
        || resolved.option.origin == ConnectionOrigin::AgentSubscription
    {
        return None;
    }
    let mut endpoints = resolved
        .endpoint_profile
        .protocol_endpoints
        .iter()
        .filter(|endpoint| {
            endpoint.protocol == target.upstream_protocol
                && endpoint.adapter_ref == target.protocol_profile_id
                && endpoint.adapter_revision == target.protocol_profile_revision
                && endpoint.base_url.strip_prefix("https://") == Some(target.authority.as_str())
                && endpoint.request_path == target.request_path
                && endpoint.authentication_semantics.as_ref() == Some(authentication)
        });
    let endpoint = endpoints.next()?;
    if endpoints.next().is_some() {
        return None;
    }
    Some(endpoint.clone())
}
