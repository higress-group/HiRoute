use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    CanonicalDigest, ConnectorRuntimeKind, GatewayAuthenticationSemanticsV1,
    GatewayOperationalTargetV1, PortResult, UpstreamProtocol,
};

use super::common::validate_identifier;

pub const NATIVE_CREDENTIAL_LEASE_REQUEST_SCHEMA_V1: &str =
    "hiroute.native-credential-lease-request/v1";
pub const HEADER_SECRET_LEASE_REQUEST_SCHEMA_V1: &str = "hiroute.header-secret-lease-request/v1";

/// Narrow, destination-independent request for one operator-selected HTTP
/// header Secret. It deliberately carries no endpoint, provider or model
/// authority; the caller already owns the request target and header name.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HeaderSecretLeaseRequestV1 {
    pub schema_version: String,
    pub secret_id: String,
    pub header_name: String,
}

impl HeaderSecretLeaseRequestV1 {
    pub fn validate(&self) -> Result<(), NativeCredentialContractErrorV1> {
        if self.schema_version != HEADER_SECRET_LEASE_REQUEST_SCHEMA_V1
            || validate_identifier(&self.secret_id).is_err()
            || !valid_header_name(&self.header_name)
        {
            return Err(NativeCredentialContractErrorV1::InvalidRequest);
        }
        Ok(())
    }
}

fn valid_header_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
        && ![
            "host",
            "content-length",
            "transfer-encoding",
            "connection",
            "content-type",
            "te",
            "trailer",
            "upgrade",
        ]
        .iter()
        .any(|owned| value.eq_ignore_ascii_case(owned))
}

/// Product-owned request facts needed to resolve one Native credential without a publication
/// lookup. The protocol-profile digest is syntax/equality checked here; the Gateway remains the
/// owner of validating that digest against the canonical profile bytes before making this call.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCredentialLeaseRequestV1 {
    pub schema_version: String,
    pub stable_binding_id: String,
    pub credential_id: String,
    pub credential_destination_ref: String,
    pub excluded_key_ids: Vec<String>,
    pub connector_runtime: ConnectorRuntimeKind,
    pub connector_id: String,
    pub upstream_protocol: UpstreamProtocol,
    pub upstream_model_id: String,
    pub native_transport_model: String,
    pub logical_endpoint: String,
    pub operational_target: GatewayOperationalTargetV1,
    pub operational_target_digest: CanonicalDigest,
    pub request_path: String,
    pub runtime_epoch: Option<u64>,
    pub target_epoch: Option<u64>,
    pub protocol_profile_digest: CanonicalDigest,
    pub authentication: GatewayAuthenticationSemanticsV1,
}

impl NativeCredentialLeaseRequestV1 {
    pub fn validate(&self) -> Result<(), NativeCredentialContractErrorV1> {
        if self.schema_version != NATIVE_CREDENTIAL_LEASE_REQUEST_SCHEMA_V1
            || self.connector_runtime != ConnectorRuntimeKind::BuiltinNative
            || self.upstream_model_id != self.native_transport_model
            || self.runtime_epoch.is_some()
            || self.target_epoch.is_some()
            || !valid_authentication(&self.authentication)
            || !self
                .operational_target
                .validate_for(self.connector_runtime, &self.logical_endpoint)
            || self.operational_target.uri() != self.logical_endpoint
            || self.operational_target.request_path() != Some(self.request_path.as_str())
            || !matches!(
                CanonicalDigest::of(&self.operational_target),
                Ok(digest) if digest == self.operational_target_digest
            )
            || CanonicalDigest::parse(self.protocol_profile_digest.as_str()).is_err()
        {
            return Err(NativeCredentialContractErrorV1::InvalidRequest);
        }
        for value in [
            &self.stable_binding_id,
            &self.credential_id,
            &self.connector_id,
        ] {
            validate_identifier(value)
                .map_err(|_| NativeCredentialContractErrorV1::InvalidRequest)?;
        }
        let Some(destination) = ["connection-option/", "compute-target/"]
            .into_iter()
            .find_map(|prefix| self.credential_destination_ref.strip_prefix(prefix))
        else {
            return Err(NativeCredentialContractErrorV1::InvalidRequest);
        };
        validate_identifier(destination)
            .map_err(|_| NativeCredentialContractErrorV1::InvalidRequest)?;
        if !self.request_path.starts_with('/')
            || self.request_path.len() < 2
            || self.request_path.contains(['?', '#'])
        {
            return Err(NativeCredentialContractErrorV1::InvalidRequest);
        }
        let excluded = self.excluded_key_ids.iter().collect::<BTreeSet<_>>();
        if excluded.len() != self.excluded_key_ids.len()
            || excluded
                .iter()
                .any(|value| validate_identifier(value).is_err())
        {
            return Err(NativeCredentialContractErrorV1::InvalidRequest);
        }
        Ok(())
    }
}

/// Transport-neutral target used by an opaque credential capability. Implementations may install
/// one sensitive Authorization value but receive no accessor for the stored credential material.
pub trait SensitiveAuthorizationTargetV1 {
    fn set_sensitive_authorization(
        &mut self,
        value: &[u8],
    ) -> Result<(), NativeCredentialCapabilityErrorV1>;

