use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use http::{HeaderMap, HeaderName, HeaderValue, header};
use thiserror::Error;

use super::ExecutionScope;

pub struct CredentialLeaseRequest<'a> {
    /// Frozen routing identity for observations, independent of source membership.
    pub execution_binding_id: &'a str,
    /// Exact original source binding used by the credential authority.
    pub stable_binding_id: &'a str,
    pub credential_ref: &'a str,
    pub credential_destination_ref: &'a str,
    pub excluded_key_ids: &'a [Arc<str>],
    pub connector_runtime: hiroute_domain::ConnectorRuntimeKind,
    pub connector_id: &'a str,
    pub upstream_protocol: crate::server::request_plan::IngressProtocol,
    pub upstream_model_id: &'a str,
    pub native_transport_model: &'a str,
    pub logical_endpoint: &'a str,
    pub operational_target: &'a str,
    pub operational_target_digest: &'a str,
    pub runtime_epoch: Option<u64>,
    pub target_epoch: Option<u64>,
    pub protocol_profile_digest: &'a str,
    pub request_path: &'a str,
    pub authentication: &'a crate::server::core_runtime::profiles::AuthenticationSemantics,
}

pub struct HeaderSecretLeaseRequest<'a> {
    pub secret_ref: &'a str,
    pub header_name: &'a str,
}

/// Opaque authority that may apply credential material but cannot expose it.
pub trait CredentialAuthorizationCapability: Send + Sync {
    fn apply_authorization(&self, headers: &mut HeaderMap) -> Result<(), CredentialError>;
}

/// Exact credential lease. The authorization material intentionally has no
/// `Debug`, `Serialize`, or public field representation.
#[derive(Clone)]
pub struct CredentialLease {
    credential_ref: Arc<str>,
    key_id: Arc<str>,
    generation: u64,
    authorization: Arc<dyn CredentialAuthorizationCapability>,
    authorization_header: Option<HeaderName>,
    transport_override: Option<CredentialTransportOverride>,
    spend: Option<Arc<dyn hiroute_domain::KeySpendAuthorityV1>>,
}

/// Request-scoped target supplied by a trusted managed connector authority.
/// Native API credentials cannot use it to redirect a sealed HTTPS target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialTransportOverride {
    address: SocketAddr,
    request_path: Arc<str>,
}

impl CredentialTransportOverride {
    pub fn managed_loopback(
        address: SocketAddr,
        request_path: impl Into<Arc<str>>,
    ) -> Result<Self, CredentialError> {
        let request_path = request_path.into();
        if !address.ip().is_loopback()
            || address.port() == 0
            || !request_path.starts_with('/')
            || request_path.len() < 2
            || request_path.contains(['?', '#'])
        {
            return Err(CredentialError::InvalidLease);
        }
        Ok(Self {
            address,
            request_path,
        })
    }

    pub const fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn request_path(&self) -> &str {
        &self.request_path
    }
}

impl CredentialLease {
    pub fn new(
        credential_ref: impl Into<Arc<str>>,
        key_id: impl Into<Arc<str>>,
        generation: u64,
        authorization: impl Into<Arc<str>>,
    ) -> Result<Self, CredentialError> {
        let credential_ref = credential_ref.into();
        let key_id = key_id.into();
        let authorization = authorization.into();
        if credential_ref.trim().is_empty()
            || key_id.trim().is_empty()
            || generation == 0
            || authorization.is_empty()
        {
            return Err(CredentialError::InvalidLease);
        }
        Ok(Self {
            credential_ref,
            key_id,
            generation,
            authorization: Arc::new(BearerAuthorization(authorization)),
            authorization_header: Some(header::AUTHORIZATION),
            transport_override: None,
            spend: None,
        })
    }

    pub fn new_header(
        credential_ref: impl Into<Arc<str>>,
        key_id: impl Into<Arc<str>>,
        generation: u64,
        header_name: HeaderName,
        value: impl Into<Arc<str>>,
    ) -> Result<Self, CredentialError> {
        let credential_ref = credential_ref.into();
        let key_id = key_id.into();
        let value = value.into();
        if credential_ref.trim().is_empty()
            || key_id.trim().is_empty()
            || generation == 0
            || value.is_empty()
        {
            return Err(CredentialError::InvalidLease);
        }
        Ok(Self {
            credential_ref,
            key_id,
            generation,
            authorization: Arc::new(HeaderAuthorization {
                name: header_name.clone(),
                value,
            }),
            authorization_header: Some(header_name),
            transport_override: None,
            spend: None,
        })
    }

