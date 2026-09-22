//! One daemon-owned execution path for an already accepted delegation run.
//!
//! This module deliberately has no queue database and no recovery replay.  A caller can wake an
//! accepted run once in the current daemon epoch; all later state comes from the durable runtime
//! record.  The profile source is the only place that may turn independently verified local
//! installation facts into a Worker profile.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hiroute_application::delegation::safety::{RunSafetyBinding, RunSafetyProjection};
use hiroute_application::publication::admission::SharedAdmissionGate;
use hiroute_application::publication::versions::ExactPlanVersionPort;
use hiroute_diagnostics::context::DiagnosticContext;
use hiroute_diagnostics::correlation::CorrelationDomain;
use hiroute_diagnostics::event::{
    AdmissionOutcome, DiagnosticEvent, RunAdmission, TaskLifecycle, TaskLifecyclePhase,
};
use hiroute_diagnostics::runtime::DiagnosticsPort;
use hiroute_domain::delegation::{
    DelegationBodyRefV1, DelegationErrorV1, DelegationRunV1, DelegationRuntimePort,
    DelegationTaskV1, RunEventV1, WorkspaceExecutionPermitV1,
};
use hiroute_domain::{
    AgentIngressProtocolV1, CanonicalDigest, CompiledAgentPlanV1, PlanExecutionRef,
    PlanVersionError, ProtectedSecret, VersionOwnerKindV1, VersionOwnerPurposeV1,
    VersionOwnerRefV1, WorkspaceId,
};
use hiroute_observation::LocalObservationStore;
use hiroute_observation::managed_text::{ManagedTextRef, ManagedTextScope, ManagedTextState};
use tokio::runtime::Builder;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::acp::{AcpRunInput, AcpSessionStart};
use super::content::read_required_body;
use super::credentials::{
    RunCredentialFingerprints, RunCredentialPair, RunCredentialRecord, RunCredentialVerifier,
};
use super::dispatcher::DelegationCancellationDispatcher;
use super::finalization::DelegationFinalization;
use super::lifecycle::{self, WorkerRunJournal};
use super::persistent_journal::{PersistentWorkerRunJournal, task_state};
use super::platform::{WorkerLaunchRequest, WorkerPlatformPort};
use super::profile::CandidateWorkerProfile;
use super::run_authority::DelegationRunAuthority;

mod control;
mod resume;
#[cfg(all(test, unix))]
mod tests;

const MAX_INPUT_BYTES: usize = 256 * 1024;

/// Non-secret, daemon-internal facts that are fixed before a Worker process is started.  The
/// model credential is move-only and must be consumed by the profile renderer; it is never put
/// in a task record, process argument, or diagnostic value.
pub struct WorkerProfileInput {
    pub task: DelegationTaskV1,
    pub run: DelegationRunV1,
    /// The already-verified frozen task version; native metadata must follow this version.
    pub compiled_plan: CompiledAgentPlanV1,
    /// Re-read immediately before profile rendering.  The accepted run retains only the exact
    /// generation; it never carries a stale permit snapshot into a child process.
    pub permit: WorkspaceExecutionPermitV1,
    pub workspace_path: PathBuf,
    pub gateway: SocketAddr,
    pub model_token: ProtectedSecret,
}

/// Converts separately revalidated installation/capability facts into the fixed process profile.
/// Implementations must not install dependencies or derive a profile from Worker-provided data.
pub trait WorkerProfileSource: Send + Sync {
    fn build(&self, input: WorkerProfileInput)
    -> Result<CandidateWorkerProfile, DelegationErrorV1>;
}

/// The default composition is explicitly unavailable until the settings/runtime owner installs a
/// measured profile source.  This makes a missing adapter a durable, actionable run failure
/// instead of selecting an ambient executable or silently downloading one.
#[derive(Default)]
pub struct UnavailableWorkerProfileSource;

impl WorkerProfileSource for UnavailableWorkerProfileSource {
    fn build(&self, _: WorkerProfileInput) -> Result<CandidateWorkerProfile, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
}

