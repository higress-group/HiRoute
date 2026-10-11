use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use hiroute_cpa_bridge::{
    CpaDownstreamCredentialCapability, CpaDownstreamCredentialPort, ExactCpaCredentialRequest,
};
use hiroute_domain::{
    CanonicalDigest, ConnectorRuntimeKind, GatewayAuthenticationSemanticsV1,
    GatewayOperationalTargetV1, HEADER_SECRET_LEASE_REQUEST_SCHEMA_V1, HeaderSecretLeaseRequestV1,
    NATIVE_CREDENTIAL_LEASE_REQUEST_SCHEMA_V1, NativeCredentialAuthorityV1,
    NativeCredentialCapabilityErrorV1, NativeCredentialLeaseRequestV1,
    SensitiveAuthorizationTargetV1, UpstreamProtocol,
};
use hiroute_gateway::ports::{
    CredentialAuthorizationCapability, CredentialError, CredentialLease, CredentialLeaseRequest,
    CredentialTransportOverride, ExecutionScope, HeaderSecretLeaseRequest,
};
use hiroute_gateway::server::composition::{CredentialResolver, PortError};
use hiroute_gateway::server::core_runtime::profiles::AuthenticationSemantics;
use hiroute_gateway::server::request_plan::IngressProtocol;
use http::{HeaderMap, HeaderName, HeaderValue, header};

mod cpa;

/// Request-scoped credential dispatcher. It owns no credential, target, or publication state.
pub struct GatewayCredentialResolver<N, C> {
    native: Arc<N>,
    cpa: Arc<C>,
    cpa_executor: cpa::CpaExecutor,
}

impl<N, C> GatewayCredentialResolver<N, C> {
    pub fn new(native: Arc<N>, cpa: Arc<C>) -> Self {
        Self {
            native,
            cpa,
            cpa_executor: cpa::CpaExecutor::new(),
        }
    }
}

