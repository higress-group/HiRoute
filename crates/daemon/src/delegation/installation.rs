use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hiroute_domain::delegation::{
    DelegationErrorV1, DelegationNativeRootReadyV1, DelegationNativeRootStateV1,
    DelegationNativeUseStateV1, DelegationRuntimePort, WorkerHarnessV1,
};
use hiroute_domain::{CanonicalDigest, WorkspaceId};
use hiroute_integrations::agents::codex_private_worker_catalog;

use super::executor::{WorkerProfileInput, WorkerProfileSource};
use super::profile::{CandidateWorkerProfile, ProfileInput, SessionRootUse, TaskSessionRoot};

#[path = "installation/availability.rs"]
mod availability;
#[path = "installation/discovery.rs"]
mod discovery;
#[cfg(test)]
#[path = "installation/installation_probe.rs"]
mod installation_probe;
#[cfg(test)]
#[path = "installation/probe_server.rs"]
mod probe_server;
pub(crate) use availability::WorkerExecutorAvailabilityRegistry;
pub(crate) use discovery::{
    discover as discover_worker_dependencies, validate_persisted_installation, validate_selection,
};

#[derive(Clone, Debug)]
pub struct WorkerInstallationConfig {
    pub harness: WorkerHarnessV1,
    /// The ACP agent server.  It may be a Node script only when `node_binary` is present.
    pub adapter: PathBuf,
    /// The native Harness executable selected by the user/installation owner.
    pub harness_binary: PathBuf,
    /// The explicitly selected Node runtime for a script adapter, if one is required.
    pub node_binary: Option<PathBuf>,
}

impl WorkerInstallationConfig {
    pub fn codex_acp(
        adapter: impl Into<PathBuf>,
        harness_binary: impl Into<PathBuf>,
        node_binary: impl Into<PathBuf>,
    ) -> Self {
        Self {
            harness: WorkerHarnessV1::CodexCli,
            adapter: adapter.into(),
            harness_binary: harness_binary.into(),
            node_binary: Some(node_binary.into()),
        }
    }

    pub fn claude_acp(
        adapter: impl Into<PathBuf>,
        harness_binary: impl Into<PathBuf>,
        node_binary: impl Into<PathBuf>,
    ) -> Self {
        Self {
            harness: WorkerHarnessV1::ClaudeCode,
            adapter: adapter.into(),
            harness_binary: harness_binary.into(),
            node_binary: Some(node_binary.into()),
        }
    }

