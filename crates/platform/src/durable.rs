//! Durable file transitions used inside the existing operation/backup journals.
//! Windows uses write-through handles and handle-relative rename; directory
//! barriers validate identity there, rather than attempting unsupported directory
//! FlushFileBuffers. Callers retain writing/committed/final recovery checkpoints.

#[cfg(any(not(windows), test))]
use std::fs;
#[cfg(not(windows))]
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

pub fn sync_file(path: impl AsRef<Path>) -> io::Result<()> {
    #[cfg(windows)]
    let file = crate::windows::security::open_write_through(path.as_ref(), false)?;
    #[cfg(not(windows))]
    let file = File::open(path)?;
    file.sync_all()
}

/// Flush an unpublished snapshot copy to the filesystem. On macOS the snapshot
/// owner MUST finish with `directory_barrier` before publishing the batch: that
/// full device flush covers the preceding per-file fsyncs. This is not a durable
/// publication by itself and must not replace key, journal or live-file writes.
/// Other platforms retain their ordinary full file barrier.
pub fn sync_unpublished_copy(path: impl AsRef<Path>) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let file = File::open(path)?;
        loop {
            match nix::unistd::fsync(&file) {
                Err(nix::errno::Errno::EINTR) => continue,
                result => return result.map_err(io::Error::from),
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    sync_file(path)
}

pub fn directory_barrier(path: impl AsRef<Path>) -> io::Result<()> {
    #[cfg(windows)]
    {
        // Namespace transitions use the write-through handle-relative operation
        // below. There is no user-mode directory fsync equivalent on Windows.
        let _held = crate::windows::security::open_identity(path.as_ref(), true)?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        File::open(path)?.sync_all()
    }
}

pub fn rename(source: impl AsRef<Path>, target: impl AsRef<Path>) -> io::Result<()> {
    #[cfg(windows)]
    {
        crate::windows::durable::rename(source.as_ref(), target.as_ref(), true)
    }
    #[cfg(not(windows))]
    {
        fs::rename(source, target.as_ref())?;
        directory_barrier(
            target
                .as_ref()
                .parent()
                .ok_or_else(|| io::Error::other("missing parent"))?,
        )
    }
}

/// Create a private file without replacing an existing entry and sync its bytes.
/// On Unix this does not sync the parent directory. Callers that must make the
/// new directory entry durable before publishing dependent state must call
/// `directory_barrier` on the parent after this function succeeds.
pub fn write_private_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    #[cfg(windows)]
    {
        write_private(path, bytes, false)
    }
    #[cfg(not(windows))]
    {
        // Preserve the released Unix create_new contract. A hard-link publication
        // would add a filesystem requirement and leave two links after a crash,
        // which the existing private-material and orphan readers correctly reject.
        let mut file = create_private_new(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    }
}

#[cfg(not(windows))]
fn create_private_new(path: &Path) -> io::Result<File> {
    crate::private_fs::create_file(path, crate::private_fs::FileMode::Data)
}

#[cfg(unix)]
struct UnpublishedStage<'a> {
    path: &'a Path,
    device: u64,
    inode: u64,
}

#[cfg(unix)]
impl Drop for UnpublishedStage<'_> {
    fn drop(&mut self) {
        use std::os::unix::fs::MetadataExt;
        if let Ok(named) = fs::symlink_metadata(self.path)
            && named.dev() == self.device
            && named.ino() == self.inode
        {
            // A completed rename removes this name. On failure, remove only
            // this writer's unpublished stage, never a replacement entry.
            let _ = fs::remove_file(self.path);
        }
    }
}

pub fn write_private_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_private(path, bytes, true)
}

fn write_private(path: &Path, bytes: &[u8], replace: bool) -> io::Result<()> {
    write_with_permissions(path, bytes, replace, None)
}

#[cfg(windows)]
pub fn write_windows_atomic(
    path: &Path,
    bytes: &[u8],
    permissions: &crate::file_security::WindowsPermissions,
) -> io::Result<()> {
    write_with_permissions(path, bytes, true, Some(permissions))
}

/// Stream a stopped native snapshot into an unpublished stage. The producer
/// must validate its held source before returning success. An error cannot
/// replace the target; flushing and native permission verification precede rename.
#[cfg(windows)]
pub fn write_windows_atomic_with(
    path: &Path,
    permissions: &crate::file_security::WindowsPermissions,
    write: impl FnOnce(&mut std::fs::File) -> io::Result<()>,
) -> io::Result<()> {
    publish_with_permissions(path, true, Some(permissions), write)
}

/// Move an already flushed native artifact without reopening it for data writes.
/// Its authenticated publication owner has admitted the source/target facts.
/// The kernel checks target WRITE_ATTRIBUTES before replacing a read-only file;
/// target attributes are never cleared through a separately resolved pathname.
#[cfg(windows)]
pub fn rename_windows_sealed_file(source: &Path, target: &Path) -> io::Result<()> {
    let file = crate::windows::security::open_delete(source, false)?;
    crate::windows::durable::rename_native_file(&file, target)
}