#[async_trait]
impl<N, C> CredentialResolver for GatewayCredentialResolver<N, C>
where
    N: NativeCredentialAuthorityV1 + 'static,
    C: CpaDownstreamCredentialPort + Send + Sync + 'static,
{
    fn acquire(
        &self,
        _credential_ref: &str,
    ) -> Result<hiroute_gateway::server::composition::CredentialLease, PortError> {
        Err(PortError::Unavailable("exact CredentialResolver required"))
    }

    async fn lease_exact(
        &self,
        request: CredentialLeaseRequest<'_>,
        scope: &ExecutionScope,
    ) -> Result<Option<CredentialLease>, PortError> {
        scope.ensure_active().map_err(|_| PortError::Rejected)?;
        let target = validate_request(&request)?;
        let lease = match request.connector_runtime {
            ConnectorRuntimeKind::BuiltinNative => {
                if request.authentication == &AuthenticationSemantics::None {
                    return Err(PortError::Rejected);
                }
                let native_request = NativeCredentialLeaseRequestV1 {
                    schema_version: NATIVE_CREDENTIAL_LEASE_REQUEST_SCHEMA_V1.into(),
                    stable_binding_id: request.stable_binding_id.into(),
                    credential_id: request.credential_ref.into(),
                    credential_destination_ref: request.credential_destination_ref.into(),
                    excluded_key_ids: request
                        .excluded_key_ids
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    connector_runtime: request.connector_runtime,
                    connector_id: request.connector_id.into(),
                    upstream_protocol: domain_protocol(request.upstream_protocol),
                    upstream_model_id: request.upstream_model_id.into(),
                    native_transport_model: request.native_transport_model.into(),
                    logical_endpoint: request.logical_endpoint.into(),
                    operational_target: target,
                    operational_target_digest: CanonicalDigest::parse(
                        request.operational_target_digest,
                    )
                    .map_err(|_| PortError::Rejected)?,
                    request_path: request.request_path.into(),
                    runtime_epoch: request.runtime_epoch,
                    target_epoch: request.target_epoch,
                    protocol_profile_digest: CanonicalDigest::parse(
                        request.protocol_profile_digest,
                    )
                    .map_err(|_| PortError::Rejected)?,
                    authentication: domain_authentication(request.authentication)?,
                };
                let Some(lease) = self
                    .native
                    .lease_native_credential(&native_request)
                    .map_err(|error| match error.code {
                        hiroute_domain::PortErrorCode::Unavailable => {
                            PortError::Unavailable("NativeCredentialAuthorityV1")
                        }
                        _ => PortError::Rejected,
                    })?
                else {
                    return Ok(None);
                };
                if lease.credential_id() != request.credential_ref
                    || request
                        .excluded_key_ids
                        .iter()
                        .any(|excluded| excluded.as_ref() == lease.key_id())
                {
                    return Err(PortError::Rejected);
                }
                let key_id = lease.key_id().to_owned();
                let generation = lease.generation();
                CredentialLease::from_capability_with_authorization_header(
                    request.credential_ref,
                    key_id,
                    generation,
                    Arc::new(NativeAuthorization(lease)),
                    authorization_header(request.authentication)?,
                )
                .map_err(|_| PortError::Rejected)?
            }
            ConnectorRuntimeKind::CpaBridge => {
                let GatewayOperationalTargetV1::ManagedCpaLoopback {
                    runtime_epoch,
                    target_epoch,
                    ..
                } = target
                else {
                    return Err(PortError::Rejected);
                };
                let address = operational_address(request.operational_target)?;
                let Some(capability) = self
                    .cpa_executor
                    .lease(
                        Arc::clone(&self.cpa),
                        ExactCpaCredentialRequest {
                            credential_id: request.credential_ref,
                            connector_id: request.connector_id,
                            upstream_model_id: request.upstream_model_id,
                            protocol: domain_protocol(request.upstream_protocol),
                            address,
                            request_path: request.request_path,
                            native_transport_model: request.native_transport_model,
                            runtime_epoch,
                            target_epoch,
                            excluded_key_ids: request.excluded_key_ids,
                        },
                        scope,
                    )
                    .await?
                else {
                    return Ok(None);
                };
                validate_cpa_lease(&request, &capability)?;
                let key_id = capability.key_id().to_owned();
                let generation = capability.generation();
                let transport = CredentialTransportOverride::managed_loopback(
                    capability.address(),
                    capability.request_path(),
                )
                .map_err(|_| PortError::Rejected)?;
                CredentialLease::from_capability_with_transport(
                    request.credential_ref,
                    key_id,
                    generation,
                    Arc::new(CpaAuthorization(capability)),
                    transport,
                )
                .map_err(|_| PortError::Rejected)?
            }
        };
        scope.ensure_active().map_err(|_| PortError::Rejected)?;
        Ok(Some(lease))
    }

    async fn lease_header_secret(
        &self,
        request: HeaderSecretLeaseRequest<'_>,
        scope: &ExecutionScope,
    ) -> Result<Option<CredentialLease>, PortError> {
        scope.ensure_active().map_err(|_| PortError::Rejected)?;
        let header_name = HeaderName::from_bytes(request.header_name.as_bytes())
            .map_err(|_| PortError::Rejected)?;
        let native_request = HeaderSecretLeaseRequestV1 {
            schema_version: HEADER_SECRET_LEASE_REQUEST_SCHEMA_V1.into(),
            secret_id: request.secret_ref.into(),
            header_name: request.header_name.into(),
        };
        native_request.validate().map_err(|_| PortError::Rejected)?;
        let Some(lease) = self
            .native
            .lease_header_secret(&native_request)
            .map_err(|error| match error.code {
                hiroute_domain::PortErrorCode::Unavailable => {
                    PortError::Unavailable("NativeCredentialAuthorityV1")
                }
                _ => PortError::Rejected,
            })?
        else {
            return Ok(None);
        };
        if lease.credential_id() != request.secret_ref || lease.generation() == 0 {
            return Err(PortError::Rejected);
        }
        let key_id = lease.key_id().to_owned();
        let generation = lease.generation();
        let lease = CredentialLease::from_capability_with_authorization_header(
            request.secret_ref,
            key_id,
            generation,
            Arc::new(NativeAuthorization(lease)),
            Some(header_name),
        )
        .map_err(|_| PortError::Rejected)?;
        scope.ensure_active().map_err(|_| PortError::Rejected)?;
        Ok(Some(lease))
    }
}

