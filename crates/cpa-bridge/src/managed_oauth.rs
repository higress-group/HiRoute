//! Read-only projections of credentials owned and refreshed exclusively by CPA.
//! No token is returned to the daemon, and no token file is rewritten by this adapter.
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use hiroute_domain::CanonicalDigest;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::accounts::ManagedAccountIdentity;
use crate::{CpaAccountKind, CpaLifecycleError};

const MAX_CREDENTIAL_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CpaManagedCredentialSummary {
    pub kind: CpaAccountKind,
    pub account_ref: String,
    pub binding_evidence_digest: CanonicalDigest,
}

/// Stable identity of a CPA-owned login. Access and refresh rotations are private to CPA.
#[derive(Clone, Eq, PartialEq)]
pub struct CpaManagedEvidence {
    kind: CpaAccountKind,
    account_digest: String,
    stock_file_name: String,
    binding: CanonicalDigest,
}

impl std::fmt::Debug for CpaManagedEvidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CpaManagedEvidence")
            .field("kind", &self.kind)
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

impl CpaManagedEvidence {
    pub fn kind(&self) -> CpaAccountKind {
        self.kind
    }

    pub fn account_ref(&self) -> String {
        format!("account/cpa/{}", self.account_digest)
    }

    pub fn evidence_digest(&self) -> &CanonicalDigest {
        &self.binding
    }

    pub fn binding_evidence_digest(&self) -> &CanonicalDigest {
        &self.binding
    }

    pub(crate) fn summary(&self) -> CpaManagedCredentialSummary {
        CpaManagedCredentialSummary {
            kind: self.kind,
            account_ref: self.account_ref(),
            binding_evidence_digest: self.binding.clone(),
        }
    }

    pub(crate) fn identity(&self) -> ManagedAccountIdentity {
        ManagedAccountIdentity {
            account_kind: self.kind,
            account_digest: self.account_digest.clone(),
            stock_file_name: self.stock_file_name.clone(),
            generation: 1,
            client_version: None,
        }
    }

    pub(crate) fn stock_file_name(&self) -> &str {
        &self.stock_file_name
    }
}

#[derive(Clone)]
pub(crate) struct ManagedOAuthCredentialSource {
    pub(crate) auth_dir: PathBuf,
    pub(crate) kind: CpaAccountKind,
}

impl ManagedOAuthCredentialSource {
    pub(crate) fn inspect(&self) -> Result<CpaManagedEvidence, CpaLifecycleError> {
        let credentials = self.list()?;
        if credentials.len() != 1 {
            return Err(if credentials.is_empty() {
                CpaLifecycleError::ManagedOAuthCredentialsMissing
            } else {
                CpaLifecycleError::InvalidManagedOAuthCredentials
            });
        }
        credentials
            .into_iter()
            .next()
            .ok_or(CpaLifecycleError::ManagedOAuthCredentialsMissing)
    }

    pub(crate) fn list(&self) -> Result<Vec<CpaManagedEvidence>, CpaLifecycleError> {
        // Inspection must not create a missing store or follow a replacement symlink.
        validate_auth_directory(&self.auth_dir)?;
        let canonical = self.auth_dir.canonicalize().map_err(|_| invalid())?;
        let mut credentials = Vec::new();
        for entry in fs::read_dir(&canonical).map_err(|_| invalid())? {
            let entry = entry.map_err(|_| invalid())?;
            let name = entry.file_name().into_string().map_err(|_| invalid())?;
            if !name.ends_with(".json") {
                continue;
            }
            if !credentials.is_empty() {
                // An instance is dedicated to one login, never an implicit account pool.
                return Err(invalid());
            }
            credentials.push(read_credential(&canonical, &name, self.kind)?);
        }
        Ok(credentials)
    }
}

fn validate_auth_directory(path: &Path) -> Result<(), CpaLifecycleError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| CpaLifecycleError::ManagedOAuthCredentialsMissing)?;
    if !path.is_absolute() || !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(invalid());
    }
    Ok(())
}

