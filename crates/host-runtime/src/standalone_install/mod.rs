//! The single Windows installation transaction and its current journal schema.
mod cleanup;
mod files;
mod journal;
mod package;
#[cfg(any(windows, test))]
mod uninstall;
pub use cleanup::{PackageCleanup, prune_packages};
pub(crate) use files::*;
pub use journal::{Entry, EntryKind, Journal, ManagerHost, Operation, Phase, plan};
pub use package::{Package, PackageFile};
use std::{io, path::Path};
#[cfg(windows)]
pub mod windows;

pub const JOURNAL_SCHEMA: &str = "hiroute.windows-install-journal/v1";
pub fn windows_machine(target: &str) -> io::Result<u16> {
    match target {
        "x86_64-pc-windows-msvc" => Ok(0x8664),
        "aarch64-pc-windows-msvc" => Ok(0xaa64),
        _ => Err(io::Error::other("INSTALL_ARCHITECTURE_UNSUPPORTED")),
    }
}
pub const MAX_RECORD: u64 = 64 * 1024;
pub(crate) fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "INSTALL_OWNERSHIP_CONFLICT")
}
pub(crate) fn portable(value: &str, limit: usize) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}
pub(crate) fn hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
pub(crate) fn within(path: &Path, root: &Path) -> bool {
    path.is_absolute()
        && path.starts_with(root)
        && !path.components().any(|p| {
            matches!(
                p,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
}
