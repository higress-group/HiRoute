//! Current-user Desktop program location. NSIS publishes InstallLocation before
//! starting the installed shell and removes it after removing the package.
use super::{background, security};
use std::{
    ffi::OsStr,
    fs::{self, File},
    io,
    os::windows::ffi::OsStrExt,
    os::windows::io::AsRawHandle,
    path::{Component, Path, PathBuf, Prefix},
};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, GENERIC_READ, GENERIC_WRITE},
    Storage::FileSystem::{
        DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetVolumeInformationByHandleW,
    },
    System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW},
};

pub fn default_root() -> io::Result<PathBuf> {
    Ok(background::local_app_data()?
        .join("Programs")
        .join("HiRoute"))
}

pub fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "INSTALL_DIRECTORY_INVALID")
}

fn folder(path: &Path) -> io::Result<()> {
    let text = path.to_str().ok_or_else(invalid)?;
    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return Err(invalid());
    };
    if !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::UNC(_, _))
        || !path.is_absolute()
        || path.file_name().is_none()
        || !matches!(components.next(), Some(Component::RootDir))
        || text[prefix.as_os_str().len()..]
            .split(['/', '\\'])
            .skip(1)
            .any(|part| {
                let stem = part
                    .split('.')
                    .next()
                    .unwrap_or("")
                    .trim_end_matches(' ')
                    .to_ascii_uppercase();
                part.is_empty()
                    || part.ends_with(['.', ' '])
                    || part.contains([':', '"', '*', '?', '<', '>', '|'])
                    || part.chars().any(char::is_control)
                    || matches!(
                        stem.as_str(),
                        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
                    )
                    || stem
                        .strip_prefix("COM")
                        .or_else(|| stem.strip_prefix("LPT"))
                        .is_some_and(|n| {
                            matches!(n, "¹" | "²" | "³")
                                || n.len() == 1 && n.as_bytes()[0].is_ascii_digit()
                        })
            })
    {
        return Err(invalid());
    }
    Ok(())
}

/// Normalize ordinary drive-letter and UNC paths for the directory picker.
/// A selected volume/share root is a parent for the product directory.
pub fn normalize_destination(path: &Path) -> io::Result<PathBuf> {
    let text = path.to_str().ok_or_else(invalid)?.replace('/', "\\");
    let mut path = PathBuf::from(&text);
    if path.is_absolute() && path.file_name().is_none() && path.parent().is_none() {
        path.push("HiRoute");
    } else {
        path = PathBuf::from(text.trim_end_matches('\\'));
    }
    folder(&path)?;
    Ok(path)
}

