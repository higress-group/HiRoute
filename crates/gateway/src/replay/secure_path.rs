use std::fs::{self, File, Metadata};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::ReplayError;

#[cfg(unix)]
#[path = "secure_path/unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "secure_path/windows.rs"]
mod platform;

const REQUEST_PREFIX: &str = "req-";

pub(super) struct SecureRoot {
    path: PathBuf,
    handle: File,
}

pub(super) struct SecureDirectory {
    root: Arc<SecureRoot>,
    name: String,
    path: PathBuf,
    handle: File,
    identity: platform::FileIdentity,
}

#[derive(Clone)]
pub(super) struct SecureFile {
    name: String,
    path: PathBuf,
    identity: platform::FileIdentity,
}

pub(super) fn prepare_root(
    path: &Path,
    orphan_ttl: Duration,
) -> Result<Arc<SecureRoot>, ReplayError> {
    reject_ambiguous_path(path)?;
    let canonical = match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if unsafe_link(&metadata) || !metadata.is_dir() {
                return Err(ReplayError::UnsafePath);
            }
            match (path.parent(), path.file_name()) {
                (Some(parent), Some(name)) => fs::canonicalize(parent)?.join(name),
                _ => fs::canonicalize(path)?,
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => create_private_root(path)?,
        Err(error) => return Err(error.into()),
    };
    let handle = platform::open_directory(&canonical)?;
    finish_open_root(canonical, handle, orphan_ttl)
}

pub(super) fn reopen_root(
    previous: &Arc<SecureRoot>,
    orphan_ttl: Duration,
) -> Result<Arc<SecureRoot>, ReplayError> {
    // The stored path was already resolved at startup. Reopen it without
    // canonicalizing again, so a substituted link cannot redirect recovery.
    let path = &previous.path;
    let handle = match platform::open_directory(path) {
        Ok(handle) => handle,
        Err(ReplayError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or(ReplayError::UnsafePath)?;
            let name = path.file_name().ok_or(ReplayError::UnsafePath)?;
            platform::create_directory(&platform::open_directory(parent)?, name)?
        }
        Err(error) => return Err(error),
    };
    if platform::identity(&handle)? == platform::identity(&previous.handle)? {
        // A missing child does not make live directories under this root orphans.
        return Ok(Arc::clone(previous));
    }
    finish_open_root(path.clone(), handle, orphan_ttl)
}

fn finish_open_root(
    path: PathBuf,
    handle: File,
    orphan_ttl: Duration,
) -> Result<Arc<SecureRoot>, ReplayError> {
    platform::validate_directory(&handle)?;
    let root = Arc::new(SecureRoot { path, handle });
    let cutoff = SystemTime::now()
        .checked_sub(orphan_ttl)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    // A usable root can serve new requests even if old scratch cannot be removed.
    platform::cleanup_orphans(&root, cutoff, REQUEST_PREFIX);
    Ok(root)
}

fn remove_expired_orphan(
    root: &Arc<SecureRoot>,
    name: &str,
    handle: File,
    cutoff: SystemTime,
) -> Result<(), ReplayError> {
    if handle.metadata()?.modified()? > cutoff {
        return Ok(());
    }
    let directory = SecureDirectory {
        root: Arc::clone(root),
        name: name.to_owned(),
        path: root.path.join(name),
        identity: platform::identity(&handle)?,
        handle,
    };
    remove_request_directory(&directory)
}

pub(super) fn create_request_directory(
    root: &Arc<SecureRoot>,
) -> Result<Arc<SecureDirectory>, ReplayError> {
    for _ in 0..32 {
        let suffix = hex(&random_bytes::<16>()?);
        let name = format!("{REQUEST_PREFIX}{suffix}");
        let path = root.path.join(&name);
        match platform::create_directory(&root.handle, name.as_ref()) {
            Ok(handle) => {
                platform::validate_directory(&handle)?;
                let identity = platform::identity(&handle)?;
                return Ok(Arc::new(SecureDirectory {
                    root: Arc::clone(root),
                    path,
                    name,
                    handle,
                    identity,
                }));
            }
            Err(ReplayError::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                continue;
            }
            Err(error) => return Err(error),
        }
    }
    Err(ReplayError::NameExhausted)
}

