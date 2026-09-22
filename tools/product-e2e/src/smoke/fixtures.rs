//! Synthetic initial inputs shared with production boundary regression tests.
use super::{Result, require};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

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
