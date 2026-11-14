//! Read-only native Claude credential adapter. Refresh authority never leaves Claude Code.
#[cfg(any(not(windows), test))]
use std::fs;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hiroute_domain::CanonicalDigest;
use hiroute_integrations::ClaudeSubscriptionLocation;
#[cfg(target_os = "macos")]
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::CpaLifecycleError;
use crate::accounts::{CpaAccountKind, ManagedAccountIdentity};
#[cfg(not(windows))]
use crate::config::ensure_private_dir;
use crate::config::private_atomic_write_auth;

const LIMIT: u64 = 128 * 1024;
const FILE_NAME: &str = "hiroute-managed-claude.json";

#[derive(Clone)]
pub struct BorrowedClaudeAuthSpec {
    location: ClaudeSubscriptionLocation,
    identity: Arc<crate::claude_profile::ProfileCache>,
    pub(crate) profile_reader: Arc<dyn ClaudeProfileReader>,
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
            identity: Arc::new(crate::claude_profile::ProfileCache::default()),
            profile_reader: Arc::new(RemoteProfileReader),
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
        #[cfg(windows)]
        let canonical = crate::borrowed_codex::native_windows::ReadGuard::open(self.source_path())
            .and_then(|guard| guard.canonical_path())
            .map_err(|_| CpaLifecycleError::BorrowedClaudeAuthUnavailable)?;
        #[cfg(not(windows))]
        let canonical = self
            .source_path()
            .canonicalize()
            .map_err(|_| CpaLifecycleError::BorrowedClaudeAuthUnavailable)?;
        Ok(self.identity_from_canonical(&canonical))
    }
    fn identity_from_canonical(&self, canonical: &Path) -> String {
        let mut hash = Sha256::new();
        hash.update(canonical.as_os_str().as_encoded_bytes());
        hash.update([0]);
        if let ClaudeSubscriptionLocation::Keychain { service, .. } = &self.location {
            hash.update(service.as_bytes());
        }
        format!("{:x}", hash.finalize())
    }
    pub fn source_path(&self) -> &Path {
        self.location.anchor()
    }
    pub fn inspect(&self) -> Result<BorrowedClaudeEvidence, CpaLifecycleError> {
        let _scope =
            crate::CpaRequestContext::new(std::time::Instant::now() + Duration::from_secs(10))
                .enter();
        Ok(self.read(false)?.evidence)
    }
    /// Only an explicit user check may request system Keychain authorization.
    pub fn inspect_for_check(&self) -> Result<BorrowedClaudeEvidence, CpaLifecycleError> {
        let _scope =
            crate::CpaRequestContext::new(std::time::Instant::now() + Duration::from_secs(10))
                .enter();
        Ok(self.read(true)?.evidence)
    }
    #[cfg(test)]
    fn seed_identity(&self, token: &str, account: &str) {
        self.identity.seed(
            &self.source_identity().unwrap(),
            &digest(token.as_bytes()),
            account,
        );
    }
    fn read_input(&self, allow_interaction: bool) -> Result<AccessInput, CpaLifecycleError> {
        crate::request_context::check()?;
        #[cfg(windows)]
        let mut native = None;
        let bytes = match &self.location {
            ClaudeSubscriptionLocation::File(path) => {
                #[cfg(windows)]
                {
                    let mut guard = crate::borrowed_codex::native_windows::ReadGuard::open(path)
                        .map_err(native_read_error)?;
                    let bytes = guard.read(LIMIT).map_err(native_read_error)?;
                    native = Some(guard);
                    bytes
                }
                #[cfg(not(windows))]
                read_private(path)?
            }
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
        #[cfg(windows)]
        let source = self.identity_from_canonical(
            &native
                .as_ref()
                .ok_or_else(invalid)?
                .canonical_path()
                .map_err(native_read_error)?,
        );
        #[cfg(not(windows))]
        let source = self.source_identity()?;
        Ok(AccessInput {
            token,
            revision,
            source,
            expires_at: oauth.expires_at,
            #[cfg(windows)]
            native,
        })
    }

    pub(crate) fn source_stamp(&self) -> Result<CanonicalDigest, CpaLifecycleError> {
        self.read_input(false)?.stamp()
    }

    fn read(&self, allow_interaction: bool) -> Result<AccessLease, CpaLifecycleError> {
        let input = self.read_input(allow_interaction)?;
        let stamp = input.stamp()?;
        let AccessInput {
            token,
            revision,
            source,
            expires_at,
            #[cfg(windows)]
            native,
        } = input;
        // Account UUID is proved with this access token, never inferred from an unrelated
        // cached CLI profile or from token bytes. Rotation revalidates the account.
        let account = self.identity.resolve(&source, &revision, || {
            // Synchronous control callers may themselves be in an async host.
            // Carry the budget explicitly: thread-local scopes are not inherited.
            let contexts = crate::request_context::capture();
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        crate::request_context::with_captured(&contexts, || {
                            self.profile_reader.fetch(&token, &self.proxy)
                        })
                    })
                    .join()
                    .map_err(|_| CpaLifecycleError::BorrowedClaudeAuthUnavailable)?
            })
        })?;
        crate::request_context::check()?;
        #[cfg(windows)]
        verify_native(&native)?;
        if self.read_input(allow_interaction)?.stamp()? != stamp {
            return Err(CpaLifecycleError::BorrowedClaudeAuthSourceChanged);
        }
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
            expires_at,
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
            #[cfg(windows)]
            native,
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