fn validate_request(
    request: &CredentialLeaseRequest<'_>,
) -> Result<GatewayOperationalTargetV1, PortError> {
    let target = match request.connector_runtime {
        ConnectorRuntimeKind::BuiltinNative => {
            let registered = GatewayOperationalTargetV1::RegisteredHttps {
                uri: request.operational_target.into(),
            };
            let configured = GatewayOperationalTargetV1::UserConfiguredNative {
                uri: request.operational_target.into(),
            };
            [registered, configured]
                .into_iter()
                .find(|candidate| {
                    CanonicalDigest::of(candidate)
                        .is_ok_and(|digest| digest.as_str() == request.operational_target_digest)
                })
                .ok_or(PortError::Rejected)?
        }
        ConnectorRuntimeKind::CpaBridge => GatewayOperationalTargetV1::ManagedCpaLoopback {
            uri: request.operational_target.into(),
            runtime_epoch: request.runtime_epoch.ok_or(PortError::Rejected)?,
            target_epoch: request.target_epoch.ok_or(PortError::Rejected)?,
        },
    };
    let destination = ["connection-option/", "compute-target/"]
        .into_iter()
        .find_map(|prefix| request.credential_destination_ref.strip_prefix(prefix))
        .filter(|value| valid_reference(value));
    let authentication_valid = domain_authentication(request.authentication).is_ok()
        && (request.connector_runtime == ConnectorRuntimeKind::BuiltinNative
            || request.authentication == &AuthenticationSemantics::Bearer);
    let target_digest = CanonicalDigest::of(&target).map_err(|_| PortError::Rejected)?;
    let unique_exclusions = request
        .excluded_key_ids
        .iter()
        .map(AsRef::as_ref)
        .collect::<BTreeSet<_>>()
        .len()
        == request.excluded_key_ids.len();
    if !valid_reference(request.stable_binding_id)
        || !valid_reference(request.credential_ref)
        || destination.is_none()
        || !valid_reference(request.connector_id)
        || !hiroute_domain::valid_upstream_model_id(request.upstream_model_id)
        || !hiroute_domain::valid_upstream_model_id(request.native_transport_model)
        || !request.request_path.starts_with('/')
        || request.request_path.len() < 2
        || request.request_path.contains(['?', '#'])
        || !unique_exclusions
        || request
            .excluded_key_ids
            .iter()
            .any(|key_id| !valid_reference(key_id))
        || !authentication_valid
        || CanonicalDigest::parse(request.protocol_profile_digest).is_err()
        || !target.validate_for(request.connector_runtime, request.logical_endpoint)
        || (request.connector_runtime == ConnectorRuntimeKind::BuiltinNative
            && target.request_path() != Some(request.request_path))
        || target_digest.as_str() != request.operational_target_digest
        || (request.connector_runtime == ConnectorRuntimeKind::BuiltinNative
            && (request.upstream_model_id != request.native_transport_model
                || request.runtime_epoch.is_some()
                || request.target_epoch.is_some()))
    {
        return Err(PortError::Rejected);
    }
    Ok(target)
}

fn valid_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains("//")
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
        })
}

fn validate_cpa_lease(
    request: &CredentialLeaseRequest<'_>,
    capability: &CpaDownstreamCredentialCapability,
) -> Result<(), PortError> {
    let reference = capability.credential_ref();
    if reference.credential_id() != request.credential_ref
        || reference.purpose() != "provider-auth"
        || reference.subject() != format!("connector/{}", request.connector_id)
        || reference.allowed_destinations()
            != &BTreeSet::from([request.credential_destination_ref.to_owned()])
        || reference.generation() == 0
        || capability.generation() != reference.generation()
        || capability.request_path() != request.request_path
        || request
            .excluded_key_ids
            .iter()
            .any(|excluded| excluded.as_ref() == capability.key_id())
    {
        return Err(PortError::Rejected);
    }
    Ok(())
}

