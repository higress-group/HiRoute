//! A borrowed read retains the complete inspected pathname without changing ACLs.

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::path::Path;

use windows_sys::Win32::Foundation::GENERIC_READ;
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

use super::{diagnostic_files, path, security};

pub struct ReadLease {
    file: File,
    parents: Vec<File>,
    parent_ids: Vec<(u32, u64)>,
    file_id: (u32, u64),
    leaf: OsString,
    installed_links: Option<u32>,
}

/// Open a regular, single-link file through held reparse-free directories.
/// The file denies data writes/deletion and every ancestor denies deletion.
/// This requests read access only and never adopts a borrowed file's ACL.
pub fn open_read_lease(location: &Path) -> io::Result<ReadLease> {
    open(location, false)
}

/// Retain an already admitted installed image, whose publisher may use hard
/// links inside its distribution. The caller must bind the exact installation,
/// capture its ACL and authenticate its bytes separately. This does not admit
/// a hard-linked private configuration file or change any borrowed ACL.
/// Other link names are not retained; verify rejects a changed link count.
pub fn open_installed_image_lease(location: &Path) -> io::Result<ReadLease> {
    open(location, true)
}

fn open(location: &Path, installed_image: bool) -> io::Result<ReadLease> {
    let (file, parents, leaf) = path::retained_read(location)?;
    let parent_ids = parents
        .iter()
        .map(security::file_identity)
        .collect::<io::Result<Vec<_>>>()?;
    let file_id = security::file_identity(&file)?;
    let installed_links = installed_image
        .then(|| security::file_link_count(&file))
        .transpose()?;
    let lease = ReadLease {
        file,
        parents,
        parent_ids,
        file_id,
        leaf,
        installed_links,
    };
    lease.verify()?;
    Ok(lease)
}

impl ReadLease {
    pub fn file(&self) -> &File {
        &self.file
    }

    /// Reopen relative to the final retained directory, never via a fresh path.
    pub fn reopen_file(&self) -> io::Result<File> {
        path::relative(
            self.parents.last().ok_or_else(unsafe_object)?,
            &self.leaf,
            false,
            GENERIC_READ,
            FILE_SHARE_READ,
            false,
            None,
        )
    }

    /// Recheck held objects and the final name. Callers separately compare
    /// before/after metadata, permission snapshots and their bounded content.
    pub fn verify(&self) -> io::Result<()> {
        for (parent, identity) in self.parents.iter().zip(&self.parent_ids) {
            diagnostic_files::stat(parent, true)?;
            if security::file_identity(parent)? != *identity {
                return Err(unsafe_object());
            }
        }
        let reopened = self.reopen_file()?;
        for file in [&self.file, &reopened] {
            if self.installed_links.is_some() {
                path::check_type(file, false)?;
            } else {
                diagnostic_files::stat(file, false)?;
            }
            if security::file_identity(file)? != self.file_id
                || security::file_link_count(file)? != self.installed_links.unwrap_or(1)
                || self.installed_links == Some(0)
            {
                return Err(unsafe_object());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admitted_installed_hardlinks_keep_identity_and_write_delete_fences() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("image.exe");
        let alias = root.path().join("image-alias.exe");
        std::fs::write(&image, b"fixed installed image").unwrap();
        std::fs::hard_link(&image, &alias).unwrap();
        assert!(
            open_read_lease(&image).is_err(),
            "private single-link policy stays strict"
        );
        let before =
            security::file_identity(&security::open_identity(&image, false).unwrap()).unwrap();
        let lease = open_installed_image_lease(&image).unwrap();
        assert_eq!(security::file_identity(lease.file()).unwrap(), before);
        assert!(
            std::fs::OpenOptions::new()
                .write(true)
                .open(&alias)
                .is_err()
        );
        assert!(
            std::fs::OpenOptions::new()
                .write(true)
                .open(&image)
                .is_err()
        );
        assert!(std::fs::remove_file(&image).is_err());
        assert!(std::fs::rename(&image, root.path().join("replacement.exe")).is_err());
        lease.verify().unwrap();
        // NTFS may unlink a different, unretained name despite the source lease.
        // The admitted path/data remain pinned; a changed link count is refused.
        let expected_links = match std::fs::remove_file(&alias) {
            Ok(()) => {
                assert_eq!(
                    lease.verify().unwrap_err().kind(),
                    io::ErrorKind::PermissionDenied
                );
                1
            }
            Err(_) => {
                lease.verify().unwrap();
                2
            }
        };
        assert_eq!(security::file_identity(lease.file()).unwrap(), before);
        assert_eq!(
            security::file_identity(&security::open_identity(&image, false).unwrap()).unwrap(),
            before
        );
        drop(lease);
        assert_eq!(std::fs::read(&image).unwrap(), b"fixed installed image");
        assert_eq!(
            security::file_link_count(&security::open_identity(&image, false).unwrap()).unwrap(),
            expected_links
        );
    }
}

fn unsafe_object() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "read lease object changed")
}
