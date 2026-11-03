//! A junction does not require Developer Mode or the symbolic-link privilege.
use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
use std::{fs::OpenOptions, io, path::Path};
use windows_sys::Win32::{
    Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT},
    System::IO::DeviceIoControl,
};

pub(crate) fn create(source: &Path, destination: &Path) -> io::Result<()> {
    let canonical = source.canonicalize()?;
    let target = canonical.to_str().ok_or(io::ErrorKind::InvalidInput)?;
    let target = target.strip_prefix(r"\\?\").unwrap_or(target);
    // NTFS mount points cannot target network shares. Do not create a broken link.
    if target.starts_with("UNC\\") || target.starts_with(r"\\") {
        return Err(io::ErrorKind::Unsupported.into());
    }
    let substitute: Vec<u16> = format!(r"\??\{target}").encode_utf16().collect();
    let print: Vec<u16> = target.encode_utf16().collect();
    let length = 16 + (substitute.len() + print.len() + 2) * 2;
    if length > 16384 {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut buffer = vec![0u8; length];
    buffer[0..4].copy_from_slice(&0xa0000003u32.to_le_bytes());
    buffer[4..6].copy_from_slice(&((length - 8) as u16).to_le_bytes());
    buffer[10..12].copy_from_slice(&((substitute.len() * 2) as u16).to_le_bytes());
    buffer[12..14].copy_from_slice(&(((substitute.len() + 1) * 2) as u16).to_le_bytes());
    buffer[14..16].copy_from_slice(&((print.len() * 2) as u16).to_le_bytes());
    for (index, word) in substitute
        .into_iter()
        .chain(Some(0))
        .chain(print)
        .chain(Some(0))
        .enumerate()
    {
        buffer[16 + index * 2..18 + index * 2].copy_from_slice(&word.to_le_bytes());
    }
    // create_dir is exclusive: an existing private skill or link is never adopted.
    std::fs::create_dir(destination)?;
    let identity = super::security::open_identity(destination, true)?;
    let file = OpenOptions::new()
        .write(true)
        .share_mode(3)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(destination)?;
    let original = super::security::file_identity(&identity)?;
    if super::security::file_identity(&file)? != original {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let mut returned = 0;
    // SAFETY: fixed FSCTL, live bounded mount-point buffer and owned no-follow handle.
    let result = unsafe {
        DeviceIoControl(
            file.as_raw_handle().cast(),
            0x000900a4,
            buffer.as_ptr().cast(),
            length as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        let error = io::Error::last_os_error();
        drop(file);
        // Preserve a replacement; cleanup authority covers only our empty directory.
        if super::security::open_identity(destination, true)
            .and_then(|now| super::security::file_identity(&now))
            .is_ok_and(|now| now == original)
        {
            let _ = std::fs::remove_dir(destination);
        }
        return Err(error);
    }
    Ok(())
}
