//! CLI identity is independent of its release label and of path-specific SDK capabilities.
use std::path::{Path, PathBuf};

pub const PI_NPM_PACKAGE: &str = "@earendil-works/pi-coding-agent";
pub const PI_SDK_CONTRACT: &str = include_str!("pi_sdk_contract.mjs");

pub struct PiCliInstallation {
    pub package_root: PathBuf,
    pub version: String,
    pub manifest_digest: hiroute_domain::CanonicalDigest,
}

pub fn pi_cli_installation(
    cli: &Path,
) -> Result<PiCliInstallation, super::AgentFilesystemScanError> {
    use super::AgentFilesystemScanError::SourceUnavailable;
    let cli = std::fs::canonicalize(cli).map_err(|_| SourceUnavailable)?;
    // npm/pnpm may move the entry; the owning manifest, not a depth or filename,
    // must bind its declared bin to this exact canonical CLI.
    for root in cli.ancestors().skip(1).take(8) {
        let manifest = root.join("package.json");
        let Ok(metadata) = std::fs::metadata(&manifest) else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > 64 * 1024 {
            continue;
        }
        let bytes = std::fs::read(&manifest).map_err(|_| SourceUnavailable)?;
        let package: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| SourceUnavailable)?;
        if package["name"] != PI_NPM_PACKAGE {
            continue;
        }
        let bin = package
            .pointer("/bin/pi")
            .and_then(serde_json::Value::as_str)
            .ok_or(SourceUnavailable)?;
        let entry = std::fs::canonicalize(root.join(bin)).map_err(|_| SourceUnavailable)?;
        let version = package["version"]
            .as_str()
            .filter(|v| !v.is_empty() && v.len() <= 128)
            .ok_or(SourceUnavailable)?;
        if entry != cli || !entry.starts_with(root) {
            return Err(SourceUnavailable);
        }
        return Ok(PiCliInstallation {
            package_root: root.to_owned(),
            version: version.into(),
            manifest_digest: hiroute_domain::CanonicalDigest::of_bytes(&bytes),
        });
    }
    Err(SourceUnavailable)
}

#[derive(Clone, Copy)]
pub enum PiSdkCapability {
    Models,
    Collaboration,
    Worker,
    Continue,
}

/// Load only the SDK exported by the selected CLI's package. No native prompt,
/// user credential store, helper, extension or network model refresh is used.
#[cfg(unix)]
pub fn check_pi_sdk_capability(
    cli: &Path,
    node: &Path,
    capability: PiSdkCapability,
) -> Result<(), super::AgentFilesystemScanError> {
    use super::AgentFilesystemScanError::SourceUnavailable;
    use std::io::Write;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    pi_cli_installation(cli)?;
    validate_pi_node(node)?;
    let mut script = tempfile::Builder::new()
        .suffix(".mjs")
        .tempfile()
        .map_err(|_| SourceUnavailable)?;
    script
        .write_all(PI_SDK_CONTRACT.as_bytes())
        .map_err(|_| SourceUnavailable)?;
    let scope = match capability {
        PiSdkCapability::Models => "models",
        PiSdkCapability::Collaboration => "collaboration",
        PiSdkCapability::Worker => "worker",
        PiSdkCapability::Continue => "continue",
    };
    let capture = tempfile::NamedTempFile::new().map_err(|_| SourceUnavailable)?;
    let child = Command::new(node)
        .arg(script.path())
        .arg("--check")
        .arg(cli)
        .arg(scope)
        .env("PI_OFFLINE", "1")
        .env("PI_TELEMETRY", "0")
        .env_remove("NODE_OPTIONS")
        .env_remove("HIROUTE_RUN_TOKEN")
        .stdin(Stdio::null())
        .stdout(capture.reopen().map_err(|_| SourceUnavailable)?)
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|_| SourceUnavailable)?;
    let mut child = super::NativeProbeProcess::new(child);
    let start = Instant::now();
    loop {
        match child.observe().map_err(|_| SourceUnavailable)? {
            Some(true) => break,
            Some(false) => return Err(SourceUnavailable),
            None => {}
        }
        if start.elapsed() > Duration::from_secs(5) {
            return Err(SourceUnavailable);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let bytes = super::native_probe_process::read_bounded(capture.path(), 128)
        .map_err(|_| SourceUnavailable)?;
    child.stop().map_err(|_| SourceUnavailable)?;
    if bytes.as_slice() != b"hiroute.pi-sdk-capability/v1:ok\n" {
        return Err(SourceUnavailable);
    }
    Ok(())
}

pub const PI_NODE_MINIMUM: (u32, u32, u32) = (22, 19, 0);

/// One bounded native version read; neither stdout nor stderr reaches product diagnostics.
#[cfg(unix)]
pub fn validate_pi_node(node: &Path) -> Result<(), super::AgentFilesystemScanError> {
    use super::AgentFilesystemScanError::SourceUnavailable;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let capture = tempfile::NamedTempFile::new().map_err(|_| SourceUnavailable)?;
    let child = Command::new(node)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(capture.reopen().map_err(|_| SourceUnavailable)?)
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|_| SourceUnavailable)?;
    let mut child = super::NativeProbeProcess::new(child);
    let start = Instant::now();
    loop {
        match child.observe().map_err(|_| SourceUnavailable)? {
            Some(true) => break,
            Some(false) => return Err(SourceUnavailable),
            None => {}
        }
        if start.elapsed() > Duration::from_secs(2) {
            return Err(SourceUnavailable);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let bytes = super::native_probe_process::read_bounded(capture.path(), 64)
        .map_err(|_| SourceUnavailable)?;
    child.stop().map_err(|_| SourceUnavailable)?;
    let raw = std::str::from_utf8(&bytes)
        .map_err(|_| SourceUnavailable)?
        .trim()
        .strip_prefix('v')
        .ok_or(SourceUnavailable)?;
    let version = raw
        .split('.')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| SourceUnavailable)?;
    if version.len() != 3 || (version[0], version[1], version[2]) < PI_NODE_MINIMUM {
        return Err(SourceUnavailable);
    }
    Ok(())
}
