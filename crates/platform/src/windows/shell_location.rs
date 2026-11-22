//! Current-user Desktop program location. NSIS publishes InstallLocation before
//! starting the installed shell and removes it after removing the package.
use super::{background, security};
use std::{
    io,
    path::{Component, Path, PathBuf, Prefix},
};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
    System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW},
};

pub fn default_root() -> io::Result<PathBuf> {
    Ok(background::local_app_data()?.join("Programs/HiRoute"))
}

pub fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "INSTALL_DIRECTORY_INVALID")
}

fn local_folder(path: &Path) -> io::Result<()> {
    let text = path.to_str().ok_or_else(invalid)?;
    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return Err(invalid());
    };
    if !matches!(prefix.kind(), Prefix::Disk(_))
        || components.next() != Some(Component::RootDir)
        || path.file_name().is_none()
        || text[2..].split(['/', '\\']).skip(1).any(|part| {
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

pub fn validate_custom(path: &Path) -> io::Result<()> {
    local_folder(path)?;
    let drive = path.ancestors().last().ok_or_else(invalid)?;
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<_> = drive.as_os_str().encode_wide().chain(Some(0)).collect();
    // GetDriveTypeW returns DRIVE_FIXED (3) for a local fixed disk.
    if unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(wide.as_ptr()) } != 3 {
        return Err(invalid());
    }
    let mut reserved_directories = Vec::new();
    for reserved in [
        background::local_app_data()?.join("HiRoute"),
        background::profile()?.join(".local"),
    ] {
        if path.ancestors().any(|p| {
            p.to_string_lossy()
                .replace('/', "\\")
                .eq_ignore_ascii_case(&reserved.to_string_lossy().replace('/', "\\"))
        }) {
            return Err(invalid());
        }
        match security::open_identity(&reserved, true) {
            Ok(file) => reserved_directories.push(security::file_identity(&file)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    let user = security::current_user_sid()?;
    // Include DOS short-name aliases and check every existing parent before use.
    for ancestor in path.ancestors() {
        match security::open_identity(ancestor, true) {
            Ok(file) => {
                if reserved_directories.contains(&security::file_identity(&file)?) {
                    return Err(invalid());
                }
                security::verify_program_parent(&file, ancestor == drive, &user)?;
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
        validate_custom(&path)?;
        security::open_owner_only(&path, true)?;
    }
    Ok(Some(path))
}

pub fn installed_root() -> io::Result<PathBuf> {
    match registered_root()? {
        Some(root) => Ok(root),
        None => default_root(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn custom_location_is_an_unambiguous_local_folder() {
        assert!(local_folder(Path::new(r"D:\My Apps\路由")).is_ok());
        for path in [
            r"D:\",
            r"\\server\share\HiRoute",
            r"\\?\D:\HiRoute",
            r"D:HiRoute",
            r"D:\apps\..\HiRoute",
            r"D:\apps.\HiRoute",
            r"D:\HiRoute:stream",
            r"D:\NUL.txt",
        ] {
            assert!(local_folder(Path::new(path)).is_err(), "{path}");
        }
    }

    #[test]
    fn custom_location_accepts_a_new_private_program_folder_and_excludes_service_state() {
        let local = background::local_app_data().unwrap();
        let parent = tempfile::tempdir_in(&local).unwrap();
        let programs = parent.path().join("My HiRoute");
        validate_custom(&programs).unwrap();
        security::create_owner_only_directory(&programs).unwrap();
        validate_custom(&programs).unwrap();
        assert!(validate_custom(&local.join("HiRoute/cli/Desktop")).is_err());
        assert!(
            validate_custom(&background::profile().unwrap().join(".local/bin/Desktop")).is_err()
        );
    }
}
