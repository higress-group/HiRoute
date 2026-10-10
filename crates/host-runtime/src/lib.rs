#![forbid(unsafe_code)]

//! Concrete host-owned contracts shared by the current Desktop and standalone CLI hosts.
//!
//! This crate intentionally contains no service-manager abstraction and no business state. It
//! owns only the one listener record and one standalone installation/layout contract that are
//! consumed by all current native entrypoints.

mod desktop_package;
mod gateway_listener;
mod releases;
mod service_proxy;
mod standalone;
mod upgrade;

pub use desktop_package::*;
pub use gateway_listener::*;
pub use releases::*;
pub use service_proxy::*;
pub use standalone::*;
pub use upgrade::*;

mod storage_progress;
pub use storage_progress::StorageUpgradePhase;

mod subscription_proxy;
pub use subscription_proxy::*;
