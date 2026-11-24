//! Publish generated Desktop files from the same volume. The custom-root
//! journal reserves this private staging directory before any writer uses it.
use super::{INVENTORY, UNINSTALLER, invalid, present};
use hiroute_platform::windows::{durable, security};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fs::File,
    io,
    path::Path,
};

pub(super) const STAGING: &str = ".hiroute-install-staging";
pub(super) const PREFERENCES: &str = "shell-install.json";

pub(super) fn reserve(root: &Path) -> io::Result<()> {
    if present(&root.join(STAGING))? {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn publish(
    root: &Path,
    custom: bool,
    name: &str,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    if !custom {
        let permissions = hiroute_platform::file_security::private_defaults(false)?;
        return hiroute_platform::durable::write_windows_atomic_with(
            &root.join(name),
            &permissions,
            write,
        );
    }
    // The transaction has admitted the program directory. Only our own
    // staging namespace needs the exact private descriptor.
    let parent = security::open_identity(root, true)?;
    let staging_name = OsStr::new(STAGING);
    let staging = match security::create_owner_only_directory_at(&parent, staging_name) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let directory = security::open_identity_at(&parent, staging_name, true)?;
            security::verify_owner_only_handle(&directory, true)?;
            directory
        }
        Err(error) => return Err(error),
    };
    let name = OsStr::new(name);
    remove_staged_file(&staging, name)?;
    let mut file = security::create_owner_only_buffered_file_at(&staging, name)?;
    write(&mut file)?;
    file.sync_all()?;
    durable::rename_file_at(&file, &parent, name, true)?;
    file.sync_all()
}

fn remove_staged_file(directory: &File, name: &OsStr) -> io::Result<()> {
    match security::open_delete_at(directory, name, false) {
        Ok(file) => {
            security::verify_owner_only_handle(&file, false)?;
            security::delete_by_handle(&file)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub(super) fn cleanup(root: &Path) -> io::Result<()> {
    let staging = match security::open_delete(&root.join(STAGING), true) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    security::verify_owner_only_handle(&staging, true)?;
    for name in [UNINSTALLER, PREFERENCES, INVENTORY] {
        remove_staged_file(&staging, OsStr::new(name))?;
    }
    security::delete_by_handle(&staging)
}

pub(super) fn prune_new_directories(
    root: &Path,
    files: &BTreeMap<String, String>,
) -> io::Result<()> {
    let mut directories = BTreeSet::new();
    for name in files.keys() {
        directories.extend(
            Path::new(name)
                .ancestors()
                .skip(1)
                .filter(|parent| !parent.as_os_str().is_empty()),
        );
    }
    for relative in directories.into_iter().rev() {
        match security::open_delete(&root.join(relative), true)
            .and_then(|directory| security::delete_by_handle(&directory))
        {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupted_publication_keeps_the_destination_and_leaves_owned_cleanup() {
        use std::{fs, io::Write};
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("Desktop");
        security::create_owner_only_directory(&root).unwrap();
        reserve(&root).unwrap();
        let target = root.join(UNINSTALLER);
        fs::write(&target, b"retained").unwrap();
        assert!(
            publish(&root, true, UNINSTALLER, |file| {
                file.write_all(b"unfinished stage")?;
                Err(io::ErrorKind::Interrupted.into())
            })
            .is_err()
        );
        assert_eq!(fs::read(&target).unwrap(), b"retained");
        assert!(root.join(STAGING).join(UNINSTALLER).is_file());
        cleanup(&root).unwrap();
        assert!(!root.join(STAGING).exists());
        publish(&root, true, UNINSTALLER, |file| file.write_all(b"new")).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        cleanup(&root).unwrap();
    }
}
