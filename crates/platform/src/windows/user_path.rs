//! Only the fixed current-account standalone CLI directory enters the user PATH.
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::{io, ptr};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
    Security::{TOKEN_DUPLICATE, TOKEN_QUERY},
    System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock},
    System::Registry::{
        HKEY_CURRENT_USER, REG_EXPAND_SZ, REG_SZ, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ,
        RRF_RT_REG_SZ, RegGetValueW, RegSetKeyValueW,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
    UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    },
};

/// Let Windows merge and expand the machine and current user's registered PATH.
/// Task Scheduler's process environment can predate changes to either source.
pub fn current_environment_path() -> io::Result<Option<OsString>> {
    let mut token = ptr::null_mut();
    // SAFETY: the out parameter receives a new handle for this process token.
    if unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY | TOKEN_DUPLICATE,
            &mut token,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: ownership of the successfully opened token is transferred here.
    let _token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut block = ptr::null_mut();
    // SAFETY: the token is live and the out pointer is writable. FALSE excludes
    // the caller's inherited environment from the result.
    if unsafe { CreateEnvironmentBlock(&mut block, token, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: Windows supplies NUL-terminated UTF-16 entries ending in an
    // extra NUL. Read only within those entries, then free the owning block.
    let path = unsafe {
        let mut entry = block.cast::<u16>();
        let mut path = None;
        while *entry != 0 {
            let mut length = 0;
            while *entry.add(length) != 0 {
                length += 1;
            }
            let variable = std::slice::from_raw_parts(entry, length);
            if let Some(value) = variable.get(5..)
                && variable[..5]
                    .iter()
                    .zip(b"PATH=")
                    .all(|(&unit, &ascii)| unit <= 127 && (unit as u8).eq_ignore_ascii_case(&ascii))
            {
                path = Some(OsString::from_wide(value));
                break;
            }
            entry = entry.add(length + 1);
        }
        DestroyEnvironmentBlock(block);
        path
    };
    Ok(path)
}

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
