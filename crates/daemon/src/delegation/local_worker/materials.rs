use hiroute_domain::delegation::DelegationErrorV1 as Error;
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

use crate::delegation::profile::RunMaterials;
pub(super) struct OwnedRoot {
    path: PathBuf,
    identity: same_file::Handle,
    cleanup_on_drop: bool,
}

fn invalid<T>() -> Result<T, Error> {
    Err(Error::InvalidArguments)
}

pub(super) fn executable(path: &Path) -> Result<(), Error> {
    if !path.is_absolute() {
        return invalid();
    }
    let meta = fs::metadata(path).map_err(|_| Error::CapabilityUnavailable)?;
    if !meta.is_file() {
        return Err(Error::CapabilityUnavailable);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return Err(Error::CapabilityUnavailable);
        }
    }
    Ok(())
}

fn relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.components().all(|c| match c {
            Component::Normal(name) => name
                .to_str()
                .is_some_and(|s| !s.contains(['\\', ':', '\0', '\r', '\n'])),
            _ => false,
        })
}

fn directory(path: &Path) -> Result<(), Error> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|_| Error::StorageUnavailable)
}

pub(super) fn prepare(
    materials: RunMaterials,
    root: PathBuf,
    session_root: PathBuf,
) -> Result<OwnedRoot, Error> {
    if !root.is_absolute()
        || materials.directories.len() + materials.files.len() > 256
        || materials
            .files
            .iter()
            .map(|f| f.contents.len())
            .sum::<usize>()
            > 1024 * 1024
    {
        return invalid();
    }
    // Canonicalize the existing parent, never create arbitrary ancestor directories.
    let parent = root.parent().ok_or(Error::InvalidArguments)?;
    let name = root.file_name().ok_or(Error::InvalidArguments)?;
    if root.components().any(|c| matches!(c, Component::ParentDir)) {
        return invalid();
    }
    let parent = fs::canonicalize(parent).map_err(|_| Error::InvalidArguments)?;
    let actual_root = parent.join(name);
    {
        let session = &session_root;
        if !session.is_absolute() {
            return invalid();
        }
        let session = fs::canonicalize(session).map_err(|_| Error::InvalidArguments)?;
        if !session.is_dir()
            || session.starts_with(&actual_root)
            || actual_root.starts_with(&session)
        {
            return invalid();
        }
    }
    let mut names = BTreeSet::new();
    for path in materials
        .directories
        .iter()
        .chain(materials.files.iter().map(|f| &f.relative_path))
    {
        if !relative(path) || !names.insert(path.clone()) {
            return invalid();
        }
    }
    // File parents must be explicitly listed. No implicit HOME/session directories.
    let dirs: BTreeSet<_> = materials.directories.iter().cloned().collect();
    for path in &names {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty())
            && !dirs.contains(parent)
        {
            return invalid();
        }
    }
    // create, never create_dir_all(root): an existing root is never ours to delete.
    directory(&actual_root)?;
    let identity =
        same_file::Handle::from_path(&actual_root).map_err(|_| Error::StorageUnavailable)?;
    let owned = OwnedRoot {
        path: actual_root,
        identity,
        cleanup_on_drop: true,
    };
    let mut dirs: Vec<_> = materials.directories.into_iter().collect();
    dirs.sort_by_key(|p| p.components().count());
    for dir in dirs {
        directory(&owned.path.join(dir))?;
    }
    for file in materials.files {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // The profile may provide a token-free native launcher. Only that explicitly
            // declared material is executable; ordinary configuration stays private data.
            options.mode(if file.executable { 0o700 } else { 0o600 });
        }
        let mut output = options
            .open(owned.path.join(file.relative_path))
            .map_err(|_| Error::StorageUnavailable)?;
        output
            .write_all(&file.contents)
            .map_err(|_| Error::StorageUnavailable)?;
    }
    Ok(owned)
}

impl OwnedRoot {
    /// Active processes keep materials. Drop must not recursively remove their live root.
    pub(super) fn retain(&mut self) {
        self.cleanup_on_drop = false;
    }

    pub(super) fn cleanup(&mut self) -> Result<(), Error> {
        let meta = match fs::symlink_metadata(&self.path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(Error::StorageUnavailable),
        };
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(Error::StorageUnavailable);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(Error::StorageUnavailable);
            }
        }
        let current =
            same_file::Handle::from_path(&self.path).map_err(|_| Error::StorageUnavailable)?;
        if current != self.identity {
            return Err(Error::StorageUnavailable);
        }
        fs::remove_dir_all(&self.path).map_err(|_| Error::StorageUnavailable)
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self.cleanup_on_drop {
            let _ = self.cleanup();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn local_worker_cancelled_preparation_keeps_cleanup_owner() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cancelled");
        let copy = path.clone();
        let session = temp.path().join("session");
        fs::create_dir(&session).unwrap();
        let (created_tx, created_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let job = tokio::task::spawn_blocking(move || {
            let owned = prepare(
                RunMaterials {
                    directories: vec![],
                    files: vec![],
                },
                copy,
                session,
            )
            .unwrap();
            created_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            owned
        });
        created_rx.await.unwrap();
        assert!(path.exists());
        // Cancel the awaiter after creation while the blocking job still owns the root.
        drop(job);
        release_tx.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while path.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}
