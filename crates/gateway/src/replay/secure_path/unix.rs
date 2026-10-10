use std::ffi::CStr;
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};
use std::time::{Duration, SystemTime};

use rustix::fs::{AtFlags, Dir, Mode, OFlags};

use super::{ReplayError, SecureDirectory, SecureRoot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FileIdentity {
    device: u64,
    inode: u64,
}

pub(super) fn open_directory(path: &Path) -> Result<File, ReplayError> {
    let mut current = File::from(os(rustix::fs::open(
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ))?);
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                current = File::from(os(rustix::fs::openat(
                    &current,
                    name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                ))?);
            }
            _ => return Err(ReplayError::UnsafePath),
        }
    }
    Ok(current)
}

pub(super) fn create_directory(
    parent: &File,
    _path: &Path,
    name: &str,
) -> Result<File, ReplayError> {
    os(rustix::fs::mkdirat(parent, name, Mode::RWXU))?;
    match os(rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )) {
        Ok(handle) => Ok(File::from(handle)),
        Err(error) => {
            let _ = rustix::fs::unlinkat(parent, name, AtFlags::REMOVEDIR);
            Err(error.into())
        }
    }
}

pub(super) fn create_file(parent: &File, _path: &Path, name: &str) -> Result<File, ReplayError> {
    Ok(File::from(os(rustix::fs::openat(
        parent,
        name,
        OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    ))?))
}

pub(super) fn validate_directory(file: &File) -> Result<(), ReplayError> {
    validate(file, true)
}

pub(super) fn validate_file(file: &File) -> Result<(), ReplayError> {
    validate(file, false)
}

pub(super) fn identity(file: &File) -> Result<FileIdentity, ReplayError> {
    let metadata = file.metadata()?;
    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

pub(super) fn remove_file(
    directory: &File,
    _path: &Path,
    name: &str,
    expected: FileIdentity,
    _file: &File,
) -> Result<(), ReplayError> {
    match identity_at(directory, name) {
        Ok((identity, false)) if identity == expected => {
            os(rustix::fs::unlinkat(directory, name, AtFlags::empty()))?;
            Ok(())
        }
        Err(ReplayError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(ReplayError::UnsafePath),
    }
}

pub(super) fn remove_directory(
    root: &File,
    directory: &File,
    _root_path: &Path,
    _directory_path: &Path,
    original_name: &str,
    expected: FileIdentity,
    request_prefix: &str,
) -> Result<(), ReplayError> {
    remove_children(directory)?;
    match entry_matches(root, original_name, expected, true) {
        Ok(true) => return remove_directory_name(root, original_name),
        Ok(false) | Err(ReplayError::UnsafePath) => {}
        Err(error) => return Err(error),
    }
    let mut entries = os(Dir::read_from(root))?;
    for entry in &mut entries {
        let entry = os(entry)?;
        let Some(name) = utf8_name(entry.file_name()) else {
            return Err(ReplayError::UnsafePath);
        };
        if name.starts_with(request_prefix) {
            match entry_matches(root, name, expected, true) {
                Ok(true) => return remove_directory_name(root, name),
                Ok(false) | Err(ReplayError::UnsafePath) => {}
                Err(error) => return Err(error),
            }
        }
    }
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
    let mut entries = os(Dir::read_from(&root.handle))?;
    for entry in &mut entries {
        let entry = os(entry)?;
        let Some(name) = utf8_name(entry.file_name()) else {
            return Err(ReplayError::UnsafePath);
        };
        if !name.starts_with(request_prefix) {
            continue;
        }
        let handle = File::from(os(rustix::fs::openat(
            &root.handle,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ))?);
        validate_directory(&handle)?;
        if handle.metadata()?.modified()? > cutoff {
            continue;
        }
        let directory = SecureDirectory {
            root: std::sync::Arc::new(SecureRoot {
                path: root.path.clone(),
                handle: root.handle.try_clone()?,
            }),
            name: name.to_owned(),
            path: root.path.join(name),
            identity: identity(&handle)?,
            handle,
        };
        super::remove_request_directory(&directory)?;
    }
    Ok(())
}

fn remove_children(directory: &File) -> Result<(), ReplayError> {
    let mut entries = os(Dir::read_from(directory))?;
    for entry in &mut entries {
        let entry = os(entry)?;
        let Some(name) = utf8_name(entry.file_name()) else {
            return Err(ReplayError::UnsafePath);
        };
        if name == "." || name == ".." {
            continue;
        }
        let (_, directory_entry) = identity_at(directory, name)?;
        if directory_entry {
            return Err(ReplayError::UnsafePath);
        }
        os(rustix::fs::unlinkat(directory, name, AtFlags::empty()))?;
    }
    Ok(())
}

fn remove_directory_name(root: &File, name: &str) -> Result<(), ReplayError> {
    match os(rustix::fs::unlinkat(root, name, AtFlags::REMOVEDIR)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn entry_matches(
    parent: &File,
    name: &str,
    expected: FileIdentity,
    directory: bool,
) -> Result<bool, ReplayError> {
    match identity_at(parent, name) {
        Ok((identity, is_directory)) => Ok(identity == expected && is_directory == directory),
        Err(ReplayError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn identity_at(parent: &File, name: &str) -> Result<(FileIdentity, bool), ReplayError> {
    let stat = os(rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW))?;
    let file_type = rustix::fs::FileType::from_raw_mode(stat.st_mode);
    if !matches!(
        file_type,
        rustix::fs::FileType::RegularFile | rustix::fs::FileType::Directory
    ) {
        return Err(ReplayError::UnsafePath);
    }
    Ok((
        FileIdentity {
            device: stat.st_dev as u64,
            inode: stat.st_ino as u64,
        },
        file_type == rustix::fs::FileType::Directory,
    ))
}

fn validate(file: &File, directory: bool) -> Result<(), ReplayError> {
    let metadata = file.metadata()?;
    if metadata.is_dir() != directory || metadata.is_file() == directory {
        return Err(ReplayError::UnsafePath);
    }
    Ok(())
}

fn utf8_name(name: &CStr) -> Option<&str> {
    name.to_str().ok()
}

fn os<T>(result: rustix::io::Result<T>) -> Result<T, std::io::Error> {
    result.map_err(std::io::Error::from)
}
