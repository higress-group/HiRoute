#![forbid(unsafe_code)]

//! Concrete host-owned contracts shared by the current Desktop and standalone CLI hosts.
//!
//! This crate intentionally contains no service-manager abstraction and no business state. It
//! owns only the one listener record and one standalone installation/layout contract that are
//! consumed by all current native entrypoints.

#[cfg(test)]
mod development_versions_tests;

mod desktop_package;
mod gateway_listener;
mod releases;
mod releases_v3;
mod service_proxy;
mod standalone;
mod upgrade;

pub use desktop_package::*;
pub use gateway_listener::*;
pub use releases::*;
pub use releases_v3::*;
pub use service_proxy::*;
pub use standalone::*;
pub use upgrade::*;

mod storage_progress;
pub use storage_progress::{StorageUpgradePhase, StorageUpgradeProgress};

mod subscription_proxy;
pub use subscription_proxy::*;

mod console_listener;
pub use console_listener::*;
pub mod standalone_install;

mod storage_recovery;
pub use storage_recovery::*;

mod recovery_package;
pub use recovery_package::*;

mod upgrade_preflight;
pub use upgrade_preflight::*;