fn read_credential(
    directory: &Path,
    name: &str,
    kind: CpaAccountKind,
) -> Result<CpaManagedEvidence, CpaLifecycleError> {
    let path = directory.join(name);
    for _ in 0..3 {
        let file = crate::borrowed_codex::open_source_nofollow(&path).map_err(|_| invalid())?;
        let before = file.metadata().map_err(|_| invalid())?;
        secure_credential_file(&before)?;
        if before.len() > MAX_CREDENTIAL_BYTES {
            return Err(invalid());
        }
        let mut bytes = Zeroizing::new(Vec::new());
        (&file)
            .take(MAX_CREDENTIAL_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid())?;
        let after = file.metadata().map_err(|_| invalid())?;
        let current = fs::symlink_metadata(&path).map_err(|_| invalid())?;
        if !crate::borrowed_codex::same_file(&before, &after)
            || !crate::borrowed_codex::same_file(&after, &current)
            || bytes.len() as u64 > MAX_CREDENTIAL_BYTES
        {
            continue;
        }
        // CPA can truncate and rewrite its own file during a refresh. A bounded reread
        // tolerates that window; it never falls back to a cached credential on failure.
        if let Ok(evidence) = parse_credential(directory, name, kind, &bytes) {
            return Ok(evidence);
        }
        std::thread::yield_now();
    }
    Err(invalid())
}

fn secure_credential_file(metadata: &fs::Metadata) -> Result<(), CpaLifecycleError> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.nlink() != 1 {
            return Err(invalid());
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct CredentialProjection<'a> {
    #[serde(rename = "type", borrow)]
    kind: &'a str,
    #[serde(borrow)]
    access_token: &'a str,
    #[serde(borrow)]
    refresh_token: &'a str,
    #[serde(default, borrow)]
    account_id: Option<&'a str>,
    #[serde(default, borrow)]
    account_uuid: Option<&'a str>,
    #[serde(default)]
    disabled: bool,
}

fn parse_credential(
    directory: &Path,
    name: &str,
    kind: CpaAccountKind,
    bytes: &[u8],
) -> Result<CpaManagedEvidence, CpaLifecycleError> {
    let projection: CredentialProjection<'_> =
        serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if projection.kind != kind.stock_provider()
        || projection.disabled
        || !valid_private_field(projection.access_token, 64 * 1024)
        || !valid_private_field(projection.refresh_token, 64 * 1024)
    {
        return Err(invalid());
    }
    let account = match kind {
        CpaAccountKind::Codex => projection.account_id,
        CpaAccountKind::Claude => projection.account_uuid,
    }
    .filter(|value| valid_private_field(value, 4096))
    .ok_or_else(invalid)?;
    // Exactly the same subject algorithm as the native borrowed adapters. The store
    // and login mode belong to evidence, never to the provider's account identity.
    let account_digest = match kind {
        CpaAccountKind::Codex => crate::borrowed_codex::account_digest(account),
        CpaAccountKind::Claude => format!(
            "{:x}",
            Sha256::digest(format!("hiroute.cpa-account/v1\0claude\0{account}\0").as_bytes())
        ),
    };
    let binding = CanonicalDigest::of(&(
        "hiroute.cpa-managed-oauth-binding/v1",
        kind.stock_provider(),
        directory.to_string_lossy(),
        name,
        &account_digest,
    ))
    .map_err(|_| invalid())?;
    Ok(CpaManagedEvidence {
        kind,
        account_digest,
        stock_file_name: name.to_owned(),
        binding,
    })
}

fn valid_private_field(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.contains(['\r', '\n', '\0'])
}

fn invalid() -> CpaLifecycleError {
    CpaLifecycleError::InvalidManagedOAuthCredentials
}

/// Closed statuses written by the pinned CPA auth manager, not provider error text.
pub(crate) fn authentication_required(status_message: &str) -> bool {
    matches!(
        status_message,
        "unauthorized" | "invalid grant (retrying)" | "disabled (invalid grant)"
    )
}

#[cfg(test)]
mod tests;
