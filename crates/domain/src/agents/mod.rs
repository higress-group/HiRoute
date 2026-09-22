//! Exact-version Agent integration profiles and probe contracts.

mod capability;
mod collaboration;
mod context_window;
mod emulator;
mod managed_launch;
mod model_grant;
mod native_artifacts;
mod plan_references;
mod profile;
mod settings;
mod skill_installation;
mod surface_check;

pub use capability::*;
pub use collaboration::*;
pub use context_window::*;
pub use emulator::*;
pub use managed_launch::*;
pub use model_grant::*;
pub use native_artifacts::*;
pub use plan_references::*;
pub use profile::*;
pub use settings::*;
pub use skill_installation::*;
pub use surface_check::*;

#[cfg(test)]
mod tests;
