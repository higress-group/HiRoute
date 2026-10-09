//! Durable lifecycle facts for one accepted Worker run.
//!
//! This journal is the bridge between the bounded ACP loop and the existing runtime/body
//! stores.  It never owns a process, grants authority, or derives a task identity from Worker
//! input.  Every state transition uses the run's current CAS revision, and output is streamed to
//! managed text rather than accumulated in the journal.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use hiroute_application::delegation::safety::{RunSafetyBinding, RunSafetyProjection};
use hiroute_diagnostics::context::DiagnosticContext;
use hiroute_diagnostics::correlation::CorrelationDomain;
use hiroute_diagnostics::event::{
    DiagnosticEvent, TaskLifecycle, TaskLifecyclePhase, TaskState, WorkerLifecycle,
    WorkerLifecyclePhase,
};
use hiroute_diagnostics::identity::CorrelationToken;
use hiroute_domain::CanonicalDigest;
use hiroute_domain::delegation::{
    DelegationBodyRefV1, DelegationCheckpointV1, DelegationErrorV1, DelegationProcessBindingV1,
    DelegationRunV1, DelegationRuntimePort, RunEventV1, RunStateV1, WorkerPermissionPolicyV1,
};
use hiroute_observation::LocalObservationStore;
use hiroute_observation::managed_text::{
    ManagedTextProgressTarget, ManagedTextPurpose, ManagedTextScope,
};

use super::acp::{AcpRunJournal, AcpRunOutcome, AcpSessionBinding};
use super::content::RunBodyWriter;
use super::credentials::RunCredentialVerifier;
use super::finalization::{DelegationFinalization, DelegationFinalizationLease};
use super::lifecycle::{WorkerProgressWriter, WorkerRunJournal};
use super::platform::WorkerProcessIdentity;
use super::progress::{ObservationProgressWriter, ProgressBatchWriter};

struct JournalState {
    run: DelegationRunV1,
    output: Option<RunBodyWriter>,
    output_incomplete: bool,
    completed: bool,
    failed: bool,
}

/// One journal is constructed only from an accepted run and its matching in-memory verifier.
/// The runtime port remains the durable authority; this object has no recovery replay path.
pub struct PersistentWorkerRunJournal {
    runtime: Arc<dyn DelegationRuntimePort + Send + Sync>,
    safety: Arc<RunSafetyProjection>,
    verifier: Arc<RunCredentialVerifier>,
    observation: Arc<LocalObservationStore>,
    permission_policy: WorkerPermissionPolicyV1,
    /// One context for the whole run: every lifecycle event of this run shares its span.
    context: DiagnosticContext,
    task_token: Option<CorrelationToken>,
    run_token: Option<CorrelationToken>,
    progress_writer: WorkerProgressWriter,
    finalization: Arc<DelegationFinalization>,
    state: Mutex<JournalState>,
}

impl PersistentWorkerRunJournal {
    pub(crate) fn new(
        runtime: Arc<dyn DelegationRuntimePort + Send + Sync>,
        safety: Arc<RunSafetyProjection>,
        verifier: Arc<RunCredentialVerifier>,
        observation: Arc<LocalObservationStore>,
        finalization: Arc<DelegationFinalization>,
        run: DelegationRunV1,
        context: DiagnosticContext,
    ) -> Result<Self, DelegationErrorV1> {
        if run.progress.state != RunStateV1::Accepted
            || run.lease_revoked
            || run.process.is_some()
            || run.session.is_some()
            || !verifier.protects_run(&run)
        {
            return Err(DelegationErrorV1::Conflict);
        }
        // The authoritative identifiers are tokenized here, where the run record is in
        // hand; the raw ids never reach a record.
        let task_token = context.token(CorrelationDomain::Task, &run.task_id);
        let run_token = context.token(CorrelationDomain::Run, &run.run_id);
        let created_at_ms = run
            .accepted_at_ms
            .ok_or(DelegationErrorV1::Conflict)
            .and_then(to_i64)?;
        let target = ManagedTextProgressTarget {
            scope: ManagedTextScope {
                workspace_id: run.workspace_id.clone(),
                task_id: run.task_id.clone(),
                run_id: run.run_id.clone(),
            },
            created_at_ms,
        };
        let writer: Arc<dyn ProgressBatchWriter> = Arc::new(ObservationProgressWriter::new(
            Arc::clone(&observation),
            target,
        ));
        let progress_writer = WorkerProgressWriter::new(writer);
        Ok(Self {
            runtime,
            safety,
            verifier,
            observation,
            permission_policy: run.configuration.permission_policy,
            context,
            task_token,
            run_token,
            progress_writer,
            finalization,
            state: Mutex::new(JournalState {
                run,
                output: None,
                output_incomplete: false,
                completed: false,
                failed: false,
            }),
        })
    }

