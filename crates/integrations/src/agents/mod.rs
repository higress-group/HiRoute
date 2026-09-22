//! Built-in Codex and managed Claude Code integration profiles.

mod claude_capabilities;
mod claude_native_effects;
mod codex_catalog;
mod codex_catalog_effects;
mod codex_catalog_plan;
pub use claude_capabilities::claude_plan_capability_preview;
mod codex_catalog_schema;
mod codex_catalog_source;
mod codex_config;
mod codex_layers;
mod codex_sources;
#[cfg(unix)]
mod collaboration_probe;
mod discovery;
mod emulator;
mod executable;
mod filesystem;
mod filesystem_config;
mod managed_launch;
#[cfg(unix)]
mod native_claude_ingress_probe;
mod native_effects;
#[cfg(unix)]
mod native_ingress_probe;
mod observed_capabilities;
mod registration;
mod registry;
mod source_candidates;
mod subscription_sources;

pub use claude_native_effects::*;
pub use codex_catalog::*;
pub use codex_catalog_effects::*;
pub use codex_catalog_plan::{codex_plan_capability_preview, codex_private_worker_catalog};
pub use codex_catalog_source::*;
pub use codex_config::*;
pub use codex_layers::*;
pub use discovery::*;
pub use emulator::*;
pub use filesystem::*;
pub use hiroute_domain::{
    AgentDiscoveryError, EffectiveConfigFieldV1, SupportedAgentInstallationV1,
};
pub use managed_launch::*;
#[cfg(unix)]
pub use native_claude_ingress_probe::*;
pub use native_effects::*;
#[cfg(unix)]
pub use native_ingress_probe::*;
pub use registration::*;
pub use registry::*;
pub use source_candidates::*;
pub use subscription_sources::*;

#[cfg(test)]
mod tests;
