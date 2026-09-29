#[cfg(not(unix))]
use std::fs::OpenOptions;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use fs2::FileExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::STOCK_CPA_CONTRACT_VERSION;
use crate::accounts::{CpaAccountKind, ManagedAccountIdentity};
use crate::config::{ensure_private_dir, private_atomic_write, validate_private_file};
use crate::errors::CpaLifecycleError;

const MANAGED_FILE_NAME: &str = "hiroute-managed-codex.json";
const STATE_FILE_NAME: &str = ".hiroute-borrowed-codex.state";
const AUTH_LOCK_FILE_NAME: &str = ".hiroute-managed-auth.lock";
const STATE_SCHEMA: &str = "hiroute.borrowed-codex-access-lease/v1";
const MAX_AUTH_BYTES: u64 = 128 * 1024;
const MAX_TOKEN_BYTES: usize = 64 * 1024;
const MAX_ACCOUNT_ID_BYTES: usize = 4 * 1024;
const MAX_LAST_REFRESH_BYTES: usize = 256;
const SOURCE_READ_ATTEMPTS: usize = 3;

mod evidence;

pub use evidence::BorrowedCodexEvidence;

/// A Codex CLI auth file borrowed by the managed CPA runtime.
///
/// The source must be an absolute, owner-only regular file. Only its current access lease is
/// materialized for CPA; the source remains responsible for OAuth refresh.
#[derive(Clone, Eq, PartialEq)]
pub struct BorrowedCodexAuthSpec {
    source_path: PathBuf,
    executable: PathBuf,
}

impl std::fmt::Debug for BorrowedCodexAuthSpec {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BorrowedCodexAuthSpec([REDACTED_PATH])")
    }
}

impl BorrowedCodexAuthSpec {
    pub fn new(source_path: impl Into<PathBuf>) -> Self {
        Self {
            source_path: source_path.into(),
            executable: PathBuf::from("codex"),
        }
    }

    pub fn with_executable(mut self, executable: PathBuf) -> Self {
        self.executable = executable;
        self
    }

    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    /// Reads stable, non-secret evidence without creating a lease, directory, or managed file.
    pub fn inspect(&self) -> Result<BorrowedCodexEvidence, CpaLifecycleError> {
        let canonical_source = canonical_private_source(self.source_path())?;
        let source = read_nested_source(&canonical_source)?;
        BorrowedCodexEvidence::from_source(&canonical_source, &source)
    }
}

pub(crate) struct ManagedAuthLease {
    _auth_dir_lock: File,
    codex: Option<BorrowedCodexLease>,
}

impl ManagedAuthLease {
    pub(crate) fn codex_generation(&self) -> Option<u64> {
        self.codex
            .as_ref()
            .and_then(|lease| lease.observed_generation)
    }

    #[cfg(test)]
    pub(crate) fn acquire(
        auth_dir: &Path,
        source: Option<&BorrowedCodexAuthSpec>,
    ) -> Result<Self, CpaLifecycleError> {
        Self::acquire_expected(auth_dir, source, None)
    }

    pub(crate) fn acquire_expected(
        auth_dir: &Path,
        source: Option<&BorrowedCodexAuthSpec>,
        expected: Option<&BorrowedCodexEvidence>,
    ) -> Result<Self, CpaLifecycleError> {
        let auth_dir_lock = acquire_lock(&auth_dir.join(AUTH_LOCK_FILE_NAME))?;
        let codex = source
            .map(|source| BorrowedCodexLease::acquire(auth_dir, source, expected))
            .transpose()?;
        Ok(Self {
            _auth_dir_lock: auth_dir_lock,
            codex,
        })
    }

    #[cfg(test)]
    pub(crate) fn refresh(&mut self) -> Result<Vec<ManagedAccountIdentity>, CpaLifecycleError> {
        self.refresh_expected(None, None)
    }

    pub(crate) fn refresh_expected(
        &mut self,
        expected: Option<&BorrowedCodexEvidence>,
        expected_account_digest: Option<&str>,
    ) -> Result<Vec<ManagedAccountIdentity>, CpaLifecycleError> {
        self.codex
            .as_mut()
            .map(|lease| lease.refresh_expected(expected, expected_account_digest))
            .transpose()
            .map(|identity| identity.into_iter().collect())
    }
}

