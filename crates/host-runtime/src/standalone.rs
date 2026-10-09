use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const STANDALONE_INSTALL_SCHEMA_V1: &str = "hiroute.standalone-install/v1";
const MAX_MARKER_BYTES: u64 = 16 * 1024;

/// Shared by Desktop, standalone and their CLI. Keep this no longer than control.sock
/// so credential input does not impose a stricter native Unix socket path limit.
pub fn protected_input_socket(runtime_root: impl AsRef<Path>) -> PathBuf {
    runtime_root.as_ref().join("hiroute/input.sock")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StandaloneLayout {
    pub home: PathBuf,
    pub state_root: PathBuf,
    pub data_root: PathBuf,
    pub runtime_root: PathBuf,
    pub marker_path: PathBuf,
}

impl StandaloneLayout {
    pub fn from_environment() -> Result<Self, StandaloneLayoutError> {
        Self::from_values(
            cfg!(target_os = "macos"),
            std::env::var_os("HOME").map(PathBuf::from),
            std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
            std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
            std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        )
    }

    pub fn from_values(
        macos: bool,
        home: Option<PathBuf>,
        xdg_state: Option<PathBuf>,
        xdg_data: Option<PathBuf>,
        xdg_runtime: Option<PathBuf>,
    ) -> Result<Self, StandaloneLayoutError> {
        let home = home
            .filter(|path| path.is_absolute())
            .ok_or(StandaloneLayoutError)?;
        let marker_path = home.join(".local/share/hiroute/standalone.json");
        if macos {
            let root = home.join("Library/Application Support/ai.hiroute.cli");
            return Ok(Self {
                data_root: home.join(".local/share/hiroute"),
                home,
                state_root: root.clone(),
                runtime_root: root.join("run"),
                marker_path,
            });
        }
        let state_root = absolute_or(xdg_state, home.join(".local/state")).join("hiroute");
        let data_root = absolute_or(xdg_data, home.join(".local/share")).join("hiroute");
        let runtime_root = xdg_runtime
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| state_root.join("run"));
        Ok(Self {
            home,
            state_root,
            data_root,
            runtime_root,
            marker_path,
        })
    }

    pub fn storage_root(&self) -> PathBuf {
        self.state_root.join("storage")
    }

    pub fn gateway_lkg(&self) -> PathBuf {
        self.state_root.join("gateway.lkg")
    }

    pub fn gateway_config_root(&self) -> &Path {
        &self.state_root
    }

    pub fn diagnostics_root(&self) -> PathBuf {
        self.state_root.join("diagnostics")
    }

    pub fn protected_input_socket(&self) -> PathBuf {
        protected_input_socket(&self.runtime_root)
    }
}

fn absolute_or(value: Option<PathBuf>, fallback: PathBuf) -> PathBuf {
    value.filter(|path| path.is_absolute()).unwrap_or(fallback)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StandaloneInstallRecordV1 {
    pub schema_version: String,
    pub version: String,
    pub target: String,
    pub install_root: PathBuf,
    pub service_definition: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub installed_skills: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpa_binary: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpa_sha256: Option<String>,
}

impl StandaloneInstallRecordV1 {
    pub fn validate(&self) -> Result<(), StandaloneLayoutError> {
        let portable = |value: &str, max: usize| {
            !value.is_empty()
                && value.len() <= max
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        };
        if self.schema_version != STANDALONE_INSTALL_SCHEMA_V1
            || !portable(&self.version, 64)
            || !portable(&self.target, 128)
            || !self.install_root.is_absolute()
            || !self.service_definition.is_absolute()
            || self.installed_skills.iter().any(|path| !path.is_absolute())
            || self
                .installed_skills
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.installed_skills.len()
            || self
                .cpa_binary
                .as_ref()
                .is_some_and(|path| !path.is_absolute())
            || self.cpa_binary.is_some() != self.cpa_sha256.is_some()
            || self.cpa_sha256.as_ref().is_some_and(|digest| {
                digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return Err(StandaloneLayoutError);
        }
        Ok(())
    }
}

pub fn read_standalone_install_record(
    path: &Path,
) -> Result<StandaloneInstallRecordV1, StandaloneLayoutError> {
    let mut file = marker_file(path)?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_MARKER_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| StandaloneLayoutError)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_MARKER_BYTES {
        return Err(StandaloneLayoutError);
    }
    let record: StandaloneInstallRecordV1 =
        serde_json::from_slice(&bytes).map_err(|_| StandaloneLayoutError)?;
    record.validate()?;
    Ok(record)
}

#[cfg(unix)]
fn marker_file(path: &Path) -> Result<std::fs::File, StandaloneLayoutError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| StandaloneLayoutError)?;
    let metadata = file.metadata().map_err(|_| StandaloneLayoutError)?;
    if !metadata.is_file()
        || metadata.uid() != nix::unistd::geteuid().as_raw()
        || metadata.permissions().mode() & 0o022 != 0
    {
        return Err(StandaloneLayoutError);
    }
    Ok(file)
}

#[cfg(not(unix))]
fn marker_file(path: &Path) -> Result<std::fs::File, StandaloneLayoutError> {
    std::fs::File::open(path).map_err(|_| StandaloneLayoutError)
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("STANDALONE_LAYOUT_INVALID")]
pub struct StandaloneLayoutError;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_and_macos_layouts_are_isolated_and_absolute() {
        let linux = StandaloneLayout::from_values(
            false,
            Some("/home/test".into()),
            None,
            None,
            Some("/run/user/1000".into()),
        )
        .unwrap();
        assert_eq!(
            linux.state_root,
            PathBuf::from("/home/test/.local/state/hiroute")
        );
        assert_eq!(linux.runtime_root, PathBuf::from("/run/user/1000"));
        assert_eq!(
            linux.marker_path,
            PathBuf::from("/home/test/.local/share/hiroute/standalone.json")
        );

        let mac = StandaloneLayout::from_values(true, Some("/Users/test".into()), None, None, None)
            .unwrap();
        assert_eq!(
            mac.state_root,
            PathBuf::from("/Users/test/Library/Application Support/ai.hiroute.cli")
        );
        assert_ne!(
            mac.state_root,
            PathBuf::from("/Users/test/Library/Application Support/ai.hiroute.desktop")
        );
        assert_eq!(
            mac.data_root,
            PathBuf::from("/Users/test/.local/share/hiroute")
        );
        assert!(
            StandaloneLayout::from_values(false, Some("relative".into()), None, None, None)
                .is_err()
        );
    }
}
