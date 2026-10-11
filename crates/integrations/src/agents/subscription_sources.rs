//! Protected native subscription source resolution.
//!
//! Discovery returns only an opaque descriptor plus an in-process path capability. It never
//! reads OAuth bytes, creates a CPA lease, or exposes a filesystem locator through serde/Debug.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use hiroute_application::compute_management::ProtectedInputSourceDescriptorV1;
use hiroute_domain::CanonicalDigest;

use super::{AgentFilesystemScanError, FilesystemAgentScannerV1};

/// Native subscription identity; credential formats remain adapter-owned.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSubscriptionKind {
    Codex,
    Claude,
}

impl AgentSubscriptionKind {
    pub const fn stock_provider(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
    pub const fn connector_id(self) -> &'static str {
        match self {
            Self::Codex => "connector.cpa.codex",
            Self::Claude => "connector.cpa.claude",
        }
    }
    pub const fn connection_option_id(self) -> &'static str {
        match self {
            Self::Codex => "codex.subscription.global.v1",
            Self::Claude => "claude.subscription.global.v1",
        }
    }
    pub const fn endpoint_profile_id(self) -> &'static str {
        match self {
            Self::Codex => "endpoint.cpa.codex",
            Self::Claude => "endpoint.cpa.claude",
        }
    }
    pub const fn required_protocol(self) -> hiroute_domain::UpstreamProtocol {
        match self {
            Self::Codex => hiroute_domain::UpstreamProtocol::Responses,
            Self::Claude => hiroute_domain::UpstreamProtocol::Messages,
        }
    }
    pub fn supports_protocol(self, protocol: hiroute_domain::UpstreamProtocol) -> bool {
        match self {
            Self::Codex => true,
            Self::Claude => protocol == hiroute_domain::UpstreamProtocol::Messages,
        }
    }
    pub fn from_connector(connector: &str) -> Option<Self> {
        [Self::Codex, Self::Claude]
            .into_iter()
            .find(|kind| kind.connector_id() == connector)
    }
    pub fn from_candidate(candidate: &str) -> Option<Self> {
        [Self::Codex, Self::Claude].into_iter().find(|kind| {
            candidate.starts_with(&format!("candidate/cpa/{}/", kind.stock_provider()))
        })
    }
}

/// A trusted in-process locator. Never serialized or included in Debug output.
#[derive(Clone, Eq, PartialEq)]
pub enum ClaudeSubscriptionLocation {
    File(PathBuf),
    Keychain {
        config_dir: PathBuf,
        service: String,
    },
}

impl ClaudeSubscriptionLocation {
    pub fn anchor(&self) -> &Path {
        match self {
            Self::File(path) => path,
            Self::Keychain { config_dir, .. } => config_dir,
        }
    }
}

pub fn claude_subscription_auth_from_environment(
    home: &Path,
) -> Result<ClaudeSubscriptionLocation, AgentFilesystemScanError> {
    let config = claude_subscription_config(home, None);

    claude_subscription_location(&config)
}

fn claude_subscription_config(home: &Path, selected: Option<&Path>) -> PathBuf {
    match std::env::var_os("CLAUDE_SECURESTORAGE_CONFIG_DIR") {
        Some(value) if value.is_empty() => home.join(".claude"),
        Some(value) => PathBuf::from(value),
        None => selected
            .map(Path::to_owned)
            .or_else(|| {
                std::env::var_os("CLAUDE_CONFIG_DIR")
                    .filter(|v| !v.is_empty())
                    .map(PathBuf::from)
            })
            .unwrap_or_else(|| home.join(".claude")),
    }
}

#[cfg(target_os = "macos")]
fn claude_keychain_service(
    selected: Option<&std::ffi::OsStr>,
) -> Result<String, AgentFilesystemScanError> {
    use sha2::{Digest, Sha256};
    use unicode_normalization::UnicodeNormalization;
    match selected.filter(|value| !value.is_empty()) {
        Some(value) => {
            // The native CLI hashes the selected string after NFC, without canonicalization.
            let normalized = value
                .to_str()
                .ok_or(AgentFilesystemScanError::SourceUnavailable)?
                .nfc()
                .collect::<String>();
            let hash = format!("{:x}", Sha256::digest(normalized.as_bytes()));
            Ok(format!("Claude Code-credentials-{}", &hash[..8]))
        }
        None => Ok("Claude Code-credentials".into()),
    }
}