    fn emit(&self, event: DiagnosticEvent) {
        self.context.emit(event);
    }

    /// The owned-process exit is a real platform observation; anything else stays out of the
    /// diagnostic log instead of being reported as a clean exit.
    fn record_worker_exit(
        &self,
        observation: &hiroute_domain::delegation::RunProcessObservationV1,
    ) {
        if let hiroute_domain::delegation::RunProcessObservationV1::Exited { code } = observation {
            self.emit(DiagnosticEvent::WorkerLifecycle(WorkerLifecycle {
                phase: WorkerLifecyclePhase::Exit,
                exit_code: *code,
                signal: None,
                task: self.task_token,
                run: self.run_token,
            }));
        }
    }

    /// Records the actual owned-process cleanup fact after `lifecycle::execute` returns.  A
    /// missing or contradictory stop proof stays explicitly unknown and never releases the run.
    pub fn record_stop(
        &self,
        stop: Option<hiroute_domain::delegation::RunStopEvidenceV1>,
    ) -> Result<(), DelegationErrorV1> {
        let mut state = self.lock_state()?;
        let event = match stop {
            Some(evidence) => DelegationCheckpointV1::ProcessStopped { evidence },
            None => DelegationCheckpointV1::ProcessObserved {
                observation: hiroute_domain::delegation::RunProcessObservationV1::Unknown,
            },
        };
        match self.checkpoint(&mut state, "stop", &event) {
            Ok(()) => {
                if let Some(evidence) = stop.as_ref() {
                    self.record_worker_exit(&evidence.observation);
                }
                Ok(())
            }
            Err(DelegationErrorV1::Conflict) => {
                // A formal Cancel advances the durable revision outside this journal before it
                // wakes the in-memory token.  That cancellation must not make the subsequent
                // owned-process stop proof permanently stale.  Refresh only the exact revoked
                // lease; this path cannot restore authority or accept post-cancel Worker output.
                self.refresh_after_external_cancel(&mut state)?;
                if state.run.progress.workspace_releasable() {
                    return Ok(());
                }
                self.checkpoint(&mut state, "stop", &event)
            }
            Err(error) => Err(error),
        }
    }

    pub fn run(&self) -> Result<DelegationRunV1, DelegationErrorV1> {
        Ok(self.lock_state()?.run.clone())
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, JournalState>, DelegationErrorV1> {
        self.state
            .lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)
    }

    fn check_current(&self, run: &DelegationRunV1) -> Result<(), DelegationErrorV1> {
        self.safety.check(&safety_binding(run), now_ms()?)?;
        let current = self
            .runtime
            .run(&run.workspace_id, &run.run_id)?
            .ok_or(DelegationErrorV1::Conflict)?;
        if !same_lease(&current, run) || current.progress.revision != run.progress.revision {
            return Err(DelegationErrorV1::Conflict);
        }
        if current.lease_revoked {
            return Err(DelegationErrorV1::PermissionDenied);
        }
        Ok(())
    }

    fn refresh_after_external_cancel(
        &self,
        state: &mut JournalState,
    ) -> Result<(), DelegationErrorV1> {
        let current = self
            .runtime
            .run(&state.run.workspace_id, &state.run.run_id)?
            .ok_or(DelegationErrorV1::Conflict)?;
        if !same_lease(&current, &state.run)
            || current.process != state.run.process
            || current.session != state.run.session
            || current.progress.revision <= state.run.progress.revision
            || !current.lease_revoked
            || !current.progress.cancel_requested
        {
            return Err(DelegationErrorV1::Conflict);
        }
        state.run = current;
        Ok(())
    }