    fn public_selection(&self) -> hiroute_application_api::WorkerDependencySelectionV1 {
        hiroute_application_api::WorkerDependencySelectionV1 {
            harness: self.harness,
            adapter_path: self.adapter.to_string_lossy().into_owned(),
            cli_path: self.harness_binary.to_string_lossy().into_owned(),
            node_path: self
                .node_binary
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WorkerInstallationSelection {
    pub(crate) config: WorkerInstallationConfig,
    pub(crate) revision: u64,
}

pub(crate) trait WorkerInstallationSelectionSource: Send + Sync {
    fn selection(
        &self,
        harness: WorkerHarnessV1,
    ) -> Result<Option<WorkerInstallationSelection>, DelegationErrorV1>;
}

pub(super) fn check_installation(
    config: &WorkerInstallationConfig,
) -> Result<(), DelegationErrorV1> {
    check_entry(&config.adapter, config.node_binary.is_none())?;
    check_entry(&config.harness_binary, true)?;
    if let Some(node) = &config.node_binary {
        check_entry(node, true)?;
    }
    Ok(())
}

fn check_entry(path: &Path, require_executable: bool) -> Result<(), DelegationErrorV1> {
    if !path.is_absolute() {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    let metadata = fs::metadata(path).map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    if !metadata.is_file() || (require_executable && !executable(&metadata)) {
        return Err(DelegationErrorV1::CapabilityUnavailable);
    }
    Ok(())
}

fn executable(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

#[derive(Clone)]
struct WorkerProfileRoots {
    sessions: PathBuf,
    runs: PathBuf,
}

impl WorkerProfileRoots {
    fn prepare(storage_root: &Path) -> Result<Self, DelegationErrorV1> {
        let storage_root =
            fs::canonicalize(storage_root).map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        let delegation = create_private_child(&storage_root, "delegation-workers")?;
        Ok(Self {
            sessions: create_private_child(&delegation, "sessions")?,
            runs: create_private_child(&delegation, "runs")?,
        })
    }
}

fn create_private_child(parent: &Path, name: &str) -> Result<PathBuf, DelegationErrorV1> {
    if !parent.is_absolute() || name.is_empty() || name.contains(['/', '\\', '\0']) {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    let parent = fs::canonicalize(parent).map_err(|_| DelegationErrorV1::StorageUnavailable)?;
    let path = parent.join(name);
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(DelegationErrorV1::StorageUnavailable),
    }
    let metadata =
        fs::symlink_metadata(&path).map_err(|_| DelegationErrorV1::StorageUnavailable)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() || !private(&metadata) {
        return Err(DelegationErrorV1::PermissionDenied);
    }
    let actual = fs::canonicalize(&path).map_err(|_| DelegationErrorV1::StorageUnavailable)?;
    if actual.parent() != Some(parent.as_path())
        || !private(&fs::metadata(&actual).map_err(|_| DelegationErrorV1::StorageUnavailable)?)
    {
        return Err(DelegationErrorV1::PermissionDenied);
    }
    Ok(actual)
}

fn private(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.uid() == nix::unistd::geteuid().as_raw() && metadata.mode() & 0o077 == 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_dir()
    }
}

pub(crate) struct ManagedWorkerProfileSource {
    storage_root: PathBuf,
    selections: Arc<dyn WorkerInstallationSelectionSource>,
    runtime: Arc<dyn DelegationRuntimePort + Send + Sync>,
}

pub(crate) struct ManagedWorkerProfiles {
    pub(crate) source: Arc<dyn WorkerProfileSource>,
    pub(crate) availability: Arc<WorkerExecutorAvailabilityRegistry>,
}

pub(crate) fn managed_profile_source(
    storage_root: &Path,
    selections: Arc<dyn WorkerInstallationSelectionSource>,
    runtime: Arc<dyn DelegationRuntimePort + Send + Sync>,
) -> ManagedWorkerProfiles {
    let availability = Arc::new(WorkerExecutorAvailabilityRegistry::new(Arc::clone(
        &selections,
    )));
    ManagedWorkerProfiles {
        source: Arc::new(ManagedWorkerProfileSource {
            storage_root: storage_root.to_owned(),
            selections,
            runtime,
        }),
        availability,
    }
}

impl WorkerProfileSource for ManagedWorkerProfileSource {
    fn build(
        &self,
        input: WorkerProfileInput,
    ) -> Result<CandidateWorkerProfile, DelegationErrorV1> {
        let selection = self
            .selections
            .selection(input.task.plan.harness)?
            .ok_or(DelegationErrorV1::DependenciesMissing)?;
        let installation = discovery::validate_persisted_installation(&selection.config)?;
        let roots = WorkerProfileRoots::prepare(&self.storage_root)?;
        let task = &input.task;
        let run = &input.run;
        if task.workspace_id != run.workspace_id
            || task.task_id != run.task_id
            || task.latest_run_id != run.run_id
            || input.permit.permit_id != run.permit_id
            || input.permit.generation != run.permit_generation
        {
            return Err(DelegationErrorV1::Conflict);
        }
        let native = self
            .runtime
            .native_root(&task.workspace_id, &task.task_id)?
            .ok_or(DelegationErrorV1::ResumeUnavailable)?;
        let usage = native
            .uses
            .iter()
            .find(|usage| usage.run_id == run.run_id)
            .ok_or(DelegationErrorV1::ResumeUnavailable)?;
        if native.root.workspace_id != task.workspace_id
            || native.root.task_id != task.task_id
            || native.root.harness != installation.harness
            || native.root.workspace_root_identity != task.workspace.root_identity
            || usage.root_generation != native.root.root_generation
            || usage.lease_id != run.lease_id
            || usage.daemon_epoch != run.daemon_epoch
            || usage.state != DelegationNativeUseStateV1::Accepted
        {
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
        let required_history = task
            .native_history_paths
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        let session_usage = if run.continued_from.is_some() {
            SessionRootUse::Continue {
                required_history: &required_history,
            }
        } else {
            SessionRootUse::New
        };
        let session = match TaskSessionRoot::prepare(
            &roots.sessions,
            &task.workspace_id,
            &task.workspace.root_identity,
            &task.task_id,
            installation.harness,
            session_usage,
        ) {
            Ok(session) => session,
            Err(error) => {
                if run.continued_from.is_none() {
                    let _ = self.runtime.mark_native_root_unknown(
                        &task.workspace_id,
                        &task.task_id,
                        native.root.root_generation,
                        &native.root.creation_nonce,
                    );
                }
                return Err(error);
            }
        };
        if run.continued_from.is_none() {
            if native.root.state != DelegationNativeRootStateV1::Creating {
                return Err(DelegationErrorV1::Conflict);
            }
            let identity = match session.create_ownership_marker(&roots.sessions, &native.root) {
                Ok(identity) => identity,
                Err(error) => {
                    let _ = self.runtime.mark_native_root_unknown(
                        &task.workspace_id,
                        &task.task_id,
                        native.root.root_generation,
                        &native.root.creation_nonce,
                    );
                    return Err(error);
                }
            };
            let ready = DelegationNativeRootReadyV1 {
                workspace_id: task.workspace_id.clone(),
                task_id: task.task_id.clone(),
                root_generation: native.root.root_generation,
                creation_nonce: native.root.creation_nonce.clone(),
                managed_base_path: roots.sessions.to_string_lossy().into_owned(),
                filesystem_identity: identity,
            };
            if let Err(error) = self.runtime.commit_native_root_ready(&ready) {
                let _ = self.runtime.mark_native_root_unknown(
                    &task.workspace_id,
                    &task.task_id,
                    native.root.root_generation,
                    &native.root.creation_nonce,
                );
                return Err(error);
            }
        } else {
            if native.root.state != DelegationNativeRootStateV1::Ready
                || session.verify_ownership(&roots.sessions, &native.root)?
                    != native
                        .root
                        .filesystem_identity
                        .clone()
                        .ok_or(DelegationErrorV1::ResumeUnavailable)?
            {
                return Err(DelegationErrorV1::ResumeUnavailable);
            }
        }
        let private_root = roots
            .runs
            .join(run_root_name(&task.workspace_id, task, run)?);
        let admitted_at_ms =
            accepted_run_admitted_at_ms(run.deadline_ms, run.execution.duration_ms)?;
        let codex_catalog = if installation.harness == WorkerHarnessV1::CodexCli {
            if input.compiled_plan.model_alias().as_str() != task.plan.model_alias {
                return Err(DelegationErrorV1::Conflict);
            }
            Some(
                codex_private_worker_catalog(&input.compiled_plan)
                    .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?,
            )
        } else {
            None
        };
        CandidateWorkerProfile::build(ProfileInput {
            harness: installation.harness,
            adapter: &installation.adapter,
            harness_binary: &installation.harness_binary,
            node_binary: installation.node_binary.as_deref(),
            private_root: &private_root,
            session_root: &session,
            workspace: &input.workspace_path,
            alias: &task.plan.model_alias,
            codex_catalog: codex_catalog.as_deref(),
            native_effort: None,
            gateway: input.gateway,
            permit: &input.permit,
            execution: &run.execution,
            permission_policy: run.configuration.permission_policy,
            admitted_at_ms,
            token: input.model_token,
        })
    }
}

fn accepted_run_admitted_at_ms(
    deadline_ms: u64,
    duration_ms: u64,
) -> Result<u64, DelegationErrorV1> {
    deadline_ms
        .checked_sub(duration_ms)
        .ok_or(DelegationErrorV1::InvalidArguments)
}

fn run_root_name(
    workspace: &WorkspaceId,
    task: &hiroute_domain::delegation::DelegationTaskV1,
    run: &hiroute_domain::delegation::DelegationRunV1,
) -> Result<String, DelegationErrorV1> {
    let digest = CanonicalDigest::of(&(
        "delegation-run-root-v1",
        workspace,
        &task.task_id,
        &run.run_id,
        &run.lease_id,
        &run.launch_nonce,
    ))
    .map_err(|_| DelegationErrorV1::InvalidArguments)?;
    Ok(digest.as_str().trim_start_matches("sha256:").to_owned())
}

#[cfg(test)]
mod tests {
    use super::installation_probe::{last_acceptance_stage, run_installation_acceptance};
    use super::*;
    use hiroute_diagnostics::runtime::DiagnosticsPort;
    use hiroute_domain::delegation::{
        WorkerExecutionIntentV1, WorkerNetworkV1, WorkerToolV1, WorkspaceAccessV1,
        WorkspaceExecutionPermitV1,
    };

    fn required_path(name: &str) -> std::path::PathBuf {
        std::env::var_os(name)
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| panic!("{name} must name an absolute installed artifact"))
    }

    #[test]
    fn first_concurrent_root_preparation_revalidates_existing_directories() {
        let root = tempfile::tempdir().unwrap();
        let barrier = std::sync::Barrier::new(2);
        let prepared = std::thread::scope(|scope| {
            let jobs = (0..2)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        WorkerProfileRoots::prepare(root.path()).unwrap()
                    })
                })
                .collect::<Vec<_>>();
            jobs.into_iter()
                .map(|job| job.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(prepared[0].sessions, prepared[1].sessions);
        assert_eq!(prepared[0].runs, prepared[1].runs);
        for path in [&prepared[0].sessions, &prepared[0].runs] {
            let metadata = fs::symlink_metadata(path).unwrap();
            assert!(metadata.is_dir() && !metadata.file_type().is_symlink());
            assert!(private(&metadata));
            assert_eq!(fs::canonicalize(path).unwrap(), *path);
        }
        fs::write(root.path().join("file"), b"not a directory").unwrap();
        assert_eq!(
            create_private_child(root.path(), "file"),
            Err(DelegationErrorV1::PermissionDenied)
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            symlink(&prepared[0].sessions, root.path().join("link")).unwrap();
            assert_eq!(
                create_private_child(root.path(), "link"),
                Err(DelegationErrorV1::PermissionDenied)
            );
            fs::create_dir(root.path().join("public")).unwrap();
            fs::set_permissions(
                root.path().join("public"),
                fs::Permissions::from_mode(0o755),
            )
            .unwrap();
            assert_eq!(
                create_private_child(root.path(), "public"),
                Err(DelegationErrorV1::PermissionDenied)
            );
        }
    }

    #[test]
    fn accepted_run_authorization_reuses_the_persisted_admission_time() {
        let intent = WorkerExecutionIntentV1 {
            root_identity: "root".into(),
            access: WorkspaceAccessV1::TrustedNative,
            tools: vec![WorkerToolV1::Read],
            network: WorkerNetworkV1::GatewayOnly,
            duration_ms: 1_000,
            delegation_depth: 1,
        };
        let permit = WorkspaceExecutionPermitV1 {
            permit_id: "run-config/run".into(),
            generation: 1,
            root_identity: intent.root_identity.clone(),
            access: intent.access,
            tools: intent.tools.clone(),
            network: intent.network,
            expires_at_ms: 11_000,
            max_run_ms: intent.duration_ms,
            max_concurrent: 2,
            revoked: false,
        };
        let admitted_at_ms =
            accepted_run_admitted_at_ms(permit.expires_at_ms, intent.duration_ms).unwrap();
        assert_eq!(admitted_at_ms, 10_000);
        assert_eq!(
            permit.authorize(&intent, admitted_at_ms, permit.generation),
            Ok(permit.expires_at_ms)
        );
        assert_eq!(
            accepted_run_admitted_at_ms(999, intent.duration_ms),
            Err(DelegationErrorV1::InvalidArguments)
        );
    }

    /// This is intentionally opt-in: it executes the selected user-installed Node adapter and
    /// Codex binary against a fresh private loopback endpoint.  It never receives an upstream
    /// credential; CI fixtures continue to cover only the deterministic component contracts.
    #[test]
    #[ignore = "requires explicit user-installed Codex ACP paths"]
    fn actual_codex_acp_acceptance_uses_the_selected_installation() {
        let config = WorkerInstallationConfig::codex_acp(
            required_path("HIROUTE_WORKER_CODEX_ACP_ADAPTER"),
            required_path("HIROUTE_WORKER_CODEX_BINARY"),
            required_path("HIROUTE_WORKER_NODE"),
        );
        run_installation_acceptance(&config, &DiagnosticsPort::default()).unwrap_or_else(|error| {
            panic!(
                "the selected Codex ACP installation must prove start and cancellation; \
                 last safe checkpoint: {:?}; error: {error:?}",
                last_acceptance_stage(),
            )
        });
    }

    /// Opt-in for the user-installed Claude ACP package. The same bounded probe verifies native
    /// Messages routing, exact session load when supported, and owned-process cancellation.
    #[test]
    #[ignore = "requires explicit user-installed Claude ACP paths"]
    fn actual_claude_acp_acceptance_uses_the_selected_installation() {
        let config = WorkerInstallationConfig::claude_acp(
            required_path("HIROUTE_WORKER_CLAUDE_ACP_ADAPTER"),
            required_path("HIROUTE_WORKER_CLAUDE_BINARY"),
            required_path("HIROUTE_WORKER_NODE"),
        );
        run_installation_acceptance(&config, &DiagnosticsPort::default()).unwrap_or_else(|error| {
            panic!(
                "the selected Claude ACP installation must prove start and cancellation; \
                 last safe checkpoint: {:?}; error: {error:?}",
                last_acceptance_stage(),
            )
        });
    }
}