fn operational_address(uri: &str) -> Result<SocketAddr, PortError> {
    let uri = uri.parse::<http::Uri>().map_err(|_| PortError::Rejected)?;
    uri.authority()
        .ok_or(PortError::Rejected)?
        .as_str()
        .parse()
        .map_err(|_| PortError::Rejected)
}

fn domain_protocol(protocol: IngressProtocol) -> UpstreamProtocol {
    match protocol {
        IngressProtocol::Responses => UpstreamProtocol::Responses,
        IngressProtocol::ChatCompletions => UpstreamProtocol::ChatCompletions,
        IngressProtocol::Messages => UpstreamProtocol::Messages,
    }
}

fn domain_authentication(
    authentication: &AuthenticationSemantics,
) -> Result<GatewayAuthenticationSemanticsV1, PortError> {
    match authentication {
        AuthenticationSemantics::Bearer => Ok(GatewayAuthenticationSemanticsV1::Bearer),
        AuthenticationSemantics::None => Ok(GatewayAuthenticationSemanticsV1::None),
        AuthenticationSemantics::ApiKeyHeader { header } => {
            HeaderName::from_bytes(header.as_bytes()).map_err(|_| PortError::Rejected)?;
            Ok(GatewayAuthenticationSemanticsV1::ApiKeyHeader {
                header: header.clone(),
            })
        }
    }
}

fn authorization_header(
    authentication: &AuthenticationSemantics,
) -> Result<Option<HeaderName>, PortError> {
    match authentication {
        AuthenticationSemantics::Bearer => Ok(Some(header::AUTHORIZATION)),
        AuthenticationSemantics::ApiKeyHeader { header } => {
            HeaderName::from_bytes(header.as_bytes())
                .map(Some)
                .map_err(|_| PortError::Rejected)
        }
        AuthenticationSemantics::None => Ok(None),
    }
}

struct NativeAuthorization(hiroute_domain::NativeCredentialLeaseV1);

impl CredentialAuthorizationCapability for NativeAuthorization {
    fn apply_authorization(&self, headers: &mut HeaderMap) -> Result<(), CredentialError> {
        self.0
            .apply_authorization(&mut HeaderAuthorizationTarget(headers))
            .map_err(|_| CredentialError::InvalidLease)
    }
}

struct HeaderAuthorizationTarget<'a>(&'a mut HeaderMap);

impl SensitiveAuthorizationTargetV1 for HeaderAuthorizationTarget<'_> {
    fn set_sensitive_authorization(
        &mut self,
        value: &[u8],
    ) -> Result<(), NativeCredentialCapabilityErrorV1> {
        let mut value = HeaderValue::from_bytes(value)
            .map_err(|_| NativeCredentialCapabilityErrorV1::Rejected)?;
        value.set_sensitive(true);
        self.0.insert(header::AUTHORIZATION, value);
        Ok(())
    }

    fn set_sensitive_header(
        &mut self,
        name: &str,
        value: &[u8],
    ) -> Result<(), NativeCredentialCapabilityErrorV1> {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| NativeCredentialCapabilityErrorV1::Rejected)?;
        let mut value = HeaderValue::from_bytes(value)
            .map_err(|_| NativeCredentialCapabilityErrorV1::Rejected)?;
        value.set_sensitive(true);
        self.0.insert(name, value);
        Ok(())
    }
}

struct CpaAuthorization(CpaDownstreamCredentialCapability);

impl CredentialAuthorizationCapability for CpaAuthorization {
    fn apply_authorization(&self, headers: &mut HeaderMap) -> Result<(), CredentialError> {
        self.0
            .apply_authorization(headers)
            .map_err(|_| CredentialError::InvalidLease)
    }
}