struct AccessInput {
    token: Zeroizing<String>,
    source: String,
    revision: String,
    expires_at: u64,
    #[cfg(windows)]
    native: Option<crate::borrowed_codex::native_windows::ReadGuard>,
}
impl AccessInput {
    fn stamp(&self) -> Result<CanonicalDigest, CpaLifecycleError> {
        #[cfg(windows)]
        verify_native(&self.native)?;
        CanonicalDigest::of(&(
            "hiroute.native-claude-read/v1",
            &self.source,
            &self.revision,
            self.expires_at,
        ))
        .map_err(|_| invalid())
    }
}

struct AccessLease {
    token: Zeroizing<String>,
    account: String,
    evidence: BorrowedClaudeEvidence,
    #[cfg(windows)]
    native: Option<crate::borrowed_codex::native_windows::ReadGuard>,
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
        #[cfg(windows)]
        let root = crate::windows_private::lease_root("claude")
            .map_err(|_| CpaLifecycleError::BorrowedCodexAuthIo)?;
        #[cfg(not(windows))]
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
        self.refresh_after_writes(expected, account, || {})
    }

    pub(super) fn refresh_after_writes(
        &mut self,
        expected: Option<&BorrowedClaudeEvidence>,
        account: Option<&str>,
        after_writes: impl FnOnce(),
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
        let generation = if self.observed.as_ref() != Some(&lease.evidence) {
            self.generation.checked_add(1).ok_or_else(invalid)?
        } else {
            self.generation
        };
        // Keep the released Unix update order. Windows publishes its transient
        // generation only after the retained native admission verifies below.
        #[cfg(not(windows))]
        {
            self.generation = generation;
        }
        let identity = ManagedAccountIdentity {
            account_kind: CpaAccountKind::Claude,
            stock_file_name: FILE_NAME.into(),
            account_digest: lease.evidence.account_digest.clone(),
            generation,
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
        #[cfg(windows)]
        verify_native(&lease.native)?;
        if !same {
            private_atomic_write_auth(&self.path, &rendered)?;
        }
        after_writes();
        #[cfg(windows)]
        {
            verify_native(&lease.native)?;
            self.generation = generation;
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
#[cfg(not(any(unix, windows)))]
fn user_id() -> u32 {
    0
}

#[cfg(windows)]
fn native_read_error(error: std::io::Error) -> CpaLifecycleError {
    if error.kind() == std::io::ErrorKind::NotFound {
        CpaLifecycleError::BorrowedClaudeAuthMissing
    } else {
        invalid()
    }
}

#[cfg(windows)]
fn verify_native(
    native: &Option<crate::borrowed_codex::native_windows::ReadGuard>,
) -> Result<(), CpaLifecycleError> {
    native
        .as_ref()
        .ok_or_else(invalid)?
        .verify()
        .map_err(|_| CpaLifecycleError::BorrowedClaudeAuthSourceChanged)
}

#[cfg(windows)]
fn read_private(path: &Path) -> Result<Zeroizing<Vec<u8>>, CpaLifecycleError> {
    crate::windows_private::read_auth_file(path, LIMIT).map_err(native_read_error)
}

#[cfg(not(windows))]
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

pub(crate) trait ClaudeProfileReader: Send + Sync {
    fn fetch(
        &self,
        token: &str,
        proxy: &crate::proxy_environment::ProxyEnvironment,
    ) -> Result<String, CpaLifecycleError>;
}

struct RemoteProfileReader;
impl ClaudeProfileReader for RemoteProfileReader {
    fn fetch(
        &self,
        token: &str,
        proxy: &crate::proxy_environment::ProxyEnvironment,
    ) -> Result<String, CpaLifecycleError> {
        fetch_account(token, proxy)
    }
}

fn fetch_account(
    token: &str,
    proxy: &crate::proxy_environment::ProxyEnvironment,
) -> Result<String, CpaLifecycleError> {
    let client = proxy
        .configure_http_client(reqwest::blocking::Client::builder())?
        .timeout(crate::request_context::remaining(Duration::from_secs(10))?)
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
static KEYCHAIN_INTERACTION: Mutex<()> = Mutex::new(());

#[cfg(target_os = "macos")]
fn with_keychain_interaction<T>(
    allow_interaction: bool,
    read: impl FnOnce() -> Result<T, CpaLifecycleError>,
) -> Result<T, CpaLifecycleError> {
    use security_framework::os::macos::keychain::SecKeychain;
    // SecItem UI flags do not suppress legacy file-based Keychain prompts.
    // Serialize every local Keychain read, including explicit interactive checks,
    // until the process-wide flag has been restored.
    let _lock = crate::request_context::lock(&KEYCHAIN_INTERACTION)?;
    let unavailable = |_| CpaLifecycleError::BorrowedClaudeAuthUnavailable;
    let was_allowed = SecKeychain::user_interaction_allowed().map_err(unavailable)?;
    // This library guard restores true, so never create it when already disabled.
    let suppression = if !allow_interaction && was_allowed {
        Some(SecKeychain::disable_user_interaction().map_err(unavailable)?)
    } else {
        None
    };
    let result = read();
    drop(suppression);
    if SecKeychain::user_interaction_allowed().map_err(unavailable)? != was_allowed {
        return Err(CpaLifecycleError::BorrowedClaudeAuthUnavailable);
    }
    result
}

#[cfg(target_os = "macos")]
fn read_keychain(
    service: &str,
    config: &Path,
    allow_interaction: bool,
) -> Result<Zeroizing<Vec<u8>>, CpaLifecycleError> {
    with_keychain_interaction(allow_interaction, || {
        read_keychain_contents(service, config, allow_interaction)
    })
}

#[cfg(target_os = "macos")]
fn read_keychain_contents(
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

#[cfg(test)]
#[path = "profile_resolution_tests.rs"]
mod profile_resolution_tests;
