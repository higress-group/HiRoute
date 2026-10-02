//! The first release owns one fixed standalone profile per original CODEX_HOME.
use super::AgentFilesystemScanError;
use super::filesystem_config::{read_system_config_bytes, read_validated_config_bytes};
use hiroute_domain::CanonicalDigest;
use std::path::{Path, PathBuf};
use toml_edit::DocumentMut;

pub const CODEX_STANDALONE_PROFILE_ID: &str = "codex-standalone-profile-v1";
pub const CODEX_MANAGED_PROFILE_NAME: &str = "hiroute";

/// Hash every inherited file, including absence. Never infer ownership from a filename.
pub fn codex_profile_dependency_digest(
    root: &Path,
    owned: bool,
) -> Result<CanonicalDigest, AgentFilesystemScanError> {
    let profile = root.with_file_name("hiroute.config.toml");
    let mut sampled = Vec::new();
    for (path, system) in [
        (PathBuf::from("/etc/codex/config.toml"), true),
        (PathBuf::from("/etc/codex/managed_config.toml"), true),
        (root.to_owned(), false),
        (profile.clone(), false),
    ] {
        let content = if system {
            read_system_config_bytes(&path)?
        } else {
            read_validated_config_bytes(&path)?
        };
        sampled.push((
            path.clone(),
            content
                .as_ref()
                .map(|(bytes, _)| CanonicalDigest::of_bytes(bytes)),
        ));
        if let Some((bytes, _)) = content {
            if path == profile && !owned {
                return Err(AgentFilesystemScanError::InvalidConfig);
            }
            let text =
                std::str::from_utf8(&bytes).map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
            let doc = text
                .parse::<DocumentMut>()
                .map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
            if path != profile
                && (doc
                    .get("model_providers")
                    .and_then(|v| v.get("hiroute"))
                    .is_some()
                    || doc.get("profiles").and_then(|v| v.get("hiroute")).is_some())
            {
                return Err(AgentFilesystemScanError::InvalidConfig);
            }
        }
    }
    CanonicalDigest::of(&sampled).map_err(|_| AgentFilesystemScanError::InvalidConfig)
}

/// Explicit HOME binding is intentional: the terminal environment is not known to the UI.
pub fn codex_profile_commands(
    home: &Path,
) -> Result<std::collections::BTreeMap<String, String>, AgentFilesystemScanError> {
    let home = home
        .to_str()
        .filter(|v| !v.contains(['\0', '\r', '\n']))
        .ok_or(AgentFilesystemScanError::InvalidConfig)?;
    let posix = format!("'{}'", home.replace('\'', "'\"'\"'"));
    let fish = format!("'{}'", home.replace('\\', "\\\\").replace('\'', "\\'"));
    let powershell = format!("'{}'", home.replace('\'', "''"));
    Ok(std::collections::BTreeMap::from([
        (
            "bash/zsh".into(),
            format!("CODEX_HOME={posix} codex --profile hiroute"),
        ),
        (
            "fish".into(),
            format!("env CODEX_HOME={fish} codex --profile hiroute"),
        ),
        (
            "PowerShell".into(),
            format!(
                "& {{ $previous = $env:CODEX_HOME; try {{ $env:CODEX_HOME = {powershell}; codex --profile hiroute }} finally {{ $env:CODEX_HOME = $previous }} }}"
            ),
        ),
    ]))
}

/// Resolve the existing ancestor without creating files or following a replacement later.
pub fn canonical_codex_config_path(path: &Path) -> PathBuf {
    let Some(parent) = path.parent() else {
        return path.to_owned();
    };
    let mut ancestor = parent.to_owned();
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        let Some(name) = ancestor.file_name().map(|v| v.to_owned()) else {
            return path.to_owned();
        };
        suffix.push(name);
        if !ancestor.pop() {
            return path.to_owned();
        }
    }
    let Ok(mut canonical) = ancestor.canonicalize() else {
        return path.to_owned();
    };
    for name in suffix.iter().rev() {
        canonical.push(name);
    }
    canonical.join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_command_binds_original_home_without_shell_interpolation() {
        let home = Path::new("/tmp/a b'\"$HOME`false`\\codex");
        let commands = codex_profile_commands(home).unwrap();
        let script = format!(
            "codex() {{ printf '%s\\n' \"$CODEX_HOME\" \"$@\"; }}\n{}",
            commands["bash/zsh"]
        );
        let output = std::process::Command::new("bash")
            .args(["--noprofile", "--norc", "-c", &script])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("{}\n--profile\nhiroute\n", home.display())
        );
        assert!(commands["PowerShell"].contains("finally"));
        assert!(codex_profile_commands(Path::new("/tmp/line\nbreak")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn home_symlink_and_missing_suffix_have_one_canonical_identity() {
        let dir = tempfile::tempdir().unwrap();
        let actual = dir.path().join("actual");
        std::fs::create_dir(&actual).unwrap();
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(&actual, &alias).unwrap();
        assert_eq!(
            canonical_codex_config_path(&actual.join("new/config.toml")),
            canonical_codex_config_path(&alias.join("new/config.toml"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn inheritance_and_file_existence_are_preview_dependencies() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = dir.path().join("config.toml");
        let absent = codex_profile_dependency_digest(&root, false).unwrap();
        std::fs::write(&root, "user_option = true\n").unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_ne!(
            absent,
            codex_profile_dependency_digest(&root, false).unwrap()
        );
        std::fs::write(&root, "[model_providers.hiroute]\nenv_key = 'SECRET'\n").unwrap();
        assert!(codex_profile_dependency_digest(&root, true).is_err());
        std::fs::write(&root, "[profiles.hiroute]\nmodel = 'legacy'\n").unwrap();
        assert!(codex_profile_dependency_digest(&root, true).is_err());
        std::fs::write(&root, "").unwrap();
        let profile = root.with_file_name("hiroute.config.toml");
        std::fs::write(&profile, "").unwrap();
        std::fs::set_permissions(&profile, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(codex_profile_dependency_digest(&root, false).is_err());
        assert!(codex_profile_dependency_digest(&root, true).is_ok());
    }
}