struct BorrowedCodexLease {
    canonical_source: PathBuf,
    client_version: Option<String>,
    source_path_digest: String,
    _source_lock: File,
    flat_path: PathBuf,
    state_path: PathBuf,
    observed_generation: Option<u64>,
}

impl BorrowedCodexLease {
    fn acquire(
        auth_dir: &Path,
        source: &BorrowedCodexAuthSpec,
        expected: Option<&BorrowedCodexEvidence>,
    ) -> Result<Self, CpaLifecycleError> {
        let canonical_source = canonical_private_source(source.source_path())?;
        let source_path_digest = path_digest(&canonical_source);
        if expected.is_some_and(|value| value.source_path_digest() != source_path_digest) {
            return Err(CpaLifecycleError::BorrowedCodexAuthSourceChanged);
        }
        let lease_root = ensure_private_dir(&source_lease_root())
            .map_err(|_| CpaLifecycleError::BorrowedCodexAuthIo)?;
        let source_lock = acquire_lock(&lease_root.join(format!("{source_path_digest}.lock")))?;
        let mut lease = Self {
            canonical_source,
            client_version: hiroute_integrations::codex_subscription_client_version(
                &source.executable,
            ),
            source_path_digest,
            _source_lock: source_lock,
            flat_path: auth_dir.join(MANAGED_FILE_NAME),
            state_path: auth_dir.join(STATE_FILE_NAME),
            observed_generation: None,
        };
        lease.refresh_expected(expected, None)?;
        Ok(lease)
    }

    fn refresh_expected(
        &mut self,
        expected: Option<&BorrowedCodexEvidence>,
        expected_account_digest: Option<&str>,
    ) -> Result<ManagedAccountIdentity, CpaLifecycleError> {
        let source = read_nested_source(&self.canonical_source)?;
        let observed = BorrowedCodexEvidence::from_source(&self.canonical_source, &source)?;
        if expected.is_some_and(|value| !value.matches(&observed)) {
            return Err(CpaLifecycleError::BorrowedCodexAuthSourceChanged);
        }
        let account_digest = account_digest(source.account_id.as_str());
        if expected_account_digest.is_some_and(|expected| expected != account_digest) {
            return Err(CpaLifecycleError::BorrowedCodexAuthSourceChanged);
        }
        let previous = load_state(&self.state_path)?;
        if previous
            .as_ref()
            .is_some_and(|state| state.source_path_digest != self.source_path_digest)
        {
            return Err(CpaLifecycleError::InvalidBorrowedCodexAuth);
        }
        if previous
            .as_ref()
            .is_some_and(|state| state.account_digest != account_digest)
        {
            return Err(CpaLifecycleError::BorrowedCodexAuthSourceChanged);
        }

        let prefix = "hiroute-codex-current".to_owned();
        let rendered = render_access_only_auth(&source, &prefix, self.client_version.as_deref())?;
        let revision_digest = revision_digest(&source);

        let changed = previous.as_ref().is_none_or(|state| {
            state.revision_digest != revision_digest
                || state.source_len != source.stamp.len
                || state.source_mtime_secs != source.stamp.mtime_secs
                || state.source_mtime_nanos != source.stamp.mtime_nanos
        });
        let generation = match previous.as_ref() {
            Some(state) if changed => state
                .generation
                .checked_add(1)
                .ok_or(CpaLifecycleError::InvalidBorrowedCodexAuth)?,
            Some(state) => state.generation,
            None => 1,
        };

        let flat_exists = self.flat_path.exists();
        if flat_exists {
            validate_private_file(&self.flat_path)
                .map_err(|_| CpaLifecycleError::InvalidBorrowedCodexAuth)?;
        }
        let flat_matches = flat_exists
            && private_file_matches(
                &self.flat_path,
                &source,
                &prefix,
                self.client_version.as_deref(),
            )?;
        if previous.is_none() && flat_exists && !flat_matches {
            return Err(CpaLifecycleError::InvalidBorrowedCodexAuth);
        }
        if !flat_matches {
            private_atomic_write(&self.flat_path, &rendered)
                .map_err(|_| CpaLifecycleError::BorrowedCodexAuthIo)?;
        }

        let state = BorrowedState {
            schema: STATE_SCHEMA.to_owned(),
            stock_contract_version: STOCK_CPA_CONTRACT_VERSION.to_owned(),
            source_path_digest: self.source_path_digest.clone(),
            account_digest: account_digest.clone(),
            revision_digest,
            generation,
            source_len: source.stamp.len,
            source_mtime_secs: source.stamp.mtime_secs,
            source_mtime_nanos: source.stamp.mtime_nanos,
            managed_file_name: MANAGED_FILE_NAME.to_owned(),
            request_retry: 0,
            disable_cooling: true,
            refresh_token_present: false,
        };
        save_state(&self.state_path, &state)?;
        self.observed_generation = Some(generation);
        Ok(ManagedAccountIdentity {
            account_kind: CpaAccountKind::Codex,
            stock_file_name: MANAGED_FILE_NAME.to_owned(),
            account_digest,
            generation,
        })
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct SourceStamp {
    len: u64,
    mtime_secs: u64,
    mtime_nanos: u32,
}

#[derive(Deserialize)]
struct NestedAuth<'a> {
    #[serde(borrow)]
    auth_mode: &'a str,
    #[serde(rename = "OPENAI_API_KEY", default)]
    api_key: Option<&'a str>,
    #[serde(borrow)]
    last_refresh: &'a str,
    #[serde(borrow)]
    tokens: NestedTokens<'a>,
}

#[derive(Deserialize)]
struct NestedTokens<'a> {
    #[serde(borrow)]
    access_token: &'a str,
    #[serde(borrow)]
    id_token: &'a str,
    #[serde(borrow)]
    account_id: &'a str,
}

