use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use hiroute_domain::CredentialRefV1;
use hiroute_integrations::CpaAccountMaterializationV1;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::InstanceSecrets;

const MAX_MODELS_PER_ACCOUNT: usize = 10_000;
const MAX_OPAQUE_STOCK_FIELD_BYTES: usize = 4_096;
const CURRENT_CODEX_PREFIX: &str = "hiroute-codex-current";
const CURRENT_CODEX_CREDENTIAL_DIGEST: &str =
    "dd1e7ca19dad3956e42a5f380695012a72aa5da50d79fb3743e544b099436ce7";

mod stock;

pub(crate) use stock::StockCpaControlPlane;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManagedAccountIdentity {
    pub(crate) account_kind: CpaAccountKind,
    pub(crate) stock_file_name: String,
    pub(crate) account_digest: String,
    pub(crate) generation: u64,
    /// Current non-secret control metadata; does not rotate account identity/generation.
    pub(crate) client_version: Option<String>,
}

impl ManagedAccountIdentity {
    pub(crate) fn validate(&self) -> Result<(), AccountDiscoveryError> {
        validate_opaque_stock_field(&self.stock_file_name)?;
        if self.client_version.as_ref().is_some_and(|version| {
            version.is_empty()
                || version.len() > 128
                || !version
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.')
        }) {
            return Err(AccountDiscoveryError::InvalidAccount);
        }
        if self.generation == 0
            || self.account_digest.len() != 64
            || !self
                .account_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(AccountDiscoveryError::InvalidAccount);
        }
        Ok(())
    }

    pub(crate) fn prefix(&self) -> Result<String, AccountDiscoveryError> {
        self.validate()?;
        Ok(match self.account_kind {
            CpaAccountKind::Codex => CURRENT_CODEX_PREFIX.to_owned(),
            CpaAccountKind::Claude => format!("hiroute-{}", &self.account_digest[..24]),
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CpaAccountKind {
    Codex,
    Claude,
}

impl CpaAccountKind {
    pub(crate) const fn stock_provider(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    pub(crate) const fn required_protocol(self) -> hiroute_domain::UpstreamProtocol {
        match self {
            Self::Codex => hiroute_domain::UpstreamProtocol::Responses,
            Self::Claude => hiroute_domain::UpstreamProtocol::Messages,
        }
    }

    pub(crate) fn supports_protocol(self, protocol: hiroute_domain::UpstreamProtocol) -> bool {
        match self {
            // The managed CPA Codex account exposes all three OpenAI/Anthropic-compatible
            // request faces over the same pinned account and model authority.
            Self::Codex => matches!(
                protocol,
                hiroute_domain::UpstreamProtocol::Responses
                    | hiroute_domain::UpstreamProtocol::ChatCompletions
                    | hiroute_domain::UpstreamProtocol::Messages
            ),
            Self::Claude => protocol == hiroute_domain::UpstreamProtocol::Messages,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CpaProfileBinding {
    pub account_kind: CpaAccountKind,
    pub connector_id: String,
    pub connection_option_id: String,
    pub endpoint_profile_id: String,
}

#[derive(Clone, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct AccountSnapshotRecord {
    pub(crate) account_kind: CpaAccountKind,
    pub(crate) stock_id: String,
    pub(crate) stock_auth_index: String,
    pub(crate) stock_file_name: String,
    pub(crate) prefix: String,
    pub(crate) account_digest: String,
    pub(crate) generation: u64,
    pub(crate) observed_model_ids: BTreeSet<String>,
    pub(crate) active: bool,
}

impl std::fmt::Debug for AccountSnapshotRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AccountSnapshotRecord")
            .field("account_kind", &self.account_kind)
            .field("account_digest", &self.account_digest)
            .field("generation", &self.generation)
            .field("model_count", &self.observed_model_ids.len())
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl AccountSnapshotRecord {
    pub(crate) fn validate_persisted(&self) -> Result<(), AccountDiscoveryError> {
        validate_opaque_stock_field(&self.stock_id)?;
        validate_opaque_stock_field(&self.stock_auth_index)?;
        validate_opaque_stock_field(&self.stock_file_name)?;
        if self.generation == 0
            || self.account_digest.len() != 64
            || !self
                .account_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.prefix
                != match self.account_kind {
                    CpaAccountKind::Codex => CURRENT_CODEX_PREFIX.to_owned(),
                    CpaAccountKind::Claude => {
                        format!("hiroute-{}", &self.account_digest[..24])
                    }
                }
            || self.observed_model_ids.len() > MAX_MODELS_PER_ACCOUNT
            || self
                .observed_model_ids
                .iter()
                .any(|model| validate_registered_id(model).is_err())
        {
            return Err(AccountDiscoveryError::InvalidAccount);
        }
        Ok(())
    }

    pub(crate) fn materialize(
        &self,
        binding: &CpaProfileBinding,
    ) -> Result<CpaAccountMaterializationV1, AccountDiscoveryError> {
        if !self.active || self.account_kind != binding.account_kind || self.generation == 0 {
            return Err(AccountDiscoveryError::Inactive);
        }
        let source_id = format!("cpa/{}", self.account_digest);
        let destination = format!("connection-option/{}", binding.connection_option_id);
        let credential_digest = match self.account_kind {
            CpaAccountKind::Codex => CURRENT_CODEX_CREDENTIAL_DIGEST,
            CpaAccountKind::Claude => &self.account_digest,
        };
        let credential_ref = CredentialRefV1::new(
            format!("credential/cpa/{credential_digest}"),
            format!("source/{source_id}"),
            format!("connector/{}", binding.connector_id),
            "provider-auth",
            [destination],
            self.generation,
        )
        .map_err(|_| AccountDiscoveryError::InvalidAccount)?;
        Ok(CpaAccountMaterializationV1 {
            connector_id: binding.connector_id.clone(),
            connection_option_id: binding.connection_option_id.clone(),
            endpoint_profile_id: binding.endpoint_profile_id.clone(),
            source_id,
            account_subject: format!("account/cpa/{}", self.account_digest),
            credential_ref,
            observed_model_ids: self.observed_model_ids.clone(),
        })
    }

    pub(crate) fn transport_model(&self, upstream_model_id: &str) -> Option<String> {
        (self.active && self.observed_model_ids.contains(upstream_model_id))
            .then(|| format!("{}/{}", self.prefix, upstream_model_id))
    }
}

pub(crate) trait CpaControlPlane: Send + Sync {
    fn probe_ready(
        &self,
        address: SocketAddr,
        secrets: &InstanceSecrets,
        expected_version: &str,
        timeout: Duration,
    ) -> Result<(), AccountDiscoveryError>;

    #[allow(clippy::too_many_arguments)]
    fn discover_and_pin(
        &self,
        address: SocketAddr,
        auth_dir: &Path,
        managed_identities: &[ManagedAccountIdentity],
        secrets: &InstanceSecrets,
        expected_version: &str,
        timeout: Duration,
        refresh_models: bool,
    ) -> Result<Vec<AccountSnapshotRecord>, AccountDiscoveryError>;
}

fn validate_opaque_stock_field(value: &str) -> Result<(), AccountDiscoveryError> {
    if value.is_empty()
        || value.len() > MAX_OPAQUE_STOCK_FIELD_BYTES
        || value.contains(['\r', '\n', '\0'])
    {
        return Err(AccountDiscoveryError::InvalidAccount);
    }
    Ok(())
}

fn validate_registered_id(value: &str) -> Result<(), AccountDiscoveryError> {
    if value.is_empty()
        || value.len() > 256
        || value.contains("//")
        || value.contains("..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
    {
        return Err(AccountDiscoveryError::InvalidAccount);
    }
    Ok(())
}

#[derive(Debug, Error)]
pub(crate) enum AccountDiscoveryError {
    #[error("CPA is not ready")]
    NotReady,
    #[error("CPA management authentication failed")]
    ManagementAuthentication,
    #[error("CPA downstream authentication failed")]
    DownstreamAuthentication,
    #[error("CPA management response omitted its version")]
    MissingVersionHeader,
    #[error("running CPA version differs from the trusted artifact")]
    RunningVersionMismatch,
    #[error("CPA auth directory could not be enumerated: {0}")]
    AuthDirectory(std::io::Error),
    #[error("CPA returned too many accounts")]
    TooManyAccounts,
    #[error("CPA returned too many models")]
    TooManyModels,
    #[error("CPA account is duplicated")]
    DuplicateAccount,
    #[error("CPA account model is duplicated")]
    DuplicateModel,
    #[error("CPA account is not a file-backed OAuth subscription")]
    NonSubscriptionAccount,
    #[error("CPA account changed during materialization")]
    AccountDisappeared,
    #[error("CPA exposed an unmanaged account for a managed provider")]
    UnexpectedManagedAccount,
    #[error("CPA exact account prefix/retry pin was not applied")]
    PinNotApplied,
    #[error("CPA account is inactive")]
    Inactive,
    #[error("CPA account metadata is invalid")]
    InvalidAccount,
    #[error("CPA returned a secret-bearing management response")]
    SecretBearingResponse,
    #[error("CPA returned an invalid management response")]
    InvalidResponse,
    #[error("CPA JSON response is invalid: {0}")]
    Json(serde_json::Error),
    #[error(transparent)]
    Http(#[from] crate::http::LoopbackHttpError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_account_and_exact_control_patch_are_fail_closed() {
        let snapshot = AccountSnapshotRecord {
            account_kind: CpaAccountKind::Codex,
            stock_id: "stock-a".into(),
            stock_auth_index: "index-a".into(),
            stock_file_name: "codex-a.json".into(),
            prefix: CURRENT_CODEX_PREFIX.into(),
            account_digest: "a".repeat(64),
            generation: 1,
            observed_model_ids: BTreeSet::from(["codex-model".into()]),
            active: true,
        };
        snapshot.validate_persisted().unwrap();
        let mut invalid = snapshot;
        invalid.prefix = "other-account".into();
        assert!(invalid.validate_persisted().is_err());
    }

    #[test]
    fn opaque_materialization_contains_only_derived_ids_and_models() {
        let snapshot = AccountSnapshotRecord {
            account_kind: CpaAccountKind::Claude,
            stock_id: "private-file-name".into(),
            stock_auth_index: "private-index".into(),
            stock_file_name: "private-file-name".into(),
            prefix: "hiroute-abcd".into(),
            account_digest: "a".repeat(64),
            generation: 1,
            observed_model_ids: BTreeSet::from(["claude-sonnet".into()]),
            active: true,
        };
        let materialization = snapshot
            .materialize(&CpaProfileBinding {
                account_kind: CpaAccountKind::Claude,
                connector_id: "connector.cpa.claude".into(),
                connection_option_id: "claude.subscription.v1".into(),
                endpoint_profile_id: "endpoint.claude.subscription".into(),
            })
            .unwrap();
        let debug = format!("{materialization:?}");
        assert!(!debug.contains("private-file-name"));
        assert!(!debug.contains("private-index"));
        assert_eq!(
            snapshot.transport_model("claude-sonnet").as_deref(),
            Some("hiroute-abcd/claude-sonnet")
        );
    }
}
