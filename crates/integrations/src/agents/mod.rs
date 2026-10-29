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
#[cfg(any(unix, windows))]
mod collaboration_probe;
mod discovery;
mod emulator;
mod executable;
mod probe_files;
pub use executable::codex_subscription_client_version;
mod filesystem;
mod filesystem_config;
mod hermes_config;
mod hermes_yaml;
pub use hermes_config::*;
mod hermes_native_effects;
pub use hermes_native_effects::*;
mod hermes_plugin_effects;
mod routing_adapters;
pub use hermes_plugin_effects::*;
pub use routing_adapters::*;
mod hermes_target;
pub use hermes_target::*;
mod hermes_support;
pub use hermes_support::*;
mod hermes_sources;
#[cfg(test)]
mod hermes_sources_tests;
pub use hermes_sources::*;
mod hermes_discovery;
pub use hermes_discovery::*;
pub use hermes_yaml::HermesNativeError;
mod managed_launch;
#[cfg(any(unix, windows))]
mod native_claude_ingress_probe;
mod native_effects;
#[cfg(any(unix, windows))]
mod native_ingress_probe;
#[cfg(any(unix, windows))]
mod native_probe_process;
#[cfg(any(unix, windows))]
pub use native_probe_process::NativeProbeProcess;
#[cfg(windows)]
pub(crate) mod windows_runtime;
#[cfg(windows)]
pub use windows_runtime::{
    spawn_windows_native_probe, windows_agent_command, windows_claude_version,
    windows_native_system_environment, windows_npm_entry, windows_npm_entry_with_inspection,
};
mod dsh_config;
mod dsh_sources;
#[cfg(test)]
mod dsh_tests;
mod native_points;
mod observed_capabilities;
mod opencode;
mod opencode_installation;
mod opencode_migrations;
mod opencode_project;
mod opencode_resources;
mod opencode_sources;
#[cfg(test)]
mod opencode_tests;
pub use opencode::{
    OpenCodeConfig, OpenCodeNativeContext, opencode_provider, validate_opencode_composition,
};
pub use opencode_installation::{opencode_native_executable, opencode_native_format};
mod pi_runtime;
mod pi_sources;
pub use native_points::*;
pub use pi_sources::{NativeApiEcosystem, NativeApiSource, PiApiSource};
mod qoder;
pub use pi_runtime::*;
mod additional_native;
mod additional_native_effects;
mod qoder_provider;
pub use additional_native_effects::*;
#[cfg(any(unix, windows))]
mod native_responses_probe;
#[cfg(any(unix, windows))]
mod opencode_probe;
#[cfg(any(unix, windows))]
mod qoder_probe;
#[cfg(any(unix, windows))]
pub use opencode_probe::OpenCodeCollaborationProbe;
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
#[cfg(any(unix, windows))]
pub use native_claude_ingress_probe::*;
pub use native_effects::*;
#[cfg(any(unix, windows))]
pub use native_ingress_probe::*;
pub use qoder::*;
#[cfg(any(unix, windows))]
pub use qoder_probe::*;
pub use registration::*;
pub use registry::*;
pub use source_candidates::*;
pub use subscription_sources::*;

#[cfg(test)]
pub(super) fn test_absolute_path(path: &str) -> std::path::PathBuf {
    assert!(
        path.starts_with('/'),
        "fixture path must have a declared root"
    );
    #[cfg(windows)]
    return std::path::PathBuf::from(format!("C:{}", path.replace('/', "\\")));
    #[cfg(not(windows))]
    std::path::PathBuf::from(path)
}

#[cfg(test)]
mod tests;

#[cfg(all(test, windows))]
mod windows_collaboration_tests;

mod codex_profile;
pub use codex_profile::*;