/// The single in-process owner for active ACP executions.  It owns only current daemon objects;
/// durable task/run state remains in `DelegationRuntimePort` and is never replayed on startup.
pub struct DelegationRunExecutor {
    runtime: Arc<dyn DelegationRuntimePort + Send + Sync>,
    versions: Arc<dyn ExactPlanVersionPort + Send + Sync>,
    gate: Arc<SharedAdmissionGate>,
    safety: Arc<RunSafetyProjection>,
    observation: Arc<LocalObservationStore>,
    authority: Arc<DelegationRunAuthority>,
    cancellation: Arc<DelegationCancellationDispatcher>,
    finalization: Arc<DelegationFinalization>,
    platform: Arc<dyn WorkerPlatformPort>,
    profiles: RwLock<Arc<dyn WorkerProfileSource>>,
    gateway: RwLock<Option<SocketAddr>>,
    diagnostics: RwLock<DiagnosticsPort>,
    active: Mutex<BTreeSet<(String, String, String)>>,
}

impl DelegationRunExecutor {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        runtime: Arc<dyn DelegationRuntimePort + Send + Sync>,
        versions: Arc<dyn ExactPlanVersionPort + Send + Sync>,
        gate: Arc<SharedAdmissionGate>,
        safety: Arc<RunSafetyProjection>,
        observation: Arc<LocalObservationStore>,
        authority: Arc<DelegationRunAuthority>,
        cancellation: Arc<DelegationCancellationDispatcher>,
        finalization: Arc<DelegationFinalization>,
        platform: Arc<dyn WorkerPlatformPort>,
        profiles: Arc<dyn WorkerProfileSource>,
    ) -> Self {
        Self {
            runtime,
            versions,
            gate,
            safety,
            observation,
            authority,
            cancellation,
            finalization,
            platform,
            profiles: RwLock::new(profiles),
            gateway: RwLock::new(None),
            diagnostics: RwLock::new(DiagnosticsPort::default()),
            active: Mutex::new(BTreeSet::new()),
        }
    }

    /// The process entry point installs the diagnostic port once the runtime exists; a
    /// missing port keeps worker/task diagnostics inert instead of failing execution.
    pub fn set_diagnostics(&self, diagnostics: DiagnosticsPort) {
        if let Ok(mut current) = self.diagnostics.write() {
            *current = diagnostics;
        }
    }

    fn diagnostics(&self) -> DiagnosticsPort {
        self.diagnostics
            .read()
            .map(|port| port.clone())
            .unwrap_or_default()
    }

    /// Revalidation replaces the whole immutable source at once.  An already-rendered Worker
    /// profile retains its measured facts, while every later run must pass the new source's
    /// artifact checks.  A poisoned lock is treated as unavailable rather than reusing old
    /// installation facts.
    pub fn set_profile_source(
        &self,
        profiles: Arc<dyn WorkerProfileSource>,
    ) -> Result<(), DelegationErrorV1> {
        *self
            .profiles
            .write()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)? = profiles;
        Ok(())
    }

    /// The role-all composition supplies the actual host-owned IPv4 listener only after Gateway
    /// starts. The endpoint is not Worker input; IPv6, wildcard client targets and zero ports are
    /// rejected rather than becoming a Worker-controlled egress path.
    pub fn set_gateway(&self, gateway: SocketAddr) -> Result<(), DelegationErrorV1> {
        if !gateway.is_ipv4() || gateway.ip().is_unspecified() || gateway.port() == 0 {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        *self
            .gateway
            .write()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)? = Some(gateway);
        Ok(())
    }

    /// Starts one background execution only for a newly accepted, unbound run.  A failed thread
    /// creation is persisted as a pre-launch failure; a replay cannot create another prompt.
    pub fn wake(
        self: &Arc<Self>,
        workspace: &WorkspaceId,
        run_id: &str,
        workspace_path: PathBuf,
    ) -> Result<(), DelegationErrorV1> {
        let run = self
            .runtime
            .run(workspace, run_id)?
            .ok_or(DelegationErrorV1::Conflict)?;
        if run.workspace_id != *workspace
            || run.run_id != run_id
            || run.process.is_some()
            || run.session.is_some()
            || run.progress.state != hiroute_domain::delegation::RunStateV1::Accepted
            || run.lease_revoked
        {
            return Err(DelegationErrorV1::Conflict);
        }
        let key = active_key(&run);
        // One context for this run's admission and lifecycle events; the tokens come from
        // the authoritative ids held here and never from a guess.
        let diagnostics = self.diagnostics();
        let context = diagnostics.handle().root_context();
        let task_token = context.token(CorrelationDomain::Task, &run.task_id);
        let run_token = context.token(CorrelationDomain::Run, &run.run_id);
        {
            let mut active = self
                .active
                .lock()
                .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
            if !active.insert(key.clone()) {
                // Another run of this workspace already holds the single execution slot.
                context.emit(DiagnosticEvent::RunAdmission(RunAdmission {
                    outcome: AdmissionOutcome::Deferred,
                    task: task_token,
                    run: run_token,
                }));
                return Ok(());
            }
        }
        context.emit(DiagnosticEvent::RunAdmission(RunAdmission {
            outcome: AdmissionOutcome::Admitted,
            task: task_token,
            run: run_token,
        }));
        let executor = Arc::clone(self);
        let workspace = workspace.clone();
        let run_id = run_id.to_owned();
        let thread_workspace = workspace.clone();
        let thread_run_id = run_id.clone();
        let thread_key = key.clone();
        let thread_context = context.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("hiroute-delegation-{}", run_id))
            .spawn(move || {
                let runtime = Builder::new_current_thread().enable_all().build();
                if let Ok(runtime) = runtime {
                    runtime.block_on(executor.execute(
                        &thread_context,
                        &thread_workspace,
                        &thread_run_id,
                        workspace_path,
                    ));
                } else {
                    let _ = executor.fail_before_launch(
                        &thread_context,
                        &thread_workspace,
                        &thread_run_id,
                        "runtime-build",
                    );
                }
                executor.remove_active(&thread_key);
            });
        if spawned.is_err() {
            self.remove_active(&key);
            self.fail_before_launch(&context, &workspace, &run_id, "thread-spawn")?;
        }
        Ok(())
    }

    fn remove_active(&self, key: &(String, String, String)) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(key);
        }
    }

    async fn execute(
        &self,
        context: &DiagnosticContext,
        workspace: &WorkspaceId,
        run_id: &str,
        workspace_path: PathBuf,
    ) {
        let result = self
            .execute_inner(context, workspace, run_id, workspace_path)
            .await;
        if result.is_err() {
            let _ = self.fail_before_launch(context, workspace, run_id, "executor-unexpected");
        }
    }

    async fn execute_inner(
        &self,
        context: &DiagnosticContext,
        workspace: &WorkspaceId,
        run_id: &str,
        workspace_path: PathBuf,
    ) -> Result<(), DelegationErrorV1> {
        let run = self
            .runtime
            .run(workspace, run_id)?
            .ok_or(DelegationErrorV1::Conflict)?;
        let task = self
            .runtime
            .task(workspace, &run.task_id)?
            .ok_or(DelegationErrorV1::Conflict)?;
        run.configuration.validate_for(&run)?;
        if task.workspace_id != *workspace
            || task.task_id != run.task_id
            || task.latest_run_id != run.run_id
            || run.progress.state != hiroute_domain::delegation::RunStateV1::Accepted
            || run.process.is_some()
            || run.lease_revoked
        {
            return Err(DelegationErrorV1::Conflict);
        }
        let session_start = match (&run.continued_from, &task.session) {
            (None, None) => AcpSessionStart::New,
            (Some(previous), Some(binding))
                if previous != &run.run_id && binding.native_session_id.is_some() =>
            {
                AcpSessionStart::Load(binding.clone())
            }
            _ => return Err(DelegationErrorV1::ResumeUnavailable),
        };
        let workspace_path = checked_workspace_path(&workspace_path, &task.workspace)?;
        if workspace_path != Path::new(&run.configuration.canonical_workspace_path) {
            return Err(DelegationErrorV1::PermissionDenied);
        }
        let gateway = self
            .gateway
            .read()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?
            .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
        let version = match self.versions.lookup_exact(&plan_reference(&task)) {
            Ok(version) => version,
            Err(_) => {
                self.fail_before_launch(context, workspace, run_id, "exact-version")?;
                return Ok(());
            }
        };
        let Some(work) = version.configuration.work.as_ref() else {
            self.fail_before_launch(context, workspace, run_id, "missing-worker-plan")?;
            return Ok(());
        };
        if !version_matches_task(
            &task,
            &version.reference,
            work,
            version.compiled.model_alias().as_str(),
        ) {
            self.fail_before_launch(context, workspace, run_id, "version-mismatch")?;
            return Ok(());
        }
        let pair = RunCredentialPair::generate()?;
        let fingerprints = pair.fingerprints();
        let RunCredentialPair {
            model: model_token,
            self_query,
        } = pair;
        drop(self_query);
        let verifier = Arc::new(RunCredentialVerifier::new(
            credential_record(&task, &run, fingerprints, work.protocol)?,
            Arc::clone(&self.safety),
        )?);
        let journal = Arc::new(PersistentWorkerRunJournal::new(
            Arc::clone(&self.runtime),
            Arc::clone(&self.safety),
            Arc::clone(&verifier),
            Arc::clone(&self.observation),
            Arc::clone(&self.finalization),
            run.clone(),
            context.clone(),
        )?);
        let prompt = match read_prompt(&self.observation, Arc::clone(&self.safety), &task, &run) {
            Ok(prompt) => prompt,
            Err(error) => {
                let _ = journal.execution_failed(error);
                return self.finish_without_worker(&journal, &run, &verifier, false, false);
            }
        };
        let permit = match run.profile_permit() {
            Ok(permit) => permit,
            Err(_) => {
                let _ = journal.execution_failed(DelegationErrorV1::PermissionDenied);
                return self.finish_without_worker(&journal, &run, &verifier, false, false);
            }
        };
        let deadline = match deadline_instant(run.deadline_ms) {
            Ok(deadline) => deadline,
            Err(error) => {
                let _ = journal.execution_failed(error);
                return self.finish_without_worker(&journal, &run, &verifier, false, false);
            }
        };
        let profiles = self
            .profiles
            .read()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?
            .clone();
        let profile_workspace_path = workspace_path.clone();
        let profile = match profiles.build(WorkerProfileInput {
            task: task.clone(),
            run: run.clone(),
            compiled_plan: version.compiled.clone(),
            permit,
            workspace_path: profile_workspace_path,
            gateway,
            model_token,
        }) {
            Ok(profile) => profile,
            Err(error) => {
                let _ = journal.execution_failed(error);
                return self.finish_without_worker(&journal, &run, &verifier, false, false);
            }
        };
        if profile.cwd != workspace_path {
            let _ = journal.execution_failed(DelegationErrorV1::PermissionDenied);
            return self.finish_without_worker(&journal, &run, &verifier, false, false);
        }
        let session_root = profile.session_root.clone();
        if self
            .authority
            .register(&task, &run, version, Arc::clone(&verifier))
            .is_err()
        {
            let _ = journal.execution_failed(DelegationErrorV1::Conflict);
            return self.finish_without_worker(&journal, &run, &verifier, false, false);
        }
        let cancellation = CancellationToken::new();
        if let Err(error) = self.cancellation.attach(
            self.runtime.as_ref(),
            workspace,
            run_id,
            cancellation.clone(),
        ) {
            let _ = journal.execution_failed(error);
            return self.finish_without_worker(&journal, &run, &verifier, true, false);
        }
        let identity_contract = profile.identity_contract.clone();
        let session_meta = profile.session_meta.clone();
        let native_session_mode = Some(profile.native_session_mode().to_owned());
        let cwd = profile.cwd.clone();
        let launch = WorkerLaunchRequest {
            launch_nonce: run.launch_nonce.clone(),
            profile,
            deadline_unix_ms: run.deadline_ms,
        };
        let worker = lifecycle::execute(
            self.platform.as_ref(),
            launch,
            AcpRunInput {
                cwd,
                prompt,
                session: session_start,
                identity_contract,
                session_meta,
                native_session_mode,
                authentication: None,
                deadline,
                cancellation,
            },
            journal.clone(),
        )
        .await;
        match worker {
            Ok(result) => {
                // `lifecycle::execute` acquired this owned lease immediately before making the
                // terminal state visible. Keep it through stop recording and continuation
                // retention so Continue can never observe the half-finalized task.
                let _finalizing = result
                    .finalization
                    .expect("the persistent Worker journal acquires finalization");
                let worker_identity = Some(result.identity);
                let native_history_settled = result.native_history_settled;
                let _ = journal.record_stop(result.stop);
                self.finish(
                    &journal,
                    &run,
                    &verifier,
                    true,
                    true,
                    worker_identity,
                    Some(&session_root),
                    native_history_settled,
                )
            }
            Err(error) => {
                let _finalizing = self.finalization.acquire();
                let _ = journal.execution_failed(error);
                self.finish(
                    &journal,
                    &run,
                    &verifier,
                    true,
                    true,
                    None,
                    Some(&session_root),
                    false,
                )
            }
        }
    }

    fn finish_without_worker(
        &self,
        journal: &PersistentWorkerRunJournal,
        run: &DelegationRunV1,
        verifier: &RunCredentialVerifier,
        registered: bool,
        attached: bool,
    ) -> Result<(), DelegationErrorV1> {
        self.finish(
            journal, run, verifier, registered, attached, None, None, false,
        )
    }

    // These arguments are independently durable cleanup checkpoints and optional platform
    // artifacts; keeping them named avoids hiding their ordering in an untyped tuple.
    #[allow(clippy::too_many_arguments)]
    fn finish(
        &self,
        journal: &PersistentWorkerRunJournal,
        run: &DelegationRunV1,
        verifier: &RunCredentialVerifier,
        registered: bool,
        attached: bool,
        worker_identity: Option<hiroute_domain::delegation::DelegationProcessBindingV1>,
        session_root: Option<&Path>,
        native_history_settled: bool,
    ) -> Result<(), DelegationErrorV1> {
        let _ = journal.stop_intent();
        if registered {
            self.authority.unregister(run, verifier);
        }
        if attached {
            self.cancellation
                .detach(&run.workspace_id, &run.run_id, &run.lease_id)?;
        }
        let updated = journal.run()?;
        if !updated.progress.workspace_releasable() {
            return Ok(());
        }
        if let Some(identity) = worker_identity.as_ref() {
            self.platform.release(identity)?;
        }
        if native_history_settled && let Some(session_root) = session_root {
            let _ = self.retain_continuation(&updated, session_root);
        }
        self.versions
            .release(&run.workspace_id, &version_owner(run))
            .map_err(map_plan_error)
    }

    /// Record the durable terminal for a failure that happened before a run journal existed.
    /// The checkpoint is the authority: only after it succeeded is the same terminal emitted
    /// on this run's own context, carrying the authoritative task/run tokens. A conflict,
    /// already-terminal run or failed checkpoint produces no event, so a failure that did not
    /// happen is never published and no second terminal can appear.
    fn fail_before_launch(
        &self,
        context: &DiagnosticContext,
        workspace: &WorkspaceId,
        run_id: &str,
        stage: &str,
    ) -> Result<(), DelegationErrorV1> {
        let run = self
            .runtime
            .run(workspace, run_id)?
            .ok_or(DelegationErrorV1::Conflict)?;
        if run.progress.state != hiroute_domain::delegation::RunStateV1::Accepted
            || run.process.is_some()
            || run.lease_revoked
        {
            return Ok(());
        }
        let event_id = format!("delegation-executor/{stage}/{}", run.lease_id);
        let updated = self.runtime.checkpoint(
            workspace,
            run_id,
            run.progress.revision,
            &event_id,
            &hiroute_domain::delegation::DelegationCheckpointV1::Progress {
                event: RunEventV1::LaunchFailedBeforeSpawn,
            },
        )?;
        context.emit(DiagnosticEvent::TaskLifecycle(TaskLifecycle {
            phase: TaskLifecyclePhase::End,
            state: task_state(updated.progress.state),
            task: context.token(CorrelationDomain::Task, &updated.task_id),
            run: context.token(CorrelationDomain::Run, &updated.run_id),
        }));
        if updated.progress.workspace_releasable() {
            let _ = self.versions.release(workspace, &version_owner(&updated));
        }
        Ok(())
    }
}