#[derive(Serialize)]
struct FlatAccessLease<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    access_token: &'a str,
    id_token: &'a str,
    account_id: &'a str,
    last_refresh: &'a str,
    prefix: &'a str,
    request_retry: u8,
    disable_cooling: bool,
    hiroute_client_version: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExistingFlatAccessLease<'a> {
    #[serde(rename = "type", borrow)]
    kind: &'a str,
    #[serde(borrow)]
    access_token: &'a str,
    #[serde(borrow)]
    id_token: &'a str,
    #[serde(borrow)]
    account_id: &'a str,
    #[serde(borrow)]
    last_refresh: &'a str,
    #[serde(borrow)]
    prefix: &'a str,
    request_retry: i64,
    disable_cooling: bool,
    #[serde(borrow)]
    hiroute_client_version: Option<&'a str>,
    #[serde(default, borrow)]
    refresh_token: Option<&'a str>,
    #[serde(default, borrow)]
    email: Option<&'a str>,
    #[serde(default, borrow)]
    expired: Option<&'a str>,
    #[serde(default)]
    disabled: Option<bool>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BorrowedState {
    schema: String,
    stock_contract_version: String,
    source_path_digest: String,
    account_digest: String,
    revision_digest: String,
    generation: u64,
    source_len: u64,
    source_mtime_secs: u64,
    source_mtime_nanos: u32,
    managed_file_name: String,
    request_retry: u8,
    disable_cooling: bool,
    refresh_token_present: bool,
}

fn read_nested_source(path: &Path) -> Result<OwnedNestedSource, CpaLifecycleError> {
    read_nested_source_after_read(path, || {})
}

