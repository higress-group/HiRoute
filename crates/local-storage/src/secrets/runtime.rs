use std::collections::BTreeSet;
use std::sync::Mutex;

use hiroute_domain::{
    CredentialRefV1, GatewayAuthenticationSemanticsV1, HeaderSecretLeaseRequestV1,
    NativeCredentialAuthorityV1, NativeCredentialAuthorizationCapabilityV1,
    NativeCredentialCapabilityErrorV1, NativeCredentialLeaseRequestV1, NativeCredentialLeaseV1,
    PortErrorCode, PortResult, ProtectedSecret, SensitiveAuthorizationTargetV1,
};
use zeroize::Zeroizing;

use super::{LocalSecretStore, port, read_entry, read_head};

/// Thread-safe owner of the local Secret authority. The Gateway-facing adapter depends only on
/// `NativeCredentialAuthorityV1`; SQLite and the master-key implementation remain private here.
pub struct LocalNativeCredentialAuthority {
    store: Mutex<LocalSecretStore>,
}

impl LocalNativeCredentialAuthority {
    pub fn new(store: LocalSecretStore) -> Self {
        Self {
            store: Mutex::new(store),
        }
    }
}

impl LocalSecretStore {
    /// Resolves one exact request against this already-open Secret store.
    ///
    /// The store itself is intentionally not `Sync`; production callers must hold their
    /// composition-owned synchronization guard while invoking this method. The standalone
    /// authority below owns that guard, while `role=all` reuses the coordinated store-set guard.
    pub fn lease_native_credential_exact(
        &self,
        request: &NativeCredentialLeaseRequestV1,
    ) -> PortResult<Option<NativeCredentialLeaseV1>> {
        request.validate().map_err(|_| {
            port(
                PortErrorCode::InvalidData,
                "secret.runtime.request_contract",
            )
        })?;
        let connection = self.connection.borrow();
        let row = read_entry(&connection, &request.credential_id)?
            .ok_or_else(|| port(PortErrorCode::NotFound, "secret.runtime.credential_missing"))?;
        let allowed_destinations: BTreeSet<String> =
            serde_json::from_str(&row.allowed_destinations_json)
                .map_err(|_| port(PortErrorCode::Corrupt, "secret.runtime.destination_decode"))?;
        let reference = CredentialRefV1::new(
            row.credential_id.clone(),
            row.owner_scope.clone(),
            row.subject.clone(),
            row.purpose.clone(),
            allowed_destinations,
            row.generation,
        )
        .map_err(|_| port(PortErrorCode::Corrupt, "secret.runtime.reference"))?;
        self.validate_reference(&reference, &row)?;
        if row.kind != "provider-api-key"
            || reference.subject() != "hirouted"
            || reference.purpose() != "provider-auth"
            || !reference
                .allowed_destinations()
                .contains(&request.credential_destination_ref)
            || read_head(&connection, reference.credential_id())? != reference.generation()
        {
            return Err(port(
                PortErrorCode::PermissionDenied,
                "secret.runtime.destination_binding",
            ));
        }
        if request
            .excluded_key_ids
            .iter()
            .any(|excluded| excluded == reference.credential_id())
        {
            return Ok(None);
        }
        // Decryption is deliberately last: every schema, target, destination, auth and generation
        // check above runs before ciphertext is opened.
        let secret = self.authenticate_row(&row)?;
        let credential_id = reference.credential_id().to_owned();
        let generation = reference.generation();
        let capability = std::sync::Arc::new(LocalNativeCapability {
            secret,
            authentication: request.authentication.clone(),
        });
        NativeCredentialLeaseV1::issue(credential_id.clone(), credential_id, generation, capability)
            .map(Some)
            .map_err(|_| port(PortErrorCode::Corrupt, "secret.runtime.lease"))
    }