fn active_key(run: &DelegationRunV1) -> (String, String, String) {
    (
        run.workspace_id.as_str().to_owned(),
        run.run_id.clone(),
        run.lease_id.clone(),
    )
}

fn plan_reference(task: &DelegationTaskV1) -> PlanExecutionRef {
    PlanExecutionRef {
        workspace_id: task.workspace_id.clone(),
        plan_id: task.plan.plan_id.clone(),
        content_revision: task.plan.plan_revision,
        content_digest: task.plan.plan_digest.clone(),
    }
}

fn version_matches_task(
    task: &DelegationTaskV1,
    reference: &PlanExecutionRef,
    work: &hiroute_domain::routing::WorkerPlanV1,
    alias: &str,
) -> bool {
    reference == &plan_reference(task)
        && task.plan.exact_reference
            == format!(
                "plan/{}/revision/{}/digest/{}",
                reference.plan_id.as_str(),
                reference.content_revision,
                reference.content_digest
            )
        && task.plan.model_alias == alias
        && CanonicalDigest::of(work)
            .map(|digest| digest == task.plan.harness_configuration_digest)
            .unwrap_or(false)
        && matches!(
            (task.plan.harness, work.harness),
            (
                hiroute_domain::delegation::WorkerHarnessV1::CodexCli,
                hiroute_domain::delegation::WorkerHarnessV1::CodexCli
            ) | (
                hiroute_domain::delegation::WorkerHarnessV1::ClaudeCode,
                hiroute_domain::delegation::WorkerHarnessV1::ClaudeCode
            )
        )
}

