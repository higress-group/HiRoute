use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::time::{Duration, SystemTime};

use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_SHARE_READ, FILE_SHARE_WRITE,
};

use super::{ReplayError, SecureDirectory, SecureRoot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FileIdentity {
    volume: u32,
    index: u64,
}

pub(super) fn open_directory(path: &Path) -> Result<File, ReplayError> {
    let file = directory_options().open(path)?;
    validate_directory(&file)?;
    Ok(file)
}

pub(super) fn create_directory(parent: &File, name: &OsStr) -> Result<File, ReplayError> {
    let file = hiroute_platform::windows::security::create_owner_only_directory_at(parent, name)?;
    validate_directory(&file)?;
    Ok(file)
}

pub(super) fn create_file(parent: &File, _path: &Path, name: &str) -> Result<File, ReplayError> {
    let file = hiroute_platform::windows::security::create_owner_only_buffered_file_at(
        parent,
        name.as_ref(),
    )?;
    validate_file(&file)?;
    Ok(file)
}

pub(super) fn validate_directory(file: &File) -> Result<(), ReplayError> {
    validate(file, true)
}

pub(super) fn validate_file(file: &File) -> Result<(), ReplayError> {
    validate(file, false)
}

pub(super) fn identity(file: &File) -> Result<FileIdentity, ReplayError> {
    let (volume, index) = super::super::windows_acl::file_identity(file)?;
    Ok(FileIdentity { volume, index })
}

pub(super) fn remove_file(
    _directory: &File,
    _path: &Path,
    _name: &str,
    expected: FileIdentity,
    file: &File,
) -> Result<(), ReplayError> {
    if identity(file)? != expected {
        return Err(ReplayError::UnsafePath);
    }
    super::super::windows_acl::delete_by_handle(file)?;
    Ok(())
}

pub(super) fn remove_directory(
    _root: &File,
    directory: &File,
    _root_path: &Path,
    directory_path: &Path,
    _original_name: &str,
    expected: FileIdentity,
    _request_prefix: &str,
) -> Result<(), ReplayError> {
    if identity(directory)? != expected {
        return Err(ReplayError::UnsafePath);
    }
    for entry in fs::read_dir(directory_path)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || !metadata.is_file() {
            return Err(ReplayError::UnsafePath);
        }
        let child = file_options(false).open(entry.path())?;
        validate_file(&child)?;
        super::super::windows_acl::delete_by_handle(&child)?;
    }
    super::super::windows_acl::delete_by_handle(directory)?;
    Ok(())
}

pub(super) fn cleanup_orphans(
    root: &SecureRoot,
    orphan_ttl: Duration,
    request_prefix: &str,
) -> Result<(), ReplayError> {
    let cutoff = SystemTime::now()
        .checked_sub(orphan_ttl)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    for entry in fs::read_dir(&root.path)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| ReplayError::UnsafePath)?;
        if !name.starts_with(request_prefix) {
            continue;
        }
        let handle = open_directory(&entry.path())?;
        if handle.metadata()?.modified()? > cutoff {
            continue;
        }
        let directory = SecureDirectory {
            root: std::sync::Arc::new(SecureRoot {
                path: root.path.clone(),
                handle: root.handle.try_clone()?,
            }),
            name,
            path: entry.path(),
            identity: identity(&handle)?,
            handle,
        };
        super::remove_request_directory(&directory)?;
    }
    Ok(())
}

fn validate(file: &File, directory: bool) -> Result<(), ReplayError> {
    let metadata = file.metadata()?;
    if metadata.is_dir() != directory
        || metadata.is_file() == directory
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(ReplayError::UnsafePath);
    }
    super::super::windows_acl::verify_owner_only_handle(file, directory)
        .map_err(|_| ReplayError::UnsafePermissions)
}

fn directory_options() -> OpenOptions {
    file_options(true)
}

fn file_options(directory: bool) -> OpenOptions {
    let mut options = OpenOptions::new();
    options.access_mode(GENERIC_READ | GENERIC_WRITE | DELETE);
    // Directory traversal/enumeration needs a second read handle. Denying
    // only FILE_SHARE_DELETE still pins the directory identity against rename
    // or junction replacement for the Replay owner's whole lifetime.
    options.share_mode(if directory {
        FILE_SHARE_READ | FILE_SHARE_WRITE
    } else {
        0
    });
    let mut flags = FILE_FLAG_OPEN_REPARSE_POINT;
    if directory {
        flags |= FILE_FLAG_BACKUP_SEMANTICS;
    }
    options.custom_flags(flags);
    options
}
