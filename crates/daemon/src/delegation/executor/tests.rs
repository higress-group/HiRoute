//! The failure entry of one accepted run: an early failure is durable before it is reported,
//! and a failure that could not be recorded claims nothing.

use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use hiroute_application::publication::versions::ExactPlanVersionPort;
use hiroute_diagnostics::runtime::{DiagnosticRuntime, RuntimeConfig};
use hiroute_domain::delegation::{
    DELEGATION_RUN_CONFIGURATION_VERSION_V1, DelegationAcceptanceV1, DelegationCancelReceiptV1,
    DelegationCheckpointV1, DelegationRunConfigurationV1, WorkerExecutionIntentV1, WorkerNetworkV1,
    WorkerPermissionPolicyV1, WorkerToolV1, WorkspaceAccessV1,
};
use hiroute_domain::{
    AgentPlanId, CanonicalDigest, OperationId, PlanExecutionRef, PlanHeadV1, PlanVersionError,
    PlanVersionV1, VersionOwnerRefV1, VersionReservationV1, WorkspaceId,
};
use hiroute_observation::DigestAuthority;

use super::*;
use crate::delegation::platform::{
    ReadyWorker, WorkerObservation, WorkerPlatformCapabilities, WorkerProcessIdentity,
    WorkerStopEvidence,
};

/// One accepted run and nothing else: every failure below happens before a Worker exists.
struct MemoryRuntime {
    run: Mutex<DelegationRunV1>,
    checkpoints_fail: AtomicBool,
}

impl MemoryRuntime {
    fn accepted() -> Self {
        Self {
            run: Mutex::new(accepted_run()),
            checkpoints_fail: AtomicBool::new(false),
        }
    }

    fn state(&self) -> (hiroute_domain::delegation::RunStateV1, u64) {
        let run = self.run.lock().unwrap();
        (run.progress.state, run.progress.revision)
    }

    fn unrecordable(&self) {
        self.checkpoints_fail.store(true, Ordering::SeqCst);
    }
}

impl DelegationRuntimePort for MemoryRuntime {
    fn task(
        &self,
        _: &WorkspaceId,
        _: &str,
    ) -> Result<Option<DelegationTaskV1>, DelegationErrorV1> {
        Ok(None)
    }

    fn run(
        &self,
        workspace: &WorkspaceId,
        run_id: &str,
    ) -> Result<Option<DelegationRunV1>, DelegationErrorV1> {
        let run = self
            .run
            .lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        Ok((run.workspace_id == *workspace && run.run_id == run_id).then(|| run.clone()))
    }

    fn find_submission(
        &self,
        _: &WorkspaceId,
        _: bool,
        _: &str,
    ) -> Result<Option<DelegationRunV1>, DelegationErrorV1> {
        Ok(None)
    }

    fn accept(&self, _: &DelegationAcceptanceV1) -> Result<DelegationRunV1, DelegationErrorV1> {
        Err(DelegationErrorV1::Conflict)
    }

    fn checkpoint(
        &self,
        workspace: &WorkspaceId,
        run_id: &str,
        expected_revision: u64,
        _: &str,
        event: &DelegationCheckpointV1,
    ) -> Result<DelegationRunV1, DelegationErrorV1> {
        if self.checkpoints_fail.load(Ordering::SeqCst) {
            return Err(DelegationErrorV1::StorageUnavailable);
        }
        let mut run = self
            .run
            .lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        if run.workspace_id != *workspace
            || run.run_id != run_id
            || run.progress.revision != expected_revision
        {
            return Err(DelegationErrorV1::Conflict);
        }
        match event {
            DelegationCheckpointV1::Progress { event } => run.progress.advance(*event)?,
            _ => return Err(DelegationErrorV1::PermissionDenied),
        }
        run.progress.revision = expected_revision
            .checked_add(1)
            .ok_or(DelegationErrorV1::Conflict)?;
        Ok(run.clone())
    }

    fn request_cancel(
        &self,
        _: &WorkspaceId,
        _: &str,
        _: &OperationId,
        _: &str,
    ) -> Result<DelegationCancelReceiptV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    fn unreconciled(&self, _: &WorkspaceId) -> Result<Vec<DelegationRunV1>, DelegationErrorV1> {
        Ok(Vec::new())
    }

    fn set_resume_materials(
        &self,
        _: &WorkspaceId,
        _: &str,
        _: &str,
        _: u64,
        _: &[DelegationBodyRefV1],
        _: &[String],
    ) -> Result<(), DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
}

/// No exact plan version is ever published here, so a run that reaches that step fails before
/// a Worker journal exists.
struct NoVersions;

impl ExactPlanVersionPort for NoVersions {
    fn current_plan(
        &self,
        _: &hiroute_application::publication::admission::AdmissionGuard<'_>,
        _: &AgentPlanId,
    ) -> Result<PlanHeadV1, PlanVersionError> {
        Err(PlanVersionError::Unavailable)
    }

    fn acquire_exact(
        &self,
        _: &hiroute_application::publication::admission::AdmissionGuard<'_>,
        _: &VersionReservationV1,
    ) -> Result<Arc<PlanVersionV1>, PlanVersionError> {
        Err(PlanVersionError::Unavailable)
    }