fn credential_record(
    task: &DelegationTaskV1,
    run: &DelegationRunV1,
    fingerprints: RunCredentialFingerprints,
    protocol: AgentIngressProtocolV1,
) -> Result<RunCredentialRecord, DelegationErrorV1> {
    Ok(RunCredentialRecord {
        task_id: task.task_id.clone(),
        run_id: run.run_id.clone(),
        lease_id: run.lease_id.clone(),
        safety: RunSafetyBinding {
            workspace: run.workspace_id.clone(),
            daemon_epoch: run.daemon_epoch.clone(),
            permit_id: run.permit_id.clone(),
            permit_generation: run.permit_generation,
            expires_at_ms: run.deadline_ms,
        },
        model_alias: task.plan.model_alias.clone(),
        protocol,
        fingerprints,
    })
}

fn read_prompt(
    observation: &LocalObservationStore,
    safety: Arc<RunSafetyProjection>,
    task: &DelegationTaskV1,
    run: &DelegationRunV1,
) -> Result<String, DelegationErrorV1> {
    let body = task
        .body_refs
        .iter()
        .find(|body| body.scope_run_id == run.run_id)
        .ok_or(DelegationErrorV1::ResumeUnavailable)?;
    let reference = managed_reference(task, run, body)?;
    let scope = reference.scope.clone();
    let bytes = read_required_body(
        observation,
        &scope,
        &reference,
        now_i64()?,
        MAX_INPUT_BYTES,
        |_| safety.check(&safety_binding(run), now_ms()?),
    )?;
    String::from_utf8(bytes).map_err(|_| DelegationErrorV1::ContentUnavailable)
}

