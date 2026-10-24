//! Read-only native metadata from a retained descriptor. These observations do not
//! authorize access, establish a stable snapshot or define a persisted security format.
use nix::libc;
use std::ffi::CStr;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;

/// Read the native ACL and BSD flags into a caller-bounded buffer. The returned bytes
/// are opaque native evidence, not a portable ACL encoding. Callers own digesting and
/// before/after identity/version checks; no borrowed path or permissions are changed.
pub fn read_acl_and_flags(file: &File, bytes: &mut [u8]) -> io::Result<usize> {
    // Length + flags + attrreference is the smallest complete result.
    if bytes.len() < 16 {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut attributes = libc::attrlist {
        bitmapcount: 5,
        reserved: 0,
        commonattr: libc::ATTR_CMN_FLAGS | libc::ATTR_CMN_EXTENDED_SECURITY,
        volattr: 0,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    bytes.fill(0);
    // SAFETY: the borrowed File retains its descriptor; the input struct and writable
    // slice remain live and have the exact sizes supplied to the kernel.
    if unsafe {
        libc::fgetattrlist(
            file.as_raw_fd(),
            (&mut attributes as *mut libc::attrlist).cast(),
            bytes.as_mut_ptr().cast(),
            bytes.len(),
            0,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    let length = u32::from_ne_bytes(bytes[..4].try_into().unwrap()) as usize;
    if !(16..=bytes.len()).contains(&length) {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(length)
}

/// List native xattr names, including compression attributes, into a caller-bounded
/// buffer. Names are NUL-separated native bytes; this function emits no diagnostics.
pub fn list_extended_attributes(file: &File, names: &mut [u8]) -> io::Result<usize> {
    // SAFETY: the borrowed descriptor and the writable output slice remain live.
    let count = unsafe {
        libc::flistxattr(
            file.as_raw_fd(),
            names.as_mut_ptr().cast(),
            names.len(),
            libc::XATTR_SHOWCOMPRESSION,
        )
    };
    checked_count(count, names.len())
}

/// Read exactly one named native xattr without suppressing compression attributes.
/// Raw values stay with the caller; buffer sizing and aggregate limits are its policy.
pub fn read_extended_attribute(file: &File, name: &CStr, value: &mut [u8]) -> io::Result<usize> {
    // SAFETY: CStr supplies a valid NUL-terminated name and the retained descriptor and
    // writable output slice remain live for the bounded kernel call.
    let count = unsafe {
        libc::fgetxattr(
            file.as_raw_fd(),
            name.as_ptr(),
            value.as_mut_ptr().cast(),
            value.len(),
            0,
            libc::XATTR_SHOWCOMPRESSION,
        )
    };
    checked_count(count, value.len())
}

fn checked_count(count: libc::ssize_t, capacity: usize) -> io::Result<usize> {
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    let count = count as usize;
    if count > capacity {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(count)
}
