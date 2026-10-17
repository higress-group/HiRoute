//! The shell's fixed current-user login entry. No script or arbitrary registry API.
use std::{
    io,
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
    System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
    },
};
const KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const NAME: &str = "HiRoute";
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn checked_binary(path: &Path) -> io::Result<()> {
    let expected = super::background::local_app_data()?.join("Programs/HiRoute/hiroute-shell.exe");
    if !path.is_absolute()
        || path != expected
        || path
            .components()
            .any(|p| matches!(p, std::path::Component::ParentDir))
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "SHELL_LOGIN_CONFLICT",
        ));
    }
    Ok(())
}
fn command_line(binary: &Path) -> String {
    // Always quote, including ordinary profiles whose path contains no spaces.
    format!("\"{}\" --background", binary.to_string_lossy())
}
fn command() -> io::Result<Option<String>> {
    let key = wide(KEY);
    let name = wide(NAME);
    let mut buffer = [0_u16; 32768];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: fixed HKCU subkey, bounded UTF-16 output, no borrowed pointers escape.
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
    if status != ERROR_SUCCESS || size < 2 || size as usize > buffer.len() * 2 || size % 2 != 0 {
        return Err(io::Error::other("SHELL_LOGIN_UNAVAILABLE"));
    }
    let count = size as usize / 2;
    if buffer[count - 1] != 0 {
        return Err(io::Error::other("SHELL_LOGIN_CONFLICT"));
    }
    String::from_utf16(&buffer[..count - 1])
        .map(Some)
        .map_err(io::Error::other)
}
pub fn configured_binary() -> io::Result<Option<PathBuf>> {
    let Some(command) = command()? else {
        return Ok(None);
    };
    let value = command
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix("\" --background"))
        .ok_or_else(|| io::Error::other("SHELL_LOGIN_CONFLICT"))?;
    let path = PathBuf::from(value);
    checked_binary(&path)?;
    Ok(Some(path))
}
pub fn enabled(binary: &Path) -> io::Result<bool> {
    checked_binary(binary)?;
    match configured_binary()? {
        Some(path) if path != binary => Err(io::Error::other("SHELL_LOGIN_CONFLICT")),
        value => Ok(value.is_some()),
    }
}
pub fn set(binary: &Path, enabled: bool) -> io::Result<()> {
    checked_binary(binary)?;
    if let Some(existing) = configured_binary()?
        && existing != binary
    {
        return Err(io::Error::other("SHELL_LOGIN_CONFLICT"));
    }
    if !enabled {
        return remove();
    }
    let _held = super::security::open_identity(binary, false)?;
    let value = wide(&command_line(binary));
    let key = wide(KEY);
    let name = wide(NAME);
    // SAFETY: fixed HKCU Run value and live, NUL-terminated UTF-16 data.
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            REG_SZ,
            value.as_ptr().cast(),
            (value.len() * 2) as u32,
        )
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}
pub fn remove() -> io::Result<()> {
    match configured_binary() {
        Ok(None) => return Ok(()),
        Err(error) if error.to_string() == "SHELL_LOGIN_CONFLICT" => return Ok(()),
        Err(error) => return Err(error),
        Ok(Some(_)) => {}
    }
    let key = wide(KEY);
    let name = wide(NAME);
    // SAFETY: deletes only the admitted fixed current-user Run value.
    let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
    if matches!(status, ERROR_SUCCESS | ERROR_FILE_NOT_FOUND) {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_login_command_is_quoted_even_for_paths_without_spaces() {
        let binary = super::super::background::local_app_data()
            .unwrap()
            .join("Programs/HiRoute/hiroute-shell.exe");
        let command = command_line(&binary);
        let decoded = command
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix("\" --background"))
            .unwrap();
        assert_eq!(Path::new(decoded), binary);
        assert!(checked_binary(&binary).is_ok());
        assert!(
            checked_binary(
                &binary
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("Other/hiroute-shell.exe")
            )
            .is_err()
        );
    }
}
