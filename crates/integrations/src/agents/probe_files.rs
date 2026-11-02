//! Private files for admitted native checks; no borrowed user root is adopted.
#[cfg(not(windows))]
use std::fs;
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
};

pub(super) struct PrivateProbeRoot {
    cleanup: tempfile::TempDir,
    path: PathBuf,
}

impl PrivateProbeRoot {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn keep(self) -> PathBuf {
        self.cleanup.keep()
    }

    fn from_cleanup(cleanup: tempfile::TempDir) -> io::Result<Self> {
        #[cfg(windows)]
        let path = {
            let path = cleanup.path().join("private");
            hiroute_platform::windows::security::create_owner_only_directory(&path)?;
            path
        };
        #[cfg(not(windows))]
        let path = cleanup.path().to_owned();
        Ok(Self { cleanup, path })
    }
}

pub(super) fn temporary(prefix: &str) -> io::Result<PrivateProbeRoot> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(prefix);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    PrivateProbeRoot::from_cleanup(builder.tempdir()?)
}

/// Retain the private object while giving native shells an ordinary drive name.
#[cfg(windows)]
pub(super) fn native_directory(path: &Path) -> io::Result<(PathBuf, File)> {
    use hiroute_platform::windows::security;
    use std::path::{Component, Prefix};
    let disk = |path: &Path| {
        matches!(
            path.components().next(),
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        )
    };
    if !path.is_absolute() || !disk(path) {
        return Err(invalid());
    }
    // Inspect the original private object before canonicalization could hide aliases.
    let held = security::open_owner_only(path, true)?;
    let canonical = std::fs::canonicalize(path)?;
    let mut components = canonical.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return Err(invalid());
    };
    let drive = match prefix.kind() {
        Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) if drive.is_ascii_alphabetic() => drive,
        _ => return Err(invalid()),
    };
    if components.next() != Some(Component::RootDir) {
        return Err(invalid());
    }
    let mut ordinary = PathBuf::from(format!("{}:\\", char::from(drive)));
    for part in components {
        let Component::Normal(name) = part else {
            return Err(invalid());
        };
        ordinary.push(name);
    }
    let roundtrip = security::open_owner_only(&ordinary, true)?;
    if std::fs::canonicalize(&ordinary)? != canonical
        || security::file_identity(&held)? != security::file_identity(&roundtrip)?
    {
        return Err(invalid());
    }
    Ok((ordinary, held))
}

#[cfg(windows)]
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "native private workspace identity",
    )
}

pub(super) fn directory(path: &Path) -> io::Result<()> {
    hiroute_platform::private_fs::create_directory(path)
}

pub(super) fn file(path: &Path) -> io::Result<File> {
    hiroute_platform::private_fs::create_file(path, hiroute_platform::private_fs::FileMode::Data)
}

pub(super) fn executable(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use hiroute_platform::{file_security, windows::security};

    #[test]
    fn windows_probe_files_attach_private_user_owner_without_adopting_outer_owner() {
        let cleanup = tempfile::tempdir().unwrap();
        let outer = security::open_identity(cleanup.path(), true).unwrap();
        let before = file_security::capture(&outer).unwrap();
        let root = PrivateProbeRoot::from_cleanup(cleanup).unwrap();
        let private = security::open_owner_only(root.path(), true).unwrap();
        security::verify_owner_only_handle(&private, true).unwrap();
        let child = file(&root.path().join("capture")).unwrap();
        security::verify_owner_only_handle(&child, false).unwrap();
        assert_eq!(
            file_security::capture(&outer).unwrap(),
            before,
            "creating private materials must preserve the cleanup container's owner and ACL"
        );
        assert_ne!(root.path(), root.cleanup.path());
    }
}
