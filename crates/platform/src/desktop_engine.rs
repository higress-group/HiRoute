use std::path::{Path, PathBuf};

/// A desktop application's engine, located either as an ordinary file or by
/// the current user's Windows package registration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesktopEngineLocation {
    File(PathBuf),
    #[cfg(windows)]
    Package(crate::windows::app_package::RegisteredPackageExecutable),
}

impl DesktopEngineLocation {
    pub fn path(&self) -> &Path {
        match self {
            Self::File(path) => path,
            #[cfg(windows)]
            Self::Package(executable) => executable.path(),
        }
    }
}
