use std::net::SocketAddr;
use std::sync::Arc;

use hiroute_domain::{CredentialRefV1, UpstreamProtocol};
use http::{HeaderMap, HeaderValue, header};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::config::SecretText;
use crate::runtime::ManagedCpaRuntime;

pub struct ExactCpaAttemptRequest<'a> {
    pub credential_ref: &'a CredentialRefV1,
    pub upstream_model_id: &'a str,
    pub protocol: UpstreamProtocol,
}

/// Exact request-owned facts required to lease the private downstream capability.
pub struct ExactCpaCredentialRequest<'a> {
    /// Opaque Product-owned credential identity. The CPA authority resolves the
    /// current generation internally after validating the sealed target epochs.
    pub credential_id: &'a str,
    pub connector_id: &'a str,
    pub upstream_model_id: &'a str,
    pub protocol: UpstreamProtocol,
    pub address: SocketAddr,
    pub request_path: &'a str,
    pub native_transport_model: &'a str,
    pub runtime_epoch: u64,
    pub target_epoch: u64,
    pub excluded_key_ids: &'a [Arc<str>],
}

/// Request-scoped private authority. It remains separate from publication materialization.
pub trait CpaDownstreamCredentialPort {
    fn lease_downstream_capability(
        &self,
        request: ExactCpaCredentialRequest<'_>,
    ) -> Result<Option<CpaDownstreamCredentialCapability>, CpaAttemptError>;