pub(super) fn create_file(
    directory: &SecureDirectory,
    suffix: &str,
) -> Result<(SecureFile, File), ReplayError> {
    for _ in 0..32 {
        let random = hex(&random_bytes::<12>()?);
        let name = format!("stream-{random}.{suffix}");
        let path = directory.path.join(&name);
        match platform::create_file(&directory.handle, &path, &name) {
            Ok(file) => {
                platform::validate_file(&file)?;
                let identity = platform::identity(&file)?;
                return Ok((
                    SecureFile {
                        path,
                        name,
                        identity,
                    },
                    file,
                ));
            }
            Err(ReplayError::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                continue;
            }
            Err(error) => return Err(error),
        }
    }
    Err(ReplayError::NameExhausted)
}

pub(super) fn remove_file(
    directory: &SecureDirectory,
    entry: &SecureFile,
    file: &File,
) -> Result<(), ReplayError> {
    if platform::identity(file)? != entry.identity {
        return Err(ReplayError::UnsafePath);
    }
    platform::remove_file(
        &directory.handle,
        &entry.path,
        &entry.name,
        entry.identity,
        file,
    )
}

pub(super) fn remove_request_directory(directory: &SecureDirectory) -> Result<(), ReplayError> {
    platform::remove_directory(
        &directory.root.handle,
        &directory.handle,
        &directory.root.path,
        &directory.path,
        &directory.name,
        directory.identity,
        REQUEST_PREFIX,
    )
}

pub(super) fn validate_owner_file_handle(file: &File) -> Result<(), ReplayError> {
    platform::validate_file(file)
}

impl SecureRoot {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl SecureDirectory {
    pub(super) fn root(&self) -> &Arc<SecureRoot> {
        &self.root
    }

    #[cfg(test)]
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl SecureFile {
    #[cfg(test)]
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

fn reject_ambiguous_path(path: &Path) -> Result<(), ReplayError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(ReplayError::UnsafePath);
    }
    Ok(())
}

fn create_private_root(path: &Path) -> Result<PathBuf, ReplayError> {
    let mut ancestor = path.parent().ok_or(ReplayError::UnsafePath)?;
    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                if unsafe_link(&metadata) || !metadata.is_dir() {
                    return Err(ReplayError::UnsafePath);
                }
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ancestor = ancestor.parent().ok_or(ReplayError::UnsafePath)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    let mut current = fs::canonicalize(ancestor)?;
    let tail = path
        .strip_prefix(ancestor)
        .map_err(|_| ReplayError::UnsafePath)?;
    for component in tail.components() {
        let Component::Normal(component) = component else {
            return Err(ReplayError::UnsafePath);
        };
        current.push(component);
        match create_directory_path(&current) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(ReplayError::UnsafePath);
            }
            Err(error) => return Err(error.into()),
        }
        let metadata = fs::symlink_metadata(&current)?;
        if unsafe_link(&metadata) || !metadata.is_dir() {
            return Err(ReplayError::UnsafePath);
        }
        set_owner_directory_permissions(&current)?;
    }
    Ok(current)
}

#[cfg(unix)]
fn create_directory_path(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700).create(path)
}

#[cfg(windows)]
fn create_directory_path(path: &Path) -> std::io::Result<()> {
    super::windows_acl::create_owner_only_directory(path).map(drop)
}

#[cfg(unix)]
fn set_owner_directory_permissions(path: &Path) -> Result<(), ReplayError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(windows)]
fn set_owner_directory_permissions(path: &Path) -> Result<(), ReplayError> {
    super::windows_acl::set_owner_only(path, true)?;
    Ok(())
}

#[cfg(windows)]
fn unsafe_link(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(unix)]
fn unsafe_link(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn random_bytes<const N: usize>() -> Result<[u8; N], ReplayError> {
    let mut value = [0_u8; N];
    getrandom::fill(&mut value).map_err(|_| ReplayError::RandomUnavailable)?;
    Ok(value)
}
