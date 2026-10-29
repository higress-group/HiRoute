//! Derive a subscription's logical target from the caller's captured catalog.
use hiroute_application_api::ComputeCandidateTargetV2;
use hiroute_cpa_bridge::CpaAccountKind;
use hiroute_domain::PortResult;
use hiroute_integrations::TrustedReleaseCatalog;

use super::error::invalid;

pub(in crate::control::runtime) fn logical_target(
    catalog: &TrustedReleaseCatalog,
    kind: CpaAccountKind,
) -> PortResult<ComputeCandidateTargetV2> {
    let resolved = catalog
        .resolve_connection_option(kind.connection_option_id())
        .map_err(|_| invalid("subscription.option.resolve"))?;
    if resolved.connector.connector_id != kind.connector_id() {
        return Err(invalid("subscription.option.connector"));
    }
    let endpoint = resolved
        .endpoint_profile
        .protocol_endpoints
        .iter()
        .find(|endpoint| endpoint.protocol == kind.required_protocol())
        .ok_or_else(|| invalid("subscription.option.endpoint"))?;
    let authority = endpoint
        .base_url
        .strip_prefix("https://")
        .filter(|value| !value.is_empty() && !value.contains('/'))
        .ok_or_else(|| invalid("subscription.option.base"))?;
    Ok(ComputeCandidateTargetV2 {
        scheme: "https".into(),
        authority: authority.into(),
        port: 443,
        request_path: endpoint.request_path.clone(),
        upstream_protocol: endpoint.protocol,
        protocol_profile_id: resolved.endpoint_profile.endpoint_profile_id.clone(),
        protocol_profile_revision: resolved.endpoint_profile.revision,
    })
}