fn write_with_permissions(
    path: &Path,
    bytes: &[u8],
    replace: bool,
    permissions: Option<&crate::file_security::WindowsPermissions>,
) -> io::Result<()> {
    publish_with_permissions(path, replace, permissions, |file| file.write_all(bytes))
}

fn publish_with_permissions(
    path: &Path,
    replace: bool,
    permissions: Option<&crate::file_security::WindowsPermissions>,
    write: impl FnOnce(&mut std::fs::File) -> io::Result<()>,
) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("missing parent"))?;
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).map_err(|error| io::Error::other(error.to_string()))?;
    let suffix: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
    let temporary = parent.join(format!(".hiroute-write-{suffix}.tmp"));
    #[cfg(windows)]
    let mut file = match permissions {
        Some(permissions) => crate::windows::file_security::create(&temporary, permissions)?,
        None => crate::windows::security::create_owner_only_file(&temporary)?,
    };
    #[cfg(not(windows))]
    let mut file = create_private_new(&temporary)?;
    #[cfg(unix)]
    let _cleanup = {
        use std::os::unix::fs::MetadataExt;
        let held = file.metadata()?;
        UnpublishedStage {
            path: &temporary,
            device: held.dev(),
            inode: held.ino(),
        }
    };
    write(&mut file)?;
    file.sync_all()?;
    #[cfg(windows)]
    {
        if let Some(permissions) = permissions {
            crate::windows::file_security::apply_read_only(&file, permissions.read_only)?;
            if crate::file_security::capture(&file)? != *permissions {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "native permissions differ",
                ));
            }
        }
        let renamed = if permissions.is_some() && replace {
            crate::windows::durable::rename_native_file(&file, path)
        } else {
            crate::windows::durable::rename_file(&file, path, replace)
        };
        if let Err(error) = renamed {
            if error.kind() == io::ErrorKind::AlreadyExists {
                // Definite no-clobber failure: remove this exact uncommitted
                // stage through its retained handle, never a replacement path.
                crate::windows::security::delete_by_handle(&file)?;
            }
            return Err(error);
        }
        file.sync_all()?;
    }
    #[cfg(not(windows))]
    {
        let _ = permissions;
        debug_assert!(replace, "Unix initial writes use create_new directly");
        rename(&temporary, path)?;
    }
    // Windows retains uncertain stages for the owning journal. Unix drops only
    // its still-unpublished, identity-matched stage on ordinary I/O failure.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initial_private_publication_never_replaces_a_committed_key() {
        let fixture = tempfile::tempdir().unwrap();
        let key = fixture.path().join("key");
        write_private_new(&key, b"complete-key-one").unwrap();
        assert!(write_private_new(&key, b"different-key").is_err());
        assert_eq!(fs::read(&key).unwrap(), b"complete-key-one");
        assert_eq!(
            fs::read_dir(fixture.path()).unwrap().count(),
            1,
            "definite no-clobber conflict must remove only its own unpublished stage"
        );
        write_private_atomic(&key, b"complete-replacement").unwrap();
        assert_eq!(fs::read(&key).unwrap(), b"complete-replacement");
    }

    #[cfg(unix)]
    #[test]
    fn failed_replacement_removes_its_stage_and_preserves_the_committed_file() {
        use std::os::unix::fs::MetadataExt;
        let fixture = tempfile::tempdir().unwrap();
        let key = fixture.path().join("key");
        write_private_new(&key, b"committed").unwrap();
        let error = publish_with_permissions(&key, true, None, |file| {
            file.write_all(b"partial")?;
            Err(io::Error::other("injected write failure"))
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "injected write failure");
        assert_eq!(fs::read(&key).unwrap(), b"committed");
        assert_eq!(fs::metadata(&key).unwrap().nlink(), 1);
        assert_eq!(fs::metadata(&key).unwrap().mode() & 0o777, 0o600);
        assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn failed_rename_removes_only_the_unpublished_stage() {
        let fixture = tempfile::tempdir().unwrap();
        let target = fixture.path().join("directory");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep"), b"unchanged").unwrap();
        assert!(write_private_atomic(&target, b"replacement").is_err());
        assert_eq!(fs::read(target.join("keep")).unwrap(), b"unchanged");
        assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn failed_writer_does_not_unlink_a_replacement_stage_entry() {
        let fixture = tempfile::tempdir().unwrap();
        let target = fixture.path().join("target");
        let moved = fixture.path().join("moved-owned-stage");
        let mut substituted = None;
        let result = publish_with_permissions(&target, true, None, |file| {
            file.write_all(b"owned")?;
            let stage = fs::read_dir(fixture.path())?.next().unwrap()?.path();
            fs::rename(&stage, &moved)?;
            fs::write(&stage, b"foreign")?;
            substituted = Some(stage);
            Err(io::Error::other("injected namespace replacement"))
        });
        assert!(result.is_err());
        assert!(!target.exists());
        assert_eq!(fs::read(moved).unwrap(), b"owned");
        assert_eq!(fs::read(substituted.unwrap()).unwrap(), b"foreign");
    }
}
