//! Bundled native routing adapters and their registered passive paths.
use super::FilesystemAgentScannerV1;
use hiroute_domain::AgentKindV1;
use std::path::PathBuf;

pub fn native_routing_adapter_uri(path: &std::path::Path) -> Option<String> {
    reqwest::Url::from_file_path(path).ok().map(Into::into)
}

pub const PI_ROUTING_ADAPTER: &str =
    include_str!("../../../../assets/agents/pi/hiroute-routing.mjs");
pub const DSH_ROUTING_ADAPTER: &str =
    include_str!("../../../../assets/agents/dsh/hiroute-routing.mjs");
pub const OPENCODE_ROUTING_ADAPTER: &str =
    include_str!("../../../../assets/agents/opencode/hiroute-routing.mjs");
pub fn native_routing_adapter(kind: AgentKindV1) -> Option<&'static str> {
    match kind {
        AgentKindV1::Pi => Some(PI_ROUTING_ADAPTER),
        AgentKindV1::DeepseekHarness => Some(DSH_ROUTING_ADAPTER),
        AgentKindV1::OpenCode => Some(OPENCODE_ROUTING_ADAPTER),
        _ => None,
    }
}
impl FilesystemAgentScannerV1 {
    pub fn native_routing_adapter_target(&self, kind: AgentKindV1) -> Option<PathBuf> {
        Some(match kind {
            AgentKindV1::Pi => self
                .pi_user_models_target()
                .parent()?
                // Pi's directory discovery accepts .js/.ts, not .mjs. Explicit
                // Worker loading must not hide an ordinary-client install gap.
                .join("extensions/hiroute-routing.js"),
            AgentKindV1::DeepseekHarness => self
                .dsh_user_models_target()
                .parent()?
                .join("hiroute-routing.mjs"),
            AgentKindV1::OpenCode => self
                .opencode_native_context()
                .ok()?
                .resource_root()
                .join("plugins/hiroute-routing.js"),
            _ => return None,
        })
    }
}
