//! macOS native ACL/flags and all visible extended attributes; values never leave this reader.
use super::*;
use hiroute_platform::file_security::macos::{
    list_extended_attributes, read_acl_and_flags, read_extended_attribute,
};
use nix::libc;
use std::ffi::CStr;

const ACL_LIMIT: usize = 16 * 1024;
const NAMES_LIMIT: usize = 8 * 1024;
const VALUES_LIMIT: usize = 64 * 1024;
const ATTRIBUTES_LIMIT: usize = 64;

fn failure(error: std::io::Error) -> Value {
    if error.kind() == std::io::ErrorKind::InvalidData {
        return json!({"state":"invalid"});
    }
    let errno = error.raw_os_error();
    json!({"state":if matches!(errno, Some(libc::ERANGE | libc::E2BIG)) {
        "limit"
    } else { "unavailable" }})
}

fn acl_and_flags(file: &File) -> Value {
    let mut bytes = vec![0u8; ACL_LIMIT];
    let length = match read_acl_and_flags(file, &mut bytes) {
        Ok(length) => length,
        Err(error) => return failure(error),
    };
    json!({"state":"complete", "digest":CanonicalDigest::of_bytes(&bytes[..length])})
}

fn xattrs(file: &File) -> Value {
    let mut names = vec![0u8; NAMES_LIMIT];
    let count = match list_extended_attributes(file, &mut names) {
        Ok(count) => count,
        Err(error) => return failure(error),
    };
    names.truncate(count);
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
        let count = match read_extended_attribute(file, key, &mut value) {
            Ok(count) => count,
            Err(error) => return failure(error),
        };
        used += count;
        if used > VALUES_LIMIT {
            return json!({"state":"limit"});
        }
        evidence.push((key.to_bytes(), CanonicalDigest::of_bytes(&value[..count])));
    }
    json!({"state":"complete", "digest":CanonicalDigest::of(&evidence).unwrap()})
}

pub(super) fn extended(file: &File) -> Value {
    json!({"acl_flags":acl_and_flags(file), "xattrs":xattrs(file)})
}
