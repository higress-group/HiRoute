//! Selected Harness configuration and native permission policy owned by MVP-20.
//! The platform checks basic installation/start/stop, not universal confinement.
use super::acp::AcpNativeIdentityContract;
use hiroute_domain::ProtectedSecret;
use hiroute_domain::delegation::{
    DelegationErrorV1, WorkerExecutionIntentV1, WorkerHarnessV1, WorkerNetworkV1,
    WorkerPermissionPolicyV1, WorkerToolV1, WorkspaceAccessV1, WorkspaceExecutionPermitV1,
};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

mod capabilities;

mod materials;
pub(crate) use materials::native_history;
pub use materials::{RunMaterialFile, RunMaterials, SessionRootUse, TaskSessionRoot};

pub struct ProfileInput<'a> {
    pub claude_context_window: Option<u64>,
    pub harness: WorkerHarnessV1,
    pub adapter: &'a Path,
    pub harness_binary: &'a Path,
    pub node_binary: Option<&'a Path>,
    pub private_root: &'a Path,
    pub session_root: &'a TaskSessionRoot,
    pub workspace: &'a Path,
    pub alias: &'a str,
    /// One alias entry derived from the exact frozen Plan for this private Codex target.
    pub codex_catalog: Option<&'a [u8]>,
    pub native_effort: Option<&'a str>,
    pub gateway: SocketAddr,
    pub permit: &'a WorkspaceExecutionPermitV1,
    pub execution: &'a WorkerExecutionIntentV1,
    pub permission_policy: WorkerPermissionPolicyV1,
    pub admitted_at_ms: u64,
    pub token: ProtectedSecret,
}

/// No Debug/Serialize/Clone: the environment contains one run's downstream secret.
pub struct CandidateWorkerProfile {
    pub executable: PathBuf,
    pub args: Vec<PathBuf>,
    pub cwd: PathBuf,
    pub private_root: PathBuf,
    pub session_root: PathBuf,
    pub materials: RunMaterials,
    pub env: BTreeMap<String, Zeroizing<String>>,
    pub session_meta: Map<String, Value>,
    pub identity_contract: AcpNativeIdentityContract,
    access: WorkspaceAccessV1,
    network: WorkerNetworkV1,
    tools: Vec<WorkerToolV1>,
    permission_policy: WorkerPermissionPolicyV1,
    native_session_mode: String,
}

pub use super::platform::WorkerPlatformCapabilities;

