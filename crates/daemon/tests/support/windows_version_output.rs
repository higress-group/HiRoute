//! Read actual GUI daemon output from the fixture's own console, without pipes.
use std::ffi::OsString;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};

use serde_json::Value;
use windows_sys::Win32::Foundation::{
    GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Console::{
    CONSOLE_SCREEN_BUFFER_INFO, GetConsoleScreenBufferInfo, ReadConsoleOutputCharacterW,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, GetExitCodeProcess, PROCESS_INFORMATION, STARTUPINFOW, WaitForSingleObject,
};

pub(super) fn without_inherited_stdio(daemon: &Path, flag: &str) -> Value {
    let name: Vec<_> = "CONOUT$\0".encode_utf16().collect();
    // SAFETY: open this fixture's already allocated console buffer only. It is
    // not a process-global standard handle and is never inherited by the child.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        )
    };
    assert_ne!(handle, INVALID_HANDLE_VALUE);
    // SAFETY: CreateFileW returned the new handle owned by this guard.
    let output = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut before: CONSOLE_SCREEN_BUFFER_INFO = unsafe { zeroed() };
    assert_ne!(
        unsafe { GetConsoleScreenBufferInfo(handle, &mut before) },
        0
    );

    let application: Vec<_> = daemon.as_os_str().encode_wide().chain([0]).collect();
    let mut arguments = OsString::from("\"");
    arguments.push(daemon);
    arguments.push(format!("\" {flag}"));
    let mut command: Vec<_> = arguments.encode_wide().chain([0]).collect();
    let mut startup: STARTUPINFOW = unsafe { zeroed() };
    startup.cb = size_of::<STARTUPINFOW>() as u32;
    let mut process: PROCESS_INFORMATION = unsafe { zeroed() };
    // SAFETY: buffers outlive CreateProcessW. No STARTF_USESTDHANDLES or handle
    // inheritance: a real GUI image starts detached with no redirected stdout.
    // Ordinary job inheritance keeps it inside the held fixture's kill-on-close Job.
    assert_ne!(
        unsafe {
            CreateProcessW(
                application.as_ptr(),
                command.as_mut_ptr(),
                null(),
                null(),
                0,
                0,
                null(),
                null(),
                &startup,
                &mut process,
            )
        },
        0
    );
    // SAFETY: a successful CreateProcessW transfers these two new handles to us.
    let child = unsafe { OwnedHandle::from_raw_handle(process.hProcess) };
    let _thread = unsafe { OwnedHandle::from_raw_handle(process.hThread) };
    assert_eq!(
        unsafe { WaitForSingleObject(child.as_raw_handle(), 10_000) },
        WAIT_OBJECT_0
    );
    let mut code = 1;
    assert_ne!(
        unsafe { GetExitCodeProcess(child.as_raw_handle(), &mut code) },
        0
    );
    assert_eq!(code, 0);

    let mut after: CONSOLE_SCREEN_BUFFER_INFO = unsafe { zeroed() };
    assert_ne!(
        unsafe { GetConsoleScreenBufferInfo(output.as_raw_handle(), &mut after) },
        0
    );
    assert_eq!(before.dwSize.X, after.dwSize.X);
    let position = |info: &CONSOLE_SCREEN_BUFFER_INFO| {
        i32::from(info.dwCursorPosition.Y) * i32::from(info.dwSize.X)
            + i32::from(info.dwCursorPosition.X)
    };
    let count = position(&after) - position(&before);
    assert!(
        (1..=4096).contains(&count),
        "version must write to the parent console"
    );
    let mut text = vec![0u16; count as usize];
    let mut read = 0;
    // SAFETY: the requested bounded UTF-16 range fits the live buffer. Only the
    // version child writes to this fresh console; the fixture's stdio uses pipes.
    assert_ne!(
        unsafe {
            ReadConsoleOutputCharacterW(
                handle,
                text.as_mut_ptr(),
                count as u32,
                before.dwCursorPosition,
                &mut read,
            )
        },
        0
    );
    assert_eq!(read, count as u32);
    serde_json::from_str(String::from_utf16(&text).unwrap().trim()).unwrap()
}
