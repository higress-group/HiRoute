//! Handle-relative traversal binds each operation to the directory actually
//! inspected. Sharing deletion on traversal handles is safe here: renaming an
//! ancestor cannot retarget a subsequent NtCreateFile RootDirectory operation.

use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Component, Path, PathBuf};
use std::ptr::{null, null_mut};
use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_CREATE, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_REPARSE_POINT,
    FILE_SYNCHRONOUS_IO_NONALERT, FILE_WRITE_THROUGH, NtCreateFile,
};
use windows_sys::Win32::Foundation::{
    GENERIC_READ, OBJ_CASE_INSENSITIVE, OBJ_DONT_REPARSE, RtlNtStatusToDosError, UNICODE_STRING,
};
use windows_sys::Win32::Security::SECURITY_DESCRIPTOR;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    SYNCHRONIZE,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

pub(super) const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;

pub(super) fn open(
    path: &Path,
    directory: bool,
    access: u32,
    share: u32,
    create: bool,
    descriptor: Option<&SECURITY_DESCRIPTOR>,
) -> io::Result<File> {
    let (root, mut names) = split(path)?;
    let mut parent = open_root(&root, SHARE_ALL)?;
    let Some(leaf) = names.pop() else {
        return if directory && !create {
            Ok(parent)
        } else {
            Err(invalid_path())
        };
    };
    for name in names {
        parent = relative(&parent, name, true, GENERIC_READ, SHARE_ALL, false, None)?;
    }
    relative(&parent, leaf, directory, access, share, create, descriptor)
}

/// Retain every inspected directory without delete sharing and the leaf without
/// write/delete sharing. Unlike ordinary relative traversal, this read lease
/// preserves the entire admitted pathname until the caller releases it.
pub(super) fn retained_read(path: &Path) -> io::Result<(File, Vec<File>, OsString)> {
    let (root, mut names) = split(path)?;
    let leaf = names.pop().ok_or_else(invalid_path)?;
    let mut parents = vec![open_root(&root, FILE_SHARE_READ | FILE_SHARE_WRITE)?];
    for name in names {
        let parent = parents.last().ok_or_else(invalid_path)?;
        parents.push(relative(
            parent,
            name,
            true,
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            false,
            None,
        )?);
    }
    let file = relative(
        parents.last().ok_or_else(invalid_path)?,
        leaf,
        false,
        GENERIC_READ,
        FILE_SHARE_READ,
        false,
        None,
    )?;
    Ok((file, parents, leaf.to_owned()))
}

fn split(path: &Path) -> io::Result<(PathBuf, Vec<&OsStr>)> {
    let mut root = PathBuf::new();
    let mut names = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir if names.is_empty() => root.push(component),
            Component::Normal(name) => {
                // Reject alternate data streams as well as unexpected separators.
                if name
                    .encode_wide()
                    .any(|word| matches!(word, 0 | 58 | 47 | 92))
                {
                    return Err(invalid_path());
                }
                names.push(name);
            }
            _ => return Err(invalid_path()),
        }
    }
    if !path.is_absolute() || !root.is_absolute() {
        return Err(invalid_path());
    }
    Ok((root, names))
}

fn open_root(root: &Path, share: u32) -> io::Result<File> {
    let parent = OpenOptions::new()
        .read(true)
        .share_mode(share)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(root)?;
    check_type(&parent, true)?;
    Ok(parent)
}

pub(super) fn relative(
    parent: &File,
    leaf: &OsStr,
    directory: bool,
    access: u32,
    share: u32,
    create: bool,
    descriptor: Option<&SECURITY_DESCRIPTOR>,
) -> io::Result<File> {
    relative_with_write_policy(
        parent, leaf, directory, access, share, create, descriptor, true,
    )
}

/// Buffered creation is reserved for request-owned temporary payloads. The
/// caller verifies complete writes before sealing; persistence uses other factories.
pub(super) fn create_buffered_file_relative(
    parent: &File,
    leaf: &OsStr,
    access: u32,
    descriptor: &SECURITY_DESCRIPTOR,
) -> io::Result<File> {
    relative_with_write_policy(
        parent,
        leaf,
        false,
        access,
        0,
        true,
        Some(descriptor),
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn relative_with_write_policy(
    parent: &File,
    leaf: &OsStr,
    directory: bool,
    access: u32,
    share: u32,
    create: bool,
    descriptor: Option<&SECURITY_DESCRIPTOR>,
    write_through: bool,
) -> io::Result<File> {
    check_type(parent, true)?;
    if leaf.is_empty()
        || leaf == OsStr::new(".")
        || leaf == OsStr::new("..")
        || leaf
            .encode_wide()
            .any(|word| matches!(word, 0 | 58 | 47 | 92))
    {
        return Err(invalid_path());
    }
    let mut name: Vec<u16> = leaf.encode_wide().collect();
    let bytes = name
        .len()
        .checked_mul(2)
        .and_then(|length| u16::try_from(length).ok())
        .ok_or_else(invalid_path)?;
    let name = UNICODE_STRING {
        Length: bytes,
        MaximumLength: bytes,
        Buffer: name.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle().cast(),
        ObjectName: &name,
        Attributes: OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE,
        SecurityDescriptor: descriptor.map_or(null(), |value| value),
        SecurityQualityOfService: null(),
    };
    let mut status = IO_STATUS_BLOCK::default();
    let mut handle = null_mut();
    // SAFETY: all attribute buffers and the held parent remain live through this
    // synchronous call. One component is resolved relative to the inspected
    // object; reparses are rejected both during parsing and on the returned handle.
    let result = unsafe {
        NtCreateFile(
            &mut handle,
            access | SYNCHRONIZE,
            &attributes,
            &mut status,
            null(),
            FILE_ATTRIBUTE_NORMAL,
            share,
            if create { FILE_CREATE } else { FILE_OPEN },
            FILE_OPEN_REPARSE_POINT
                | FILE_SYNCHRONOUS_IO_NONALERT
                | if write_through { FILE_WRITE_THROUGH } else { 0 }
                | if directory {
                    FILE_DIRECTORY_FILE
                } else {
                    FILE_NON_DIRECTORY_FILE
                },
            null(),
            0,
        )
    };
    if result < 0 {
        // SAFETY: conversion consumes a numeric NTSTATUS and owns no memory.
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(result) } as i32,
        ));
    }
    if handle.is_null() {
        return Err(io::Error::other("missing filesystem handle"));
    }
    // SAFETY: successful NtCreateFile returns one uniquely owned handle.
    let file = unsafe { File::from_raw_handle(handle.cast()) };
    check_type(&file, directory)?;
    Ok(file)
}

fn check_type(file: &File, directory: bool) -> io::Result<()> {
    let metadata = file.metadata()?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || metadata.is_dir() != directory
        || metadata.is_file() == directory
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unsafe object type",
        ));
    }
    Ok(())
}

fn invalid_path() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "absolute normalized filesystem path required",
    )
}
