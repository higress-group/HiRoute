//! Current-account background identity; this file never contains credentials.
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    FOLDERID_LocalAppData, FOLDERID_Profile, FOLDERID_ProgramData, FOLDERID_ProgramFiles,
    SHGetKnownFolderPath,
};
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;

pub const SCHEMA: &str = "hiroute.windows-background/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Manager {
    Task,
    Scm,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub schema: String,
    pub manager: Manager,
    pub name: String,
    pub owner_sid: String,
    pub daemon: PathBuf,
    pub daemon_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_host: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_host_sha256: Option<String>,
}

impl Definition {
    pub fn new(manager: Manager, daemon: PathBuf) -> io::Result<Self> {
        let owner_sid = current_sid_string()?;
        let mut image = super::security::open_identity(&daemon, false)?;
        Self::from_owner_image(manager, daemon, &owner_sid, &mut image)
    }

    /// The caller authenticates this SID and opens the image while impersonating
    /// that owner. This constructor hashes the retained object, never its path.
    pub fn from_owner_image(
        manager: Manager,
        daemon: PathBuf,
        owner_sid: &str,
        image: &mut std::fs::File,
    ) -> io::Result<Self> {
        let name = name_for_sid(owner_sid);
        image.seek(SeekFrom::Start(0))?;
        let daemon_sha256 = digest(&mut *image)?;
        image.seek(SeekFrom::Start(0))?;
        let (service_host, service_host_sha256) = if manager == Manager::Scm {
            (
                Some(
                    program_files()?
                        .join("HiRoute/service-host")
                        .join(&daemon_sha256)
                        .join("hirouted.exe"),
                ),
                Some(daemon_sha256.clone()),
            )
        } else {
            (None, None)
        };
        let value = Self {
            schema: SCHEMA.into(),
            manager,
            name,
            owner_sid: owner_sid.into(),
            daemon,
            daemon_sha256,
            service_host,
            service_host_sha256,
        };
        value.validate_for_owner_sid(owner_sid)?;
        Ok(value)
    }

    pub fn validate(&self) -> io::Result<()> {
        self.validate_for_owner_sid(&current_sid_string()?)
    }

    pub fn validate_for_owner_sid(&self, owner_sid: &str) -> io::Result<()> {
        if self.schema != SCHEMA
            || self.owner_sid != owner_sid
            || self.name != name_for_sid(&self.owner_sid)
            || !self.daemon.is_absolute()
            || self.daemon.file_name().and_then(|v| v.to_str()) != Some("hirouted.exe")
            || self.daemon_sha256.len() != 64
            || !self.daemon_sha256.bytes().all(|v| v.is_ascii_hexdigit())
            || (self.manager == Manager::Task
                && (self.service_host.is_some() || self.service_host_sha256.is_some()))
            || (self.manager == Manager::Scm
                && (self.service_host.as_ref()
                    != Some(
                        &program_files()?
                            .join("HiRoute/service-host")
                            .join(&self.daemon_sha256)
                            .join("hirouted.exe"),
                    )
                    || self.service_host_sha256.as_deref() != Some(self.daemon_sha256.as_str())))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "invalid background owner or image",
            ));
        }
        Ok(())
    }

    pub fn verify_image(&self) -> io::Result<()> {
        self.validate()?;
        let file = super::security::open_identity(&self.daemon, false)?;
        if digest(file)? != self.daemon_sha256 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "background image digest differs",
            ));
        }
        Ok(())
    }

    pub fn arguments(&self) -> String {
        match self.manager {
            Manager::Task => "--role all --standalone".into(),
            Manager::Scm => format!("--role all --standalone --windows-service {}", self.name),
        }
    }
}

fn name_for_sid(sid: &str) -> String {
    format!("HiRoute-{:x}", Sha256::digest(sid.as_bytes()))
}

pub fn current_sid_string() -> io::Result<String> {
    sid_string(&super::security::current_user_sid()?)
}

pub fn sid_string(sid: &[u8]) -> io::Result<String> {
    if sid.len() < 8 || sid[0] != 1 || sid[1] > 15 || sid.len() != 8 + usize::from(sid[1]) * 4 {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let mut sid = sid.to_vec();
    let mut value = std::ptr::null_mut();
    // SAFETY: the validated SID buffer and output remain live until conversion.
    if unsafe { ConvertSidToStringSidW(sid.as_mut_ptr().cast(), &mut value) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut length = 0;
    // SAFETY: Windows returned a NUL-terminated SID string allocated by LocalAlloc.
    while length < 200 && unsafe { *value.add(length) } != 0 {
        length += 1;
    }
    // SAFETY: conversion returned `length` UTF-16 units; free only after copying.
    let result = String::from_utf16(unsafe { std::slice::from_raw_parts(value, length) })
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidData));
    unsafe {
        LocalFree(value.cast());
    }
    result
}