    fn checkpoint(
        &self,
        state: &mut JournalState,
        stage: &str,
        event: &DelegationCheckpointV1,
    ) -> Result<(), DelegationErrorV1> {
        let event_id = event_id(&state.run, stage)?;
        let next = self.runtime.checkpoint(
            &state.run.workspace_id,
            &state.run.run_id,
            state.run.progress.revision,
            &event_id,
            event,
        )?;
        if !same_lease(&next, &state.run) {
            return Err(DelegationErrorV1::Conflict);
        }
        state.run = next;
        Ok(())
    }

    fn create_output_writer(&self, state: &mut JournalState) {
        let now = match now_ms().and_then(to_i64) {
            Ok(now) => now,
            Err(_) => {
                state.output_incomplete = true;
                return;
            }
        };
        let scope = ManagedTextScope {
            workspace_id: state.run.workspace_id.clone(),
            task_id: state.run.task_id.clone(),
            run_id: state.run.run_id.clone(),
        };
        let writer = RunBodyWriter::create(
            Arc::clone(&self.observation),
            scope,
            ManagedTextPurpose::Result,
            format!("delegation-result/{}", state.run.run_id),
            now,
            now,
        );
        match writer {
            Ok(writer) => state.output = Some(writer),
            Err(_) => state.output_incomplete = true,
        }
    }

    fn record_completed(
        &self,
        state: &mut JournalState,
        outcome: &AcpRunOutcome,
    ) -> Result<(), DelegationErrorV1> {
        if state.completed {
            return Ok(());
        }
        if !matches!(
            state.run.progress.state,
            RunStateV1::Running | RunStateV1::Cancelling
        ) {
            return Err(DelegationErrorV1::Conflict);
        }
        let now = to_i64(now_ms()?)?;
        let mut incomplete = state.output_incomplete || outcome.content_incomplete;
        let body = match state.output.take() {
            Some(writer) => match writer.finish(now) {
                Ok(reference) => Some(DelegationBodyRefV1 {
                    opaque_id: reference.opaque_id,
                    scope_run_id: state.run.run_id.clone(),
                    visibility_generation: reference.visibility_generation,
                    original_retention_deadline_ms: reference.original_retention_deadline_ms,
                }),
                Err(_) => {
                    incomplete = true;
                    None
                }
            },
            None => {
                incomplete = true;
                None
            }
        };
        self.checkpoint(
            state,
            "result",
            &DelegationCheckpointV1::ResultRecorded { body, incomplete },
        )?;
        state.completed = true;
        if outcome.stop_reason == "cancelled" || state.run.progress.state == RunStateV1::Cancelling
        {
            self.emit(DiagnosticEvent::TaskLifecycle(TaskLifecycle {
                phase: TaskLifecyclePhase::End,
                state: TaskState::Cancelled,
                task: self.task_token,
                run: self.run_token,
            }));
            return Ok(());
        }
        self.checkpoint(
            state,
            "prompt-completed",
            &DelegationCheckpointV1::Progress {
                event: RunEventV1::PromptCompleted,
            },
        )?;
        self.emit(DiagnosticEvent::TaskLifecycle(TaskLifecycle {
            phase: TaskLifecyclePhase::End,
            state: TaskState::Completed,
            task: self.task_token,
            run: self.run_token,
        }));
        Ok(())
    }

    fn record_failure(
        &self,
        state: &mut JournalState,
        error: DelegationErrorV1,
    ) -> Result<(), DelegationErrorV1> {
        if state.failed || state.completed {
            return Ok(());
        }
        let (stage, event) = if error == DelegationErrorV1::PromptFailed {
            (
                "prompt-failed",
                DelegationCheckpointV1::Progress {
                    event: RunEventV1::PromptFailed,
                },
            )
        } else if state.run.process.is_some() {
            (
                "execution-failed",
                DelegationCheckpointV1::Progress {
                    event: RunEventV1::ConnectionLost,
                },
            )
        } else {
            (
                "execution-failed",
                DelegationCheckpointV1::Progress {
                    event: RunEventV1::LaunchFailedBeforeSpawn,
                },
            )
        };
        self.checkpoint(state, stage, &event)?;
        state.failed = true;
        if state.run.process.is_none()
            && state.run.progress.state == RunStateV1::Failed
            && let DelegationErrorV1::DependencyCheckFailed(failure) = error
            && let Ok(now) = now_ms().and_then(to_i64)
        {
            self.progress_writer.dependency_failure(failure, now);
        }
        // The end state is the product run state this checkpoint actually produced; a
        // cancel-after-prompt becomes Unknown rather than being reported as failed.
        let end_state = task_state(state.run.progress.state);
        self.emit(DiagnosticEvent::TaskLifecycle(TaskLifecycle {
            phase: TaskLifecyclePhase::End,
            state: end_state,
            task: self.task_token,
            run: self.run_token,
        }));
        Ok(())
    }
}

