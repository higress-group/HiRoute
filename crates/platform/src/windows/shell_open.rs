use std::{io, ptr};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
pub fn url(value: &str) -> io::Result<()> {
    let url = url::Url::parse(value).map_err(io::Error::other)?;
    if value.len() > 2048
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
        || !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let operation: Vec<u16> = "open\0".encode_utf16().collect();
    let url: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
    // SAFETY: fixed shell association verb, admitted URL, live UTF-16 buffers.
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            operation.as_ptr(),
            url.as_ptr(),
            ptr::null(),
            ptr::null(),
            1,
        )
    };
    if result as isize <= 32 {
        Err(io::Error::other("BROWSER_UNAVAILABLE"))
    } else {
        Ok(())
    }
}
pub fn startup_failure() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    let title: Vec<u16> = "HiRoute\0".encode_utf16().collect();
    let message:Vec<u16>="HiRoute could not open its Windows window (SHELL_RUNTIME_UNAVAILABLE). Repair Microsoft Edge WebView2 Runtime or reinstall HiRoute. Existing service data is preserved.\0".encode_utf16().collect();
    // SAFETY: fixed diagnostic text and a native-owned modal; no caller HTML or commands.
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}