fn read_nested_source_after_read(
    path: &Path,
    mut after_read: impl FnMut(),
) -> Result<OwnedNestedSource, CpaLifecycleError> {
    for _ in 0..SOURCE_READ_ATTEMPTS {
        let mut file = open_source_nofollow(path)?;
        let before = file.metadata().map_err(map_source_io)?;
        validate_source_metadata(&before)?;
        let mut bytes = Zeroizing::new(Vec::new());
        file.by_ref()
            .take(MAX_AUTH_BYTES + 1)
            .read_to_end(bytes.as_mut())
            .map_err(map_source_io)?;
        if bytes.len() as u64 > MAX_AUTH_BYTES {
            return Err(CpaLifecycleError::InvalidBorrowedCodexAuth);
        }
        after_read();
        let after = file.metadata().map_err(map_source_io)?;
        let path_after = fs::symlink_metadata(path).map_err(map_source_io)?;
        if !same_file(&before, &after) || !same_file(&after, &path_after) {
            continue;
        }
        let stamp = source_stamp(&after)?;
        let parsed: NestedAuth<'_> = serde_json::from_slice(bytes.as_slice())
            .map_err(|_| CpaLifecycleError::InvalidBorrowedCodexAuth)?;
        validate_nested(&parsed)?;

        // The returned references must not outlive the zeroized input. Consume the parsed fields
        // immediately into a second zeroizing buffer in the caller-facing representation.
        return Ok(OwnedNestedSource {
            access_token: Zeroizing::new(parsed.tokens.access_token.to_owned()),
            id_token: Zeroizing::new(parsed.tokens.id_token.to_owned()),
            account_id: Zeroizing::new(parsed.tokens.account_id.to_owned()),
            last_refresh: Zeroizing::new(parsed.last_refresh.to_owned()),
            stamp,
        });
    }
    Err(CpaLifecycleError::BorrowedCodexAuthSourceChanged)
}

struct OwnedNestedSource {
    access_token: Zeroizing<String>,
    id_token: Zeroizing<String>,
    account_id: Zeroizing<String>,
    last_refresh: Zeroizing<String>,
    stamp: SourceStamp,
}

fn validate_nested(value: &NestedAuth<'_>) -> Result<(), CpaLifecycleError> {
    if value.auth_mode != "chatgpt"
        || value.api_key.is_some()
        || !valid_secret(value.tokens.access_token, MAX_TOKEN_BYTES)
        || !valid_secret(value.tokens.id_token, MAX_TOKEN_BYTES)
        || !valid_text(value.tokens.account_id, MAX_ACCOUNT_ID_BYTES)
        || !valid_text(value.last_refresh, MAX_LAST_REFRESH_BYTES)
    {
        return Err(CpaLifecycleError::InvalidBorrowedCodexAuth);
    }
    Ok(())
}

fn valid_secret(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.contains(['\r', '\n', '\0'])
}

fn valid_text(value: &str, max: usize) -> bool {
    valid_secret(value, max) && !value.chars().any(char::is_control)
}

fn render_access_only_auth(
    source: &OwnedNestedSource,
    prefix: &str,
    client_version: Option<&str>,
) -> Result<Zeroizing<Vec<u8>>, CpaLifecycleError> {
    serde_json::to_vec(&FlatAccessLease {
        kind: "codex",
        access_token: source.access_token.as_str(),
        id_token: source.id_token.as_str(),
        account_id: source.account_id.as_str(),
        last_refresh: source.last_refresh.as_str(),
        prefix,
        request_retry: 0,
        disable_cooling: true,
        hiroute_client_version: client_version,
    })
    .map(Zeroizing::new)
    .map_err(|_| CpaLifecycleError::InvalidBorrowedCodexAuth)
}

fn revision_digest(source: &OwnedNestedSource) -> String {
    digest_parts(
        b"hiroute.borrowed-codex-revision/v1\0",
        &[
            source.access_token.as_bytes(),
            source.id_token.as_bytes(),
            source.account_id.as_bytes(),
            source.last_refresh.as_bytes(),
        ],
    )
}

pub(crate) fn account_digest(account_id: &str) -> String {
    digest_parts(
        b"hiroute.cpa-account/v1\0",
        &[b"codex", account_id.as_bytes()],
    )
}

fn digest_parts(domain: &[u8], parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    for part in parts {
        hasher.update(part);
        hasher.update(b"\0");
    }
    format!("{:x}", hasher.finalize())
}

fn path_digest(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"hiroute.borrowed-codex-source-path/v1\0");
    update_path_digest(&mut hasher, path);
    format!("{:x}", hasher.finalize())
}

#[cfg(unix)]
fn update_path_digest(hasher: &mut Sha256, path: &Path) {
    use std::os::unix::ffi::OsStrExt as _;
    hasher.update(path.as_os_str().as_bytes());
}