impl AcpRunJournal for PersistentWorkerRunJournal {
    fn session_bound(&self, binding: &AcpSessionBinding) -> Result<(), DelegationErrorV1> {
        let mut state = self.lock_state()?;
        self.check_current(&state.run)?;
        self.checkpoint(
            &mut state,
            "session-bound",
            &DelegationCheckpointV1::SessionBound {
                binding: binding.clone(),
            },
        )
    }

    fn before_prompt(&self) -> Result<(), DelegationErrorV1> {
        let mut state = self.lock_state()?;
        self.check_current(&state.run)?;
        self.checkpoint(
            &mut state,
            "prompt-intent",
            &DelegationCheckpointV1::Progress {
                event: RunEventV1::PromptSendIntent,
            },
        )?;
        self.create_output_writer(&mut state);
        Ok(())
    }

    fn text_update(&self, text: &str) -> Result<(), DelegationErrorV1> {
        let mut state = self.lock_state()?;
        let result = match state.output.as_mut() {
            Some(writer) => writer.append(text.as_bytes(), to_i64(now_ms()?)?),
            None => Err(DelegationErrorV1::ContentUnavailable),
        };
        if result.is_err() {
            state.output = None;
            state.output_incomplete = true;
        }
        result
    }

    fn allow_permission_once(&self, request: &serde_json::Value) -> bool {
        let Ok(state) = self.lock_state() else {
            return false;
        };
        if self.check_current(&state.run).is_err()
            || state.run.progress.state != RunStateV1::Running
        {
            return false;
        }
        match self.permission_policy {
            WorkerPermissionPolicyV1::ApproveAll => true,
            WorkerPermissionPolicyV1::ApproveReads => {
                request
                    .pointer("/toolCall/kind")
                    .and_then(serde_json::Value::as_str)
                    == Some("read")
            }
            WorkerPermissionPolicyV1::DenyAll => false,
        }
    }
}

impl WorkerRunJournal for PersistentWorkerRunJournal {
    fn progress_writer(&self) -> Option<WorkerProgressWriter> {
        Some(self.progress_writer.clone())
    }

    fn begin_finalization(&self) -> Option<DelegationFinalizationLease> {
        Some(self.finalization.acquire())
    }

    fn before_launch(&self) -> Result<(), DelegationErrorV1> {
        let mut state = self.lock_state()?;
        self.check_current(&state.run)?;
        if state.run.progress.state != RunStateV1::Accepted {
            return Err(DelegationErrorV1::Conflict);
        }
        self.checkpoint(
            &mut state,
            "preparing",
            &DelegationCheckpointV1::Progress {
                event: RunEventV1::Preparing,
            },
        )
    }

    fn process_spawned(&self, identity: &WorkerProcessIdentity) -> Result<(), DelegationErrorV1> {
        let mut state = self.lock_state()?;
        self.check_current(&state.run)?;
        if identity.launch_nonce != state.run.launch_nonce {
            return Err(DelegationErrorV1::Conflict);
        }
        self.checkpoint(
            &mut state,
            "process-spawned",
            &DelegationCheckpointV1::ProcessSpawned {
                binding: DelegationProcessBindingV1 {
                    launch_nonce: identity.launch_nonce.clone(),
                    handle_id: identity.handle_id.clone(),
                    creation_identity: identity.creation_identity.clone(),
                },
            },
        )?;
        self.emit(DiagnosticEvent::WorkerLifecycle(WorkerLifecycle {
            phase: WorkerLifecyclePhase::Spawn,
            exit_code: None,
            signal: None,
            task: self.task_token,
            run: self.run_token,
        }));
        Ok(())
    }

