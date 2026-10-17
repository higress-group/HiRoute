//! Only the fixed current-account standalone CLI directory enters the user PATH.
use std::{io, ptr};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
    System::Registry::{
        HKEY_CURRENT_USER, REG_EXPAND_SZ, REG_SZ, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ,
        RRF_RT_REG_SZ, RegGetValueW, RegSetKeyValueW,
    },
    UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    },
};
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
pub fn configure(enabled: bool) -> io::Result<bool> {
    let entry = super::background::profile()?
        .join(".local/bin")
        .to_string_lossy()
        .replace('/', "\\");
    let key = wide("Environment");
    let name = wide("Path");
    let mut buffer = [0_u16; 32768];
    let mut size = (buffer.len() * 2) as u32;
    let mut kind = REG_EXPAND_SZ;
    // SAFETY: fixed current-user Environment value and bounded buffers.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND,
            &mut kind,
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    let original = if status == ERROR_FILE_NOT_FOUND {
        String::new()
    } else {
        if status != ERROR_SUCCESS || size < 2 || size % 2 != 0 || size as usize > buffer.len() * 2
        {
            return Err(io::Error::other("USER_PATH_UNAVAILABLE"));
        }
        if buffer[size as usize / 2 - 1] != 0 {
            return Err(io::Error::other("USER_PATH_UNAVAILABLE"));
        }
        String::from_utf16(&buffer[..size as usize / 2 - 1]).map_err(io::Error::other)?
    };
    let normalized = |v: &str| {
        v.trim()
            .trim_matches('"')
            .trim_end_matches('\\')
            .to_lowercase()
    };
    let wanted = normalized(&entry);
    let present = original.split(';').any(|v| normalized(v) == wanted);
    if enabled == present {
        return Ok(false);
    }
    let value = if enabled {
        format!(
            "{}{}{}",
            original,
            if original.is_empty() || original.ends_with(';') {
                ""
            } else {
                ";"
            },
            entry
        )
    } else {
        original
            .split(';')
            .filter(|v| normalized(v) != wanted)
            .collect::<Vec<_>>()
            .join(";")
    };
    let value = wide(&value);
    if value.len() > 32767 {
        return Err(io::Error::other("USER_PATH_UNAVAILABLE"));
    }
    if !matches!(kind, REG_SZ | REG_EXPAND_SZ) {
        return Err(io::Error::other("USER_PATH_UNAVAILABLE"));
    }
    // SAFETY: replace the fixed current-user PATH, preserving all unrelated entries and type.
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            kind,
            value.as_ptr().cast(),
            (value.len() * 2) as u32,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let environment = wide("Environment");
    // SAFETY: advisory broadcast, bounded timeout, immutable UTF-16 environment label.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            environment.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            2000,
            ptr::null_mut(),
        );
    }
    Ok(true)
}