#[cfg(windows)]
fn update_path_digest(hasher: &mut Sha256, path: &Path) {
    use std::os::windows::ffi::OsStrExt as _;
    for unit in path.as_os_str().encode_wide() {
        hasher.update(unit.to_le_bytes());
    }
}

#[cfg(not(any(unix, windows)))]
fn update_path_digest(hasher: &mut Sha256, path: &Path) {
    hasher.update(path.to_string_lossy().as_bytes());
}

fn canonical_private_source(path: &Path) -> Result<PathBuf, CpaLifecycleError> {
    if !path.is_absolute() {
        return Err(CpaLifecycleError::InvalidBorrowedCodexAuth);
    }
    let metadata = fs::symlink_metadata(path).map_err(map_source_io)?;
    validate_source_metadata(&metadata)?;
    let canonical = path.canonicalize().map_err(map_source_io)?;
    let canonical_metadata = fs::symlink_metadata(&canonical).map_err(map_source_io)?;
    validate_source_metadata(&canonical_metadata)?;
    if !same_file(&metadata, &canonical_metadata) {
        return Err(CpaLifecycleError::InvalidBorrowedCodexAuth);
    }
    Ok(canonical)
}

#[cfg(unix)]
fn validate_source_metadata(metadata: &fs::Metadata) -> Result<(), CpaLifecycleError> {
    use std::os::unix::fs::MetadataExt as _;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.nlink() != 1
    {
        return Err(CpaLifecycleError::InvalidBorrowedCodexAuth);
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_source_metadata(metadata: &fs::Metadata) -> Result<(), CpaLifecycleError> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(CpaLifecycleError::InvalidBorrowedCodexAuth);
    }
    Ok(())
}

#[cfg(unix)]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
}

#[cfg(not(unix))]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.len() == right.len() && left.modified().ok() == right.modified().ok()
}

fn source_stamp(metadata: &fs::Metadata) -> Result<SourceStamp, CpaLifecycleError> {
    let modified = metadata
        .modified()
        .map_err(|_| CpaLifecycleError::InvalidBorrowedCodexAuth)?
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CpaLifecycleError::InvalidBorrowedCodexAuth)?;
    Ok(SourceStamp {
        len: metadata.len(),
        mtime_secs: modified.as_secs(),
        mtime_nanos: modified.subsec_nanos(),
    })
}

#[cfg(unix)]
fn open_source_nofollow(path: &Path) -> Result<File, CpaLifecycleError> {
    use rustix::fs::{Mode, OFlags};
    rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|error| map_source_io(std::io::Error::from(error)))
}

#[cfg(not(unix))]
fn open_source_nofollow(path: &Path) -> Result<File, CpaLifecycleError> {
    let metadata = fs::symlink_metadata(path).map_err(map_source_io)?;
    validate_source_metadata(&metadata)?;
    File::open(path).map_err(map_source_io)
}

fn map_source_io(error: std::io::Error) -> CpaLifecycleError {
    if error.kind() == std::io::ErrorKind::NotFound {
        CpaLifecycleError::BorrowedCodexAuthMissing
    } else {
        CpaLifecycleError::BorrowedCodexAuthIo
    }
}

fn source_lease_root() -> PathBuf {
    std::env::temp_dir().join(source_lease_root_name())
}

#[cfg(unix)]
fn source_lease_root_name() -> String {
    format!(
        "hiroute-cpa-borrowed-leases-v1-{}",
        rustix::process::geteuid().as_raw()
    )
}

#[cfg(not(unix))]
fn source_lease_root_name() -> String {
    "hiroute-cpa-borrowed-leases-v1".to_owned()
}

fn acquire_lock(path: &Path) -> Result<File, CpaLifecycleError> {
    let parent = path
        .parent()
        .ok_or(CpaLifecycleError::BorrowedCodexAuthIo)?;
    ensure_private_dir(parent).map_err(|_| CpaLifecycleError::BorrowedCodexAuthIo)?;
    let file = open_lock_nofollow(path)?;
    validate_private_file(path).map_err(|_| CpaLifecycleError::InvalidBorrowedCodexAuth)?;
    match file.try_lock_exclusive() {
        Ok(()) => Ok(file),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            Err(CpaLifecycleError::BorrowedCodexAuthAlreadyLeased)
        }
        Err(_) => Err(CpaLifecycleError::BorrowedCodexAuthIo),
    }
}

