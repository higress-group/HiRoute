//! Selected Harness configuration and native permission policy owned by MVP-20.
//! The platform checks basic installation/start/stop, not universal confinement.
use super::acp::AcpNativeIdentityContract;
use hiroute_domain::ProtectedSecret;
use hiroute_domain::delegation::{
    DelegationErrorV1, WorkerExecutionIntentV1, WorkerHarnessV1, WorkerLaunchFormV1,
    WorkerNetworkV1, WorkerPermissionPolicyV1, WorkerToolV1, WorkspaceAccessV1,
    WorkspaceExecutionPermitV1,
};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

mod capabilities;
mod claude;
mod claude_runtime;
mod codex;
mod native_context;
mod pi;
mod qoder;
pub use native_context::NativeWorkerContext;

mod materials;
pub(crate) use materials::native_history;
pub use materials::{
    BorrowedNativeContext, RunMaterialFile, RunMaterials, SessionRootUse, TaskSessionRoot,
};

pub struct ProfileInput<'a> {
    pub context_window_tokens: Option<u64>,
    /// Qoder's frozen catalogue output upper bound; other Harness renderers are unchanged.
    pub max_output_tokens: Option<u64>,
    pub harness: WorkerHarnessV1,
    pub adapter: Option<&'a Path>,
    pub harness_binary: &'a Path,
    pub node_binary: Option<&'a Path>,
    pub private_root: &'a Path,
    pub session_root: &'a TaskSessionRoot,
    pub native_context: &'a NativeWorkerContext,
    pub workspace: &'a Path,
    pub alias: &'a str,
    /// One alias entry derived from the exact frozen Plan, stored in the owned task root.
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
    native_selected_model_id: String,
    codex_initialization_root: Option<PathBuf>,
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
        let selection = hiroute_domain::WorkerDependencySelectionRecordV1 {
            harness: input.harness,
            adapter_path: input.adapter.map(path_string).transpose()?,
            cli_path: path_string(input.harness_binary)?,
            node_path: input.node_binary.map(path_string).transpose()?,
        };
        let launch = selection
            .validated_launch()
            .map_err(|_| DelegationErrorV1::InvalidArguments)?;
        if ![input.harness_binary, input.private_root, input.workspace]
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
        secret_str(&input.token)?;
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
        if input.native_context.is_borrowed() {
            if input.native_context.home().starts_with(&private_root)
                || input
                    .native_context
                    .config_root()
                    .starts_with(&private_root)
                || input
                    .native_context
                    .config_root()
                    .starts_with(input.session_root.path())
            {
                return Err(DelegationErrorV1::InvalidArguments);
            }
        } else if input.native_context.home() != private_root.join("home")
            || input.native_context.config_root() != input.session_root.path()
        {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        let native_session_mode = match (input.harness, input.permission_policy) {
            (WorkerHarnessV1::CodexCli, WorkerPermissionPolicyV1::ApproveAll) => {
                "agent-full-access"
            }
            (WorkerHarnessV1::CodexCli, _) => "read-only",
            (WorkerHarnessV1::ClaudeCode, WorkerPermissionPolicyV1::ApproveAll) => {
                "bypassPermissions"
            }
            (WorkerHarnessV1::ClaudeCode, _) => "default",
            (WorkerHarnessV1::QoderCli, WorkerPermissionPolicyV1::ApproveAll) => "yolo",
            (WorkerHarnessV1::Pi, WorkerPermissionPolicyV1::ApproveAll) => "approve-all",
            (WorkerHarnessV1::Pi, _) => return Err(DelegationErrorV1::CapabilityUnavailable),
            (WorkerHarnessV1::QoderCli, _) => return Err(DelegationErrorV1::CapabilityUnavailable),
        };
        let mut env = BTreeMap::new();
        // This is a new child environment, never merged with std::env::vars().
        // Tool discovery is not authentication. Keep an explicit command search
        // path while leaving ambient model credentials behind. Native context selectors are explicit.
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
            Zeroizing::new(path_string(input.native_context.home())?),
        );
        env.insert(
            "TMPDIR".into(),
            Zeroizing::new(path_string(&private_root.join("tmp"))?),
        );
        if input.harness == WorkerHarnessV1::ClaudeCode && input.native_context.is_borrowed() {
            // Only the explicitly selected Node can establish the adapter environment guard.
            // An executable adapter discovered without a runtime remains discoverable, but
            // cannot silently start with unguarded borrowed settings.
            input
                .node_binary
                .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
            claude_runtime::require_host_managed_provider(
                input.harness_binary,
                env["PATH"].as_str(),
            )?;
        }
        let rendered = match input.harness {
            WorkerHarnessV1::CodexCli => {
                codex::render(&input, &projected_tools, network, native_session_mode)?
            }
            WorkerHarnessV1::ClaudeCode => claude::render(&input, &projected_tools)?,
            WorkerHarnessV1::QoderCli => qoder::render(&input, &private_root)?,
            WorkerHarnessV1::Pi => pi::render(&input)?,
        };
        env.extend(rendered.env);
        let (executable, args) = match launch {
            WorkerLaunchFormV1::NativeSdk { cli, node } => (
                PathBuf::from(node),
                vec![
                    private_root.join("pi-worker-bridge.mjs"),
                    PathBuf::from(cli),
                ],
            ),
            WorkerLaunchFormV1::NativeAcp { cli } => (PathBuf::from(cli), rendered.native_args),
            WorkerLaunchFormV1::AdapterAcp { adapter, node, .. } => {
                if let Some(bootstrap) = rendered.adapter_bootstrap {
                    (
                        PathBuf::from(node.ok_or(DelegationErrorV1::CapabilityUnavailable)?),
                        vec![private_root.join(bootstrap), PathBuf::from(adapter)],
                    )
                } else {
                    node.map_or_else(
                        || (PathBuf::from(adapter), vec![]),
                        |node| (PathBuf::from(node), vec![PathBuf::from(adapter)]),
                    )
                }
            }
        };
        Ok(Self {
            executable,
            args,
            cwd: input.workspace.to_owned(),
            private_root,
            session_root: input.session_root.path().to_owned(),
            materials: RunMaterials {
                directories: if input.native_context.is_borrowed() {
                    vec!["tmp".into()]
                } else {
                    vec!["home".into(), "tmp".into()]
                },
                files: rendered.files,
            },
            env,
            session_meta: rendered.session_meta,
            identity_contract,
            access,
            network,
            tools: projected_tools,
            permission_policy: input.permission_policy,
            native_session_mode: native_session_mode.to_owned(),
            native_selected_model_id: rendered.native_model_id,
            codex_initialization_root: (input.harness == WorkerHarnessV1::CodexCli
                && input.native_context.is_borrowed())
            .then(|| input.native_context.config_root().to_owned()),
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

    /// The exact native selector ID. Plan/Gateway authorization still uses the raw alias.
    pub fn native_selected_model_id(&self) -> &str {
        &self.native_selected_model_id
    }

    /// The installation owner has already bound the canonical effective native root.
    /// Never derive this coordination key from cwd, a model alias, or mutable child env.
    pub(crate) fn codex_initialization_root(&self) -> Option<&Path> {
        self.codex_initialization_root.as_deref()
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

#[derive(Default)]
struct RenderedHarnessProfile {
    env: BTreeMap<String, Zeroizing<String>>,
    session_meta: Map<String, Value>,
    files: Vec<RunMaterialFile>,
    adapter_bootstrap: Option<PathBuf>,
    native_args: Vec<PathBuf>,
    native_model_id: String,
}

fn secret_str(secret: &ProtectedSecret) -> Result<&str, DelegationErrorV1> {
    let value =
        std::str::from_utf8(secret.expose()).map_err(|_| DelegationErrorV1::InvalidArguments)?;
    if value.len() > 4096 || !value.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    Ok(value)
}

fn secret_string(secret: &ProtectedSecret) -> Result<String, DelegationErrorV1> {
    secret_str(secret).map(str::to_owned)
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
    adapter: Option<&Path>,
) -> Result<String, DelegationErrorV1> {
    let mut paths = Vec::new();
    for executable in [node, Some(harness), adapter].into_iter().flatten() {
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