    fn lookup_exact(&self, _: &PlanExecutionRef) -> Result<Arc<PlanVersionV1>, PlanVersionError> {
        Err(PlanVersionError::Unavailable)
    }

    fn renew(
        &self,
        _: &WorkspaceId,
        _: &VersionOwnerRefV1,
        _: i64,
    ) -> Result<(), PlanVersionError> {
        Err(PlanVersionError::Unavailable)
    }

    fn release(&self, _: &WorkspaceId, _: &VersionOwnerRefV1) -> Result<(), PlanVersionError> {
        Ok(())
    }

    fn reconcile(
        &self,
        _: &WorkspaceId,
        _: &[VersionReservationV1],
        _: i64,
    ) -> Result<(), PlanVersionError> {
        Ok(())
    }
}

/// No Worker can ever be launched: reaching this backend fails the test's premise loudly.
struct UnusedPlatform;

#[async_trait]
impl WorkerPlatformPort for UnusedPlatform {
    fn capabilities(
        &self,
        _: &CandidateWorkerProfile,
    ) -> Result<WorkerPlatformCapabilities, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    async fn launch(&self, _: WorkerLaunchRequest) -> Result<ReadyWorker, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    async fn observe(
        &self,
        _: &WorkerProcessIdentity,
    ) -> Result<WorkerObservation, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    async fn terminate(
        &self,
        _: &WorkerProcessIdentity,
        _: u64,
    ) -> Result<WorkerStopEvidence, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
}

fn accepted_run() -> DelegationRunV1 {
    DelegationRunV1 {
        workspace_id: WorkspaceId::default(),
        task_id: "task/one".into(),
        run_id: "run/one".into(),
        ordinal: 1,
        continued_from: None,
        idempotency_key: "start/one".into(),
        request_digest: CanonicalDigest::of_bytes(b"request"),
        admission_sequence: 1,
        accepted_at_ms: Some(1),
        execution_owner_ref: "delegation-run/run/one".into(),
        lease_id: "lease/one".into(),
        daemon_epoch: "epoch".into(),
        permit_id: "run-config/run/one".into(),
        permit_generation: 1,
        configuration: DelegationRunConfigurationV1 {
            format_version: DELEGATION_RUN_CONFIGURATION_VERSION_V1,
            scope_id: "run-config/run/one".into(),
            generation: 1,
            canonical_workspace_path: "/workspace".into(),
            permission_policy: WorkerPermissionPolicyV1::ApproveAll,
        },
        execution: WorkerExecutionIntentV1 {
            root_identity: "root".into(),
            access: WorkspaceAccessV1::TrustedNative,
            tools: vec![WorkerToolV1::Read],
            network: WorkerNetworkV1::Allowed,
            duration_ms: 30_000,
            delegation_depth: 1,
        },
        deadline_ms: 30_000,
        lease_revoked: false,
        launch_nonce: "launch/one".into(),
        process: None,
        session: None,
        progress: Default::default(),
        stop_evidence: None,
        result_body: None,
        result_incomplete: false,
    }
}

fn diagnostic_report(root: &std::path::Path) -> DiagnosticRuntime {
    DiagnosticRuntime::start(RuntimeConfig {
        root: root.join("diagnostics"),
        role: hiroute_diagnostics::event::ProcessRole::Daemon,
        component: hiroute_diagnostics::record::Component::Daemon,
        parent_session_id: None,
        level_override: Some(hiroute_diagnostics::level::DiagnosticLevel::Debug),
    })
}

fn executor(
    runtime: Arc<MemoryRuntime>,
    observation: Arc<LocalObservationStore>,
    safety: Arc<RunSafetyProjection>,
) -> DelegationRunExecutor {
    DelegationRunExecutor::new(
        runtime,
        Arc::new(NoVersions),
        Arc::new(SharedAdmissionGate::new()),
        safety,
        observation,
        Arc::new(DelegationRunAuthority::default()),
        Arc::new(DelegationCancellationDispatcher::default()),
        Arc::new(crate::delegation::finalization::DelegationFinalization::default()),
        Arc::new(UnusedPlatform),
        Arc::new(UnavailableWorkerProfileSource),
    )
}

/// Drive the executor's own failure entry, exactly as `wake` does on its worker thread.
fn run_failure_entry(executor: &DelegationRunExecutor, context: &DiagnosticContext) {
    let handle = Builder::new_current_thread().enable_all().build().unwrap();
    handle.block_on(executor.execute(
        context,
        &WorkspaceId::default(),
        "run/one",
        PathBuf::from("/workspace"),
    ));
}

fn task_lifecycles(log: &str) -> Vec<serde_json::Value> {
    log.lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("record is JSON"))
        .filter_map(|record| {
            let event = record.get("event")?.get("task_lifecycle")?.clone();
            Some(serde_json::json!({"event": event, "span_id": record["span_id"]}))
        })
        .collect()
}