fn managed_reference(
    task: &DelegationTaskV1,
    run: &DelegationRunV1,
    body: &DelegationBodyRefV1,
) -> Result<ManagedTextRef, DelegationErrorV1> {
    body.validate()?;
    if body.scope_run_id != run.run_id {
        return Err(DelegationErrorV1::Conflict);
    }
    Ok(ManagedTextRef {
        opaque_id: body.opaque_id.clone(),
        scope: ManagedTextScope {
            workspace_id: task.workspace_id.clone(),
            task_id: task.task_id.clone(),
            run_id: run.run_id.clone(),
        },
        visibility_generation: body.visibility_generation,
        original_retention_deadline_ms: body.original_retention_deadline_ms,
        state: ManagedTextState::Complete,
    })
}

fn checked_workspace_path(
    path: &Path,
    expected: &hiroute_domain::delegation::DelegationWorkspaceV1,
) -> Result<PathBuf, DelegationErrorV1> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if !path.is_absolute() {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        let canonical =
            std::fs::canonicalize(path).map_err(|_| DelegationErrorV1::InvalidArguments)?;
        let metadata =
            std::fs::metadata(&canonical).map_err(|_| DelegationErrorV1::InvalidArguments)?;
        if !metadata.is_dir() {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        let mut ancestry = Vec::new();
        for ancestor in canonical.ancestors() {
            let metadata =
                std::fs::metadata(ancestor).map_err(|_| DelegationErrorV1::InvalidArguments)?;
            ancestry.push(format!("inode/{}/{}", metadata.dev(), metadata.ino()));
        }
        ancestry.reverse();
        let actual = hiroute_domain::delegation::DelegationWorkspaceV1 {
            root_identity: format!("inode/{}/{}", metadata.dev(), metadata.ino()),
            volume_identity: format!("volume/{}", metadata.dev()),
            ancestry,
        };
        if &actual != expected {
            return Err(DelegationErrorV1::PermissionDenied);
        }
        Ok(canonical)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, expected);
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
}