    fn set_sensitive_header(
        &mut self,
        name: &str,
        value: &[u8],
    ) -> Result<(), NativeCredentialCapabilityErrorV1> {
        if name.eq_ignore_ascii_case("authorization") {
            self.set_sensitive_authorization(value)
        } else {
            Err(NativeCredentialCapabilityErrorV1::Rejected)
        }
    }
}

fn valid_authentication(authentication: &GatewayAuthenticationSemanticsV1) -> bool {
    !matches!(authentication, GatewayAuthenticationSemanticsV1::None)
        && super::management::validate_authentication(authentication).is_ok()
}

pub trait NativeCredentialAuthorizationCapabilityV1: Send + Sync {
    fn apply_authorization(
        &self,
        target: &mut dyn SensitiveAuthorizationTargetV1,
    ) -> Result<(), NativeCredentialCapabilityErrorV1>;
}

/// Exact Native lease. It intentionally has neither `Debug` nor a serde representation.
#[derive(Clone)]
pub struct NativeCredentialLeaseV1 {
    credential_id: Arc<str>,
    key_id: Arc<str>,
    generation: u64,
    authorization: Arc<dyn NativeCredentialAuthorizationCapabilityV1>,
}

impl NativeCredentialLeaseV1 {
    pub fn issue(
        credential_id: impl Into<Arc<str>>,
        key_id: impl Into<Arc<str>>,
        generation: u64,
        authorization: Arc<dyn NativeCredentialAuthorizationCapabilityV1>,
    ) -> Result<Self, NativeCredentialContractErrorV1> {
        let credential_id = credential_id.into();
        let key_id = key_id.into();
        if validate_identifier(&credential_id).is_err()
            || validate_identifier(&key_id).is_err()
            || generation == 0
        {
            return Err(NativeCredentialContractErrorV1::InvalidLease);
        }
        Ok(Self {
            credential_id,
            key_id,
            generation,
            authorization,
        })
    }

    pub fn credential_id(&self) -> &str {
        &self.credential_id
    }

    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn apply_authorization(
        &self,
        target: &mut dyn SensitiveAuthorizationTargetV1,
    ) -> Result<(), NativeCredentialCapabilityErrorV1> {
        self.authorization.apply_authorization(target)
    }
}

/// Replaceable Product authority. Local storage, a Vault client, or a remote authority can
/// implement this same port; Gateway adapters never depend on SQLite types.
pub trait NativeCredentialAuthorityV1: Send + Sync {
    /// Decision membership is separate from generative model authority.
    fn admit_decision_credentials(
        &self,
        _binding: &crate::DecisionCredentialBindingV1,
    ) -> Result<crate::DecisionCredentialAdmissionV1, NativeCredentialLeaseErrorV1> {
        Err(crate::PortError::new(
            crate::PortErrorCode::Unavailable,
            "decision.credential.unsupported",
        )
        .into())
    }

    fn lease_decision_credential(
        &self,
        _request: &crate::DecisionCredentialLeaseRequestV1,
    ) -> Result<Option<NativeCredentialLeaseV1>, NativeCredentialLeaseErrorV1> {
        Err(crate::PortError::new(
            crate::PortErrorCode::Unavailable,
            "decision.credential.unsupported",
        )
        .into())
    }

    /// Non-secret current order for an already authorized Native candidate. `None` keeps
    /// legacy exact users unchanged; an empty list means the managed candidate is unavailable.
    /// This reads no secret and issues no lease. Current permission denials retain their
    /// closed cause; the frozen exact request supplies target proof.
    fn current_native_credential_refs(
        &self,
        _request: &NativeCredentialLeaseRequestV1,
    ) -> Result<Option<Vec<String>>, NativeCredentialLeaseErrorV1> {
        Ok(None)
    }

    /// An empty lease means exact exhaustion. Current managed source/Key permission
    /// denials retain their closed reason before any secret is decrypted.
    fn lease_native_credential(
        &self,
        request: &NativeCredentialLeaseRequestV1,
    ) -> Result<Option<NativeCredentialLeaseV1>, NativeCredentialLeaseErrorV1>;

    fn lease_header_secret(
        &self,
        request: &HeaderSecretLeaseRequestV1,
    ) -> PortResult<Option<NativeCredentialLeaseV1>>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeCredentialAccessDeniedV1 {
    SourceDisabled,
    CredentialDisabled,
}

/// In-process Native permission failure, separate from persisted credential and request schemas.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum NativeCredentialLeaseErrorV1 {
    #[error("current managed credential permission was denied: {0:?}")]
    Denied(NativeCredentialAccessDeniedV1),
    #[error(transparent)]
    Authority(#[from] crate::PortError),
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum NativeCredentialContractErrorV1 {
    #[error("native credential request is invalid")]
    InvalidRequest,
    #[error("native credential lease is invalid")]
    InvalidLease,
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum NativeCredentialCapabilityErrorV1 {
    #[error("native credential authorization target rejected the value")]
    Rejected,
}