    fn lease_downstream_capability_scoped(
        &self,
        request: ExactCpaCredentialRequest<'_>,
        context: &crate::CpaRequestContext,
    ) -> Result<Option<CpaDownstreamCredentialCapability>, CpaAttemptError> {
        context.run(|| {
            context
                .ensure_active()
                .map_err(|_| CpaAttemptError::Unavailable)?;
            let result = self.lease_downstream_capability(request);
            context
                .ensure_active()
                .map_err(|_| CpaAttemptError::Unavailable)?;
            result
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedCpaTarget {
    connector_id: String,
    upstream_model_id: String,
    address: SocketAddr,
    request_path: String,
    native_transport_model: String,
    credential_ref: CredentialRefV1,
    protocol: UpstreamProtocol,
    runtime_epoch: u64,
    target_epoch: u64,
}

impl PreparedCpaTarget {
    pub fn connector_id(&self) -> &str {
        &self.connector_id
    }

    pub fn upstream_model_id(&self) -> &str {
        &self.upstream_model_id
    }

    pub const fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn request_path(&self) -> &str {
        &self.request_path
    }

    pub fn native_transport_model(&self) -> &str {
        &self.native_transport_model
    }

    pub fn credential_ref(&self) -> &CredentialRefV1 {
        &self.credential_ref
    }

    pub const fn protocol(&self) -> UpstreamProtocol {
        self.protocol
    }

    pub const fn runtime_epoch(&self) -> u64 {
        self.runtime_epoch
    }

    pub const fn target_epoch(&self) -> u64 {
        self.target_epoch
    }
}

/// Opaque downstream bridge authorization. Deliberately not `Debug` or serializable.
pub struct CpaDownstreamCredentialCapability {
    credential_ref: CredentialRefV1,
    key_id: Arc<str>,
    authorization: Arc<SecretText>,
    address: SocketAddr,
    request_path: Arc<str>,
    epochs: Arc<crate::runtime::RuntimeEpochState>,
    runtime_epoch: u64,
    target_epoch: u64,
}

impl CpaDownstreamCredentialCapability {
    pub fn credential_ref(&self) -> &CredentialRefV1 {
        &self.credential_ref
    }

    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    pub const fn generation(&self) -> u64 {
        self.credential_ref.generation()
    }

    /// Current authority-owned loopback target for this exact lease. A durable
    /// publication may contain the target from an earlier daemon lifetime.
    pub const fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn request_path(&self) -> &str {
        &self.request_path
    }

    /// Applies only the random downstream bridge capability after an epoch recheck.
    pub fn apply_authorization(&self, headers: &mut HeaderMap) -> Result<(), CpaAttemptError> {
        if !self.epochs.matches(self.runtime_epoch, self.target_epoch) {
            return Err(CpaAttemptError::RevokedCredential);
        }
        let text = Zeroizing::new(format!("Bearer {}", self.authorization.expose()));
        let mut value = HeaderValue::from_str(text.as_str())
            .map_err(|_| CpaAttemptError::InvalidAuthorization)?;
        value.set_sensitive(true);
        headers.insert(header::AUTHORIZATION, value);
        Ok(())
    }
}

pub(crate) fn prepare_from_accounts(
    runtime: &ManagedCpaRuntime,
    accounts: &[crate::accounts::AccountSnapshotRecord],
    address: SocketAddr,
    (runtime_epoch, target_epoch): (u64, u64),
    request: ExactCpaAttemptRequest<'_>,
) -> Result<PreparedCpaTarget, CpaAttemptError> {
    let exact = resolve_exact_account(
        runtime,
        accounts,
        request.upstream_model_id,
        request.protocol,
        |credential| same_stable_credential_binding(credential, request.credential_ref),
    )?;
    Ok(PreparedCpaTarget {
        connector_id: exact.connector_id.to_owned(),
        upstream_model_id: request.upstream_model_id.to_owned(),
        address,
        request_path: exact.request_path.to_owned(),
        native_transport_model: exact.native_transport_model,
        credential_ref: exact.credential_ref,
        protocol: request.protocol,
        runtime_epoch,
        target_epoch,
    })
}

impl CpaDownstreamCredentialPort for ManagedCpaRuntime {
    fn lease_downstream_capability(
        &self,
        request: ExactCpaCredentialRequest<'_>,
    ) -> Result<Option<CpaDownstreamCredentialCapability>, CpaAttemptError> {
        let _scope = self.operation_context().enter();
        // Explicit shutdown revokes the old request before any restart logic.
        // A live managed runtime resolves the exact current target from the
        // credential/model identity instead of trusting durable address/epochs.
        let observed = {
            let accounts = self
                .observed_accounts()
                .ok_or(CpaAttemptError::RevokedCredential)?;
            resolve_exact_account(
                self,
                &accounts,
                request.upstream_model_id,
                request.protocol,
                |credential| credential.credential_id() == request.credential_id,
            )?
        };
        // Rejected requests must not restart a stopped/retiring refresh writer or
        // touch its credentials. The cached exact binding is sufficient to deny
        // admission; fresh authority validation below is still required to grant it.
        if !self.account_execution_is_admitted(&observed.account) {
            return Err(CpaAttemptError::RevokedCredential);
        }
        if request.connector_id != observed.connector_id
            || request.request_path != observed.request_path
            || request.native_transport_model != observed.native_transport_model
        {
            return Err(CpaAttemptError::UnregisteredTarget);
        }
        let prepared = if Some(observed.account.account_kind) == self.managed_kind() {
            let preparation_generation = self.preparation_generation();
            let source_stamp = self.native_source_stamp();
            match self.prepare_native_subscription() {
                Ok(prepared)
                    if prepared.as_ref().is_none_or(|evidence| {
                        evidence.account_ref()
                            == format!("account/cpa/{}", observed.account.account_digest)
                    }) =>
                {
                    prepared
                }
                _ => {
                    if self.native_source_stamp() == source_stamp {
                        self.reject_preparation(preparation_generation);
                    }
                    return Err(CpaAttemptError::RevokedCredential);
                }
            }
        } else {
            None
        };
        let mut inner = self
            .lock_lifecycle()
            .map_err(|_| CpaAttemptError::Unavailable)?;
        if !self.account_execution_is_admitted(&observed.account) {
            return Err(CpaAttemptError::RevokedCredential);
        }
        self.ensure_ready_locked(&mut inner, None)
            .map_err(|_| CpaAttemptError::Unavailable)?;
        self.ensure_attempt_account_current_locked(
            &mut inner,
            &observed.account,
            prepared.as_ref(),
        )
        .map_err(|_| CpaAttemptError::RevokedCredential)?;
        self.ensure_process_accounts_current_locked(&mut inner)
            .map_err(|_| CpaAttemptError::Unavailable)?;
        let exact = {
            let live = inner.live.as_ref().ok_or(CpaAttemptError::Unavailable)?;
            resolve_exact_account(
                self,
                &live.accounts,
                request.upstream_model_id,
                request.protocol,
                |credential| credential.credential_id() == request.credential_id,
            )?
        };
        if !self.account_execution_is_admitted(&exact.account) {
            return Err(CpaAttemptError::RevokedCredential);
        }
        let (runtime_epoch, target_epoch) = self.epochs.current();
        let live = inner.live.as_ref().ok_or(CpaAttemptError::Unavailable)?;
        if !live.address.ip().is_loopback()
            || request.connector_id != exact.connector_id
            || request.request_path != exact.request_path
            || request.native_transport_model != exact.native_transport_model
        {
            return Err(CpaAttemptError::UnregisteredTarget);
        }
        let key_id: Arc<str> = format!("cpa-downstream/{}/{}", runtime_epoch, target_epoch).into();
        if request
            .excluded_key_ids
            .iter()
            .any(|excluded| excluded.as_ref() == key_id.as_ref())
        {
            return Ok(None);
        }
        self.commit_admitted(&exact.account, || {
            Some(CpaDownstreamCredentialCapability {
                credential_ref: exact.credential_ref,
                key_id,
                authorization: Arc::clone(&live.secrets.downstream),
                address: live.address,
                request_path: exact.request_path.into(),
                epochs: Arc::clone(&self.epochs),
                runtime_epoch,
                target_epoch,
            })
        })
    }
}

struct ExactAccount {
    account: crate::accounts::AccountSnapshotRecord,
    connector_id: String,
    request_path: String,
    native_transport_model: String,
    credential_ref: CredentialRefV1,
}

fn same_stable_credential_binding(left: &CredentialRefV1, right: &CredentialRefV1) -> bool {
    left.credential_id() == right.credential_id()
        && left.owner_scope() == right.owner_scope()
        && left.subject() == right.subject()
        && left.purpose() == right.purpose()
        && left.allowed_destinations() == right.allowed_destinations()
}

fn resolve_exact_account(
    runtime: &ManagedCpaRuntime,
    accounts: &[crate::accounts::AccountSnapshotRecord],
    upstream_model_id: &str,
    protocol: UpstreamProtocol,
    credential_matches: impl Fn(&CredentialRefV1) -> bool,
) -> Result<ExactAccount, CpaAttemptError> {
    for account in accounts.iter().filter(|account| account.active) {
        let Some(binding) = runtime
            .spec
            .bindings
            .iter()
            .find(|binding| binding.account_kind == account.account_kind)
        else {
            continue;
        };
        let material = account
            .materialize(binding)
            .map_err(|_| CpaAttemptError::RevokedCredential)?;
        if !credential_matches(&material.credential_ref) {
            continue;
        }
        if !account.account_kind.supports_protocol(protocol) {
            return Err(CpaAttemptError::UnregisteredTarget);
        }
        let resolved = runtime
            .catalog
            .resolve_connection_option(&binding.connection_option_id)
            .map_err(|_| CpaAttemptError::UnregisteredTarget)?;
        // The client-bundled catalog registers the provider-native account contract. CPA's pinned
        // local protocol faces are bridge capabilities, not additional provider endpoints.
        let registered_protocol = account.account_kind.required_protocol();
        let capability_registered = runtime
            .catalog
            .model_data()
            .model_endpoint_capabilities
            .iter()
            .any(|capability| {
                capability.endpoint_profile_id == binding.endpoint_profile_id
                    && capability.upstream_model_id == upstream_model_id
                    && capability.upstream_protocol == registered_protocol
            });
        let protocol_registered = resolved
            .endpoint_profile
            .protocol_endpoints
            .iter()
            .any(|endpoint| endpoint.protocol == registered_protocol);
        let runtime_fallback_qualified = account.observed_model_ids.contains(upstream_model_id)
            && runtime
                .catalog
                .runtime_fallback_allows_observed_text(upstream_model_id);
        if (!capability_registered && !runtime_fallback_qualified) || !protocol_registered {
            return Err(CpaAttemptError::UnregisteredTarget);
        }
        let native_transport_model = account
            .transport_model(upstream_model_id)
            .ok_or(CpaAttemptError::ModelUnavailable)?;
        return Ok(ExactAccount {
            account: account.clone(),
            connector_id: resolved.connector.connector_id,
            request_path: stock_request_path(protocol).into(),
            native_transport_model,
            credential_ref: material.credential_ref,
        });
    }
    Err(CpaAttemptError::RevokedCredential)
}

fn stock_request_path(protocol: UpstreamProtocol) -> &'static str {
    match protocol {
        UpstreamProtocol::Responses => "/v1/responses",
        UpstreamProtocol::ChatCompletions => "/v1/chat/completions",
        UpstreamProtocol::Messages => "/v1/messages",
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CpaAttemptError {
    #[error("CPA runtime is unavailable")]
    Unavailable,
    #[error("CPA credential is absent, stale, or revoked")]
    RevokedCredential,
    #[error("CPA model is not available for the exact account")]
    ModelUnavailable,
    #[error("CPA target is not registered for the exact profile/protocol/model")]
    UnregisteredTarget,
    #[error("CPA downstream authorization could not be constructed")]
    InvalidAuthorization,
}