fn deadline_instant(deadline_ms: u64) -> Result<Instant, DelegationErrorV1> {
    let remaining = deadline_ms
        .checked_sub(now_ms()?)
        .filter(|remaining| *remaining > 0)
        .ok_or(DelegationErrorV1::DeadlineExceeded)?;
    Instant::now()
        .checked_add(Duration::from_millis(remaining))
        .ok_or(DelegationErrorV1::DeadlineExceeded)
}

fn version_owner(run: &DelegationRunV1) -> VersionOwnerRefV1 {
    VersionOwnerRefV1 {
        kind: VersionOwnerKindV1::Run,
        owner_id: run.execution_owner_ref.clone(),
        purpose: VersionOwnerPurposeV1::Execution,
    }
}

fn safety_binding(run: &DelegationRunV1) -> RunSafetyBinding {
    RunSafetyBinding {
        workspace: run.workspace_id.clone(),
        daemon_epoch: run.daemon_epoch.clone(),
        permit_id: run.permit_id.clone(),
        permit_generation: run.permit_generation,
        expires_at_ms: run.deadline_ms,
    }
}

fn now_ms() -> Result<u64, DelegationErrorV1> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DelegationErrorV1::DeadlineExceeded)
        .and_then(|duration| {
            u64::try_from(duration.as_millis()).map_err(|_| DelegationErrorV1::DeadlineExceeded)
        })
}

fn now_i64() -> Result<i64, DelegationErrorV1> {
    i64::try_from(now_ms()?).map_err(|_| DelegationErrorV1::DeadlineExceeded)
}

fn map_plan_error(error: PlanVersionError) -> DelegationErrorV1 {
    match error {
        PlanVersionError::Invalid => DelegationErrorV1::InvalidArguments,
        PlanVersionError::Conflict | PlanVersionError::Stale => DelegationErrorV1::Conflict,
        PlanVersionError::Unavailable
        | PlanVersionError::Disabled
        | PlanVersionError::RecoveryRequired => DelegationErrorV1::CapabilityUnavailable,
        PlanVersionError::Retained | PlanVersionError::StorageUnavailable => {
            DelegationErrorV1::StorageUnavailable
        }
    }
}
