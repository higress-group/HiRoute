//! Read-only native Claude credential adapter. Refresh authority never leaves Claude Code.
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hiroute_domain::CanonicalDigest;
use hiroute_integrations::ClaudeSubscriptionLocation;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::CpaLifecycleError;
use crate::accounts::{CpaAccountKind, ManagedAccountIdentity};
use crate::config::{ensure_private_dir, private_atomic_write};

const LIMIT: u64 = 128 * 1024;
const FILE_NAME: &str = "hiroute-managed-claude.json";

#[derive(Clone)]
pub struct BorrowedClaudeAuthSpec {
    location: ClaudeSubscriptionLocation,
    identity: Arc<Mutex<Option<(String, String)>>>,
    proxy: crate::proxy_environment::ProxyEnvironment,
}
impl std::fmt::Debug for BorrowedClaudeAuthSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BorrowedClaudeAuthSpec([REDACTED])")
    }
}
impl BorrowedClaudeAuthSpec {
    pub fn new(location: ClaudeSubscriptionLocation) -> Self {
        Self {
            location,
            identity: Arc::new(Mutex::new(None)),
            proxy: crate::proxy_environment::ProxyEnvironment::capture(std::env::vars_os()),
        }
    }
    pub(crate) fn set_proxy_environment(
        &mut self,
        proxy: crate::proxy_environment::ProxyEnvironment,
    ) {
        self.proxy = proxy;
    }
    fn source_identity(&self) -> Result<String, CpaLifecycleError> {
        let canonical = self
            .source_path()
            .canonicalize()
            .map_err(|_| CpaLifecycleError::BorrowedClaudeAuthUnavailable)?;
        let mut hash = Sha256::new();
        hash.update(canonical.as_os_str().as_encoded_bytes());
        hash.update([0]);
        if let ClaudeSubscriptionLocation::Keychain { service, .. } = &self.location {
            hash.update(service.as_bytes());
        }
        Ok(format!("{:x}", hash.finalize()))
    }
    pub fn source_path(&self) -> &Path {
        self.location.anchor()
    }
    pub fn inspect(&self) -> Result<BorrowedClaudeEvidence, CpaLifecycleError> {
        Ok(self.read(false)?.evidence)
    }
    /// Only an explicit user check may request system Keychain authorization.
    pub fn inspect_for_check(&self) -> Result<BorrowedClaudeEvidence, CpaLifecycleError> {
        Ok(self.read(true)?.evidence)
    }
    fn read(&self, allow_interaction: bool) -> Result<AccessLease, CpaLifecycleError> {
        let bytes = match &self.location {
            ClaudeSubscriptionLocation::File(path) => read_private(path)?,
            ClaudeSubscriptionLocation::Keychain {
                service,
                config_dir,
            } => read_keychain(service, config_dir, allow_interaction)?,
        };
        let document: NativeDocument = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        let oauth = document.claude_ai_oauth;
        let token = Zeroizing::new(oauth.access_token);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid())?
            .as_millis();
        if token.is_empty()
            || token.len() > 64 * 1024
            || token.contains(['\n', '\r', '\0'])
            || u128::from(oauth.expires_at) <= now
            || !oauth.scopes.iter().any(|scope| scope == "user:inference")
        {
            return Err(invalid());
        }
        let revision = digest(token.as_bytes());
        // Account UUID is proved with this access token, never inferred from an unrelated
        // cached CLI profile or from token bytes. Rotation revalidates the account.
        let mut cached = self.identity.lock();
        let account = if let Some((previous, account)) = cached
            .as_ref()
            .filter(|(previous, _)| previous == &revision)
        {
            let _ = previous;
            account.clone()
        } else {
            // This port is also called from async Gateway consumers. Construct/drop the
            // blocking HTTP client on its own thread, outside any Tokio runtime.
            let account = std::thread::scope(|scope| {
                scope
                    .spawn(|| fetch_account(&token, &self.proxy))
                    .join()
                    .map_err(|_| CpaLifecycleError::BorrowedClaudeAuthUnavailable)?
            })?;
            *cached = Some((revision.clone(), account.clone()));
            account
        };
        drop(cached);
        let source = self.source_identity()?;
        let account_digest =
            digest(format!("hiroute.cpa-account/v1\0claude\0{account}\0").as_bytes());
        let binding = CanonicalDigest::of(&(
            "hiroute.borrowed-claude-binding/v1",
            &source,
            &account_digest,
        ))
        .map_err(|_| invalid())?;
        let evidence_digest = CanonicalDigest::of(&(
            "hiroute.borrowed-claude-evidence/v1",
            &binding,
            &revision,
            oauth.expires_at,
        ))
        .map_err(|_| invalid())?;
        Ok(AccessLease {
            token,
            account,
            evidence: BorrowedClaudeEvidence {
                source,
                account_digest,
                revision,
                binding,
                evidence_digest,
            },
        })
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct BorrowedClaudeEvidence {
    source: String,
    account_digest: String,
    revision: String,
    binding: CanonicalDigest,
    evidence_digest: CanonicalDigest,
}
impl std::fmt::Debug for BorrowedClaudeEvidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BorrowedClaudeEvidence([REDACTED])")
    }
}
impl BorrowedClaudeEvidence {
    pub fn account_ref(&self) -> String {
        format!("account/cpa/{}", self.account_digest)
    }
    pub fn evidence_digest(&self) -> &CanonicalDigest {
        &self.evidence_digest
    }
    pub fn binding_evidence_digest(&self) -> &CanonicalDigest {
        &self.binding
    }
}

