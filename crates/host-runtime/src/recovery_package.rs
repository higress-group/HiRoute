//! Installer-owned previous package handoff for native standalone processes.
use hiroute_platform::{durable, private_fs};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

pub const RETAINED_PACKAGE_FILE: &str = "upgrade-package.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsDesktopRestore {
    pub schema: String,
    pub program: PathBuf,
    pub desktop: Option<PathBuf>,
    pub task_name: String,
    pub task_autostart: bool,
    pub shell_login: bool,
    pub files: BTreeMap<String, PathBuf>,
}
impl WindowsDesktopRestore {
    pub fn validate(&self) -> io::Result<()> {
        if self.schema != "hiroute.windows-desktop-restore/v1"
            || !self.program.is_absolute()
            || self
                .desktop
                .as_ref()
                .is_some_and(|path| !path.is_absolute())
            || self.files.is_empty()
            || self
                .task_name
                .strip_prefix("HiRoute-")
                .is_none_or(|suffix| {
                    suffix.len() != 64
                        || !suffix
                            .bytes()
                            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                })
            || self.files.len() > 6000
            || !self.files.contains_key("installation/standalone.json")
            || !self
                .files
                .contains_key("installation/windows-background.json")
            || self.files.iter().any(|(name, target)| {
                !name.starts_with("installation/")
                    || !target.is_absolute()
                    || name.contains(['\\', ':'])
                    || name.split('/').any(|part| {
                        part.is_empty()
                            || part == "."
                            || part == ".."
                            || !part
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
                    })
            })
        {
            return Err(invalid());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedRecoveryPackage {
    pub schema: String,
    pub storage_root: PathBuf,
    pub package_root: PathBuf,
    pub target_daemon_sha256: String,
    pub files: BTreeMap<String, String>,
}
fn invalid() -> io::Error {
    io::Error::other("UPGRADE_RECOVERY_PACKAGE_INVALID")
}
fn hash(path: &Path) -> io::Result<String> {
    let mut file = private_fs::open_regular_file(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
impl RetainedRecoveryPackage {
    pub fn capture(
        storage_root: &Path,
        package_root: &Path,
        target_daemon: &Path,
    ) -> io::Result<Self> {
        fn collect(
            root: &Path,
            path: &Path,
            files: &mut BTreeMap<String, String>,
        ) -> io::Result<()> {
            let metadata = fs::symlink_metadata(path)?;
            if metadata.file_type().is_symlink() {
                return Err(invalid());
            }
            if metadata.is_dir() {
                for entry in fs::read_dir(path)? {
                    collect(root, &entry?.path(), files)?;
                }
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| invalid())?
                    .to_str()
                    .ok_or_else(invalid)?
                    .replace('\\', "/");
                files.insert(relative, hash(path)?);
            } else {
                return Err(invalid());
            }
            if files.len() > 6000 {
                return Err(invalid());
            }
            Ok(())
        }
        let mut files = BTreeMap::new();
        collect(package_root, package_root, &mut files)?;
        if files.is_empty() {
            return Err(invalid());
        }
        Ok(Self {
            schema: "hiroute.retained-recovery-package/v1".into(),
            storage_root: storage_root.to_path_buf(),
            package_root: package_root.to_path_buf(),
            target_daemon_sha256: hash(target_daemon)?,
            files,
        })
    }
    pub fn store(&self) -> io::Result<()> {
        durable::write_private_atomic(
            &self
                .storage_root
                .parent()
                .ok_or_else(invalid)?
                .join(RETAINED_PACKAGE_FILE),
            &serde_json::to_vec(self)?,
        )
    }
    pub fn read(storage: &Path) -> io::Result<Option<Self>> {
        let path = storage
            .parent()
            .ok_or_else(invalid)?
            .join(RETAINED_PACKAGE_FILE);
        let mut file = match private_fs::open_private_file(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        private_fs::verify_single_link(&file)?;
        let mut bytes = Vec::new();
        file.by_ref()
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 1024 * 1024 {
            return Err(invalid());
        }
        let value: Self = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if value.schema != "hiroute.retained-recovery-package/v1"
            || value.storage_root != storage
            || !value.package_root.is_absolute()
            || value.files.is_empty()
            || value.files.len() > 6000
            || value.target_daemon_sha256.len() != 64
        {
            return Err(invalid());
        }
        Ok(Some(value))
    }
    pub fn verify(&self, target_daemon: &Path) -> io::Result<()> {
        if hash(target_daemon)? != self.target_daemon_sha256 {
            return Err(invalid());
        }
        if !fs::symlink_metadata(&self.package_root)?.is_dir()
            || fs::symlink_metadata(&self.package_root)?
                .file_type()
                .is_symlink()
        {
            return Err(invalid());
        }
        if Self::capture(&self.storage_root, &self.package_root, target_daemon)?.files != self.files
        {
            return Err(invalid());
        }
        for (name, expected) in &self.files {
            if name.is_empty()
                || name.len() > 512
                || name.contains('\\')
                || name.split('/').any(|s| {
                    s.is_empty()
                        || s == "."
                        || s == ".."
                        || !s
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_.@".contains(&b))
                })
            {
                return Err(invalid());
            }
            let path = self.package_root.join(name);
            let mut parent = path.parent().ok_or_else(invalid)?;
            while parent != self.package_root {
                let m = fs::symlink_metadata(parent)?;
                if !m.is_dir() || m.file_type().is_symlink() {
                    return Err(invalid());
                }
                parent = parent.parent().ok_or_else(invalid)?;
            }
            if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_desktop_restore_metadata_rejects_unknown_or_foreign_destinations() {
        let directory = tempfile::tempdir().unwrap();
        let value = WindowsDesktopRestore {
            schema: "hiroute.windows-desktop-restore/v1".into(),
            program: directory.path().join("old-program"),
            desktop: Some(directory.path().join("old-desktop")),
            task_name: format!("HiRoute-{}", "a".repeat(64)),
            task_autostart: false,
            shell_login: true,
            files: BTreeMap::from([
                (
                    "installation/standalone.json".into(),
                    directory.path().join("standalone.json"),
                ),
                (
                    "installation/windows-background.json".into(),
                    directory.path().join("background.json"),
                ),
            ]),
        };
        value.validate().unwrap();
        let mut unknown = serde_json::to_value(&value).unwrap();
        unknown["run_arbitrary_command"] = "ignored fields must not be accepted".into();
        assert!(serde_json::from_value::<WindowsDesktopRestore>(unknown).is_err());
        let mut unsafe_record = value.clone();
        unsafe_record.files.insert(
            "installation/../foreign".into(),
            directory.path().join("foreign"),
        );
        assert!(unsafe_record.validate().is_err());
        unsafe_record = value.clone();
        unsafe_record.desktop = Some("relative-desktop".into());
        assert!(unsafe_record.validate().is_err());
        unsafe_record = value.clone();
        unsafe_record
            .files
            .remove("installation/windows-background.json");
        assert!(unsafe_record.validate().is_err());
        unsafe_record = value;
        unsafe_record.task_name = "unrelated-task".into();
        assert!(unsafe_record.validate().is_err());
    }
    #[test]
    fn retained_source_rejects_changed_package_and_wrong_target() {
        let dir = tempfile::tempdir().unwrap();
        let package = dir.path().join("old");
        private_fs::prepare_storage_directory(&package).unwrap();
        durable::write_private_new(&package.join("hirouted"), b"old program").unwrap();
        let target = dir.path().join("new");
        durable::write_private_new(&target, b"new program").unwrap();
        let storage = dir.path().join("storage");
        let record = RetainedRecoveryPackage::capture(&storage, &package, &target).unwrap();
        record.verify(&target).unwrap();
        record.store().unwrap();
        let loaded = RetainedRecoveryPackage::read(&storage).unwrap().unwrap();
        loaded.verify(&target).unwrap();
        fs::write(&target, b"replaced new program").unwrap();
        assert!(loaded.verify(&target).is_err());
        fs::write(&target, b"new program").unwrap();
        fs::write(package.join("hirouted"), b"wrong old program").unwrap();
        assert!(loaded.verify(&target).is_err());
    }
}
