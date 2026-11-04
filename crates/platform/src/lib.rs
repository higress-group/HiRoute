//! Operating-system primitives shared by headless hosts.
//!
//! Business authorization and lifecycle policy remain with their existing owners.
//! Windows FFI is isolated here; callers use owned handles and checked operations.

pub mod directory_link;
pub mod durable;
pub mod file_security;
pub mod local_record;
pub mod private_fs;

#[cfg(windows)]
pub mod windows;