    pub fn from_capability(
        credential_ref: impl Into<Arc<str>>,
        key_id: impl Into<Arc<str>>,
        generation: u64,
        authorization: Arc<dyn CredentialAuthorizationCapability>,
    ) -> Result<Self, CredentialError> {
        let credential_ref = credential_ref.into();
        let key_id = key_id.into();
        if credential_ref.trim().is_empty() || key_id.trim().is_empty() || generation == 0 {
            return Err(CredentialError::InvalidLease);
        }
        Ok(Self {
            credential_ref,
            key_id,
            generation,
            authorization,
            authorization_header: Some(header::AUTHORIZATION),
            transport_override: None,
            spend: None,
        })
    }

    pub fn from_capability_with_authorization_header(
        credential_ref: impl Into<Arc<str>>,
        key_id: impl Into<Arc<str>>,
        generation: u64,
        authorization: Arc<dyn CredentialAuthorizationCapability>,
        authorization_header: Option<HeaderName>,
    ) -> Result<Self, CredentialError> {
        let mut lease = Self::from_capability(credential_ref, key_id, generation, authorization)?;
        lease.authorization_header = authorization_header;
        Ok(lease)
    }

    pub fn from_capability_with_transport(
        credential_ref: impl Into<Arc<str>>,
        key_id: impl Into<Arc<str>>,
        generation: u64,
        authorization: Arc<dyn CredentialAuthorizationCapability>,
        transport_override: CredentialTransportOverride,
    ) -> Result<Self, CredentialError> {
        let mut lease = Self::from_capability(credential_ref, key_id, generation, authorization)?;
        lease.transport_override = Some(transport_override);
        Ok(lease)
    }

    pub fn with_spend_authority(
        mut self,
        authority: Option<Arc<dyn hiroute_domain::KeySpendAuthorityV1>>,
    ) -> Self {
        self.spend = authority;
        self
    }
    pub fn reserve_spending(
        &self,
        estimate: Option<&hiroute_domain::KeySpendEstimateV1>,
    ) -> Result<Option<KeySpendGuard>, hiroute_domain::KeyBudgetGateErrorV1> {
        let Some(authority) = &self.spend else {
            return Ok(None);
        };
        let Some(reservation) = authority.reserve(self.key_id(), self.generation(), estimate)?
        else {
            return Ok(None);
        };
        Ok(Some(KeySpendGuard {
            authority: Arc::clone(authority),
            id: reservation.reservation_id,
            finished: std::sync::atomic::AtomicBool::new(false),
            sent: false,
        }))
    }
    pub fn credential_ref(&self) -> &str {
        &self.credential_ref
    }

    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn transport_override(&self) -> Option<&CredentialTransportOverride> {
        self.transport_override.as_ref()
    }

    pub fn apply_authorization(&self, headers: &mut HeaderMap) -> Result<(), CredentialError> {
        let before = headers.clone();
        if self
            .authorization_header
            .as_ref()
            .is_some_and(|name| before.contains_key(name))
            || self.authorization.apply_authorization(headers).is_err()
        {
            *headers = before;
            return Err(CredentialError::InvalidLease);
        }
        let Some(name) = self.authorization_header.as_ref() else {
            if headers != &before {
                *headers = before;
                return Err(CredentialError::InvalidLease);
            }
            return Ok(());
        };
        let mut values = headers.get_all(name).iter();
        let authorization = values.next().filter(|value| value.is_sensitive()).cloned();
        let duplicate = values.next().is_some();
        headers.remove(name);
        let other_headers_changed = headers != &before;
        let Some(authorization) = authorization else {
            *headers = before;
            return Err(CredentialError::InvalidLease);
        };
        if duplicate || other_headers_changed {
            *headers = before;
            return Err(CredentialError::InvalidLease);
        }
        headers.insert(name.clone(), authorization);
        Ok(())
    }
}

struct BearerAuthorization(Arc<str>);

