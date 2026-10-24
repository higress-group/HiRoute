//! Native permission facts used by conditional configuration restoration.
//! A Windows descriptor is not represented as an invented POSIX mode.
use serde::{Deserialize, Serialize};

#[cfg(target_os = "macos")]
pub mod macos;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsPermissions {
    pub descriptor: String,
    pub read_only: bool,
}

#[cfg(windows)]
pub use crate::windows::file_security::{capture, private_defaults};