impl CandidateWorkerProfile {
    pub fn build(input: ProfileInput<'_>) -> Result<Self, DelegationErrorV1> {
        input.permit.authorize(
            input.execution,
            input.admitted_at_ms,
            input.permit.generation,
        )?;
        let identity_contract = capabilities::identity_contract(&input)?;
        if ![
            input.adapter,
            input.harness_binary,
            input.private_root,
            input.workspace,
        ]
        .iter()
        .all(|p| p.is_absolute())
            || input.node_binary.is_some_and(|p| !p.is_absolute())
            || !input.gateway.is_ipv4()
            || input.gateway.ip().is_unspecified()
            || input.gateway.port() == 0
            || input.alias.is_empty()
            || input.alias.len() > 128
            || !input
                .alias
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/._-:".contains(&b))
            || input.permit.revoked
        {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        let token = std::str::from_utf8(input.token.expose())
            .map_err(|_| DelegationErrorV1::InvalidArguments)?;
        if token.len() > 4096 || !token.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        input
            .session_root
            .check_binding(input.harness, &input.execution.root_identity)?;
        let (access, network, projected_tools) = match input.permission_policy {
            WorkerPermissionPolicyV1::ApproveAll => (
                WorkspaceAccessV1::TrustedNative,
                WorkerNetworkV1::Allowed,
                vec![WorkerToolV1::Read, WorkerToolV1::Edit, WorkerToolV1::Shell],
            ),
            WorkerPermissionPolicyV1::ApproveReads => (
                WorkspaceAccessV1::ReadOnly,
                WorkerNetworkV1::GatewayOnly,
                vec![WorkerToolV1::Read],
            ),
            WorkerPermissionPolicyV1::DenyAll => (
                WorkspaceAccessV1::ReadOnly,
                WorkerNetworkV1::GatewayOnly,
                Vec::new(),
            ),
        };
        let private_root = materials::checked_run_root(input.private_root, input.session_root)?;
        let origin = format!("http://{}", input.gateway);
        let native_session_mode = match (input.harness, input.permission_policy) {
            (WorkerHarnessV1::CodexCli, WorkerPermissionPolicyV1::ApproveAll) => {
                "agent-full-access"
            }
            (WorkerHarnessV1::CodexCli, _) => "read-only",
            (WorkerHarnessV1::ClaudeCode, WorkerPermissionPolicyV1::ApproveAll) => {
                "bypassPermissions"
            }
            (WorkerHarnessV1::ClaudeCode, _) => "default",
        };
        let mut env = BTreeMap::new();
        // This is a new child environment, never merged with std::env::vars().
        // Tool discovery is not authentication. Keep an explicit command search
        // path while leaving ambient model credentials and configuration behind.
        env.insert(
            "PATH".into(),
            Zeroizing::new(worker_path(
                input.harness_binary,
                input.node_binary,
                input.adapter,
            )?),
        );
        env.insert(
            "HOME".into(),
            Zeroizing::new(path_string(&private_root.join("home"))?),
        );
        env.insert(
            "TMPDIR".into(),
            Zeroizing::new(path_string(&private_root.join("tmp"))?),
        );
        let mut session_meta = Map::new();
        match input.harness {
            WorkerHarnessV1::CodexCli => {
                let catalog_path = input
                    .codex_catalog
                    .map(|catalog| {
                        input
                            .session_root
                            .prepare_codex_catalog(input.alias, catalog)
                    })
                    .transpose()?;
                input
                    .session_root
                    .prepare_codex_provider(input.gateway, catalog_path.as_deref())?;
                let (approval_policy, sandbox_mode) = match input.permission_policy {
                    WorkerPermissionPolicyV1::ApproveAll => ("never", "danger-full-access"),
                    WorkerPermissionPolicyV1::ApproveReads | WorkerPermissionPolicyV1::DenyAll => {
                        ("on-request", "read-only")
                    }
                };
                let mut config = json!({
                    "model":input.alias, "model_provider":"hiroute", "approval_policy":approval_policy,
                    "sandbox_mode":sandbox_mode,
                    "model_providers":{"hiroute":{
                        "name":"HiRoute managed run", "base_url":format!("{origin}/v1"),
                        "wire_api":"responses", "env_key":"HIROUTE_RUN_TOKEN", "requires_openai_auth":false
                    }},
                    "features":{"multi_agent":false,"shell_tool":projected_tools.contains(&WorkerToolV1::Shell)},
                    "web_search": if network == WorkerNetworkV1::Allowed { "live" } else { "disabled" },
                    "mcp_servers":{},
                    "sandbox_workspace_write":{"network_access":network == WorkerNetworkV1::Allowed},
                });
                if let Some(effort) = input.native_effort {
                    config["model_reasoning_effort"] = json!(effort);
                }
                if let Some(path) = catalog_path {
                    config["model_catalog_json"] = json!(path_string(&path)?);
                }
                env.insert("CODEX_CONFIG".into(), Zeroizing::new(config.to_string()));
                // codex-acp owns a per-turn AgentMode that overrides the app-server defaults.
                // Seed that exact native mode and verify it again over ACP before the prompt.
                env.insert(
                    "INITIAL_AGENT_MODE".into(),
                    Zeroizing::new(native_session_mode.into()),
                );
                env.insert("MODEL_PROVIDER".into(), Zeroizing::new("hiroute".into()));
                env.insert(
                    "CODEX_PATH".into(),
                    Zeroizing::new(path_string(input.harness_binary)?),
                );
                env.insert(
                    "CODEX_HOME".into(),
                    Zeroizing::new(path_string(input.session_root.path())?),
                );
                env.insert("HIROUTE_RUN_TOKEN".into(), Zeroizing::new(token.to_owned()));
            }
            WorkerHarnessV1::ClaudeCode => {
                if let Some(window) = input.claude_context_window {
                    let values = hiroute_domain::claude_context_environment(window)
                        .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
                    for (key, value) in values {
                        env.insert(key, Zeroizing::new(value));
                    }
                }
                env.insert(
                    "CLAUDE_CODE_EXECUTABLE".into(),
                    Zeroizing::new(path_string(input.harness_binary)?),
                );
                env.insert(
                    "CLAUDE_CONFIG_DIR".into(),
                    Zeroizing::new(path_string(input.session_root.path())?),
                );
                // Claude Code may otherwise contact update, telemetry, and error-reporting
                // endpoints independently of the selected model provider. A managed Worker
                // process is allowed to reach its HiRoute Gateway (and projected tools may
                // separately have network access), but ambient product traffic must stay off.
                env.insert(
                    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".into(),
                    Zeroizing::new("1".into()),
                );
                env.insert("ANTHROPIC_BASE_URL".into(), Zeroizing::new(origin));
                env.insert(
                    "ANTHROPIC_AUTH_TOKEN".into(),
                    Zeroizing::new(token.to_owned()),
                );
                env.insert(
                    "ANTHROPIC_MODEL".into(),
                    Zeroizing::new(input.alias.to_owned()),
                );
                let mut tools = vec![];
                for tool in &projected_tools {
                    match tool {
                        WorkerToolV1::Read => tools.extend(["Read", "Glob", "Grep"]),
                        WorkerToolV1::Edit => tools.extend(["Edit", "Write"]),
                        WorkerToolV1::Shell => tools.push("Bash"),
                    }
                }
                if input.permission_policy == WorkerPermissionPolicyV1::ApproveAll {
                    tools.extend(["WebSearch", "WebFetch"]);
                }
                let mut options = json!({
                    "settingSources":[], "model":input.alias, "tools":tools,
                    "disallowedTools":["Agent","Task","AskUserQuestion"],
                    "mcpServers":{}, "plugins":[], "hooks":{},
                    "settings":{"apiKeyHelper":""},
                });
                if let Some(effort) = input.native_effort {
                    options["effort"] = json!(effort);
                }
                session_meta.insert("claudeCode".into(), json!({"options":options}));
            }
        }
        let (executable, args) = input.node_binary.map_or_else(
            || (input.adapter.to_owned(), vec![]),
            |node| (node.to_owned(), vec![input.adapter.to_owned()]),
        );
        Ok(Self {
            executable,
            args,
            cwd: input.workspace.to_owned(),
            private_root,
            session_root: input.session_root.path().to_owned(),
            materials: RunMaterials {
                directories: vec!["home".into(), "tmp".into()],
                files: vec![],
            },
            env,
            session_meta,
            identity_contract,
            access,
            network,
            tools: projected_tools,
            permission_policy: input.permission_policy,
            native_session_mode: native_session_mode.to_owned(),
        })
    }

    pub fn access(&self) -> WorkspaceAccessV1 {
        self.access
    }
    pub fn network(&self) -> WorkerNetworkV1 {
        self.network
    }
    pub fn tools(&self) -> &[WorkerToolV1] {
        &self.tools
    }

    /// Exact adapter-owned session mode that must be advertised and selected before prompting.
    pub fn native_session_mode(&self) -> &str {
        &self.native_session_mode
    }

    /// Compatibility check for callers that require a native Harness profile. Restricted
    /// policies are Harness configuration, not a different process-confinement capability.
    pub fn require_native_mode(&self) -> Result<(), DelegationErrorV1> {
        Ok(())
    }

    pub fn permission_limitations(&self) -> &'static str {
        "Caller-selected Harness approval/tool settings; no same-user filesystem or network sandbox"
    }