    pub fn lease_header_secret_exact(
        &self,
        request: &HeaderSecretLeaseRequestV1,
    ) -> PortResult<Option<NativeCredentialLeaseV1>> {
        request.validate().map_err(|_| {
            port(
                PortErrorCode::InvalidData,
                "secret.runtime.header_request_contract",
            )
        })?;
        let connection = self.connection.borrow();
        let row = read_entry(&connection, &request.secret_id)?.ok_or_else(|| {
            port(
                PortErrorCode::NotFound,
                "secret.runtime.header_secret_missing",
            )
        })?;
        let allowed_destinations: BTreeSet<String> =
            serde_json::from_str(&row.allowed_destinations_json).map_err(|_| {
                port(
                    PortErrorCode::Corrupt,
                    "secret.runtime.header_secret_destination_decode",
                )
            })?;
        let reference = CredentialRefV1::new(
            row.credential_id.clone(),
            row.owner_scope.clone(),
            row.subject.clone(),
            row.purpose.clone(),
            allowed_destinations,
            row.generation,
        )
        .map_err(|_| {
            port(
                PortErrorCode::Corrupt,
                "secret.runtime.header_secret_reference",
            )
        })?;
        self.validate_reference(&reference, &row)?;
        if row.kind != "provider-api-key"
            || reference.subject() != "hirouted"
            || reference.purpose() != "http-header"
            || !reference.allowed_destinations().is_empty()
            || read_head(&connection, reference.credential_id())? != reference.generation()
        {
            return Err(port(
                PortErrorCode::PermissionDenied,
                "secret.runtime.header_secret_scope",
            ));
        }
        let secret = self.authenticate_row(&row)?;
        let credential_id = reference.credential_id().to_owned();
        let generation = reference.generation();
        let capability = std::sync::Arc::new(LocalNativeCapability {
            secret,
            authentication: GatewayAuthenticationSemanticsV1::ApiKeyHeader {
                header: request.header_name.clone(),
            },
        });
        NativeCredentialLeaseV1::issue(credential_id.clone(), credential_id, generation, capability)
            .map(Some)
            .map_err(|_| port(PortErrorCode::Corrupt, "secret.runtime.header_secret_lease"))
    }
}

impl NativeCredentialAuthorityV1 for LocalNativeCredentialAuthority {
    fn lease_native_credential(
        &self,
        request: &NativeCredentialLeaseRequestV1,
    ) -> PortResult<Option<NativeCredentialLeaseV1>> {
        self.store
            .lock()
            .map_err(|_| port(PortErrorCode::Unavailable, "secret.runtime.authority_lock"))?
            .lease_native_credential_exact(request)
    }

    fn lease_header_secret(
        &self,
        request: &HeaderSecretLeaseRequestV1,
    ) -> PortResult<Option<NativeCredentialLeaseV1>> {
        self.store
            .lock()
            .map_err(|_| port(PortErrorCode::Unavailable, "secret.runtime.authority_lock"))?
            .lease_header_secret_exact(request)
    }
}

struct LocalNativeCapability {
    secret: ProtectedSecret,
    authentication: hiroute_domain::GatewayAuthenticationSemanticsV1,
}

impl NativeCredentialAuthorizationCapabilityV1 for LocalNativeCapability {
    fn apply_authorization(
        &self,
        target: &mut dyn SensitiveAuthorizationTargetV1,
    ) -> Result<(), NativeCredentialCapabilityErrorV1> {
        match &self.authentication {
            hiroute_domain::GatewayAuthenticationSemanticsV1::Bearer => {
                let mut value = Zeroizing::new(Vec::with_capacity(7 + self.secret.expose().len()));
                value.extend_from_slice(b"Bearer ");
                value.extend_from_slice(self.secret.expose());
                target.set_sensitive_authorization(&value)
            }
            hiroute_domain::GatewayAuthenticationSemanticsV1::ApiKeyHeader { header } => {
                target.set_sensitive_header(header, self.secret.expose())
            }
            hiroute_domain::GatewayAuthenticationSemanticsV1::None => {
                Err(NativeCredentialCapabilityErrorV1::Rejected)
            }
        }
    }
}