fn canonical(path: &Path) -> io::Result<PathBuf> {
    let resolved = fs::canonicalize(path)?;
    let value = resolved.to_str().ok_or_else(invalid)?;
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        return Ok(PathBuf::from(format!(r"\\{rest}")));
    }
    Ok(PathBuf::from(value.strip_prefix(r"\\?\").unwrap_or(value)))
}

/// Resolve junctions and drive aliases before selecting a publication path.
/// Apply the same reserved-directory rule to the actual target.
pub fn resolve_destination(path: &Path) -> io::Result<PathBuf> {
    folder(path)?;
    for existing in path.ancestors() {
        match canonical(existing) {
            Ok(mut resolved) => {
                resolved.push(path.strip_prefix(existing).map_err(|_| invalid())?);
                let resolved = PathBuf::from(
                    resolved
                        .to_str()
                        .ok_or_else(invalid)?
                        .trim_end_matches('\\'),
                );
                folder(&resolved)?;
                validate_reserved(&resolved)?;
                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // An existing dangling junction is not a directory we may create.
                match fs::symlink_metadata(existing) {
                    Err(missing) if missing.kind() == io::ErrorKind::NotFound => {}
                    Err(other) => return Err(other),
                    Ok(_) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        }
    }
    Err(io::ErrorKind::NotFound.into())
}

pub fn same_name(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .replace('/', "\\")
        .eq_ignore_ascii_case(&right.to_string_lossy().replace('/', "\\"))
}

pub fn validate_reserved(path: &Path) -> io::Result<()> {
    let mut reserved_directories = Vec::new();
    for reserved in [
        background::local_app_data()?.join("HiRoute"),
        background::profile()?.join(".local"),
    ] {
        let reserved = canonical(&reserved).or_else(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                Ok(reserved)
            } else {
                Err(error)
            }
        })?;
        if path.ancestors().any(|p| same_name(p, &reserved))
            || reserved.ancestors().any(|p| same_name(p, path))
        {
            return Err(reserved_destination());
        }
        match security::open_identity(&reserved, true) {
            Ok(file) => reserved_directories.push(security::file_identity(&file)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    for part in path.ancestors() {
        match security::open_identity(part, true) {
            Ok(file) => {
                if reserved_directories.contains(&security::file_identity(&file)?) {
                    return Err(reserved_destination());
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn reserved_destination() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "INSTALL_DESTINATION_OVERLAPS_DATA",
    )
}

/// A package destination is borrowed. Create only missing directories and
/// leave their native permissions alone, including on filesystems without ACLs.
pub fn ensure_package_directory(path: &Path) -> io::Result<File> {
    match security::open_identity(path, true) {
        Ok(file) => return Ok(file),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let parent = ensure_package_directory(path.parent().ok_or_else(invalid)?)?;
    let leaf = path.file_name().ok_or_else(invalid)?;
    match super::path::relative(
        &parent,
        leaf,
        true,
        GENERIC_READ,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        true,
        None,
    ) {
        Ok(file) => Ok(file),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            security::open_identity_at(&parent, leaf, true)
        }
        Err(error) => Err(error),
    }
}

pub fn create_package_at(directory: &File, name: &OsStr) -> io::Result<File> {
    // Create-new, never replace an object at a borrowed destination.
    super::path::create_buffered_file_relative(
        directory,
        name,
        GENERIC_READ | GENERIC_WRITE | DELETE,
        None,
    )
}

pub fn open_package_for_removal(directory: &File, name: &OsStr) -> io::Result<File> {
    // Deny write and delete sharing while checking the held file.
    super::path::relative(
        directory,
        name,
        false,
        GENERIC_READ | DELETE,
        FILE_SHARE_READ,
        false,
        None,
    )
}

pub fn validate_custom(path: &Path) -> io::Result<()> {
    folder(path)?;
    let drive = path.ancestors().last().ok_or_else(invalid)?;
    let wide: Vec<_> = drive.as_os_str().encode_wide().chain(Some(0)).collect();
    // Drive type selects where code runs. Any usable volume may store a copy
    // of the installer, but only stable local volumes are executed in place.
    if unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(wide.as_ptr()) } != 3 {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    validate_reserved(path)?;
    let volume = security::open_identity(drive, true)?;
    let mut flags = 0;
    // SAFETY: the volume directory handle and output flags remain live.
    let status = unsafe {
        GetVolumeInformationByHandleW(
            volume.as_raw_handle().cast(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut flags,
            std::ptr::null_mut(),
            0,
        )
    };
    if status == 0 {
        return Err(io::Error::last_os_error());
    }
    const FILE_PERSISTENT_ACLS: u32 = 0x0000_0008;
    if flags & FILE_PERSISTENT_ACLS == 0 {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let user = security::current_user_sid()?;
    // Include DOS short-name aliases and check every existing parent before use.
    for ancestor in path.ancestors() {
        match security::open_identity(ancestor, true) {
            Ok(file) => {
                security::verify_program_parent(&file, ancestor == drive, ancestor == path, &user)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub fn registered_root() -> io::Result<Option<PathBuf>> {
    let key: Vec<_> = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\HiRoute"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let name: Vec<_> = "InstallLocation".encode_utf16().chain(Some(0)).collect();
    let mut buffer = [0u16; 1024];
    let mut size = size_of_val(&buffer) as u32;
    // The x64 shell reads the same HKCU view used by the NSIS SetContext macro.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let units = buffer.get(..size as usize / 2).ok_or_else(invalid)?;
    let value =
        String::from_utf16(units.strip_suffix(&[0]).ok_or_else(invalid)?).map_err(|_| invalid())?;
    let path = PathBuf::from(
        value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .ok_or_else(invalid)?,
    );
    if path != default_root()? {
        validate_existing_or_missing_custom(&path)?;
    }
    Ok(Some(path))
}

/// A registration or prepared journal can outlive its directory. Repair may
/// recreate the missing folder after checking the remaining path and owner.
pub fn validate_existing_or_missing_custom(path: &Path) -> io::Result<()> {
    validate_custom(path)
}

pub fn installed_root() -> io::Result<PathBuf> {
    match registered_root()? {
        Some(root) => Ok(root),
        None => default_root(),
    }
}

pub fn same_existing_path(left: &Path, right: &Path, directory: bool) -> io::Result<bool> {
    let left = security::open_identity(left, directory)?;
    let right = security::open_identity(right, directory)?;
    Ok(security::file_identity(&left)? == security::file_identity(&right)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn custom_location_is_an_unambiguous_local_folder() {
        assert!(folder(Path::new(r"D:\My Apps\路由")).is_ok());
        assert!(folder(Path::new(r"\\server\share\HiRoute")).is_ok());
        assert_eq!(
            normalize_destination(Path::new(r"A:\")).unwrap(),
            Path::new(r"A:\HiRoute")
        );
        assert_eq!(
            normalize_destination(Path::new(r"\\server\share\")).unwrap(),
            Path::new(r"\\server\share\HiRoute")
        );
        for path in [
            r"D:\",
            r"\\?\D:\HiRoute",
            r"D:HiRoute",
            r"D:\apps\..\HiRoute",
            r"D:\apps.\HiRoute",
            r"D:\HiRoute:stream",
            r"D:\NUL.txt",
        ] {
            assert!(folder(Path::new(path)).is_err(), "{path}");
        }
    }

    #[test]
    fn custom_location_accepts_a_new_private_program_folder_and_excludes_service_state() {
        let local = background::local_app_data().unwrap();
        let parent = tempfile::tempdir_in(&local).unwrap();
        let programs = parent.path().join("My HiRoute");
        validate_existing_or_missing_custom(&programs).unwrap();
        // A normal inherited ACL in the current user's profile is admissible.
        std::fs::create_dir(&programs).unwrap();
        validate_existing_or_missing_custom(&programs).unwrap();
        std::fs::remove_dir(&programs).unwrap();
        security::create_owner_only_directory(&programs).unwrap();
        validate_existing_or_missing_custom(&programs).unwrap();
        std::fs::remove_dir(&programs).unwrap();
        validate_existing_or_missing_custom(&programs).unwrap();
        assert!(validate_custom(&local.join("HiRoute/cli/Desktop")).is_err());
        assert!(
            validate_custom(&background::profile().unwrap().join(".local/bin/Desktop")).is_err()
        );
    }
}