#[cfg(unix)]
fn open_lock_nofollow(path: &Path) -> Result<File, CpaLifecycleError> {
    use rustix::fs::{Mode, OFlags};
    rustix::fs::open(
        path,
        OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::RUSR | Mode::WUSR,
    )
    .map(File::from)
    .map_err(|_| CpaLifecycleError::BorrowedCodexAuthIo)
}

#[cfg(not(unix))]
fn open_lock_nofollow(path: &Path) -> Result<File, CpaLifecycleError> {
    if path
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(CpaLifecycleError::InvalidBorrowedCodexAuth);
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .map_err(|_| CpaLifecycleError::BorrowedCodexAuthIo)
}

fn private_file_matches(
    path: &Path,
    source: &OwnedNestedSource,
    prefix: &str,
    client_version: Option<&str>,
) -> Result<bool, CpaLifecycleError> {
    let file = File::open(path).map_err(|_| CpaLifecycleError::BorrowedCodexAuthIo)?;
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX_AUTH_BYTES + 1)
        .read_to_end(bytes.as_mut())
        .map_err(|_| CpaLifecycleError::BorrowedCodexAuthIo)?;
    if bytes.len() as u64 > MAX_AUTH_BYTES {
        return Err(CpaLifecycleError::InvalidBorrowedCodexAuth);
    }
    let existing: ExistingFlatAccessLease<'_> = serde_json::from_slice(bytes.as_slice())
        .map_err(|_| CpaLifecycleError::InvalidBorrowedCodexAuth)?;
    Ok(existing.kind == "codex"
        && existing.hiroute_client_version == client_version
        && existing.access_token == source.access_token.as_str()
        && existing.id_token == source.id_token.as_str()
        && existing.account_id == source.account_id.as_str()
        && existing.last_refresh == source.last_refresh.as_str()
        && existing.prefix == prefix
        && existing.request_retry == 0
        && existing.disable_cooling
        && existing
            .refresh_token
            .is_none_or(|value| value.trim().is_empty())
        && existing.email.is_none_or(str::is_empty)
        && existing.expired.is_none_or(str::is_empty)
        && existing.disabled.is_none_or(|value| !value))
}

fn load_state(path: &Path) -> Result<Option<BorrowedState>, CpaLifecycleError> {
    if !path.exists() {
        return Ok(None);
    }
    validate_private_file(path).map_err(|_| CpaLifecycleError::InvalidBorrowedCodexAuth)?;
    let bytes = fs::read(path).map_err(|_| CpaLifecycleError::BorrowedCodexAuthIo)?;
    if bytes.len() as u64 > MAX_AUTH_BYTES {
        return Err(CpaLifecycleError::InvalidBorrowedCodexAuth);
    }
    let state: BorrowedState =
        serde_json::from_slice(&bytes).map_err(|_| CpaLifecycleError::InvalidBorrowedCodexAuth)?;
    if state.schema != STATE_SCHEMA
        || state.stock_contract_version != STOCK_CPA_CONTRACT_VERSION
        || state.managed_file_name != MANAGED_FILE_NAME
        || state.generation == 0
        || state.request_retry != 0
        || !state.disable_cooling
        || state.refresh_token_present
        || !valid_digest(&state.source_path_digest)
        || !valid_digest(&state.account_digest)
        || !valid_digest(&state.revision_digest)
    {
        return Err(CpaLifecycleError::InvalidBorrowedCodexAuth);
    }
    Ok(Some(state))
}

fn save_state(path: &Path, state: &BorrowedState) -> Result<(), CpaLifecycleError> {
    let bytes =
        serde_json::to_vec(state).map_err(|_| CpaLifecycleError::InvalidBorrowedCodexAuth)?;
    private_atomic_write(path, &bytes).map_err(|_| CpaLifecycleError::BorrowedCodexAuthIo)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests;
