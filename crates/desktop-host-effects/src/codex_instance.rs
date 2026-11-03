//! Prepare only the isolated instance. Existing skill entries are never replaced or removed.
use hiroute_platform::private_fs::{open_regular_file, prepare_storage_directory};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
    path::Path,
};

#[derive(Debug, Default, Eq, PartialEq)]
pub struct SharedCodexSkills {
    pub linked: usize,
    pub preserved: usize,
}

pub fn prepare_codex_instance(
    codex_home: &Path,
    user_data: &Path,
    source_home: &Path,
    shared_skills: &Path,
) -> io::Result<SharedCodexSkills> {
    // Two native IPC calls can overlap even when the UI prevents double submission.
    static PREPARATION: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = PREPARATION
        .lock()
        .map_err(|_| io::Error::other("instance preparation unavailable"))?;
    let root = codex_home.parent().ok_or(io::ErrorKind::InvalidInput)?;
    if !root.is_absolute()
        || codex_home == source_home
        || codex_home.file_name() != Some(std::ffi::OsStr::new("codex"))
        || user_data != root.join("desktop")
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    for path in [root, codex_home, user_data] {
        prepare_storage_directory(path).map_err(io::Error::other)?;
    }
    let destination = codex_home.join("skills");
    prepare_storage_directory(&destination).map_err(io::Error::other)?;
    let receipt = root.join("shared-skills.json");
    let mut recorded: BTreeMap<String, String> = match open_regular_file(&receipt) {
        Ok(file) => {
            let mut bytes = Vec::new();
            file.take(1_048_577).read_to_end(&mut bytes)?;
            if bytes.len() > 1_048_576 {
                return Err(io::ErrorKind::InvalidData.into());
            }
            serde_json::from_slice(&bytes)?
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
        Err(error) => return Err(error),
    };
    let entries = match fs::read_dir(source_home.join("skills")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(SharedCodexSkills::default());
        }
        Err(error) => return Err(error),
    };
    let mut result = SharedCodexSkills::default();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str().filter(|name| !name.starts_with('.')) else {
            continue;
        };
        let source = entry.path();
        if !source.is_dir() || !source.join("SKILL.md").is_file() {
            continue;
        }
        // Codex already discovers ~/.agents/skills. Avoid adding a second entry for it.
        if fs::symlink_metadata(shared_skills.join(name)).is_ok() {
            continue;
        }
        let target = destination.join(name);
        match hiroute_platform::directory_link::create(&source, &target) {
            Ok(()) => {
                recorded.insert(name.into(), source.to_string_lossy().into_owned());
                result.linked += 1;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if fs::canonicalize(&target).ok() != fs::canonicalize(&source).ok() {
                    result.preserved += 1;
                }
            }
            Err(error) => return Err(error),
        }
    }
    // This receipt is informational. It never grants authority to delete an entry later.
    if result.linked > 0 {
        let mut file = tempfile::NamedTempFile::new_in(root)?;
        file.write_all(&serde_json::to_vec(&recorded)?)?;
        file.as_file().sync_all()?;
        file.persist(&receipt).map_err(|error| error.error)?;
    }
    Ok(result)
}

#[cfg(target_os = "macos")]
pub(super) fn launch(codex_home: &Path, user_data: &Path) -> Result<(), &'static str> {
    let bundle = super::workspace::codex_desktop_bundle().ok_or("CODEX_DESKTOP_NOT_INSTALLED")?;
    if !bundle.is_absolute()
        || !bundle
            .join("Contents/Resources/codex-cli/bin/codex")
            .is_file()
    {
        return Err("CODEX_DESKTOP_NOT_INSTALLED");
    }
    let env = |key: &str, value: &Path| {
        let mut result = std::ffi::OsString::from(key);
        result.push("=");
        result.push(value);
        result
    };
    let mut user_arg = std::ffi::OsString::from("--user-data-dir=");
    user_arg.push(user_data);
    let mut child = std::process::Command::new("/usr/bin/open")
        .arg("-n")
        .arg("--env")
        .arg(env("CODEX_HOME", codex_home))
        .arg("--env")
        .arg(env("CODEX_SQLITE_HOME", codex_home))
        .arg("--env")
        .arg(env("CODEX_ELECTRON_USER_DATA_PATH", user_data))
        .arg(bundle)
        .arg("--args")
        .arg(user_arg)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "CODEX_DESKTOP_LAUNCH_FAILED")?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err("CODEX_DESKTOP_LAUNCH_FAILED")
                };
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("CODEX_DESKTOP_LAUNCH_FAILED");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codex_desktop_shares_skills_without_adopting_existing_entries() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("instance");
        let source = dir.path().join("ordinary");
        let shared = dir.path().join("agents/skills");
        for name in ["one", "duplicate", "conflict", ".system"] {
            fs::create_dir_all(source.join("skills").join(name)).unwrap();
            fs::write(source.join("skills").join(name).join("SKILL.md"), "skill").unwrap();
        }
        fs::create_dir_all(shared.join("duplicate")).unwrap();
        fs::write(source.join("auth.json"), "keep private").unwrap();
        fs::create_dir_all(root.join("codex/skills/conflict")).unwrap();
        let prepare = || {
            prepare_codex_instance(&root.join("codex"), &root.join("desktop"), &source, &shared)
                .unwrap()
        };
        assert_eq!(
            prepare(),
            SharedCodexSkills {
                linked: 1,
                preserved: 1
            }
        );
        assert!(!root.join("codex/skills/.system").exists());
        assert!(!root.join("codex/skills/duplicate").exists());
        assert!(!root.join("codex/auth.json").exists());
        fs::create_dir_all(source.join("skills/one/scripts")).unwrap();
        fs::write(source.join("skills/one/scripts/run"), "updated").unwrap();
        assert_eq!(
            fs::read_to_string(root.join("codex/skills/one/scripts/run")).unwrap(),
            "updated"
        );
        #[cfg(unix)]
        fs::remove_file(root.join("codex/skills/one")).unwrap();
        #[cfg(windows)]
        fs::remove_dir(root.join("codex/skills/one")).unwrap();
        fs::create_dir(root.join("codex/skills/one")).unwrap();
        assert_eq!(
            prepare(),
            SharedCodexSkills {
                linked: 0,
                preserved: 2
            }
        );
        assert!(root.join("codex/skills/one").is_dir());
        assert_eq!(
            serde_json::from_slice::<BTreeMap<String, String>>(
                &fs::read(root.join("shared-skills.json")).unwrap()
            )
            .unwrap()
            .len(),
            1
        );
    }
}
