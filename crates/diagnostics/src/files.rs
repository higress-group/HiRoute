//! Safe file access under a verified private directory handle.
//!
//! On Unix every operation goes through an already-open directory handle (`openat`,
//! `mkdirat`, `renameat`, `unlinkat`) with `O_NOFOLLOW`, and each opened file is verified
//! by `fstat`: regular file, exactly one hard link, with no owner or mode admission gate. Directory and file identities (`dev`, `ino`) are re-checked
//! before destructive operations, so a replaced path is refused instead of followed.
//!
//! Platforms without equivalent private-path primitives report
//! [`FileSafetyError::UnsupportedPlatform`]; this module never silently falls back to
//! ordinary path-based reads and writes. It also never changes permissions of files or
//! directories it did not create in this call.

use std::io::ErrorKind;

/// The only file names this subsystem reads or writes.
pub const SETTINGS_FILE: &str = "settings.json";
/// Temporary name used for atomic settings replacement.
pub const SETTINGS_TEMP_FILE: &str = "settings.tmp";
pub const SETTINGS_LOCK_FILE: &str = "settings.lock";
pub const CORRELATION_KEY_FILE: &str = "correlation.key";
pub const WRITER_LOCK_FILE: &str = "writer.lock";
pub const CURRENT_LOG_FILE: &str = "current.jsonl";
/// How many rotated files a role keeps in addition to the current file.
pub const PREVIOUS_LOG_FILES: usize = 4;

pub fn previous_log_file(index: usize) -> String {
    format!("previous-{index}.jsonl")
}

pub fn role_dir_name(role: crate::event::ProcessRole) -> &'static str {
    match role {
        crate::event::ProcessRole::Desktop => "desktop",
        crate::event::ProcessRole::Daemon => "daemon",
        // Control and CLI roles share the daemon role directory in this MVP.
        crate::event::ProcessRole::Control | crate::event::ProcessRole::Cli => "daemon",
    }
}

/// Whether a directory entry name is one this subsystem owns. Unknown files are never
/// read, exported, rotated or deleted.
pub fn is_allowed_name(name: &str) -> bool {
    if matches!(
        name,
        SETTINGS_FILE
            | SETTINGS_TEMP_FILE
            | SETTINGS_LOCK_FILE
            | CORRELATION_KEY_FILE
            | WRITER_LOCK_FILE
    ) {
        return true;
    }
    if name == CURRENT_LOG_FILE {
        return true;
    }
    parse_previous_index(name).is_some()
}

