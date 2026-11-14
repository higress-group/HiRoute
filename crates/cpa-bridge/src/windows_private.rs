//! CPA-owned Windows private objects. External native-client credentials have
//! a different admission contract and must not call these helpers.
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use hiroute_platform::windows::{diagnostic_files as native, durable, security};
use zeroize::Zeroizing;

pub(crate) fn directory(path: &Path) -> io::Result<File> {
    let directory = security::ensure_private_directory(path)?;
    security::verify_owner_only_handle(&directory, true)?;
    Ok(directory)
}

pub(crate) fn existing_directory(path: &Path) -> io::Result<File> {
    security::open_owner_only(path, true)
}

pub(crate) fn file(path: &Path) -> io::Result<File> {
    let file = security::open_owner_only(path, false)?;
    native::stat(&file, false)?;
    Ok(file)
}

/// CPA can replace an auth file with a new file that inherits the private
/// directory's ACL. Read through the verified file handle.
pub(crate) fn auth_file(path: &Path) -> io::Result<File> {
    let parent = existing_directory(path.parent().ok_or(io::ErrorKind::InvalidInput)?)?;
    security::open_inherited_private_file_at(
        &parent,
        path.file_name().ok_or(io::ErrorKind::InvalidInput)?,
    )
}

pub(crate) fn read(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let file = file(path)?;
    if file.metadata()?.len() > limit {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(bytes)
}

pub(crate) fn read_auth_file(path: &Path, limit: u64) -> io::Result<Zeroizing<Vec<u8>>> {
    let file = auth_file(path)?;
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(limit + 1).read_to_end(bytes.as_mut())?;
    if bytes.len() as u64 > limit {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(bytes)
}

pub(crate) fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_with_existing_check(path, bytes, |_, file| {
        security::verify_owner_only_handle(file, false)
    })
}

pub(crate) fn write_auth_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_with_existing_check(path, bytes, security::verify_inherited_private_file_handle)
}

fn write_with_existing_check(
    path: &Path,
    bytes: &[u8],
    verify_existing: impl FnOnce(&File, &File) -> io::Result<()>,
) -> io::Result<()> {
    let parent_path = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let parent = existing_directory(parent_path)?;
    let leaf = path.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    let leaf_text = leaf.to_str().ok_or(io::ErrorKind::InvalidInput)?;
    let replace = match native::open_file(&parent, leaf_text, native::OpenMode::Read) {
        Ok(existing) => {
            verify_existing(&parent, &existing)?;
            true
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    let nonce = crate::config::SecretText::generate().map_err(io::Error::other)?;
    let stage_name = format!(".write-{}.tmp", nonce.expose());
    let mut stage = security::create_owner_only_file_at(&parent, stage_name.as_ref())?;
    stage.write_all(bytes)?;
    stage.sync_all()?;
    crate::request_context::io_check()?;
    if let Err(error) = durable::rename_file_at(&stage, &parent, leaf, replace) {
        // A definite failed publication may remove only the held stage created
        // by this attempt. An uncertain transition remains recoverable.
        if error.kind() == io::ErrorKind::AlreadyExists {
            security::delete_by_handle(&stage)?;
        }
        return Err(error);
    }
    stage.sync_all()?;
    security::verify_owner_only_handle(&stage, false)?;
    native::stat(&stage, false)?;
    Ok(())
}

pub(crate) fn new_directory(path: &Path) -> io::Result<()> {
    let parent = existing_directory(path.parent().ok_or(io::ErrorKind::InvalidInput)?)?;
    drop(security::create_owner_only_directory_at(
        &parent,
        path.file_name().ok_or(io::ErrorKind::InvalidInput)?,
    )?);
    Ok(())
}

pub(crate) fn lock(path: &Path) -> io::Result<File> {
    let parent = directory(path.parent().ok_or(io::ErrorKind::InvalidInput)?)?;
    let leaf = path.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    let identity = match file(path) {
        Ok(file) => security::file_identity(&file)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match security::create_owner_only_file_at(&parent, leaf) {
                Ok(file) => {
                    file.sync_all()?;
                    security::file_identity(&file)?
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    security::file_identity(&file(path)?)?
                }
                Err(error) => return Err(error),
            }
        }
        Err(error) => return Err(error),
    };
    let file = security::open_read_write(path)?;
    security::verify_owner_only_handle(&file, false)?;
    native::stat(&file, false)?;
    if security::file_identity(&file)? != identity {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(file)
}

pub(crate) fn names(path: &Path) -> io::Result<Vec<String>> {
    let directory = existing_directory(path)?;
    native::list_names(&directory)
}

pub(crate) fn remove_file(path: &Path) -> io::Result<()> {
    let parent = existing_directory(path.parent().ok_or(io::ErrorKind::InvalidInput)?)?;
    let file = security::open_delete_at(
        &parent,
        path.file_name().ok_or(io::ErrorKind::InvalidInput)?,
        false,
    )?;
    security::verify_owner_only_handle(&file, false)?;
    native::stat(&file, false)?;
    security::delete_by_handle(&file)
}

pub(crate) fn remove_empty_directory(path: &Path) -> io::Result<()> {
    let parent = existing_directory(path.parent().ok_or(io::ErrorKind::InvalidInput)?)?;
    let directory = security::open_delete_at(
        &parent,
        path.file_name().ok_or(io::ErrorKind::InvalidInput)?,
        true,
    )?;
    security::verify_owner_only_handle(&directory, true)?;
    security::delete_by_handle(&directory)
}

/// A managed session owns this entire tree. CPA generates only flat auth files;
/// all directory and HiRoute metadata descriptors remain strict. Unknown ACLs,
/// links and reparses stop cleanup without touching the foreign object.
pub(crate) fn remove_session_tree(path: &Path, auth_files: bool) -> io::Result<()> {
    let parent = existing_directory(path.parent().ok_or(io::ErrorKind::InvalidInput)?)?;
    let directory = security::open_delete_at(
        &parent,
        path.file_name().ok_or(io::ErrorKind::InvalidInput)?,
        true,
    )?;
    security::verify_owner_only_handle(&directory, true)?;
    remove_children(&directory, auth_files, 0)?;
    security::delete_by_handle(&directory)
}

fn remove_children(parent: &File, auth_files: bool, depth: usize) -> io::Result<()> {
    if depth > 32 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    for name in native::list_names(parent)? {
        match security::open_delete_at(parent, name.as_ref(), false) {
            Ok(file) => {
                if auth_files {
                    security::verify_inherited_private_file_handle(parent, &file)?;
                } else {
                    security::verify_owner_only_handle(&file, false)?;
                    native::stat(&file, false)?;
                }
                security::delete_by_handle(&file)?;
            }
            Err(_) => {
                let directory = security::open_delete_at(parent, name.as_ref(), true)?;
                security::verify_owner_only_handle(&directory, true)?;
                remove_children(&directory, name == "auth", depth + 1)?;
                security::delete_by_handle(&directory)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn lease_root(label: &str) -> io::Result<PathBuf> {
    let root = hiroute_platform::windows::background::local_app_data()?.join("HiRoute/cpa-leases");
    directory(&root)?;
    let path = root.join(label);
    directory(&path)?;
    Ok(path)
}

#[cfg(test)]
#[path = "windows_private_tests.rs"]
mod tests;