struct AccessLease {
    token: Zeroizing<String>,
    account: String,
    evidence: BorrowedClaudeEvidence,
}
#[derive(Deserialize)]
struct NativeDocument {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: NativeOauth,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeOauth {
    access_token: String,
    expires_at: u64,
    scopes: Vec<String>,
}

pub(crate) struct BorrowedClaudeLease {
    spec: BorrowedClaudeAuthSpec,
    path: PathBuf,
    _lock: File,
    observed: Option<BorrowedClaudeEvidence>,
    generation: u64,
}
impl BorrowedClaudeLease {
    pub(crate) fn acquire(
        auth_dir: &Path,
        spec: &BorrowedClaudeAuthSpec,
        expected: Option<&BorrowedClaudeEvidence>,
    ) -> Result<Self, CpaLifecycleError> {
        let root = ensure_private_dir(
            &std::env::temp_dir().join(format!("hiroute-claude-leases-{}", user_id())),
        )?;
        let lock = crate::borrowed_codex::acquire_lock(
            &root.join(format!("{}.lock", spec.source_identity()?)),
        )?;
        let mut lease = Self {
            spec: spec.clone(),
            path: auth_dir.join(FILE_NAME),
            _lock: lock,
            observed: None,
            generation: 0,
        };
        lease.refresh(expected, None)?;
        Ok(lease)
    }
    pub(crate) fn generation(&self) -> Option<u64> {
        (self.generation > 0).then_some(self.generation)
    }
    pub(crate) fn refresh(
        &mut self,
        expected: Option<&BorrowedClaudeEvidence>,
        account: Option<&str>,
    ) -> Result<ManagedAccountIdentity, CpaLifecycleError> {
        let lease = self.spec.read(false)?;
        if expected.is_some_and(|expected| expected != &lease.evidence)
            || account.is_some_and(|account| account != lease.evidence.account_digest)
            || (expected.is_none()
                && self.observed.as_ref().is_some_and(|previous| {
                    previous.account_digest != lease.evidence.account_digest
                }))
        {
            return Err(CpaLifecycleError::BorrowedClaudeAuthSourceChanged);
        }
        if self.observed.as_ref() != Some(&lease.evidence) {
            self.generation = self.generation.checked_add(1).ok_or_else(invalid)?;
        }
        let identity = ManagedAccountIdentity {
            account_kind: CpaAccountKind::Claude,
            stock_file_name: FILE_NAME.into(),
            account_digest: lease.evidence.account_digest.clone(),
            generation: self.generation,
            client_version: None,
        };
        let prefix = identity.prefix().map_err(|_| invalid())?;
        // Explicit allowlist: no refresh token, native credential document or expiry-driven
        // refresh metadata can be serialized into CPA's private auth directory.
        #[derive(Serialize)]
        struct ManagedAccess<'a> {
            r#type: &'a str,
            access_token: &'a str,
            account_uuid: &'a str,
            prefix: &'a str,
            request_retry: u8,
            disable_cooling: bool,
        }
        let rendered = Zeroizing::new(
            serde_json::to_vec(&ManagedAccess {
                r#type: "claude",
                access_token: &lease.token,
                account_uuid: &lease.account,
                prefix: &prefix,
                request_retry: 0,
                disable_cooling: true,
            })
            .map_err(|_| invalid())?,
        );
        let same = match read_private(&self.path) {
            Ok(previous) => {
                let old: serde_json::Value =
                    serde_json::from_slice(&previous).map_err(|_| invalid())?;
                if old.get("refresh_token").is_some() || old.get("refreshToken").is_some() {
                    return Err(invalid());
                }
                let previous_account = old
                    .get("account_uuid")
                    .and_then(|value| value.as_str())
                    .ok_or_else(invalid)?;
                // A process restart drops `observed`, but cannot approve a different account.
                // The protected managed identity remains authoritative until a fresh check.
                if expected.is_none() && previous_account != lease.account {
                    return Err(CpaLifecycleError::BorrowedClaudeAuthSourceChanged);
                }
                old.get("access_token").and_then(|v| v.as_str()) == Some(lease.token.as_str())
                    && old.get("account_uuid").and_then(|v| v.as_str())
                        == Some(lease.account.as_str())
                    && old.get("prefix").and_then(|v| v.as_str()) == Some(prefix.as_str())
            }
            Err(CpaLifecycleError::BorrowedClaudeAuthMissing) => false,
            Err(error) => return Err(error),
        };
        if !same {
            private_atomic_write(&self.path, &rendered)?;
        }
        self.observed = Some(lease.evidence);
        Ok(identity)
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn invalid() -> CpaLifecycleError {
    CpaLifecycleError::InvalidBorrowedClaudeAuth
}
#[cfg(unix)]
fn user_id() -> u32 {
    rustix::process::geteuid().as_raw()
}
#[cfg(not(unix))]
fn user_id() -> u32 {
    0
}

fn read_private(path: &Path) -> Result<Zeroizing<Vec<u8>>, CpaLifecycleError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            CpaLifecycleError::BorrowedClaudeAuthMissing
        } else {
            invalid()
        }
    })?;
    crate::borrowed_codex::validate_source_metadata(&metadata).map_err(|_| invalid())?;
    let file = crate::borrowed_codex::open_source_nofollow(path).map_err(|_| invalid())?;
    let before = file.metadata().map_err(|_| invalid())?;
    if !crate::borrowed_codex::same_file(&metadata, &before) {
        return Err(invalid());
    }
    let mut bytes = Zeroizing::new(Vec::new());
    (&file)
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid())?;
    let after = file.metadata().map_err(|_| invalid())?;
    if bytes.len() as u64 > LIMIT || !crate::borrowed_codex::same_file(&before, &after) {
        return Err(invalid());
    }
    Ok(bytes)
}

