//! The first release owns one fixed standalone profile per original CODEX_HOME.
use super::AgentFilesystemScanError;
use super::filesystem_config::{read_system_config_bytes, read_validated_config_bytes};
use hiroute_domain::CanonicalDigest;
use std::path::{Path, PathBuf};
use toml_edit::DocumentMut;

pub const CODEX_STANDALONE_PROFILE_ID: &str = "codex-standalone-profile-v1";
pub const CODEX_MANAGED_PROFILE_NAME: &str = "hiroute";

/// Hash every inherited file, including absence. Existing user preferences can be merged;
/// only an existing journal may authorize replacing the reserved HiRoute provider.
pub fn codex_profile_dependency_digest(
    root: &Path,
    owned: bool,
) -> Result<CanonicalDigest, AgentFilesystemScanError> {
    let profile = root.with_file_name("hiroute.config.toml");
    let mut sampled = Vec::new();
    let mut paths = codex_system_paths()?
        .into_iter()
        .map(|path| (path, true))
        .collect::<Vec<_>>();
    paths.extend([(root.to_owned(), false), (profile.clone(), false)]);
    for (path, system) in paths {
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
            let text =
                std::str::from_utf8(&bytes).map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
            let doc = text
                .parse::<DocumentMut>()
                .map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
            if (path != profile || !owned)
                && (doc
                    .get("model_providers")
                    .and_then(|v| v.get("hiroute"))
                    .is_some()
                    || doc.get("profiles").and_then(|v| v.get("hiroute")).is_some()
                    || doc.get("model_provider").and_then(|v| v.as_str()) == Some("hiroute")
                    || doc.get("profile").and_then(|v| v.as_str())
                        == Some(CODEX_MANAGED_PROFILE_NAME))
            {
                return Err(AgentFilesystemScanError::InvalidConfig);
            }
        }
    }
    CanonicalDigest::of(&sampled).map_err(|_| AgentFilesystemScanError::InvalidConfig)
}

/// Native loader namespace for the frozen Codex 0.158.0/0.160.1 contracts.
/// Windows resolves ProgramData from the OS rather than a process-controlled variable.
pub(super) fn codex_system_paths() -> Result<Vec<PathBuf>, AgentFilesystemScanError> {
    #[cfg(windows)]
    {
        let base = hiroute_platform::windows::background::program_data()
            .map_err(|_| AgentFilesystemScanError::SourceUnavailable)?
            .join("OpenAI/Codex");
        Ok(vec![
            base.join("config.toml"),
            base.join("requirements.toml"),
        ])
    }
    #[cfg(not(windows))]
    {
        Ok(vec![
            "/etc/codex/config.toml".into(),
            "/etc/codex/managed_config.toml".into(),
        ])
    }
}

#[cfg(any(unix, windows))]
pub(super) fn codex_has_system_configuration() -> Result<bool, AgentFilesystemScanError> {
    for path in codex_system_paths()? {
        match std::fs::symlink_metadata(path) {
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(false)
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
        #[cfg(unix)]
        {
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
        }
        #[cfg(windows)]
        {
            let home = Path::new(r"C:\a b' % ! &\codex");
            let commands = codex_profile_commands(home).unwrap();
            let script = format!(
                "function codex {{ [Console]::WriteLine($env:CODEX_HOME); foreach ($argument in $args) {{ [Console]::WriteLine($argument) }} }}; $env:CODEX_HOME = 'previous'; {}; if ($env:CODEX_HOME -ne 'previous') {{ throw 'context was not restored' }}",
                commands["PowerShell"]
            );
            let powershell = hiroute_platform::windows::launcher::system_executable(Path::new(
                r"WindowsPowerShell\v1.0\powershell.exe",
            ))
            .unwrap();
            let mut command = std::process::Command::new(powershell);
            command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
            let output = super::super::windows_runtime::bounded(
                &command,
                true,
                std::time::Duration::from_secs(10),
                4096,
            )
            .unwrap();
            assert_eq!(
                std::str::from_utf8(&output)
                    .unwrap()
                    .lines()
                    .collect::<Vec<_>>(),
                [home.to_str().unwrap(), "--profile", "hiroute"]
            );
        }
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
    fn inheritance_and_existing_preferences_are_preview_dependencies() {
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
        std::fs::write(&root, "profile = 'hiroute'\n").unwrap();
        assert!(codex_profile_dependency_digest(&root, false).is_err());
        assert!(codex_profile_dependency_digest(&root, true).is_err());
        std::fs::write(&root, "profile = 'user_keep'\n").unwrap();
        assert!(codex_profile_dependency_digest(&root, false).is_ok());
        std::fs::write(&root, "").unwrap();
        let profile = root.with_file_name("hiroute.config.toml");
        std::fs::write(&profile, "").unwrap();
        std::fs::set_permissions(&profile, std::fs::Permissions::from_mode(0o600)).unwrap();
        let empty = codex_profile_dependency_digest(&root, false).unwrap();
        std::fs::write(&profile, "# native preferences\nmodel_reasoning_effort = 'high'\nscreen_reader_detection_done = true\n").unwrap();
        assert_ne!(
            empty,
            codex_profile_dependency_digest(&root, false).unwrap()
        );
        assert!(codex_profile_dependency_digest(&root, true).is_ok());
        for conflict in [
            "[model_providers.hiroute]\nname = 'Other owner'\n",
            "model_provider = 'hiroute'\n",
            "[profiles.hiroute]\nmodel = 'native'\n",
            "profile = 'hiroute'\n",
        ] {
            std::fs::write(&profile, conflict).unwrap();
            assert!(codex_profile_dependency_digest(&root, false).is_err());
        }
        std::fs::write(&profile, "model = 'one'\nmodel = 'two'\n").unwrap();
        assert!(codex_profile_dependency_digest(&root, false).is_err());
        assert!(codex_profile_dependency_digest(&root, true).is_err());
    }
}
