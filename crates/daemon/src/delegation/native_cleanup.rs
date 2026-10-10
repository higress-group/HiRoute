//! Bounded no-follow deletion for one already-claimed native session root.

use std::path::Path;

use hiroute_domain::delegation::{DelegationNativeCleanupFailureKindV1, DelegationNativeRootV1};

const DELETE_BATCH_LIMIT: usize = 64;
/// Worker-owned session roots are deliberately shallow.  This cap keeps a batch's read-only
/// descent bounded independently from its destructive-operation budget.  Deeper trees are
/// retained as unsafe instead of making cleanup latency or stack use attacker-controlled.
const DELETE_DIRECTORY_DEPTH_LIMIT: usize = 32;

/// `processed_entries` counts attempted unlink/rmdir operations. Read-only descent has the
/// independent `DELETE_DIRECTORY_DEPTH_LIMIT` bound.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NativeDeletionOutcome {
    Deferred {
        processed_entries: usize,
    },
    Removed {
        processed_entries: usize,
    },
    Blocked {
        kind: DelegationNativeCleanupFailureKindV1,
        processed_entries: usize,
    },
}

pub(crate) fn delete_native_root_batch(
    root: &DelegationNativeRootV1,
    budget: usize,
) -> NativeDeletionOutcome {
    if budget == 0 || budget > DELETE_BATCH_LIMIT {
        return NativeDeletionOutcome::Blocked {
            kind: DelegationNativeCleanupFailureKindV1::UnsafeEntry,
            processed_entries: 0,
        };
    }
    let (Some(base), Some(identity)) = (
        root.managed_base_path.as_deref(),
        root.filesystem_identity.as_ref(),
    ) else {
        // A claimed NoNative root has no filesystem target and is completed by the caller.
        return NativeDeletionOutcome::Blocked {
            kind: DelegationNativeCleanupFailureKindV1::UnsupportedIdentity,
            processed_entries: 0,
        };
    };
    #[cfg(unix)]
    {
        unix::delete(Path::new(base), &root.relative_root, identity, root, budget)
    }
    #[cfg(not(unix))]
    {
        let _ = (base, identity);
        NativeDeletionOutcome::Blocked {
            kind: DelegationNativeCleanupFailureKindV1::UnsupportedPlatform,
            processed_entries: 0,
        }
    }
}

