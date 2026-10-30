use super::{DirectoryError, FileMode};
use crate::windows::security;
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};
use windows_sys::Win32::Foundation::ERROR_DIRECTORY;

pub(super) fn prepare_storage_directory(path: &Path) -> Result<(), DirectoryError> {
    // Classify the native directory-open refusal without a separate path lookup.
    // Normalization and reparse-free traversal must precede any inspection.
    security::ensure_private_directory(path)
        .map(drop)
        .map_err(|error| {
            if error.raw_os_error() == Some(ERROR_DIRECTORY as i32) {
                DirectoryError::NotDirectory
            } else {
                DirectoryError::Io(error)
            }
        })
}

pub fn create_directory(path: &Path) -> io::Result<()> {
    security::create_owner_only_directory(path).map(drop)
}

pub fn create_file(path: &Path, _mode: FileMode) -> io::Result<File> {
    security::create_owner_only_file(path)
}

pub fn open_regular_file(path: &Path) -> io::Result<File> {
    security::open_identity(path, false)
}

pub(super) fn open_private_file(path: &Path) -> io::Result<File> {
    security::open_owner_only(path, false)
}

pub(super) fn verify_single_link(file: &File) -> io::Result<()> {
    if security::file_link_count(file)? != 1 {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(())
}

pub(super) fn write_material(path: &Path, bytes: &[u8], _mode: FileMode) -> io::Result<()> {
    crate::durable::write_private_new(path, bytes)
}

/// A freshly created root pinned against deletion/rename; cleanup resolves children
/// from the retained handle. Lifecycle and Drop policy belong to the caller.
pub struct OwnedDirectory {
    path: PathBuf,
    identity: (u32, u64),
    held: Option<File>,
}

impl OwnedDirectory {
    pub fn create(path: &Path) -> io::Result<Self> {
        let held = security::create_owner_only_directory(path)?;
        let identity = security::file_identity(&held)?;
        Ok(Self {
            path: path.to_owned(),
            identity,
            held: Some(held),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn remove(&mut self) -> io::Result<()> {
        use security::{delete_by_handle, file_identity, open_delete_at};
        fn children(
            parent: &std::fs::File,
            display: &Path,
            remaining: &mut usize,
            depth: usize,
        ) -> io::Result<()> {
            if depth > 32 {
                return Err(io::Error::other("private directory cleanup failed"));
            }
            for entry in fs::read_dir(display)? {
                if *remaining == 0 {
                    return Err(io::Error::other("private directory cleanup failed"));
                }
                let entry = entry?;
                let directory = entry.file_type()?.is_dir();
                let child = open_delete_at(parent, &entry.file_name(), directory)?;
                if file_identity(&child)?.0 != file_identity(parent)?.0 {
                    return Err(io::Error::other("private directory cleanup failed"));
                }
                if directory {
                    children(&child, &entry.path(), remaining, depth + 1)?;
                }
                *remaining -= 1;
                delete_by_handle(&child)?;
            }
            Ok(())
        }

        let Some(held) = self.held.as_ref() else {
            return Ok(());
        };
        if file_identity(held)? != self.identity {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let current = security::open_identity(&self.path, true)?;
        if file_identity(&current)? != self.identity {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        drop(current);
        // An ancestor rename can make enumeration fail, never redirect deletion.
        children(held, &self.path, &mut 4096, 0)?;
        delete_by_handle(held)?;
        drop(self.held.take());
        Ok(())
    }
}