    /// Compatibility fallback for an unexpected request from the selected Harness. Approve-all
    /// normally uses the Harness's native autonomous mode and should not reach this callback.
    /// The journal additionally checks the current run lifecycle; no path grants persistence.
    pub fn allows_permission_once(&self, request: &Value) -> bool {
        match self.permission_policy {
            WorkerPermissionPolicyV1::ApproveAll => true,
            WorkerPermissionPolicyV1::ApproveReads => {
                request.pointer("/toolCall/kind").and_then(Value::as_str) == Some("read")
            }
            WorkerPermissionPolicyV1::DenyAll => false,
        }
    }

    pub fn require_platform(
        &self,
        facts: &WorkerPlatformCapabilities,
    ) -> Result<(), DelegationErrorV1> {
        if !facts.can_start || !facts.can_stop {
            return Err(DelegationErrorV1::CapabilityUnavailable);
        }
        self.require_native_mode()?;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn test_codex_catalog(alias: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"models": [{
        "slug": alias, "display_name": "Private Worker", "priority": 0,
        "visibility": "list", "supported_in_api": true,
        "shell_type": "shell_command", "support_verbosity": false,
        "supported_reasoning_levels": [], "supports_parallel_tool_calls": false,
        "model_messages": {"instructions_template": "Private test Worker"},
        "truncation_policy": {"mode": "bytes", "limit": 10000},
        "experimental_supported_tools": []
    }]}))
    .unwrap()
}

fn path_string(path: &Path) -> Result<String, DelegationErrorV1> {
    path.to_str()
        .filter(|s| !s.contains(['\n', '\r', '\0']))
        .map(str::to_owned)
        .ok_or(DelegationErrorV1::InvalidArguments)
}

fn worker_path(
    harness: &Path,
    node: Option<&Path>,
    adapter: &Path,
) -> Result<String, DelegationErrorV1> {
    let mut paths = Vec::new();
    for executable in [node, Some(harness), Some(adapter)].into_iter().flatten() {
        if let Some(parent) = executable.parent() {
            paths.push(parent.to_owned());
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path).filter(|path| path.is_absolute()));
    }
    #[cfg(unix)]
    paths.extend(
        [
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/usr/bin",
            "/bin",
            "/usr/sbin",
            "/sbin",
        ]
        .map(PathBuf::from),
    );
    let mut unique = Vec::new();
    for path in paths {
        if !unique.contains(&path) {
            unique.push(path);
        }
    }
    std::env::join_paths(unique)
        .map_err(|_| DelegationErrorV1::InvalidArguments)?
        .into_string()
        .map_err(|_| DelegationErrorV1::InvalidArguments)
}

#[cfg(test)]
mod tests;
