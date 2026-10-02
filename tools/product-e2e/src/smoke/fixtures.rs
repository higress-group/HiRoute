//! Synthetic initial inputs shared with production boundary regression tests.
use super::{Result, require};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

/// Establish current-format storage through the real daemon before injecting untrusted files.
/// The subsequent start tests catalog authority, rather than unsupported-source admission.
pub fn initialize_current_storage(
    daemon: &Path,
    storage: &Path,
    logs: &Path,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    deadline: std::time::Instant,
) -> Result<()> {
    use std::os::unix::fs::FileTypeExt;
    let isolated = tempfile::tempdir()?;
    fs::set_permissions(isolated.path(), fs::Permissions::from_mode(0o700))?;
    let endpoint = isolated.path().join("runtime/hiroute/control.sock");
    let mut command = std::process::Command::new(daemon);
    command
        .env_clear()
        .env("HOME", isolated.path())
        .env("PATH", "/usr/bin:/bin")
        .args(["--role", "control", "--storage-root"])
        .arg(storage)
        .arg("--runtime-root")
        .arg(isolated.path().join("runtime"))
        .args(["--diagnostic-level-override", "debug"]);
    let mut child = super::process::Process::spawn(&mut command, logs, cancel)?;
    loop {
        if let Err(error) = child.check() {
            let (_, stderr) = child.output(16 * 1024)?;
            eprintln!(
                "initial storage daemon: {}",
                String::from_utf8_lossy(&stderr)
            );
            return Err(error);
        }
        if fs::symlink_metadata(&endpoint).is_ok_and(|m| m.file_type().is_socket()) {
            break;
        }
        require(
            std::time::Instant::now() < deadline,
            "initial_storage_ready_timeout",
        )?;
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    child.stop()?;
    let (_, stderr) = child.output(1024 * 1024)?;
    require(stderr.is_empty(), "initial_storage_unexpected_stderr")
}

pub fn install_storage_catalog_tampering(storage: &Path) -> Result<()> {
    let release_facts = storage.join("release-facts");
    let root = release_facts.join("current");
    fs::create_dir_all(&root)?;
    for directory in [storage.to_path_buf(), release_facts, root.clone()] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }
    for (name, bytes) in [
        ("manifest.json", b"{\"schema\":\"untrusted\"}\n".as_slice()),
        ("connector-registry.json", b"{}\n".as_slice()),
        ("model-data.json", b"{}\n".as_slice()),
    ] {
        let path = root.join(name);
        fs::write(&path, bytes)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn discovery(root: &Path, secret: &str) -> Result<(PathBuf, PathBuf)> {
    let home = root.join("agent-home");
    let bin = root.join("agent-bin");
    fs::create_dir_all(home.join(".claude"))?;
    fs::create_dir_all(&bin)?;
    // These are synthetic homes; native artifact registration rejects writable
    // parents. Do not inherit the caller's umask for this fixture contract.
    for directory in [&home, &home.join(".claude"), &bin] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }
    let profiles: Value = serde_json::from_str(include_str!(
        "../../../../assets/release-facts/current/bundle/agent-profiles.json"
    ))?;
    for (binary, profile_id, prefix, diagnostic_version) in [
        ("codex", "codex-responses-v1", "codex-cli", Some("99.99.99")),
        (
            "claude",
            "claude-messages-v1",
            "Claude Code",
            Some("unknown"),
        ),
    ] {
        let profile = profiles["profiles"]
            .as_array()
            .and_then(|p| p.iter().find(|p| p["profile_id"] == profile_id))
            .ok_or(super::SmokeError("invalid_agent_fixture"))?;
        let version = diagnostic_version
            .or_else(|| profile["diagnostic_version"].as_str())
            .ok_or(super::SmokeError("invalid_agent_fixture"))?;
        require(
            version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b)),
            "invalid_agent_fixture",
        )?;
        let path = bin.join(binary);
        fs::write(
            &path,
            format!("#!/bin/sh\nprintf '%s\\n' '{prefix} {version}'\n"),
        )?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    let settings = home.join(".claude/settings.json");
    fs::write(
        &settings,
        serde_json::to_vec(&json!({"env":{
        "ANTHROPIC_BASE_URL":"https://open.bigmodel.cn/api/anthropic",
        "ANTHROPIC_MODEL":"claude-opus-5", "ANTHROPIC_DEFAULT_OPUS_MODEL":"glm-5.3[1m]",
        "ANTHROPIC_AUTH_TOKEN":secret}}))?,
    )?;
    fs::set_permissions(settings, fs::Permissions::from_mode(0o600))?;
    Ok((home, bin))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_fixture_removes_group_writable_agent_parents() {
        let root = tempfile::tempdir().unwrap();
        // Deterministically reproduce a permissive caller umask without changing
        // the test process's global umask or touching a real Agent home.
        let home = root.path().join("agent-home");
        let claude = home.join(".claude");
        fs::create_dir_all(&claude).unwrap();
        for path in [&home, &claude] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o775)).unwrap();
        }
        discovery(root.path(), "fixture-secret").unwrap();
        for path in [&home, &claude] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o022,
                0,
                "native artifact parent must not be group/world writable: {}",
                path.display()
            );
        }
    }
}