impl CredentialAuthorizationCapability for BearerAuthorization {
    fn apply_authorization(&self, headers: &mut HeaderMap) -> Result<(), CredentialError> {
        let mut value =
            HeaderValue::from_str(&self.0).map_err(|_| CredentialError::InvalidLease)?;
        value.set_sensitive(true);
        headers.insert(header::AUTHORIZATION, value);
        Ok(())
    }
}

struct HeaderAuthorization {
    name: HeaderName,
    value: Arc<str>,
}

impl CredentialAuthorizationCapability for HeaderAuthorization {
    fn apply_authorization(&self, headers: &mut HeaderMap) -> Result<(), CredentialError> {
        let mut value =
            HeaderValue::from_str(&self.value).map_err(|_| CredentialError::InvalidLease)?;
        value.set_sensitive(true);
        headers.insert(self.name.clone(), value);
        Ok(())
    }
}

#[async_trait]
pub trait CredentialResolver: Send + Sync {
    /// Returns one exact lease that is not in `excluded_key_ids`. Returning
    /// `None` means this credential reference is exhausted; it never exposes
    /// the complete Secret pool to the runtime.
    async fn lease_exact(
        &self,
        request: CredentialLeaseRequest<'_>,
        scope: &ExecutionScope,
    ) -> Result<Option<CredentialLease>, CredentialError>;

    async fn lease_header_secret(
        &self,
        request: HeaderSecretLeaseRequest<'_>,
        scope: &ExecutionScope,
    ) -> Result<Option<CredentialLease>, CredentialError>;
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CredentialError {
    #[error("credential authority is unavailable")]
    Unavailable,
    #[error("credential authority rejected the exact lease request")]
    Rejected,
    #[error("credential authority returned an invalid lease")]
    InvalidLease,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MutatingCapability;

    impl CredentialAuthorizationCapability for MutatingCapability {
        fn apply_authorization(&self, headers: &mut HeaderMap) -> Result<(), CredentialError> {
            let mut authorization = HeaderValue::from_static("Bearer private");
            authorization.set_sensitive(true);
            headers.insert(header::AUTHORIZATION, authorization);
            headers.insert("x-unsealed-capability", HeaderValue::from_static("mutated"));
            Ok(())
        }
    }

    #[test]
    fn opaque_capability_can_only_install_one_sensitive_authorization_value() {
        let lease =
            CredentialLease::from_capability("credential", "key", 1, Arc::new(MutatingCapability))
                .unwrap();
        let mut headers =
            HeaderMap::from_iter([(header::HOST, HeaderValue::from_static("provider.invalid"))]);
        let before = headers.clone();
        assert_eq!(
            lease.apply_authorization(&mut headers),
            Err(CredentialError::InvalidLease)
        );
        assert_eq!(headers, before);

        let valid = CredentialLease::new("credential", "key", 1, "Bearer private").unwrap();
        valid.apply_authorization(&mut headers).unwrap();
        assert!(headers[header::AUTHORIZATION].is_sensitive());
        headers.remove(header::AUTHORIZATION);
        assert_eq!(headers, before);
    }
}

/// A dropped/cancelled in-flight request retains its reservation. Only proven
/// unsent requests or complete protocol usage release it.
pub struct KeySpendGuard {
    authority: Arc<dyn hiroute_domain::KeySpendAuthorityV1>,
    id: String,
    finished: std::sync::atomic::AtomicBool,
    sent: bool,
}
impl KeySpendGuard {
    pub fn mark_sent(&mut self) -> hiroute_domain::PortResult<()> {
        self.authority.mark_sent(&self.id)?;
        self.sent = true;
        Ok(())
    }
    pub fn settle(&self, settlement: hiroute_domain::KeySpendSettlementV1) {
        use std::sync::atomic::Ordering;
        if self.finished.load(Ordering::Acquire) {
            return;
        }
        if self.authority.settle(&self.id, &settlement).is_ok() {
            self.finished.store(true, Ordering::Release);
        }
    }
}
impl Drop for KeySpendGuard {
    fn drop(&mut self) {
        self.settle(if self.sent {
            hiroute_domain::KeySpendSettlementV1::Unknown
        } else {
            hiroute_domain::KeySpendSettlementV1::Unsent
        });
    }
}
