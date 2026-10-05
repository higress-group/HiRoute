//! Bind the Worker SDK to the user's selected official npm CLI, without executing it.
use std::path::{Path, PathBuf};

pub const PI_WORKER_VERSION: &str = "1.0.2";
pub const PI_NPM_PACKAGE: &str = "@earendil-works/pi-coding-agent";

pub struct PiSdkInstallation {
    pub package_root: PathBuf,
}

pub fn pi_sdk_installation(
    cli: &Path,
) -> Result<PiSdkInstallation, super::AgentFilesystemScanError> {
    use super::AgentFilesystemScanError::SourceUnavailable;
    let cli = std::fs::canonicalize(cli).map_err(|_| SourceUnavailable)?;
    let root = cli.ancestors().nth(3).ok_or(SourceUnavailable)?;
    let metadata = std::fs::metadata(root.join("package.json")).map_err(|_| SourceUnavailable)?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 {
        return Err(SourceUnavailable);
    }
    let bytes = std::fs::read(root.join("package.json")).map_err(|_| SourceUnavailable)?;
    let package: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| SourceUnavailable)?;
    if package["name"] != PI_NPM_PACKAGE
        || package["version"] != PI_WORKER_VERSION
        || package
            .pointer("/bin/pi")
            .and_then(serde_json::Value::as_str)
            != Some("dist/bundle/cli.js")
        || root.join("dist/bundle/cli.js") != cli
        || !root.join("dist/index.js").is_file()
    {
        return Err(SourceUnavailable);
    }
    Ok(PiSdkInstallation {
        package_root: root.to_owned(),
    })
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
