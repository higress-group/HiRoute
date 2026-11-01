//! Private material primitives. Callers retain namespace, format and lifecycle policy.
//!
//! Storage directory admission preserves released Unix modes on existing directories;
//! it is not permission repair. Borrowed native configuration keeps its own permission
//! snapshot/restore contract and must never be adopted through these creation APIs.
use std::{fs::File, io, path::Path};

#[cfg(unix)]
#[path = "unix.rs"]
mod native;
#[cfg(windows)]
#[path = "windows.rs"]
mod native;

pub use native::{OwnedDirectory, create_directory, create_file, open_regular_file};
mod removal;
pub use removal::{DirectoryRemoval, RemovalProgress, directory_identity};

/// A rejected existing entry is distinct from an OS failure. Storage callers keep
/// their released Permission versus Io error mapping on both platforms.
#[derive(Debug)]
pub enum DirectoryError {
    NotDirectory,
    Io(io::Error),
}

impl From<io::Error> for DirectoryError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl std::fmt::Display for DirectoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotDirectory => f.write_str("storage entry is not a real directory"),
            Self::Io(error) => std::fmt::Display::fmt(error, f),
        }
    }
}
impl std::error::Error for DirectoryError {}

#[derive(Clone, Copy)]
pub enum FileMode {
    Data,
    Executable,
}

pub fn prepare_storage_directory(path: &Path) -> Result<(), DirectoryError> {
    native::prepare_storage_directory(path)
}

/// Read through a regular non-link handle. Windows verifies the private user
/// descriptor; Unix preserves existing mode admission without chmod/adoption.
/// Payload length, format, identity bindings and optional single-link policy
/// remain the caller's responsibility.
pub fn open_private_file(path: &Path) -> io::Result<File> {
    native::open_private_file(path)
}

pub fn verify_single_link(file: &File) -> io::Result<()> {
    native::verify_single_link(file)
}

/// Ephemeral run material: preserve Unix create_new/no-flush behavior and Windows
/// complete private publication. Only declared launcher materials use executable mode.
pub fn write_material(path: &Path, bytes: &[u8], mode: FileMode) -> io::Result<()> {
    native::write_material(path, bytes, mode)
}

#[cfg(test)]
mod tests;