fn fetch_account(
    token: &str,
    proxy: &crate::proxy_environment::ProxyEnvironment,
) -> Result<String, CpaLifecycleError> {
    let client = proxy
        .configure_http_client(reqwest::blocking::Client::builder())?
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| CpaLifecycleError::BorrowedClaudeAuthUnavailable)?;
    let response = client
        .get("https://api.anthropic.com/api/oauth/profile")
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .map_err(|_| CpaLifecycleError::BorrowedClaudeAuthUnavailable)?;
    if !response.status().is_success() {
        return Err(CpaLifecycleError::BorrowedClaudeAuthUnavailable);
    }
    let mut bytes = Zeroizing::new(Vec::new());
    response
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid())?;
    if bytes.len() as u64 > LIMIT {
        return Err(invalid());
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let uuid = value
        .pointer("/account/uuid")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty() && v.len() <= 256)
        .ok_or_else(invalid)?;
    Ok(uuid.to_owned())
}

#[cfg(not(target_os = "macos"))]
fn read_keychain(_: &str, _: &Path, _: bool) -> Result<Zeroizing<Vec<u8>>, CpaLifecycleError> {
    Err(CpaLifecycleError::BorrowedClaudeAuthUnavailable)
}
#[cfg(target_os = "macos")]
fn read_keychain(
    service: &str,
    config: &Path,
    allow_interaction: bool,
) -> Result<Zeroizing<Vec<u8>>, CpaLifecycleError> {
    use security_framework::item::{ItemClass, ItemSearchOptions, Limit, SearchResult};
    let unavailable = || CpaLifecycleError::BorrowedClaudeAuthUnavailable;
    // Inspect item existence without secret access. A denied/locked existing item must
    // never silently select a potentially stale filesystem fallback.
    let mut metadata = ItemSearchOptions::new();
    metadata
        .class(ItemClass::generic_password())
        .service(service)
        .load_refs(true)
        .limit(Limit::Max(2));
    match metadata.search() {
        Ok(items) if items.len() == 1 => {}
        Err(error) if error.code() == -25300 => {
            return read_private(&config.join(".credentials.json"));
        }
        _ => return Err(unavailable()),
    }
    let mut query = ItemSearchOptions::new();
    query
        .class(ItemClass::generic_password())
        .service(service)
        .load_data(true)
        .limit(Limit::Max(2))
        .skip_authenticated_items(!allow_interaction);
    let mut items = query.search().map_err(|_| unavailable())?;
    if items.len() != 1 {
        return Err(unavailable());
    }
    match items.pop() {
        Some(SearchResult::Data(bytes)) => {
            let bytes = Zeroizing::new(bytes);
            if bytes.len() as u64 > LIMIT {
                return Err(invalid());
            }
            Ok(bytes)
        }
        _ => Err(unavailable()),
    }
}

#[cfg(test)]
#[path = "borrowed_claude_tests.rs"]
mod tests;
