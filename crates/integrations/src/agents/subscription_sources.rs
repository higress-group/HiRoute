//! Protected native subscription source resolution.
//!
//! Discovery returns only an opaque descriptor plus an in-process path capability. It never
//! reads OAuth bytes, creates a CPA lease, or exposes a filesystem locator through serde/Debug.

use std::path::{Path, PathBuf};

use hiroute_application::compute_management::ProtectedInputSourceDescriptorV1;
use hiroute_domain::CanonicalDigest;

use super::{AgentFilesystemScanError, FilesystemAgentScannerV1};

/// Shared source selection for discovery and CPA. This only selects a path; CPA owns
/// no-follow, ownership, mode and access-only credential validation.
pub fn codex_subscription_auth_from_environment(
    home: &Path,
) -> Result<PathBuf, AgentFilesystemScanError> {
    let config =
        super::filesystem::codex_config_path(home, std::env::var_os("CODEX_HOME").as_deref());
    let selected = std::env::var_os("HIROUTE_CODEX_AUTH_SOURCE").map(PathBuf::from);
    subscription_auth_path(&config, selected.as_deref())
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
}

impl ProtectedAgentSubscriptionSourceV1 {
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
    /// The CPA bridge performs the authoritative no-follow/owner/mode/content validation later.
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
        }))
    }
}