fn claude_subscription_location(
    config: &Path,
) -> Result<ClaudeSubscriptionLocation, AgentFilesystemScanError> {
    if !config.is_absolute()
        || config
            .components()
            .any(|c| c == std::path::Component::ParentDir)
    {
        return Err(AgentFilesystemScanError::SourceUnavailable);
    }
    // Select Keychain first; the credential adapter uses the file fallback only if
    // the item is absent, never after denial or a locked keychain.
    #[cfg(target_os = "macos")]
    {
        Ok(ClaudeSubscriptionLocation::Keychain {
            config_dir: config.to_owned(),
            service: claude_keychain_service(
                std::env::var_os("CLAUDE_SECURESTORAGE_CONFIG_DIR")
                    .or_else(|| std::env::var_os("CLAUDE_CONFIG_DIR"))
                    .as_deref(),
            )?,
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(ClaudeSubscriptionLocation::File(
            config.join(".credentials.json"),
        ))
    }
}

/// Shared source selection for discovery and CPA. This only selects a path; CPA owns
/// no-follow, stable file identity and access-only credential validation.
pub fn codex_subscription_auth_from_environment(
    home: &Path,
) -> Result<PathBuf, AgentFilesystemScanError> {
    let config =
        super::filesystem::codex_config_path(home, std::env::var_os("CODEX_HOME").as_deref());
    let selected = std::env::var_os("HIROUTE_CODEX_AUTH_SOURCE").map(PathBuf::from);
    subscription_auth_path(&config, selected.as_deref())
}

/// Inspect only the selected user config. No Keychain access, CLI invocation or fallback.
/// Explicit HiRoute auth-file overrides bypass this policy at source selection.
pub fn codex_subscription_uses_file_store(config: &Path) -> Result<bool, AgentFilesystemScanError> {
    let Some((bytes, _)) = super::filesystem_config::read_validated_config_bytes(config)? else {
        return Ok(true); // Codex's default on every platform is `file`.
    };
    let text = std::str::from_utf8(&bytes).map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
    let value: toml_edit::DocumentMut = text
        .parse()
        .map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
    Ok(match value.get("cli_auth_credentials_store") {
        None => true,
        Some(mode) => mode.as_str() == Some("file"),
    })
}

fn subscription_auth_path(
    config: &Path,
    selected: Option<&Path>,
) -> Result<PathBuf, AgentFilesystemScanError> {
    let source = match selected {
        Some(path) => path.to_owned(),
        None => config
            .parent()
            .ok_or(AgentFilesystemScanError::SourceUnavailable)?
            .join("auth.json"),
    };
    if !source.is_absolute()
        || source
            .components()
            .any(|part| part == std::path::Component::ParentDir)
    {
        return Err(AgentFilesystemScanError::SourceUnavailable);
    }
    Ok(source)
}

#[derive(Clone)]
pub struct ProtectedAgentSubscriptionSourceV1 {
    descriptor: ProtectedInputSourceDescriptorV1,
    evidence_digest: CanonicalDigest,
    source_path: PathBuf,
    kind: AgentSubscriptionKind,
    claude_location: Option<ClaudeSubscriptionLocation>,
    codex_store_config: Option<PathBuf>,
}

impl ProtectedAgentSubscriptionSourceV1 {
    pub fn codex_store_config(&self) -> Option<&Path> {
        self.codex_store_config.as_deref()
    }
    pub fn kind(&self) -> AgentSubscriptionKind {
        self.kind
    }
    pub fn claude_location(&self) -> Option<&ClaudeSubscriptionLocation> {
        self.claude_location.as_ref()
    }
    pub fn descriptor(&self) -> &ProtectedInputSourceDescriptorV1 {
        &self.descriptor
    }

    /// Stable discovery evidence for the canonical native-account context. Token revisions stay
    /// private to CPA and do not change the subscription candidate or its audit identity.
    pub fn evidence_digest(&self) -> &CanonicalDigest {
        &self.evidence_digest
    }

    /// Trusted adapter-only capability. This type has no serde or Debug surface.
    pub fn source_path(&self) -> &Path {
        &self.source_path
    }
}

impl FilesystemAgentScannerV1 {
    /// Resolves the selected subscription source without reading the file.
    /// The CPA bridge performs the authoritative no-follow/content validation later.
    pub fn codex_subscription_source(
        &self,
    ) -> Result<Option<ProtectedAgentSubscriptionSourceV1>, AgentFilesystemScanError> {
        let source_path = subscription_auth_path(
            &self.layout.codex_user_config,
            self.layout.codex_subscription_auth_override.as_deref(),
        )?;
        let metadata = match std::fs::symlink_metadata(&source_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(AgentFilesystemScanError::SourceUnavailable),
        };
        if !metadata.file_type().is_file() || metadata.len() == 0 {
            return Err(AgentFilesystemScanError::SourceUnavailable);
        }
        let canonical = std::fs::canonicalize(&source_path)
            .map_err(|_| AgentFilesystemScanError::SourceUnavailable)?;
        let identity = CanonicalDigest::of(&(
            "hiroute.codex-subscription-source/v1",
            canonical.as_os_str().as_encoded_bytes(),
        ))
        .map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
        let source_ref = format!(
            "codex/subscription/{}",
            identity.as_str().trim_start_matches("sha256:")
        );
        let evidence_digest = CanonicalDigest::of(&(
            "hiroute.codex-subscription-discovery-evidence/v2",
            source_ref.as_str(),
        ))
        .map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
        Ok(Some(ProtectedAgentSubscriptionSourceV1 {
            descriptor: ProtectedInputSourceDescriptorV1::DiscoveredConfig {
                scanner_id: "builtin.agent-filesystem".to_owned(),
                scanner_version: "1".to_owned(),
                source_ref,
                field_selector: "native-subscription".to_owned(),
                observed_revision: 1,
            },
            evidence_digest,
            source_path: canonical,
            kind: AgentSubscriptionKind::Codex,
            claude_location: None,
            codex_store_config: self
                .layout
                .codex_subscription_auth_override
                .is_none()
                .then(|| self.layout.codex_user_config.clone()),
        }))
    }
}

impl FilesystemAgentScannerV1 {
    pub fn subscription_source(
        &self,
        kind: AgentSubscriptionKind,
    ) -> Result<Option<ProtectedAgentSubscriptionSourceV1>, AgentFilesystemScanError> {
        if kind == AgentSubscriptionKind::Codex {
            return self.codex_subscription_source();
        }
        let config = self
            .layout
            .claude_user_settings
            .parent()
            .ok_or(AgentFilesystemScanError::SourceUnavailable)?;
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or(AgentFilesystemScanError::SourceUnavailable)?;
        let config = claude_subscription_config(&home, Some(config));
        let location = claude_subscription_location(&config)?;
        let metadata = match std::fs::symlink_metadata(location.anchor()) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(AgentFilesystemScanError::SourceUnavailable),
        };
        if metadata.file_type().is_symlink()
            || match &location {
                ClaudeSubscriptionLocation::File(_) => !metadata.is_file() || metadata.len() == 0,
                ClaudeSubscriptionLocation::Keychain { .. } => !metadata.is_dir(),
            }
        {
            return Err(AgentFilesystemScanError::SourceUnavailable);
        }
        let canonical = location
            .anchor()
            .canonicalize()
            .map_err(|_| AgentFilesystemScanError::SourceUnavailable)?;
        let identity = CanonicalDigest::of(&(
            "hiroute.claude-subscription-source/v1",
            canonical.as_os_str().as_encoded_bytes(),
            match &location {
                ClaudeSubscriptionLocation::File(_) => None,
                ClaudeSubscriptionLocation::Keychain { service, .. } => Some(service.as_str()),
            },
        ))
        .map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
        let source_ref = format!(
            "claude/subscription/{}",
            identity.as_str().trim_start_matches("sha256:")
        );
        let evidence_digest = CanonicalDigest::of(&(
            "hiroute.claude-subscription-discovery-evidence/v1",
            &source_ref,
        ))
        .map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
        Ok(Some(ProtectedAgentSubscriptionSourceV1 {
            descriptor: ProtectedInputSourceDescriptorV1::DiscoveredConfig {
                scanner_id: "builtin.agent-filesystem".into(),
                scanner_version: "1".into(),
                source_ref,
                field_selector: "native-subscription".into(),
                observed_revision: 1,
            },
            evidence_digest,
            source_path: canonical,
            kind,
            claude_location: Some(location),
            codex_store_config: None,
        }))
    }
}

#[cfg(all(test, target_os = "macos"))]
mod keychain_tests {
    use super::*;
    use std::ffi::OsStr;
    #[test]
    fn native_service_selection_preserves_explicit_and_unicode_store_identity() {
        assert_eq!(
            claude_keychain_service(None).unwrap(),
            "Claude Code-credentials"
        );
        assert_eq!(
            claude_keychain_service(Some(OsStr::new(""))).unwrap(),
            "Claude Code-credentials"
        );
        assert_ne!(
            claude_keychain_service(Some(OsStr::new("/Users/test/.claude"))).unwrap(),
            "Claude Code-credentials"
        );
        assert_eq!(
            claude_keychain_service(Some(OsStr::new("/tmp/café"))).unwrap(),
            claude_keychain_service(Some(OsStr::new("/tmp/cafe\u{301}"))).unwrap()
        );
        assert_ne!(
            claude_keychain_service(Some(OsStr::new("/tmp/a/../b"))).unwrap(),
            claude_keychain_service(Some(OsStr::new("/tmp/b"))).unwrap()
        );
    }
}
