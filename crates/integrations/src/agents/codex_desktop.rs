//! One separately owned Desktop context; the ordinary Codex home remains borrowed.
use std::path::{Path, PathBuf};

use super::{AgentFilesystemScanError, FilesystemAgentDiscoveryV1, FilesystemAgentScannerV1};

pub const CODEX_DESKTOP_AGENT_ID: &str = "agent_codex_desktop";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexDesktopInstancePaths {
    pub root: PathBuf,
    pub codex_home: PathBuf,
    pub user_data: PathBuf,
}

impl CodexDesktopInstancePaths {
    /// Native history is outside the Core storage group and its upgrade snapshots. Include
    /// the storage basename so independent daemon roots never acquire the same instance.
    pub fn for_storage_root(storage: &Path) -> Result<Self, AgentFilesystemScanError> {
        if !storage.is_absolute() {
            return Err(AgentFilesystemScanError::InvalidConfig);
        }
        let mut name = storage
            .file_name()
            .ok_or(AgentFilesystemScanError::InvalidConfig)?
            .to_os_string();
        name.push(".codex-desktop");
        Ok(Self::at_root(storage.with_file_name(name)))
    }

    pub fn at_root(root: PathBuf) -> Self {
        Self {
            codex_home: root.join("codex"),
            user_data: root.join("desktop"),
            root,
        }
    }
}

impl FilesystemAgentScannerV1 {
    pub fn with_dynamic_codex_desktop_location(mut self) -> Self {
        self.dynamic_codex_desktop_location = true;
        self
    }

    pub fn refresh_codex_desktop_location(&self, resolve: impl FnOnce() -> Option<PathBuf>) {
        if !self.dynamic_codex_desktop_location {
            return;
        }
        let current = resolve();
        let changed = if let Ok(mut location) = self.codex_desktop_location.lock()
            && *location != current
        {
            *location = current;
            true
        } else {
            false
        };
        if changed {
            self.invalidate_codex_ingress();
        }
    }

    pub(super) fn codex_desktop_engine_path(&self) -> Option<PathBuf> {
        self.codex_desktop_location.lock().ok()?.clone()
    }

    pub fn with_codex_desktop_instance(mut self, paths: CodexDesktopInstancePaths) -> Self {
        self.layout.codex_desktop_instance_root = Some(paths.root);
        self
    }

    pub fn codex_desktop_instance_paths(&self) -> Option<CodexDesktopInstancePaths> {
        self.layout
            .codex_desktop_instance_root
            .clone()
            .map(CodexDesktopInstancePaths::at_root)
    }

    pub fn codex_desktop_config_target(&self) -> Option<PathBuf> {
        Some(
            self.codex_desktop_instance_paths()?
                .codex_home
                .join("config.toml"),
        )
    }

    pub fn codex_home_overlaps_desktop(&self) -> bool {
        self.codex_desktop_config_target().is_some_and(|target| {
            super::canonical_codex_config_path(&target)
                == super::canonical_codex_config_path(&self.codex_user_config_target())
        })
    }

    /// Reuse the native Codex reader with this exact home and the application-owned engine.
    /// The independently installed CLI is never a fallback for the Desktop surface.
    pub fn codex_desktop_scanner(&self) -> Option<Self> {
        let paths = self.codex_desktop_instance_paths()?;
        let mut scanner = self.clone();
        scanner.layout.codex_user_config = paths.codex_home.join("config.toml");
        scanner.layout.codex_subscription_auth_override = None;
        scanner.layout.codex_executable = self
            .codex_desktop_engine_path()
            .unwrap_or_else(|| paths.root.join("desktop-not-installed"));
        Some(scanner)
    }

    pub fn codex_desktop_settings_discovery(&self) -> Option<FilesystemAgentDiscoveryV1> {
        let scanner = self.codex_desktop_scanner()?;
        let mut discovery = scanner.scan_codex_for(CODEX_DESKTOP_AGENT_ID);
        if let super::AgentDiscoveryOutcomeV1::Supported { installation } = &mut discovery.outcome {
            super::observed_capabilities::attach_target_file_capabilities(
                installation,
                &scanner.layout.codex_user_config,
            );
        }
        Some(discovery)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_is_outside_core_and_distinct_for_neighboring_storage_roots() {
        let root = tempfile::tempdir().unwrap();
        let first =
            CodexDesktopInstancePaths::for_storage_root(&root.path().join("first")).unwrap();
        let second =
            CodexDesktopInstancePaths::for_storage_root(&root.path().join("second")).unwrap();
        assert_ne!(first.root, second.root);
        assert!(!first.root.starts_with(root.path().join("first")));
        assert_eq!(first.codex_home, first.root.join("codex"));
        assert_eq!(first.user_data, first.root.join("desktop"));
    }
}
