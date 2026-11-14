//! Read a known executable by its registered Windows package location.
use super::{path, security};
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisteredPackageExecutable {
    path: PathBuf,
    identity: (u32, u64),
}

impl RegisteredPackageExecutable {
    /// The caller must derive the path from the OS package registration and
    /// manifest. User-supplied or cached executables use ordinary file admission.
    pub fn open_registered(path: PathBuf) -> io::Result<Self> {
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "package executable",
            ));
        }
        let file = path::open_package_file(&path)?;
        Ok(Self {
            path,
            identity: security::file_identity(&file)?,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn reopen(&self) -> io::Result<File> {
        let file = path::open_package_file(&self.path)?;
        if security::file_identity(&file)? != self.identity {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "package executable changed",
            ));
        }
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn package_image_refuses_ancestor_junctions_and_changed_files() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("package");
        let alias = root.path().join("alias");
        fs::create_dir(&package).unwrap();
        let image = package.join("agent.exe");
        fs::write(&image, b"installed image").unwrap();
        let registered = RegisteredPackageExecutable::open_registered(image.clone()).unwrap();
        assert!(registered.reopen().is_ok());
        crate::directory_link::create(&package, &alias).unwrap();
        assert!(RegisteredPackageExecutable::open_registered(alias.join("agent.exe")).is_err());
        fs::remove_dir(alias).unwrap();
        fs::rename(&image, package.join("old.exe")).unwrap();
        fs::write(&image, b"replaced image").unwrap();
        assert!(registered.reopen().is_err());
    }
}