/// Resolve the process account profile, including SCM's service-logon account.
/// HOME/USERPROFILE supplied by a parent cannot redirect standalone ownership.
pub fn profile() -> io::Result<PathBuf> {
    known_folder(&FOLDERID_Profile)
}
pub fn local_app_data() -> io::Result<PathBuf> {
    known_folder(&FOLDERID_LocalAppData)
}
pub fn program_files() -> io::Result<PathBuf> {
    known_folder(&FOLDERID_ProgramFiles)
}

pub fn program_data() -> io::Result<PathBuf> {
    known_folder(&FOLDERID_ProgramData)
}

fn known_folder(id: &windows::core::GUID) -> io::Result<PathBuf> {
    // SAFETY: a null token selects the current process account, not LocalSystem.
    let value = unsafe { SHGetKnownFolderPath(id, Default::default(), None) }.map_err(com_error)?;
    // SAFETY: Shell returns a live NUL-terminated UTF-16 allocation.
    let text = unsafe { value.to_string() }.map_err(io::Error::other);
    // SAFETY: SHGetKnownFolderPath transfers this CoTaskMem allocation to us.
    unsafe {
        CoTaskMemFree(Some(value.0.cast()));
    }
    let path = PathBuf::from(text?);
    if !path.is_absolute() {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    Ok(path)
}

pub fn read_definition(path: &Path) -> io::Result<Definition> {
    let mut file = super::security::open_owner_only(path, false)?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(16 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    let definition: Definition = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    definition.validate()?;
    Ok(definition)
}

/// Publish one bounded owner-only record. Full contents are flushed before a
/// Held-object, same-volume replacement and a flushed source; the installation
/// journal supplies process-crash replay. Consumer disk/controller behavior
/// through sudden power loss remains outside this publication guarantee.
pub fn write_private_record(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if bytes.is_empty() || bytes.len() > 64 * 1024 {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let held_parent = super::security::ensure_private_directory(parent)?;
    if path.try_exists()? {
        super::security::open_owner_only(path, false)?;
    }
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let mut file = loop {
        let leaf = std::ffi::OsString::from(format!(
            ".hiroute-record-{}-{epoch}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        match super::security::create_owner_only_file_at(&held_parent, &leaf) {
            Ok(file) => break file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let leaf = path
        .file_name()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut published = false;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        super::durable::rename_file_at(&file, &held_parent, leaf, true)?;
        published = true;
        file.sync_all()?;
        super::security::verify_owner_only_handle(&file, false)
    })();
    if result.is_err() && !published {
        // Clean only the object created here, even if an ancestor was renamed.
        let _ = super::security::delete_by_handle(&file);
    }
    result
}

pub fn write_definition(path: &Path, definition: &Definition) -> io::Result<()> {
    definition.validate()?;
    write_private_record(
        path,
        &serde_json::to_vec(definition).map_err(io::Error::other)?,
    )
}

pub(crate) fn digest(mut file: impl Read) -> io::Result<String> {
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[derive(Debug)]
struct BackgroundHresult(u32);

impl std::fmt::Display for BackgroundHresult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Windows background HRESULT {:08x}", self.0)
    }
}

impl std::error::Error for BackgroundHresult {}

pub(crate) fn com_error(error: windows::core::Error) -> io::Error {
    io::Error::other(BackgroundHresult(error.code().0 as u32))
}

pub fn hresult(error: &io::Error) -> Option<u32> {
    error
        .get_ref()?
        .downcast_ref::<BackgroundHresult>()
        .map(|code| code.0)
}

/// Quote one native command-line argument using CommandLineToArgvW/CRT rules.
/// No shell parses this result. Task actions use a separate unquoted Path field.
pub fn quote_argument(value: &str) -> io::Result<String> {
    if value.contains('\0') {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let mut output = String::from("\"");
    let mut slashes = 0;
    for ch in value.chars() {
        if ch == '\\' {
            slashes += 1;
            continue;
        }
        output.extend(std::iter::repeat_n(
            '\\',
            if ch == '"' { slashes * 2 + 1 } else { slashes },
        ));
        slashes = 0;
        output.push(ch);
    }
    output.extend(std::iter::repeat_n('\\', slashes * 2));
    output.push('"');
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_argv_quotes_spaces_quotes_and_trailing_slashes_without_shell() {
        assert_eq!(
            quote_argument(r"C:\a b\hirouted.exe").unwrap(),
            r#""C:\a b\hirouted.exe""#
        );
        assert_eq!(quote_argument("a\\\"b\\").unwrap(), "\"a\\\\\\\"b\\\\\"");
        assert!(quote_argument("a\0b").is_err());
    }
    #[test]
    fn profile_is_absolute_and_definition_rejects_unknown_or_foreign_owner() {
        assert!(profile().unwrap().is_absolute());
        assert!(local_app_data().unwrap().is_absolute());
        assert!(
            serde_json::from_str::<Definition>(r#"{"schema":"x","password":"secret"}"#).is_err()
        );
        assert!(
            name_for_sid(&current_sid_string().unwrap())
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        );
    }
}
