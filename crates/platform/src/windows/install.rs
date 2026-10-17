//! Installation admission uses the native architecture, including under emulation.
use sha2::{Digest, Sha256};
use std::io;
use std::{fs::File, os::windows::io::FromRawHandle};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};

pub fn native_target() -> io::Result<&'static str> {
    let mut process = 0_u16;
    let mut native = 0_u16;
    // SAFETY: current process and two correctly sized IMAGE_FILE_MACHINE outputs.
    if unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, &mut native) } == 0 {
        return Err(io::Error::last_os_error());
    }
    match native {
        0x8664 => Ok("x86_64-pc-windows-msvc"),
        0xaa64 => Ok("aarch64-pc-windows-msvc"),
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "INSTALL_ARCHITECTURE_UNSUPPORTED",
        )),
    }
}
pub fn require_x64() -> io::Result<()> {
    if native_target()? != "x86_64-pc-windows-msvc" || !cfg!(target_arch = "x86_64") {
        return Err(io::Error::other("INSTALL_ARCHITECTURE_UNSUPPORTED"));
    }
    Ok(())
}
pub fn desktop_mutex_name() -> io::Result<String> {
    Ok(format!(
        "Global\\HiRouteDesktopInstall-{:x}",
        Sha256::digest(super::security::current_user_sid()?)
    ))
}
pub struct DesktopInstallGuard {
    _handle: File,
}
/// NSIS holds the same named object for its complete process lifetime, across helpers.
pub fn desktop_install_guard() -> io::Result<DesktopInstallGuard> {
    use windows_sys::Win32::{
        Foundation::{ERROR_ALREADY_EXISTS, GetLastError},
        System::Threading::CreateMutexW,
    };
    let name: Vec<u16> = desktop_mutex_name()?
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: fixed per-account namespace, no borrowed pointers or initial mutex ownership.
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    // SAFETY: this unique kernel handle is closed by File; no file operations are exposed.
    let file = unsafe { File::from_raw_handle(handle.cast()) };
    if exists {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "INSTALL_MAINTENANCE_BUSY",
        ));
    }
    Ok(DesktopInstallGuard { _handle: file })
}