pub fn parse_previous_index(name: &str) -> Option<usize> {
    let index = name.strip_prefix("previous-")?.strip_suffix(".jsonl")?;
    let index: usize = index.parse().ok()?;
    (1..=PREVIOUS_LOG_FILES).contains(&index).then_some(index)
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FileSafetyError {
    #[error("diagnostic file operations are not supported on this platform")]
    UnsupportedPlatform,
    #[error("diagnostic directory is not a safe private directory")]
    UnsafeDirectory,
    #[error("diagnostic file is not a safe private regular file")]
    UnsafeFile,
    #[error("diagnostic file does not exist")]
    NotFound,
    #[error("diagnostic file already exists")]
    AlreadyExists,
    #[error("diagnostic file is locked by another writer")]
    Locked,
    #[error("diagnostic file content is invalid")]
    InvalidData,
    #[error("diagnostic file operation failed")]
    Io {
        kind: ErrorKind,
        os_errno: Option<i32>,
    },
    #[error("diagnostic file changed while it was being used")]
    IdentityChanged,
}

impl FileSafetyError {
    pub fn os_errno(&self) -> Option<i32> {
        match self {
            FileSafetyError::Io { os_errno, .. } => *os_errno,
            _ => None,
        }
    }

    pub(crate) fn io(error: std::io::Error) -> Self {
        let os_errno = error.raw_os_error();
        FileSafetyError::Io {
            kind: error.kind(),
            os_errno,
        }
    }
}

#[cfg(unix)]
mod unix {
    use std::ffi::OsString;
    use std::fs::File;
    use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
    use std::os::fd::{AsFd, OwnedFd};
    use std::path::{Path, PathBuf};

    use fs2::FileExt;
    use nix::fcntl::{OFlag, open, openat, renameat};
    use nix::sys::stat::{FileStat, Mode, SFlag, fstat, mkdirat};
    use nix::unistd::{UnlinkatFlags, fsync, unlinkat};

    use super::{FileSafetyError, FileSafetyError as E};

    /// Stable identity of an opened directory or file.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FileIdentity {
        dev: u64,
        ino: u64,
    }

    // `st_dev` is `u64` on Linux and `i32` on macOS, so this cast is a no-op on one of them.
    #[allow(clippy::unnecessary_cast)]
    fn identity_of(stat: &FileStat) -> FileIdentity {
        FileIdentity {
            dev: stat.st_dev as u64,
            ino: stat.st_ino,
        }
    }

    fn verify_directory(stat: &FileStat) -> Result<(), FileSafetyError> {
        let is_dir = SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT == SFlag::S_IFDIR;
        if !is_dir {
            return Err(E::UnsafeDirectory);
        }
        Ok(())
    }

    fn verify_file(stat: &FileStat) -> Result<(), FileSafetyError> {
        let kind = SFlag::from_bits_truncate(stat.st_mode);
        let regular = kind & SFlag::S_IFMT == SFlag::S_IFREG;
        if !regular || stat.st_nlink != 1 {
            return Err(E::UnsafeFile);
        }
        Ok(())
    }

    /// A verified directory handle. All file operations are relative to
    /// this handle, so a path swapped after verification is not followed.
    #[derive(Debug)]
    pub struct PrivateDir {
        path: PathBuf,
        fd: OwnedFd,
        identity: FileIdentity,
    }

    /// Open `path` level by level from the filesystem root, each component relative to the
    /// already verified handle of its parent and with `O_NOFOLLOW`, so a symlinked ancestor
    /// is refused instead of followed. Only the two documented macOS system aliases (`/tmp`
    /// and `/var` to `/private/*`) are resolved, using the same rule as the resident
    /// ownership validator; nothing else is canonicalized.
    ///
    /// Intermediate levels must be real directories. Every level retains its existing permissions.
    /// With `create`, missing levels below the nearest existing ancestor are created 0700,
    /// which is how a first start reaches its own root; existing levels are left as they are.
    fn open_directory(path: &Path, create: bool) -> Result<PrivateDir, FileSafetyError> {
        let absolute = std::path::absolute(path).map_err(map_io)?;
        let mut names: Vec<OsString> = Vec::new();
        for component in absolute.components() {
            match component {
                std::path::Component::RootDir => {}
                std::path::Component::Normal(name) => names.push(name.to_os_string()),
                // `.` never survives `components`; `..` and prefixes are refused.
                _ => return Err(E::UnsafeDirectory),
            }
        }
        let mut fd = open(
            Path::new("/"),
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .map_err(map_errno)?;
        for (index, name) in names.iter().enumerate() {
            let name = Path::new(name);
            let child = match openat(
                &fd,
                name,
                OFlag::O_RDONLY
                    | OFlag::O_DIRECTORY
                    | OFlag::O_NOFOLLOW
                    | OFlag::O_NONBLOCK
                    | OFlag::O_CLOEXEC,
                Mode::empty(),
            ) {
                Ok(child) => child,
                // O_NOFOLLOW reports ELOOP on Linux and ENOTDIR on macOS for a symlinked
                // component; both consult the documented system aliases first.
                Err(nix::errno::Errno::ELOOP) | Err(nix::errno::Errno::ENOTDIR) => {
                    match system_alias(&fd, name, index == 0)? {
                        Some(child) => child,
                        None => return Err(E::UnsafeDirectory),
                    }
                }
                Err(nix::errno::Errno::ENOENT) if create => {
                    match mkdirat(&fd, name, Mode::from_bits_truncate(0o700)) {
                        Ok(()) => {}
                        // A creation race: the winner's directory is opened and checked.
                        Err(nix::errno::Errno::EEXIST) => {}
                        Err(errno) => return Err(map_errno(errno)),
                    }
                    openat(
                        &fd,
                        name,
                        OFlag::O_RDONLY
                            | OFlag::O_DIRECTORY
                            | OFlag::O_NOFOLLOW
                            | OFlag::O_NONBLOCK
                            | OFlag::O_CLOEXEC,
                        Mode::empty(),
                    )
                    .map_err(map_errno)?
                }
                Err(nix::errno::Errno::ENOENT) => return Err(E::NotFound),
                Err(errno) => return Err(map_errno(errno)),
            };
            fd = child;
        }
        let stat = fstat(fd.as_fd()).map_err(map_errno)?;
        verify_directory(&stat)?;
        Ok(PrivateDir {
            path: absolute,
            identity: identity_of(&stat),
            fd,
        })
    }

    /// The two documented macOS system aliases. A symlink that does not resolve exactly to
    /// its documented private target is not an alias and stays refused.
    fn system_alias(
        parent: &OwnedFd,
        name: &Path,
        at_root: bool,
    ) -> Result<Option<OwnedFd>, FileSafetyError> {
        if !at_root {
            return Ok(None);
        }
        let expected = match name.to_str() {
            Some("tmp") => "/private/tmp",
            Some("var") => "/private/var",
            _ => return Ok(None),
        };
        let lexical = Path::new("/").join(name);
        if std::fs::canonicalize(&lexical).ok().as_deref() != Some(Path::new(expected)) {
            return Ok(None);
        }
        let private = openat(
            parent,
            Path::new("private"),
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .map_err(map_errno)?;
        match openat(
            &private,
            name,
            OFlag::O_RDONLY
                | OFlag::O_DIRECTORY
                | OFlag::O_NOFOLLOW
                | OFlag::O_NONBLOCK
                | OFlag::O_CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => Ok(Some(fd)),
            Err(nix::errno::Errno::ENOENT)
            | Err(nix::errno::Errno::ENOTDIR)
            | Err(nix::errno::Errno::ELOOP) => Ok(None),
            Err(errno) => Err(map_errno(errno)),
        }
    }

    impl PrivateDir {
        /// Open an existing directory and verify its type and stable identity.
        pub fn open_existing(path: &Path) -> Result<Self, FileSafetyError> {
            open_directory(path, false)
        }

        /// Open the directory, creating the missing levels below the nearest existing
        /// ancestor (0700) when needed: the application data root may not exist yet on a
        /// first start. A pre-existing directory is never chmodded; only a directory created
        /// by this call may be corrected within this call.
        pub fn open_or_create(path: &Path) -> Result<Self, FileSafetyError> {
            open_directory(path, true)
        }

        pub fn path(&self) -> &Path {
            &self.path
        }

        pub fn identity(&self) -> FileIdentity {
            self.identity
        }

        /// Re-verify the directory handle identity before a destructive operation.
        pub fn recheck_identity(&self) -> Result<(), FileSafetyError> {
            let stat = fstat(self.fd.as_fd()).map_err(map_errno)?;
            verify_directory(&stat)?;
            if identity_of(&stat) != self.identity {
                return Err(E::IdentityChanged);
            }
            Ok(())
        }

        /// Open (or create) a child directory under this verified handle.
        pub fn child_dir(&self, name: &str) -> Result<PrivateDir, FileSafetyError> {
            if !is_plain_component(name) {
                return Err(E::UnsafeDirectory);
            }
            match fstatat_child(&self.fd, name) {
                Ok(_) => {}
                Err(E::NotFound) => {
                    mkdirat(&self.fd, name, Mode::from_bits_truncate(0o700)).map_err(map_errno)?;
                }
                Err(other) => return Err(other),
            };
            let fd = openat(
                &self.fd,
                name,
                OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
                Mode::empty(),
            )
            .map_err(map_errno)?;
            let stat = fstat(fd.as_fd()).map_err(map_errno)?;
            verify_directory(&stat)?;
            let identity = identity_of(&stat);
            let child = PrivateDir {
                path: self.path.join(name),
                fd,
                identity,
            };
            Ok(child)
        }

        /// Open an existing file for reading, verifying its identity after the open.
        pub fn open_read(&self, name: &str) -> Result<Option<VerifiedFile>, FileSafetyError> {
            let Some(fd) = self.open_optional(name, OFlag::O_RDONLY)? else {
                return Ok(None);
            };
            let stat = fstat(fd.as_fd()).map_err(map_errno)?;
            verify_file(&stat)?;
            Ok(Some(VerifiedFile {
                file: File::from(fd),
                name: name.to_string(),
                identity: identity_of(&stat),
                len: stat.st_size.max(0) as u64,
            }))
        }

        /// Open a file for appending, creating it (0600) when missing.
        pub fn open_append_or_create(&self, name: &str) -> Result<VerifiedFile, FileSafetyError> {
            let fd = openat(
                &self.fd,
                name,
                OFlag::O_WRONLY
                    | OFlag::O_APPEND
                    | OFlag::O_CREAT
                    | OFlag::O_NOFOLLOW
                    | OFlag::O_NONBLOCK
                    | OFlag::O_CLOEXEC,
                Mode::from_bits_truncate(0o600),
            )
            .map_err(map_errno)?;
            let stat = fstat(fd.as_fd()).map_err(map_errno)?;
            verify_file(&stat)?;
            Ok(VerifiedFile {
                file: File::from(fd),
                name: name.to_string(),
                identity: identity_of(&stat),
                len: stat.st_size.max(0) as u64,
            })
        }

        /// Create a new file (0600). Fails when the name already exists, including when
        /// it exists as a symlink or another non-regular file.
        pub fn create_new(&self, name: &str) -> Result<VerifiedFile, FileSafetyError> {
            let fd = openat(
                &self.fd,
                name,
                OFlag::O_WRONLY
                    | OFlag::O_CREAT
                    | OFlag::O_EXCL
                    | OFlag::O_NOFOLLOW
                    | OFlag::O_NONBLOCK
                    | OFlag::O_CLOEXEC,
                Mode::from_bits_truncate(0o600),
            )
            .map_err(map_errno)?;
            let stat = fstat(fd.as_fd()).map_err(map_errno)?;
            verify_file(&stat)?;
            Ok(VerifiedFile {
                file: File::from(fd),
                name: name.to_string(),
                identity: identity_of(&stat),
                len: 0,
            })
        }

        /// Open or create a lock file (0600) that is not part of the log data.
        pub fn open_lock(&self, name: &str) -> Result<VerifiedFile, FileSafetyError> {
            let fd = openat(
                &self.fd,
                name,
                OFlag::O_RDWR
                    | OFlag::O_CREAT
                    | OFlag::O_NOFOLLOW
                    | OFlag::O_NONBLOCK
                    | OFlag::O_CLOEXEC,
                Mode::from_bits_truncate(0o600),
            )
            .map_err(map_errno)?;
            let stat = fstat(fd.as_fd()).map_err(map_errno)?;
            verify_file(&stat)?;
            Ok(VerifiedFile {
                file: File::from(fd),
                name: name.to_string(),
                identity: identity_of(&stat),
                len: stat.st_size.max(0) as u64,
            })
        }

        fn open_optional(
            &self,
            name: &str,
            oflag: OFlag,
        ) -> Result<Option<OwnedFd>, FileSafetyError> {
            match openat(
                &self.fd,
                name,
                oflag | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
                Mode::empty(),
            ) {
                Ok(fd) => Ok(Some(fd)),
                Err(nix::errno::Errno::ENOENT) => Ok(None),
                Err(nix::errno::Errno::ELOOP) => Err(E::UnsafeFile),
                Err(errno) => Err(map_errno(errno)),
            }
        }

        /// Rename a file within this directory after re-verifying the directory and the
        /// source identity. Never follows the replaced path, and never replaces a target
        /// that is not a private regular file we own.
        pub fn rename_verified(
            &self,
            from: &str,
            to: &str,
            expect: FileIdentity,
        ) -> Result<(), FileSafetyError> {
            self.recheck_identity()?;
            let fd = openat(
                &self.fd,
                from,
                OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
                Mode::empty(),
            )
            .map_err(map_errno)?;
            let stat = fstat(fd.as_fd()).map_err(map_errno)?;
            verify_file(&stat)?;
            if identity_of(&stat) != expect {
                return Err(E::IdentityChanged);
            }
            self.reject_unsafe_target(to)?;
            renameat(&self.fd, from, &self.fd, to).map_err(map_errno)
        }

        /// A rename replaces its target. Only an absent entry or a private single-link
        /// regular file is accepted; a symlink, FIFO or foreign object is refused instead
        /// of being silently unlinked.
        fn reject_unsafe_target(&self, name: &str) -> Result<(), FileSafetyError> {
            match openat(
                &self.fd,
                name,
                OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
                Mode::empty(),
            ) {
                Ok(fd) => {
                    let stat = fstat(fd.as_fd()).map_err(map_errno)?;
                    verify_file(&stat)
                }
                Err(nix::errno::Errno::ENOENT) => Ok(()),
                Err(nix::errno::Errno::ELOOP) => Err(E::UnsafeFile),
                Err(errno) => Err(map_errno(errno)),
            }
        }

        /// Remove a file after re-verifying the directory and file identity.
        pub fn remove_verified(
            &self,
            name: &str,
            expect: FileIdentity,
        ) -> Result<(), FileSafetyError> {
            self.recheck_identity()?;
            let fd = openat(
                &self.fd,
                name,
                OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
                Mode::empty(),
            )
            .map_err(map_errno)?;
            let stat = fstat(fd.as_fd()).map_err(map_errno)?;
            verify_file(&stat)?;
            if identity_of(&stat) != expect {
                return Err(E::IdentityChanged);
            }
            unlinkat(&self.fd, name, UnlinkatFlags::NoRemoveDir).map_err(map_errno)
        }

        /// Directory entry names, without following anything. Used for rotation and for
        /// refusing to touch unknown files.
        pub fn list_names(&self) -> Result<Vec<String>, FileSafetyError> {
            let duplicate = self.fd.try_clone().map_err(map_io)?;
            let mut dir = nix::dir::Dir::from_fd(duplicate).map_err(map_errno)?;
            let mut names = Vec::new();
            for entry in dir.iter() {
                let entry = entry.map_err(map_errno)?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == "." || name == ".." {
                    continue;
                }
                names.push(name);
            }
            Ok(names)
        }

        pub fn sync(&self) -> Result<(), FileSafetyError> {
            fsync(&self.fd).map_err(map_errno)
        }
    }

    fn fstatat_child(fd: &OwnedFd, name: &str) -> Result<FileStat, FileSafetyError> {
        let child = match openat(
            fd,
            name,
            OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
            Mode::empty(),
        ) {
            Ok(child) => child,
            Err(nix::errno::Errno::ELOOP) | Err(nix::errno::Errno::ENOTDIR) => {
                return Err(E::UnsafeDirectory);
            }
            Err(errno) => return Err(map_errno(errno)),
        };
        fstat(child.as_fd()).map_err(map_errno)
    }

    fn is_plain_component(name: &str) -> bool {
        !name.is_empty()
            && !name.contains('/')
            && !name.contains('\0')
            && name != "."
            && name != ".."
    }

    fn map_errno(errno: nix::errno::Errno) -> FileSafetyError {
        match errno {
            nix::errno::Errno::ENOENT => return E::NotFound,
            nix::errno::Errno::EEXIST => return E::AlreadyExists,
            nix::errno::Errno::ELOOP => return E::UnsafeFile,
            _ => {}
        }
        let kind = match errno {
            nix::errno::Errno::EACCES | nix::errno::Errno::EPERM => ErrorKind::PermissionDenied,
            nix::errno::Errno::EWOULDBLOCK => ErrorKind::WouldBlock,
            _ => ErrorKind::Other,
        };
        E::Io {
            kind,
            os_errno: Some(errno as i32),
        }
    }

    fn map_io(error: std::io::Error) -> FileSafetyError {
        E::io(error)
    }

    /// A file opened under a verified directory handle, with a re-checkable identity.
    #[derive(Debug)]
    pub struct VerifiedFile {
        file: File,
        name: String,
        identity: FileIdentity,
        len: u64,
    }

    impl VerifiedFile {
        pub fn name(&self) -> &str {
            &self.name
        }

        pub fn len(&self) -> u64 {
            self.len
        }

        pub fn is_empty(&self) -> bool {
            self.len == 0
        }

        pub fn identity(&self) -> FileIdentity {
            self.identity
        }

        /// Re-check the open file's identity and type against the stored values.
        pub fn recheck(&mut self) -> Result<(), FileSafetyError> {
            let stat = fstat(self.file.as_fd()).map_err(map_errno)?;
            verify_file(&stat)?;
            if identity_of(&stat) != self.identity {
                return Err(E::IdentityChanged);
            }
            Ok(())
        }

        /// Read the verified length-prefix of the file without unbounded buffering.
        pub fn read_prefix(&mut self, max_bytes: u64) -> Result<Vec<u8>, FileSafetyError> {
            let limit = self.len.min(max_bytes);
            self.file.seek(SeekFrom::Start(0)).map_err(map_io)?;
            let mut buffer = Vec::with_capacity(limit as usize);
            let mut limited = (&mut self.file).take(limit);
            limited.read_to_end(&mut buffer).map_err(map_io)?;
            Ok(buffer)
        }

        /// Read at most the final `max_bytes` of the file.
        pub fn read_tail(&mut self, max_bytes: u64) -> Result<Vec<u8>, FileSafetyError> {
            let offset = self.len.saturating_sub(max_bytes);
            self.file.seek(SeekFrom::Start(offset)).map_err(map_io)?;
            let mut buffer = Vec::new();
            let mut limited = (&mut self.file).take(self.len - offset);
            limited.read_to_end(&mut buffer).map_err(map_io)?;
            Ok(buffer)
        }

        /// Append after re-checking the open file: a file that became group readable,
        /// multiply linked or replaced since it was opened is never written to.
        pub fn append(&mut self, bytes: &[u8]) -> Result<(), FileSafetyError> {
            self.recheck()?;
            self.file.write_all(bytes).map_err(map_io)?;
            self.len += bytes.len() as u64;
            Ok(())
        }

        pub fn replace_contents(&mut self, bytes: &[u8]) -> Result<(), FileSafetyError> {
            self.recheck()?;
            self.file.set_len(0).map_err(map_io)?;
            self.file.seek(SeekFrom::Start(0)).map_err(map_io)?;
            self.file.write_all(bytes).map_err(map_io)?;
            self.len = bytes.len() as u64;
            Ok(())
        }

        pub fn sync(&mut self) -> Result<(), FileSafetyError> {
            self.file.sync_data().map_err(map_io)
        }

        /// Non-blocking exclusive lock used for single-writer and settings mutex files.
        pub fn try_lock_exclusive(&self) -> Result<(), FileSafetyError> {
            match FileExt::try_lock_exclusive(&self.file) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == ErrorKind::WouldBlock => Err(E::Locked),
                Err(error) => Err(map_io(error)),
            }
        }

        pub fn unlock(&self) -> Result<(), FileSafetyError> {
            FileExt::unlock(&self.file).map_err(map_io)
        }

        #[cfg(test)]
        pub(crate) fn duplicate_for_test(&self) -> Result<Self, FileSafetyError> {
            Ok(Self {
                file: self.file.try_clone().map_err(map_io)?,
                name: self.name.clone(),
                identity: self.identity,
                len: self.len,
            })
        }
    }
}

#[cfg(unix)]
pub use unix::{FileIdentity, PrivateDir, VerifiedFile};

#[cfg(not(unix))]
mod other {
    use std::path::Path;

    use super::FileSafetyError;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FileIdentity;

    #[derive(Debug)]
    pub struct PrivateDir;

    #[derive(Debug)]
    pub struct VerifiedFile;

    impl PrivateDir {
        pub fn open_existing(_path: &Path) -> Result<Self, FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
        pub fn open_or_create(_path: &Path) -> Result<Self, FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
        pub fn path(&self) -> &Path {
            Path::new("")
        }
        pub fn identity(&self) -> FileIdentity {
            FileIdentity
        }
        pub fn recheck_identity(&self) -> Result<(), FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
        pub fn child_dir(&self, _name: &str) -> Result<PrivateDir, FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
        pub fn open_read(&self, _name: &str) -> Result<Option<VerifiedFile>, FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
        pub fn open_append_or_create(&self, _name: &str) -> Result<VerifiedFile, FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
        pub fn create_new(&self, _name: &str) -> Result<VerifiedFile, FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
        pub fn open_lock(&self, _name: &str) -> Result<VerifiedFile, FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
        pub fn rename_verified(
            &self,
            _from: &str,
            _to: &str,
            _expect: FileIdentity,
        ) -> Result<(), FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
        pub fn remove_verified(
            &self,
            _name: &str,
            _expect: FileIdentity,
        ) -> Result<(), FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
        pub fn list_names(&self) -> Result<Vec<String>, FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
        pub fn sync(&self) -> Result<(), FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
    }

    impl VerifiedFile {
        pub fn name(&self) -> &str {
            ""
        }
        pub fn len(&self) -> u64 {
            0
        }
        pub fn is_empty(&self) -> bool {
            true
        }
        pub fn identity(&self) -> FileIdentity {
            FileIdentity
        }
        pub fn recheck(&mut self) -> Result<(), FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
        pub fn read_prefix(&mut self, _max_bytes: u64) -> Result<Vec<u8>, FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
        /// Kept compilable for callers that bound rotated-file retention; this platform
        /// still performs no file I/O at all.
        pub fn read_tail(&mut self, _max_bytes: u64) -> Result<Vec<u8>, FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
        pub fn append(&mut self, _bytes: &[u8]) -> Result<(), FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
        pub fn replace_contents(&mut self, _bytes: &[u8]) -> Result<(), FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
        pub fn sync(&mut self) -> Result<(), FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
        pub fn try_lock_exclusive(&self) -> Result<(), FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
        pub fn unlock(&self) -> Result<(), FileSafetyError> {
            Err(FileSafetyError::UnsupportedPlatform)
        }
    }
}

#[cfg(not(unix))]
pub use other::{FileIdentity, PrivateDir, VerifiedFile};
