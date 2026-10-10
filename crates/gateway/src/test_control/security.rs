use std::fs::File;
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;
use zeroize::Zeroizing;

#[cfg(windows)]
// Windows owner and protected-DACL inspection is available only through FFI.
// Keep the crate-level unsafe exception scoped to this verifier.
#[allow(unsafe_code)]
mod windows;

const NONCE_BYTES: usize = 64;

#[derive(Clone, Debug)]
pub struct E2eControlOptions {
    pub listen: SocketAddr,
    pub nonce_file: PathBuf,
}

pub(super) struct E2eControlConfig {
    pub(super) listen: SocketAddr,
    nonce: Zeroizing<String>,
}

impl E2eControlConfig {
    pub(super) fn load(options: E2eControlOptions) -> Result<Self, E2eControlError> {
        if !options.listen.ip().is_loopback() || options.listen.port() == 0 {
            return Err(E2eControlError::Listener(options.listen));
        }
        if !options.nonce_file.is_absolute() {
            return Err(E2eControlError::UnsafeNoncePath);
        }
        let bytes = read_secure_nonce(&options.nonce_file)?;
        if bytes.len() != NONCE_BYTES
            || !bytes
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return Err(E2eControlError::InvalidNonce);
        }
        let nonce = String::from_utf8(bytes).map_err(|_| E2eControlError::InvalidNonce)?;
        Ok(Self {
            listen: options.listen,
            nonce: Zeroizing::new(nonce),
        })
    }

    pub(super) fn authenticates(&self, candidate: Option<&str>) -> bool {
        let Some(candidate) = candidate else {
            return false;
        };
        constant_time_eq(candidate.as_bytes(), self.nonce.as_bytes())
    }
}

#[derive(Debug, Error)]
pub enum E2eControlError {
    #[error("E2E control listener must use a nonzero loopback address, got {0}")]
    Listener(SocketAddr),
    #[error("E2E control listener and product listener must be distinct")]
    ListenerCollision,
    #[error("E2E control nonce path must be absolute and contain only ordinary components")]
    UnsafeNoncePath,
    #[error("E2E control nonce path is not owner-only")]
    UnsafeNoncePermissions,
    #[error("E2E control nonce file is invalid")]
    InvalidNonce,
    #[error("E2E control nonce file cannot be read securely: {0}")]
    NonceIo(std::io::Error),
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(unix)]
fn read_secure_nonce(path: &Path) -> Result<Vec<u8>, E2eControlError> {
    use rustix::fs::{AtFlags, Mode, OFlags};

    if !path.is_absolute() {
        return Err(E2eControlError::UnsafeNoncePath);
    }
    let file_name = path.file_name().ok_or(E2eControlError::UnsafeNoncePath)?;
    let parent = path.parent().ok_or(E2eControlError::UnsafeNoncePath)?;
    // Resolve platform-owned aliases such as macOS `/var` once, then pin and
    // traverse every resulting directory handle without following symlinks.
    // The nonce leaf itself is always opened with NOFOLLOW.
    let parent = std::fs::canonicalize(parent).map_err(E2eControlError::NonceIo)?;
    let directories = parent
        .components()
        .filter_map(|component| match component {
            Component::RootDir => None,
            Component::Normal(name) => Some(Ok(name)),
            _ => Some(Err(E2eControlError::UnsafeNoncePath)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut current = File::from(os(rustix::fs::open(
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ))?);
    for directory in &directories {
        current = File::from(os(rustix::fs::openat(
            &current,
            *directory,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ))?);
    }
    let parent_metadata = current.metadata().map_err(E2eControlError::NonceIo)?;
    if !parent_metadata.is_dir() {
        return Err(E2eControlError::UnsafeNoncePermissions);
    }
    let mut file = File::from(os(rustix::fs::openat(
        &current,
        file_name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ))?);
    let metadata = file.metadata().map_err(E2eControlError::NonceIo)?;
    if !metadata.is_file() {
        return Err(E2eControlError::UnsafeNoncePermissions);
    }
    let bytes = read_bounded(&mut file)?;
    // The final directory is owned by this process identity and cannot be
    // changed by other identities; both open and unlink are pinned to that
    // handle. The listener is not constructed until the secret is unlinked.
    os(rustix::fs::unlinkat(&current, file_name, AtFlags::empty()))?;
    Ok(bytes)
}

#[cfg(windows)]
fn read_secure_nonce(path: &Path) -> Result<Vec<u8>, E2eControlError> {
    let mut file = windows::open_owner_only_file(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            E2eControlError::UnsafeNoncePermissions
        } else {
            E2eControlError::NonceIo(error)
        }
    })?;
    read_bounded(&mut file)
}

#[cfg(not(any(unix, windows)))]
fn read_secure_nonce(_path: &Path) -> Result<Vec<u8>, E2eControlError> {
    Err(E2eControlError::UnsafeNoncePermissions)
}

fn read_bounded(file: &mut File) -> Result<Vec<u8>, E2eControlError> {
    let mut bytes = Vec::with_capacity(NONCE_BYTES + 1);
    file.take((NONCE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(E2eControlError::NonceIo)?;
    Ok(bytes)
}

#[cfg(unix)]
fn os<T>(result: rustix::io::Result<T>) -> Result<T, E2eControlError> {
    result
        .map_err(std::io::Error::from)
        .map_err(E2eControlError::NonceIo)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[cfg(unix)]
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    use super::*;

    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn listener_must_be_nonzero_loopback_before_nonce_is_touched() {
        let error = E2eControlConfig::load(E2eControlOptions {
            listen: "0.0.0.0:8318".parse().unwrap(),
            nonce_file: PathBuf::from("not-opened"),
        })
        .err()
        .unwrap();
        assert!(matches!(error, E2eControlError::Listener(_)));
    }

    #[cfg(unix)]
    #[test]
    fn nonce_accepts_accessible_modes_and_requires_regular_file_and_exact_shape() {
        let directory = temporary_directory();
        let path = directory.join("nonce");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        let mut file = options.open(&path).unwrap();
        file.write_all(b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .unwrap();
        drop(file);
        let config = E2eControlConfig::load(E2eControlOptions {
            listen: "127.0.0.1:8318".parse().unwrap(),
            nonce_file: path.clone(),
        })
        .unwrap();
        assert!(config.authenticates(Some(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        )));
        assert!(!path.exists(), "secure load must consume the nonce path");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true).mode(0o644);
        let mut file = options.open(&path).unwrap();
        file.write_all(b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .unwrap();
        drop(file);
        for parent_mode in [0o700, 0o770] {
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(parent_mode))
                .unwrap();
            std::fs::write(
                &path,
                b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            let config = E2eControlConfig::load(E2eControlOptions {
                listen: "127.0.0.1:8318".parse().unwrap(),
                nonce_file: path.clone(),
            })
            .unwrap();
            assert!(config.authenticates(Some(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            )));
            assert!(!path.exists());
            assert_eq!(
                std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                parent_mode
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    fn temporary_directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "hiroute-e2e-control-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
}
