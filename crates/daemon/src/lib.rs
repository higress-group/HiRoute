#![forbid(unsafe_code)]

//! Production daemon composition and authenticated, versioned Local Control transport.

pub mod control;
pub mod delegation;
mod model_feed;
mod release_catalog;
#[cfg(any(unix, windows))]
pub mod role_all;

pub mod gateway_ports;
pub use control::{ControlEndpoint, LocalControlDaemon, serve_production_control};
#[cfg(any(unix, windows))]
pub use role_all::{RoleAllConfig, RoleAllCpaConfig, RoleAllError, RoleAllHandle, start_role_all};
mod publication_failpoint;

#[cfg(test)]
mod test_support;

pub mod console;
