//! macOS native ACL/flags and all visible extended attributes; values never leave this reader.
use super::*;
use nix::libc;
use std::ffi::CStr;
use std::os::fd::AsRawFd;

const ACL_LIMIT: usize = 16 * 1024;
const NAMES_LIMIT: usize = 8 * 1024;
const VALUES_LIMIT: usize = 64 * 1024;
const ATTRIBUTES_LIMIT: usize = 64;

fn failure() -> Value {
    let errno = std::io::Error::last_os_error().raw_os_error();
    json!({"state":if matches!(errno, Some(libc::ERANGE | libc::E2BIG)) {
        "limit"
    } else { "unavailable" }})
}

fn acl_and_flags(file: &File) -> Value {
    let mut attributes = libc::attrlist {
        bitmapcount: 5,
        reserved: 0,
        commonattr: libc::ATTR_CMN_FLAGS | libc::ATTR_CMN_EXTENDED_SECURITY,
        volattr: 0,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    let mut bytes = vec![0u8; ACL_LIMIT];
    // SAFETY: the selected regular file stays open; both buffers have their declared sizes.
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
        return failure();
    }
    let length = u32::from_ne_bytes(bytes[..4].try_into().unwrap()) as usize;
    // Length + flags + attrreference; reject any incomplete kernel result.
    if !(16..=ACL_LIMIT).contains(&length) {
        return json!({"state":"invalid"});
    }
    json!({"state":"complete", "digest":CanonicalDigest::of_bytes(&bytes[..length])})
}

fn xattrs(file: &File) -> Value {
    let mut names = vec![0u8; NAMES_LIMIT];
    // SHOWCOMPRESSION prevents hidden compression attributes from silently escaping evidence.
    let flags = libc::XATTR_SHOWCOMPRESSION;
    // SAFETY: the output buffer and retained descriptor remain live for this bounded call.
    let count = unsafe {
        libc::flistxattr(
            file.as_raw_fd(),
            names.as_mut_ptr().cast(),
            names.len(),
            flags,
        )
    };
    if count < 0 {
        return failure();
    }
    names.truncate(count as usize);
    let mut keys = Vec::new();
    let mut rest = names.as_slice();
    while !rest.is_empty() {
        let Ok(key) = CStr::from_bytes_until_nul(rest) else {
            return json!({"state":"invalid"});
        };
        if key.to_bytes().is_empty() || keys.len() == ATTRIBUTES_LIMIT {
            return json!({"state":"limit"});
        }
        rest = &rest[key.to_bytes_with_nul().len()..];
        keys.push(key);
    }
    keys.sort_by_key(|key| key.to_bytes());
    let mut evidence = Vec::new();
    let mut used = 0usize;
    let mut value = vec![0u8; VALUES_LIMIT];
    for key in keys {
        // SAFETY: names are validated NUL-terminated kernel results; value is bounded.
        let count = unsafe {
            libc::fgetxattr(
                file.as_raw_fd(),
                key.as_ptr(),
                value.as_mut_ptr().cast(),
                value.len(),
                0,
                flags,
            )
        };
        if count < 0 {
            return failure();
        }
        used += count as usize;
        if used > VALUES_LIMIT {
            return json!({"state":"limit"});
        }
        evidence.push((
            key.to_bytes(),
            CanonicalDigest::of_bytes(&value[..count as usize]),
        ));
    }
    json!({"state":"complete", "digest":CanonicalDigest::of(&evidence).unwrap()})
}

pub(super) fn extended(file: &File) -> Value {
    json!({"acl_flags":acl_and_flags(file), "xattrs":xattrs(file)})
}
