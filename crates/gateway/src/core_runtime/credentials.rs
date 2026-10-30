//! Request admission binds one current managed Native list; attempt leases remain exact.

use hiroute_gateway_core::core::execution_plan::{CredentialRef, ResolvedTargetBindingId};

use crate::ports::CredentialLeaseRequest;
use crate::server::composition::{PortError, ProductionPorts};
use crate::server::publication::PreparedCandidateExecution;
use crate::server::request_plan::AuthorizedRequestPlan;

pub(super) fn bind_current(
    ports: &ProductionPorts,
    authorized: &mut AuthorizedRequestPlan,
    binding: ResolvedTargetBindingId,
    prepared: &PreparedCandidateExecution,
) -> Result<bool, PortError> {
    let execution = prepared.execution();
    if execution.connector_runtime != hiroute_domain::ConnectorRuntimeKind::BuiltinNative {
        return Ok(true);
    }
    let profile = &execution.protocol_profile;
    let authentication = profile
        .connector
        .authentication
        .exact()
        .ok_or(PortError::Rejected)?;
    if authentication == &super::profiles::AuthenticationSemantics::None {
        return Ok(true);
    }
    let attempt = authorized
        .core_binding()
        .resolve_attempt(binding)
        .map_err(|_| PortError::Rejected)?;
    let original = attempt
        .plan()
        .credential_refs
        .first()
        .ok_or(PortError::Rejected)?;
    let target = prepared.lease_target();
    let digest = prepared.lease_target_digest();
    let destination = profile
        .native_target
        .as_ref()
        .map_or(execution.credential_destination_ref.as_str(), |target| {
            target.credential_destination_ref.as_str()
        });
    let current = ports
        .credentials
        .current_credential_refs(CredentialLeaseRequest {
            execution_binding_id: &execution.stable_target_key,
            stable_binding_id: execution.credential_binding_id(),
            credential_ref: original.as_str(),
            credential_destination_ref: destination,
            excluded_key_ids: &[],
            connector_runtime: execution.connector_runtime,
            connector_id: &profile.connector.connector_id,
            upstream_protocol: profile.capability.upstream_protocol,
            upstream_model_id: &execution.upstream_model_id,
            native_transport_model: &execution.native_transport_model,
            logical_endpoint: target.uri(),
            operational_target: target.uri(),
            operational_target_digest: digest.as_str(),
            runtime_epoch: target.runtime_epoch(),
            target_epoch: target.target_epoch(),
            protocol_profile_digest: &execution.profile_digest,
            request_path: &profile.connector.request_path,
            authentication,
        })?;
    let Some(current) = current else {
        return Ok(true);
    };
    if current.is_empty() {
        return Ok(false);
    }
    let credentials = current
        .into_iter()
        .map(CredentialRef::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| PortError::Rejected)?;
    authorized
        .core_binding_mut()
        .bind_candidate_credentials(binding, credentials.into())
        .map_err(|_| PortError::Rejected)?;
    Ok(true)
}
