//! Metadata-only OpenCode native installation selection, never wrapper execution or hashing.
use super::{QoderNativeError, qoder_error};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn package(root: &Path) -> Result<Option<Value>, QoderNativeError> {
    let Some((bytes, _)) =
        super::filesystem_config::read_system_config_bytes(&root.join("package.json"))
            .map_err(|_| qoder_error("OpenCode package metadata"))?
    else {
        return Ok(None);
    };
    if bytes.len() > 64 * 1024 {
        return Err(qoder_error("OpenCode package metadata bound"));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| qoder_error("OpenCode package metadata"))
}

/// Resolves the explicitly located official npm wrapper to a single declared native package.
/// A directly selected standalone binary remains selectable without format capability.
pub fn opencode_native_executable(selected: &Path) -> Result<Option<PathBuf>, QoderNativeError> {
    let Some(path) =
        super::executable::resolve(selected).map_err(|_| qoder_error("OpenCode executable"))?
    else {
        return Ok(None);
    };
    #[cfg(windows)]
    let path = if path
        .extension()
        .is_some_and(|e| e.as_encoded_bytes().eq_ignore_ascii_case(b"cmd"))
    {
        let mut identities = Vec::new();
        let entry = super::windows_runtime::windows_npm_entry_with_inspection(&path, |path| {
            identities.push(hiroute_platform::windows::security::open_identity(
                path, false,
            )?);
            Ok(())
        })
        .map_err(|_| qoder_error("OpenCode npm entry"))?;
        super::executable::resolve(&entry)
            .map_err(|_| qoder_error("OpenCode npm entry"))?
            .ok_or_else(|| qoder_error("OpenCode npm entry"))?
    } else {
        path
    };
    let root = path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| qoder_error("OpenCode installation"))?;
    let metadata = package(root)?;
    if metadata.as_ref().and_then(|m| m["name"].as_str()) != Some("opencode-ai") {
        if path
            .extension()
            .is_some_and(|x| x == "cmd" || x == "js" || x == "mjs")
        {
            return Err(qoder_error("OpenCode native executable required"));
        }
        return Ok(Some(path));
    }
    let version = metadata
        .as_ref()
        .and_then(|m| m["version"].as_str())
        .ok_or_else(|| qoder_error("OpenCode package version"))?;
    let modules = [
        root.join("node_modules"),
        root.parent()
            .ok_or_else(|| qoder_error("OpenCode package root"))?
            .to_owned(),
    ];
    let mut candidates = Vec::new();
    for modules in modules.iter().filter(|p| p.is_dir()) {
        for entry in std::fs::read_dir(modules)
            .map_err(|_| qoder_error("OpenCode platform packages"))?
            .take(256)
        {
            let entry = entry.map_err(|_| qoder_error("OpenCode platform packages"))?;
            if !entry.file_name().to_string_lossy().starts_with("opencode-") {
                continue;
            }
            let Some(meta) = package(&entry.path())? else {
                continue;
            };
            if meta["version"].as_str() != Some(version)
                || !platform_matches(&meta)
                || !official_package(&meta)
                || metadata
                    .as_ref()
                    .and_then(|m| m["optionalDependencies"].get(meta["name"].as_str()?))
                    .and_then(Value::as_str)
                    != Some(version)
            {
                continue;
            }
            let binary = entry.path().join("bin").join(if cfg!(windows) {
                "opencode.exe"
            } else {
                "opencode"
            });
            if let Some(executable) = super::executable::resolve(&binary)
                .map_err(|_| qoder_error("OpenCode platform binary"))?
            {
                candidates.push(executable);
            }
        }
    }
    candidates.sort();
    candidates.dedup();
    if candidates.len() != 1 {
        return Err(qoder_error("ambiguous OpenCode platform package"));
    }
    Ok(candidates.pop())
}

fn platform_matches(meta: &Value) -> bool {
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(windows) {
        "win32"
    } else {
        "linux"
    };
    let cpu = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86_64") {
        "x64"
    } else {
        "unsupported"
    };
    let exact = |key: &str, expected: &str| {
        meta[key]
            .as_array()
            .is_some_and(|values| values.len() == 1 && values[0].as_str() == Some(expected))
    };
    exact("os", os)
        && exact("cpu", cpu)
        && match meta.get("libc") {
            None => !cfg!(target_env = "musl"),
            Some(_) => exact(
                "libc",
                if cfg!(target_env = "musl") {
                    "musl"
                } else {
                    "glibc"
                },
            ),
        }
}

/// Bounded audited format evidence, independent of general discovery/selection availability.
pub fn opencode_native_format(
    selected: &Path,
) -> Result<&'static [&'static str], QoderNativeError> {
    let path = opencode_native_executable(selected)?
        .ok_or_else(|| qoder_error("OpenCode executable missing"))?;
    let root = path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| qoder_error("OpenCode package root"))?;
    let meta = package(root)?.ok_or_else(|| qoder_error("OpenCode format metadata unavailable"))?;
    let name = meta["name"]
        .as_str()
        .ok_or_else(|| qoder_error("OpenCode package name"))?;
    if !official_package(&meta)
        || root.file_name().and_then(|s| s.to_str()) != Some(name)
        || path.file_name().and_then(|s| s.to_str())
            != Some(if cfg!(windows) {
                "opencode.exe"
            } else {
                "opencode"
            })
        || meta["version"] != "1.19.3"
        || !platform_matches(&meta)
    {
        return Err(qoder_error("OpenCode format metadata unsupported"));
    }
    Ok(super::opencode_migrations::IDS)
}

fn official_package(meta: &Value) -> bool {
    matches!(
        meta["name"].as_str(),
        Some(
            "opencode-darwin-arm64"
                | "opencode-darwin-x64"
                | "opencode-darwin-x64-baseline"
                | "opencode-linux-x64"
                | "opencode-linux-x64-baseline"
                | "opencode-linux-x64-musl"
                | "opencode-linux-x64-baseline-musl"
                | "opencode-linux-arm64"
                | "opencode-linux-arm64-musl"
                | "opencode-windows-x64"
                | "opencode-windows-x64-baseline"
                | "opencode-windows-arm64"
        )
    )
}

#[cfg(all(test, windows))]
#[path = "opencode_installation_windows_tests.rs"]
mod windows_tests;
