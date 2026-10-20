//! CLI identity is independent of its release label and of path-specific SDK capabilities.
use std::path::{Path, PathBuf};

pub const PI_NPM_PACKAGE: &str = "@earendil-works/pi-coding-agent";
pub const PI_SDK_CONTRACT: &str = include_str!("pi_sdk_contract.mjs");

/// Native SDKs append different resource paths: Anthropic owns `/v1/messages`,
/// while OpenAI Responses appends `/responses`. Used by saved and Worker routes.
pub fn pi_native_provider_api(
    protocol: hiroute_domain::AgentIngressProtocolV1,
    gateway_v1_base: &str,
) -> Result<(&'static str, &str), super::AgentFilesystemScanError> {
    let root = gateway_v1_base
        .strip_suffix("/v1")
        .ok_or(super::AgentFilesystemScanError::InvalidConfig)?;
    Ok(match protocol {
        hiroute_domain::AgentIngressProtocolV1::Responses => ("openai-responses", gateway_v1_base),
        hiroute_domain::AgentIngressProtocolV1::Messages => ("anthropic-messages", root),
        hiroute_domain::AgentIngressProtocolV1::ChatCompletions => {
            return Err(super::AgentFilesystemScanError::InvalidConfig);
        }
    })
}

pub struct PiCliInstallation {
    pub package_root: PathBuf,
    pub version: String,
    pub manifest_digest: hiroute_domain::CanonicalDigest,
}

pub fn pi_cli_installation(
    cli: &Path,
) -> Result<PiCliInstallation, super::AgentFilesystemScanError> {
    pi_cli_installation_with_inspection(cli, |_| Ok(()))
}

/// Inspect only files the existing package reader uses: qualifying manifests
/// before their bytes are read, then the declared entry before canonicalizing.
/// Callers can retain native leases without changing the nearest-package search
/// or interpreting npm declarations a second time.
pub fn pi_cli_installation_with_inspection(
    cli: &Path,
    mut inspect: impl FnMut(&Path) -> std::io::Result<()>,
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
        inspect(&manifest).map_err(|_| SourceUnavailable)?;
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
        let declared = root.join(bin);
        inspect(&declared).map_err(|_| SourceUnavailable)?;
        let entry = std::fs::canonicalize(declared).map_err(|_| SourceUnavailable)?;
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
#[cfg(any(unix, windows))]
pub fn check_pi_sdk_capability(
    cli: &Path,
    node: &Path,
    capability: PiSdkCapability,
) -> Result<(), NativeDependencyFailureV1> {
    use std::io::Write;
    use std::process::Command;
    use std::time::Duration;
    let failure = |reason| NativeDependencyFailureV1 {
        check: NativeDependencyCheckV1::PiSdk,
        reason,
    };
    pi_cli_installation(cli).map_err(|_| NativeDependencyFailureV1 {
        check: NativeDependencyCheckV1::PiPackage,
        reason: NativeDependencyFailureReasonV1::Unsupported,
    })?;
    validate_pi_node(node)?;
    let scripts = super::probe_files::temporary("hiroute-pi-sdk-")
        .map_err(|_| failure(NativeDependencyFailureReasonV1::Unavailable))?;
    let script = scripts.path().join("capability.mjs");
    {
        let mut output = super::probe_files::file(&script)
            .map_err(|_| failure(NativeDependencyFailureReasonV1::Unavailable))?;
        output
            .write_all(PI_SDK_CONTRACT.as_bytes())
            .map_err(|_| failure(NativeDependencyFailureReasonV1::Unavailable))?;
    }
    let scope = match capability {
        PiSdkCapability::Models => "models",
        PiSdkCapability::Collaboration => "collaboration",
        PiSdkCapability::Worker => "worker",
        PiSdkCapability::Continue => "continue",
    };
    let mut command = Command::new(node);
    command
        .arg(&script)
        .arg("--check")
        .arg(cli)
        .arg(scope)
        .env("PI_OFFLINE", "1")
        .env("PI_TELEMETRY", "0");
    let bytes = bounded_check(
        command,
        NativeDependencyCheckV1::PiSdk,
        Duration::from_secs(15),
        128,
    )?;
    if bytes.as_slice() != b"hiroute.pi-sdk-capability/v1:ok\n" {
        return Err(failure(NativeDependencyFailureReasonV1::InvalidOutput));
    }
    Ok(())
}

pub const PI_NODE_MINIMUM: (u32, u32, u32) = (22, 19, 0);

/// One bounded native version read; neither stdout nor stderr reaches product diagnostics.
#[cfg(any(unix, windows))]
pub fn validate_pi_node(node: &Path) -> Result<(), NativeDependencyFailureV1> {
    use std::process::Command;
    use std::time::Duration;
    let failure = |reason| NativeDependencyFailureV1 {
        check: NativeDependencyCheckV1::PiNodeVersion,
        reason,
    };
    let mut command = Command::new(node);
    command.arg("--version");
    let bytes = bounded_check(
        command,
        NativeDependencyCheckV1::PiNodeVersion,
        Duration::from_secs(10),
        64,
    )?;
    let raw = std::str::from_utf8(&bytes)
        .map_err(|_| failure(NativeDependencyFailureReasonV1::InvalidOutput))?
        .trim()
        .strip_prefix('v')
        .ok_or(failure(NativeDependencyFailureReasonV1::InvalidOutput))?;
    let version = raw
        .split('.')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| failure(NativeDependencyFailureReasonV1::InvalidOutput))?;
    if version.len() != 3 {
        return Err(failure(NativeDependencyFailureReasonV1::InvalidOutput));
    }
    if (version[0], version[1], version[2]) < PI_NODE_MINIMUM {
        return Err(failure(NativeDependencyFailureReasonV1::Unsupported));
    }
    Ok(())
}

#[cfg(any(unix, windows))]
use hiroute_domain::delegation::{
    NativeDependencyCheckV1, NativeDependencyFailureReasonV1, NativeDependencyFailureV1,
};

#[cfg(any(unix, windows))]
fn bounded_check(
    mut command: std::process::Command,
    check: NativeDependencyCheckV1,
    timeout: std::time::Duration,
    output_limit: usize,
) -> Result<zeroize::Zeroizing<Vec<u8>>, NativeDependencyFailureV1> {
    command
        .env_remove("NODE_OPTIONS")
        .env_remove("HIROUTE_RUN_TOKEN");
    super::native_probe_process::capture(command, true, timeout, output_limit)
        .map_err(|reason| NativeDependencyFailureV1 { check, reason })
}

#[cfg(all(test, unix))]
#[path = "pi_runtime_tests.rs"]
mod tests;