fn fixture(
    temporary: &tempfile::TempDir,
    seed: u8,
) -> (DiagnosticRuntime, Arc<MemoryRuntime>, DelegationRunExecutor) {
    let report = diagnostic_report(temporary.path());
    let runtime = Arc::new(MemoryRuntime::accepted());
    let observation = Arc::new(
        LocalObservationStore::open(temporary.path(), DigestAuthority::new([seed; 32])).unwrap(),
    );
    let safety = Arc::new(
        RunSafetyProjection::new(Arc::new(SharedAdmissionGate::new()), "epoch".into()).unwrap(),
    );
    safety.finish_startup_recovery();
    let executor = executor(Arc::clone(&runtime), observation, safety);
    (report, runtime, executor)
}

/// R6: a failure before a run journal exists records the real durable terminal first, then
/// reports that same terminal on this run's own context with the authoritative task/run
/// tokens; a second pass over the now-terminal run adds nothing.
#[test]
fn an_early_failure_records_one_terminal_on_the_run_context() {
    let temporary = crate::test_support::private_tempdir();
    let (report, runtime, executor) = fixture(&temporary, 9);
    let context = report.port().handle().root_context();
    let expected_task = serde_json::to_value(context.token(CorrelationDomain::Task, "task/one"))
        .expect("task token");
    let expected_run =
        serde_json::to_value(context.token(CorrelationDomain::Run, "run/one")).expect("run token");
    let expected_span = serde_json::to_value(context.span_id()).expect("span");

    run_failure_entry(&executor, &context);
    assert_eq!(
        runtime.state(),
        (hiroute_domain::delegation::RunStateV1::Failed, 2),
        "the durable state is the authority for the terminal"
    );
    // The run is terminal now: the same entry point cannot record a second terminal.
    run_failure_entry(&executor, &context);
    assert_eq!(runtime.state().1, 2);

    report.shutdown();
    let log = std::fs::read_to_string(temporary.path().join("diagnostics/daemon/current.jsonl"))
        .expect("read the diagnostic log");
    let terminals = task_lifecycles(&log);
    assert_eq!(terminals.len(), 1, "exactly one terminal: {log}");
    let terminal = &terminals[0];
    assert_eq!(terminal["event"]["phase"], "end");
    assert_eq!(terminal["event"]["state"], "failed");
    assert_eq!(
        terminal["event"]["task"], expected_task,
        "the authoritative task token of this run"
    );
    assert_eq!(terminal["event"]["run"], expected_run);
    assert_eq!(
        terminal["span_id"], expected_span,
        "the terminal belongs to the same context as the run's other events"
    );
    assert_ne!(
        terminal["event"]["task"], terminal["event"]["run"],
        "the two domains never share one token"
    );
    assert!(
        !log.contains("run/one"),
        "the raw run id never reaches a record"
    );
}

/// R6: a failure whose checkpoint cannot be written, and a run that already left acceptance,
/// are both left unexplained in the log instead of being reported as launch failures.
#[test]
fn an_early_failure_that_cannot_be_recorded_claims_nothing() {
    let temporary = crate::test_support::private_tempdir();
    let (report, runtime, executor) = fixture(&temporary, 10);
    runtime.unrecordable();
    let context = report.port().handle().root_context();

    run_failure_entry(&executor, &context);
    assert_eq!(
        runtime.state(),
        (hiroute_domain::delegation::RunStateV1::Accepted, 1),
        "a failed checkpoint changes nothing"
    );

    // A run that already left acceptance is a conflict, not a launch failure.
    {
        let mut run = runtime.run.lock().unwrap();
        run.progress.advance(RunEventV1::Preparing).unwrap();
    }
    run_failure_entry(&executor, &context);
    assert_eq!(
        runtime.state(),
        (hiroute_domain::delegation::RunStateV1::Preparing, 2),
        "no checkpoint was written for the conflict"
    );

    report.shutdown();
    let log = std::fs::read_to_string(temporary.path().join("diagnostics/daemon/current.jsonl"))
        .expect("read the diagnostic log");
    assert!(
        task_lifecycles(&log).is_empty(),
        "no terminal is claimed for a failure that was not recorded: {log}"
    );
}

#[test]
fn cancel_before_executor_preparation_is_cancelled_without_launching() {
    let temporary = crate::test_support::private_tempdir();
    let (report, runtime, executor) = fixture(&temporary, 11);
    {
        let mut run = runtime.run.lock().unwrap();
        run.progress.advance(RunEventV1::CancelRequested).unwrap();
        run.lease_revoked = true;
    }
    let context = report.port().handle().root_context();
    run_failure_entry(&executor, &context);
    let run = runtime.run.lock().unwrap();
    assert_eq!(
        run.progress.state,
        hiroute_domain::delegation::RunStateV1::Cancelled
    );
    assert!(run.progress.workspace_releasable());
    assert!(run.process.is_none());
    assert!(!run.progress.prompt_may_have_executed);
    drop(run);
    run_failure_entry(&executor, &context);
    report.shutdown();
    let log =
        std::fs::read_to_string(temporary.path().join("diagnostics/daemon/current.jsonl")).unwrap();
    let terminals = task_lifecycles(&log);
    assert!(
        terminals.is_empty(),
        "the cancel transaction already settled the run"
    );
}