    fn stop_intent(&self) -> Result<(), DelegationErrorV1> {
        self.verifier.revoke();
        let mut state = self.lock_state()?;
        if matches!(
            state.run.progress.state,
            RunStateV1::Accepted
                | RunStateV1::Preparing
                | RunStateV1::Running
                | RunStateV1::Unknown
        ) {
            let task_state = task_state(state.run.progress.state);
            self.checkpoint(
                &mut state,
                "stop-intent",
                &DelegationCheckpointV1::Progress {
                    event: RunEventV1::CancelRequested,
                },
            )?;
            self.emit(DiagnosticEvent::TaskLifecycle(TaskLifecycle {
                phase: TaskLifecyclePhase::Cancel,
                state: task_state,
                task: self.task_token,
                run: self.run_token,
            }));
        }
        Ok(())
    }

    fn completed(&self, outcome: &AcpRunOutcome) -> Result<(), DelegationErrorV1> {
        let mut state = self.lock_state()?;
        self.record_completed(&mut state, outcome)
    }

    fn execution_failed(&self, error: DelegationErrorV1) -> Result<(), DelegationErrorV1> {
        let mut state = self.lock_state()?;
        match self.record_failure(&mut state, error) {
            Err(DelegationErrorV1::Conflict) => {
                self.refresh_after_external_cancel(&mut state)?;
                if state.run.progress.workspace_releasable() {
                    return Ok(());
                }
                self.record_failure(&mut state, error)
            }
            result => result,
        }
    }

    fn spawned_unrecorded(&self) -> Result<(), DelegationErrorV1> {
        let mut state = self.lock_state()?;
        if state.run.process.is_some() {
            return Err(DelegationErrorV1::Conflict);
        }
        let event = DelegationCheckpointV1::ProcessObserved {
            observation: hiroute_domain::delegation::RunProcessObservationV1::Unknown,
        };
        match self.checkpoint(&mut state, "spawned-unrecorded", &event) {
            Err(DelegationErrorV1::Conflict) => {
                self.refresh_after_external_cancel(&mut state)?;
                self.checkpoint(&mut state, "spawned-unrecorded", &event)?;
            }
            result => result?,
        }
        state.failed = true;
        Ok(())
    }
}

/// Mirrors the product run state onto the diagnostic task state; no new state is invented.
pub(super) fn task_state(state: RunStateV1) -> TaskState {
    match state {
        RunStateV1::Succeeded => TaskState::Completed,
        RunStateV1::Failed => TaskState::Failed,
        RunStateV1::Cancelled => TaskState::Cancelled,
        RunStateV1::Unknown => TaskState::Unknown,
        RunStateV1::Accepted
        | RunStateV1::Preparing
        | RunStateV1::Running
        | RunStateV1::Cancelling => TaskState::Running,
    }
}

fn same_lease(left: &DelegationRunV1, right: &DelegationRunV1) -> bool {
    left.workspace_id == right.workspace_id
        && left.task_id == right.task_id
        && left.run_id == right.run_id
        && left.lease_id == right.lease_id
        && left.daemon_epoch == right.daemon_epoch
        && left.launch_nonce == right.launch_nonce
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

fn event_id(run: &DelegationRunV1, stage: &str) -> Result<String, DelegationErrorV1> {
    let digest = CanonicalDigest::of(&(
        "hiroute.delegation-persistent-journal/v1",
        &run.workspace_id,
        &run.run_id,
        &run.lease_id,
        stage,
    ))
    .map_err(|_| DelegationErrorV1::InvalidArguments)?;
    Ok(format!("delegation-journal/{stage}/{digest}"))
}

fn now_ms() -> Result<u64, DelegationErrorV1> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DelegationErrorV1::DeadlineExceeded)?
        .as_millis();
    u64::try_from(millis).map_err(|_| DelegationErrorV1::DeadlineExceeded)
}

fn to_i64(value: u64) -> Result<i64, DelegationErrorV1> {
    i64::try_from(value).map_err(|_| DelegationErrorV1::DeadlineExceeded)
}

#[cfg(test)]
#[path = "persistent_journal_tests.rs"]
mod tests;