#[cfg(unix)]
mod unix {
    use std::ffi::CStr;
    use std::fs::File;
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Component, Path};

    use hiroute_domain::delegation::{
        DELEGATION_NATIVE_ROOT_MARKER_FILE_V1, DelegationNativeCleanupFailureKindV1,
        DelegationNativeFilesystemIdentityV1, DelegationNativeRootMarkerV1, DelegationNativeRootV1,
    };
    use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};

    use super::{DELETE_DIRECTORY_DEPTH_LIMIT, NativeDeletionOutcome};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Identity {
        device: u64,
        inode: u64,
    }

    pub(super) fn delete(
        base_path: &Path,
        root_name: &str,
        expected: &DelegationNativeFilesystemIdentityV1,
        root_record: &DelegationNativeRootV1,
        budget: usize,
    ) -> NativeDeletionOutcome {
        if expected.scheme != "unix-dev-inode-v1"
            || root_name.is_empty()
            || root_name.contains(['/', '\\', '\0'])
        {
            return NativeDeletionOutcome::Blocked {
                kind: DelegationNativeCleanupFailureKindV1::UnsupportedIdentity,
                processed_entries: 0,
            };
        }
        let Ok(base) = open_absolute_directory(base_path) else {
            return NativeDeletionOutcome::Blocked {
                kind: DelegationNativeCleanupFailureKindV1::BaseUnavailable,
                processed_entries: 0,
            };
        };
        let Ok(base_metadata) = base.metadata() else {
            return NativeDeletionOutcome::Blocked {
                kind: DelegationNativeCleanupFailureKindV1::IoUnavailable,
                processed_entries: 0,
            };
        };
        if !private_directory(&base_metadata)
            || base_metadata.dev() != expected.base_device
            || base_metadata.ino() != expected.base_inode
        {
            return NativeDeletionOutcome::Blocked {
                kind: DelegationNativeCleanupFailureKindV1::IdentityMismatch,
                processed_entries: 0,
            };
        }
        let root = match rustix::fs::openat(
            &base,
            root_name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(root) => File::from(root),
            Err(error) if std::io::Error::from(error).kind() == std::io::ErrorKind::NotFound => {
                // A claimed, formerly-ready identity that disappeared can only be finalized by
                // the runtime's crash-window rule; absence alone is not a new deletion grant.
                return NativeDeletionOutcome::Removed {
                    processed_entries: 0,
                };
            }
            Err(_) => {
                return NativeDeletionOutcome::Blocked {
                    kind: DelegationNativeCleanupFailureKindV1::BaseUnavailable,
                    processed_entries: 0,
                };
            }
        };
        let Ok(metadata) = root.metadata() else {
            return NativeDeletionOutcome::Blocked {
                kind: DelegationNativeCleanupFailureKindV1::IoUnavailable,
                processed_entries: 0,
            };
        };
        if !private_directory(&metadata)
            || metadata.dev() != expected.root_device
            || metadata.ino() != expected.root_inode
            || metadata.dev() != expected.base_device
        {
            return NativeDeletionOutcome::Blocked {
                kind: DelegationNativeCleanupFailureKindV1::IdentityMismatch,
                processed_entries: 0,
            };
        }
        if let Err(kind) = verify_marker(&root, expected, root_record) {
            return NativeDeletionOutcome::Blocked {
                kind,
                processed_entries: 0,
            };
        }
        let mut remaining = budget;
        let emptied = match remove_children(&root, expected.root_device, &mut remaining, 0, true) {
            Ok(emptied) => emptied,
            Err(kind) => {
                return NativeDeletionOutcome::Blocked {
                    kind,
                    processed_entries: budget - remaining,
                };
            }
        };
        let processed_entries = budget - remaining;
        if !emptied {
            return NativeDeletionOutcome::Deferred { processed_entries };
        }
        // Marker removal and root rmdir are two destructive attempts.  Do not spend the final
        // unit unlinking the marker when this batch cannot also attempt the root removal.
        if remaining < 2 {
            return NativeDeletionOutcome::Deferred { processed_entries };
        }
        if let Err(kind) = verify_marker(&root, expected, root_record) {
            return NativeDeletionOutcome::Blocked {
                kind,
                processed_entries,
            };
        }
        remaining -= 1;
        let processed_entries = budget - remaining;
        if let Err(kind) = unlink_exact_marker(&root, expected) {
            return NativeDeletionOutcome::Blocked {
                kind,
                processed_entries,
            };
        }
        let Ok((current, directory)) = identity_at(&base, root_name) else {
            return NativeDeletionOutcome::Blocked {
                kind: DelegationNativeCleanupFailureKindV1::IoUnavailable,
                processed_entries,
            };
        };
        if !directory
            || current
                != (Identity {
                    device: expected.root_device,
                    inode: expected.root_inode,
                })
        {
            return NativeDeletionOutcome::Blocked {
                kind: DelegationNativeCleanupFailureKindV1::IdentityMismatch,
                processed_entries,
            };
        }
        remaining -= 1;
        let processed_entries = budget - remaining;
        match rustix::fs::unlinkat(&base, root_name, AtFlags::REMOVEDIR) {
            Ok(()) => NativeDeletionOutcome::Removed { processed_entries },
            Err(error) if std::io::Error::from(error).kind() == std::io::ErrorKind::NotFound => {
                NativeDeletionOutcome::Removed { processed_entries }
            }
            // The marker is already gone at this point. Retrying this root as ordinary deferred
            // work would fail the next batch's ownership check, so record the interruption as a
            // durable blocked result and leave the claim pending for explicit recovery.
            Err(error)
                if matches!(
                    std::io::Error::from(error).kind(),
                    std::io::ErrorKind::DirectoryNotEmpty | std::io::ErrorKind::WouldBlock
                ) =>
            {
                NativeDeletionOutcome::Blocked {
                    kind: DelegationNativeCleanupFailureKindV1::IoUnavailable,
                    processed_entries,
                }
            }
            Err(_) => NativeDeletionOutcome::Blocked {
                kind: DelegationNativeCleanupFailureKindV1::IoUnavailable,
                processed_entries,
            },
        }
    }

    fn remove_children(
        directory: &File,
        root_device: u64,
        remaining: &mut usize,
        depth: usize,
        preserve_root_marker: bool,
    ) -> Result<bool, DelegationNativeCleanupFailureKindV1> {
        if *remaining == 0 {
            return Ok(false);
        }
        let mut entries = Dir::read_from(directory)
            .map_err(|_| DelegationNativeCleanupFailureKindV1::IoUnavailable)?;
        for entry in &mut entries {
            let entry = entry.map_err(|_| DelegationNativeCleanupFailureKindV1::IoUnavailable)?;
            let name = utf8_name(entry.file_name())
                .ok_or(DelegationNativeCleanupFailureKindV1::UnsafeEntry)?;
            if name == "." || name == ".." {
                continue;
            }
            if preserve_root_marker && name == DELEGATION_NATIVE_ROOT_MARKER_FILE_V1 {
                continue;
            }
            if *remaining == 0 {
                return Ok(false);
            }
            let stat = rustix::fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|_| DelegationNativeCleanupFailureKindV1::IoUnavailable)?;
            let kind = FileType::from_raw_mode(stat.st_mode);
            match kind {
                FileType::Directory => {
                    if depth >= DELETE_DIRECTORY_DEPTH_LIMIT || stat.st_dev as u64 != root_device {
                        return Err(DelegationNativeCleanupFailureKindV1::UnsafeEntry);
                    }
                    let child = File::from(
                        rustix::fs::openat(
                            directory,
                            name,
                            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                            Mode::empty(),
                        )
                        .map_err(|_| DelegationNativeCleanupFailureKindV1::IoUnavailable)?,
                    );
                    let metadata = child
                        .metadata()
                        .map_err(|_| DelegationNativeCleanupFailureKindV1::IoUnavailable)?;
                    if metadata.dev() != root_device || metadata.ino() != stat.st_ino as u64 {
                        return Err(DelegationNativeCleanupFailureKindV1::IdentityMismatch);
                    }
                    if !remove_children(&child, root_device, remaining, depth + 1, false)? {
                        return Ok(false);
                    }
                    if *remaining == 0 {
                        return Ok(false);
                    }
                    // The global batch budget counts destructive entry attempts.  Read-only
                    // descent is separately bounded by DELETE_DIRECTORY_DEPTH_LIMIT, so every
                    // supported non-empty tree either deletes at least one entry or returns
                    // Blocked.
                    *remaining -= 1;
                    let (current, is_directory) = identity_at(directory, name)
                        .map_err(|_| DelegationNativeCleanupFailureKindV1::IoUnavailable)?;
                    if !is_directory
                        || current
                            != (Identity {
                                device: stat.st_dev as u64,
                                inode: stat.st_ino as u64,
                            })
                    {
                        return Err(DelegationNativeCleanupFailureKindV1::IdentityMismatch);
                    }
                    match rustix::fs::unlinkat(directory, name, AtFlags::REMOVEDIR) {
                        Ok(()) => {}
                        Err(error)
                            if std::io::Error::from(error).kind()
                                == std::io::ErrorKind::DirectoryNotEmpty =>
                        {
                            return Ok(false);
                        }
                        Err(_) => {
                            return Err(DelegationNativeCleanupFailureKindV1::IoUnavailable);
                        }
                    }
                }
                FileType::RegularFile | FileType::Symlink => {
                    // unlinkat never follows the final component. A same-UID replacement can at
                    // worst cause this isolated root to be blocked; it cannot escape the handle.
                    *remaining -= 1;
                    rustix::fs::unlinkat(directory, name, AtFlags::empty())
                        .map_err(|_| DelegationNativeCleanupFailureKindV1::IoUnavailable)?;
                }
                _ => return Err(DelegationNativeCleanupFailureKindV1::UnsafeEntry),
            }
        }
        Ok(true)
    }

    fn verify_marker(
        directory: &File,
        expected: &DelegationNativeFilesystemIdentityV1,
        root: &DelegationNativeRootV1,
    ) -> Result<(), DelegationNativeCleanupFailureKindV1> {
        let marker = match rustix::fs::openat(
            directory,
            DELEGATION_NATIVE_ROOT_MARKER_FILE_V1,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(marker) => File::from(marker),
            Err(error) => {
                let kind = std::io::Error::from(error).kind();
                return Err(if kind == std::io::ErrorKind::NotFound {
                    DelegationNativeCleanupFailureKindV1::IdentityMismatch
                } else if rustix::fs::statat(
                    directory,
                    DELEGATION_NATIVE_ROOT_MARKER_FILE_V1,
                    AtFlags::SYMLINK_NOFOLLOW,
                )
                .ok()
                .is_some_and(|stat| FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile)
                {
                    DelegationNativeCleanupFailureKindV1::UnsafeEntry
                } else {
                    DelegationNativeCleanupFailureKindV1::IoUnavailable
                });
            }
        };
        let metadata = marker
            .metadata()
            .map_err(|_| DelegationNativeCleanupFailureKindV1::IoUnavailable)?;
        if !metadata.is_file() || metadata.len() > 4096 {
            return Err(DelegationNativeCleanupFailureKindV1::UnsafeEntry);
        }
        if metadata.dev() != expected.marker_device
            || metadata.ino() != expected.marker_inode
            || metadata.dev() != expected.root_device
        {
            return Err(DelegationNativeCleanupFailureKindV1::IdentityMismatch);
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        marker
            .take(4097)
            .read_to_end(&mut bytes)
            .map_err(|_| DelegationNativeCleanupFailureKindV1::IoUnavailable)?;
        if bytes.len() > 4096 {
            return Err(DelegationNativeCleanupFailureKindV1::UnsafeEntry);
        }
        let actual: DelegationNativeRootMarkerV1 = serde_json::from_slice(&bytes)
            .map_err(|_| DelegationNativeCleanupFailureKindV1::IdentityMismatch)?;
        if actual != root.ownership_marker() {
            return Err(DelegationNativeCleanupFailureKindV1::IdentityMismatch);
        }
        Ok(())
    }

    fn unlink_exact_marker(
        directory: &File,
        expected: &DelegationNativeFilesystemIdentityV1,
    ) -> Result<(), DelegationNativeCleanupFailureKindV1> {
        let stat = rustix::fs::statat(
            directory,
            DELEGATION_NATIVE_ROOT_MARKER_FILE_V1,
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(|_| DelegationNativeCleanupFailureKindV1::IdentityMismatch)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
            || stat.st_dev as u64 != expected.marker_device
            || stat.st_ino as u64 != expected.marker_inode
        {
            return Err(DelegationNativeCleanupFailureKindV1::IdentityMismatch);
        }
        rustix::fs::unlinkat(
            directory,
            DELEGATION_NATIVE_ROOT_MARKER_FILE_V1,
            AtFlags::empty(),
        )
        .map_err(|_| DelegationNativeCleanupFailureKindV1::IoUnavailable)
    }

    fn open_absolute_directory(path: &Path) -> Result<File, ()> {
        if !path.is_absolute() {
            return Err(());
        }
        let mut current = File::from(
            rustix::fs::open(
                "/",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| ())?,
        );
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => {
                    current = File::from(
                        rustix::fs::openat(
                            &current,
                            name,
                            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                            Mode::empty(),
                        )
                        .map_err(|_| ())?,
                    );
                }
                _ => return Err(()),
            }
        }
        Ok(current)
    }

    fn identity_at(parent: &File, name: &str) -> Result<(Identity, bool), ()> {
        let stat = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|_| ())?;
        let file_type = FileType::from_raw_mode(stat.st_mode);
        Ok((
            Identity {
                device: stat.st_dev as u64,
                inode: stat.st_ino as u64,
            },
            file_type == FileType::Directory,
        ))
    }

    fn private_directory(metadata: &std::fs::Metadata) -> bool {
        metadata.is_dir()
    }

    fn utf8_name(name: &CStr) -> Option<&str> {
        name.to_str().ok()
    }
}

#[cfg(all(test, unix))]
#[path = "native_cleanup_tests.rs"]
mod tests;
