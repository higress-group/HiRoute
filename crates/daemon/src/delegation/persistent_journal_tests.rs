use super::*;
use hiroute_application::publication::admission::SharedAdmissionGate;
use hiroute_domain::delegation::{
    DELEGATION_RUN_CONFIGURATION_VERSION_V1, DelegationAcceptanceV1, DelegationCancelReceiptV1,
    DelegationRunConfigurationV1, DelegationSessionBindingV1, DelegationTaskV1, RunCleanupV1,
    RunProcessObservationV1, RunStopEvidenceV1, RunStopScopeV1, WorkerExecutionIntentV1,
    WorkerNetworkV1, WorkerPermissionPolicyV1, WorkerToolV1, WorkspaceAccessV1,
};
use hiroute_domain::{AgentIngressProtocolV1, OperationId, WorkspaceId};
use hiroute_observation::DigestAuthority;
use hiroute_observation::managed_text::{ManagedTextRef, ManagedTextState, PAGE_BYTES};

use crate::delegation::content::read_required_body;
use crate::delegation::credentials::{
    RunCredentialAudience, RunCredentialFingerprints, RunCredentialPair, RunCredentialRecord,
};
use crate::delegation::finalization::DelegationFinalization;

struct MemoryRuntime(Mutex<DelegationRunV1>);

impl MemoryRuntime {
    fn new(run: DelegationRunV1) -> Self {
        Self(Mutex::new(run))
    }

    fn cancel_outside_journal(&self) {
        let mut run = self.0.lock().unwrap();
        run.progress.advance(RunEventV1::CancelRequested).unwrap();
        run.lease_revoked = true;
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
            .0
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
        let mut run = self
            .0
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
            DelegationCheckpointV1::ProcessSpawned { binding } => {
                run.process = Some(binding.clone());
                run.progress.advance(RunEventV1::ProcessRunning)?;
            }
            DelegationCheckpointV1::SessionBound { binding } => {
                run.session = Some(binding.clone());
            }
            DelegationCheckpointV1::ResultRecorded { body, incomplete } => {
                run.result_body = body.clone();
                run.result_incomplete = *incomplete;
            }
            DelegationCheckpointV1::ProcessStopped { evidence } => {
                run.stop_evidence = Some(*evidence);
                run.progress.advance(RunEventV1::ManagedScopeStopped {
                    success: evidence.scope_stopped,
                })?;
            }
            DelegationCheckpointV1::ProcessObserved { observation } => {
                run.progress.advance(match observation {
                    RunProcessObservationV1::Running => RunEventV1::ProcessRunning,
                    RunProcessObservationV1::Exited { code } => RunEventV1::RootProcessExited {
                        success: *code == Some(0),
                    },
                    RunProcessObservationV1::Unknown => RunEventV1::ProcessUnknown,
                })?;
            }
            DelegationCheckpointV1::ResidualConfirmed { .. } => {
                return Err(DelegationErrorV1::PermissionDenied);
            }
        }
        if run.progress.revision == expected_revision {
            run.progress.revision = expected_revision
                .checked_add(1)
                .ok_or(DelegationErrorV1::Conflict)?;
        }
        Ok(run.clone())
    }

    fn request_cancel(
        &self,
        _: &WorkspaceId,
        _: &str,
        _: &OperationId,
        _: &str,
    ) -> Result<DelegationCancelReceiptV1, DelegationErrorV1> {
        Err(DelegationErrorV1::Conflict)
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
        Err(DelegationErrorV1::Conflict)
    }
}

fn run(deadline_ms: u64) -> DelegationRunV1 {
    DelegationRunV1 {
        workspace_id: WorkspaceId::default(),
        task_id: "task/one".into(),
        run_id: "run/one".into(),
        ordinal: 1,
        continued_from: None,
        idempotency_key: "start/one".into(),
        request_digest: CanonicalDigest::of_bytes(b"request"),
        admission_sequence: 1,
        accepted_at_ms: Some(deadline_ms.saturating_sub(30_000)),
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
        deadline_ms,
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

#[test]
fn persistent_journal_streams_result_before_revoking_the_exact_run_credential() {
    let deadline_ms = now_ms().unwrap() + 30_000;
    let run = run(deadline_ms);
    let pair = RunCredentialPair::generate().unwrap();
    let safety = Arc::new(
        RunSafetyProjection::new(Arc::new(SharedAdmissionGate::new()), "epoch".into()).unwrap(),
    );
    safety.finish_startup_recovery();
    let verifier = Arc::new(
        RunCredentialVerifier::new(
            RunCredentialRecord {
                task_id: run.task_id.clone(),
                run_id: run.run_id.clone(),
                lease_id: run.lease_id.clone(),
                safety: safety_binding(&run),
                model_alias: "worker-model".into(),
                protocol: AgentIngressProtocolV1::Responses,
                fingerprints: RunCredentialFingerprints {
                    model: pair.fingerprints().model,
                    self_query: pair.fingerprints().self_query,
                },
            },
            Arc::clone(&safety),
        )
        .unwrap(),
    );
    let temporary = tempfile::tempdir().unwrap();
    let observation = Arc::new(
        LocalObservationStore::open(temporary.path(), DigestAuthority::new([6; 32])).unwrap(),
    );
    let runtime = Arc::new(MemoryRuntime::new(run.clone()));
    let journal = PersistentWorkerRunJournal::new(
        runtime,
        safety,
        Arc::clone(&verifier),
        Arc::clone(&observation),
        Arc::new(DelegationFinalization::default()),
        run,
        hiroute_diagnostics::runtime::DiagnosticsPort::default()
            .handle()
            .root_context(),
    )
    .unwrap();

    journal.before_launch().unwrap();
    journal
        .process_spawned(&WorkerProcessIdentity {
            launch_nonce: "launch/one".into(),
            handle_id: "held/one".into(),
            creation_identity: "created/one".into(),
        })
        .unwrap();
    let session = DelegationSessionBindingV1 {
        acp_session_id: "session/one".into(),
        native_session_id: Some("native/one".into()),
    };
    journal.session_bound(&session).unwrap();
    journal.before_prompt().unwrap();
    assert!(journal.allow_permission_once(&serde_json::json!({
        "toolCall": {"kind": "read"}
    })));
    journal.text_update("first ").unwrap();
    journal.text_update("answer").unwrap();
    journal
        .completed(&AcpRunOutcome {
            session,
            stop_reason: "end_turn".into(),
            text: "first answer".into(),
            content_incomplete: false,
        })
        .unwrap();
    let completed = journal.run().unwrap();
    assert_eq!(completed.progress.state, RunStateV1::Succeeded);
    assert!(!completed.result_incomplete);
    let body = completed.result_body.unwrap();
    assert_eq!(body.visibility_generation, 0);
    body.validate().unwrap();
    let scope = ManagedTextScope {
        workspace_id: completed.workspace_id.clone(),
        task_id: completed.task_id.clone(),
        run_id: completed.run_id.clone(),
    };
    let reference = ManagedTextRef {
        opaque_id: body.opaque_id,
        scope: scope.clone(),
        visibility_generation: body.visibility_generation,
        original_retention_deadline_ms: body.original_retention_deadline_ms,
        state: ManagedTextState::Complete,
    };
    assert_eq!(
        read_required_body(
            &observation,
            &scope,
            &reference,
            to_i64(now_ms().unwrap()).unwrap(),
            PAGE_BYTES,
            |_| Ok(()),
        )
        .unwrap(),
        b"first answer"
    );
    journal.stop_intent().unwrap();
    assert!(
        verifier
            .authenticate(
                pair.model.expose(),
                RunCredentialAudience::Model,
                now_ms().unwrap()
            )
            .is_err()
    );
    assert_ne!(completed.progress.cleanup, RunCleanupV1::Complete);
    journal
        .record_stop(Some(RunStopEvidenceV1 {
            scope: RunStopScopeV1::ProcessGroup,
            observation: RunProcessObservationV1::Exited { code: Some(0) },
            scope_stopped: true,
            residual_unknown: false,
        }))
        .unwrap();
    assert_eq!(
        journal.run().unwrap().progress.cleanup,
        RunCleanupV1::Complete
    );
}

fn cancellation_fixture() -> (
    tempfile::TempDir,
    Arc<MemoryRuntime>,
    PersistentWorkerRunJournal,
) {
    let deadline_ms = now_ms().unwrap() + 30_000;
    let run = run(deadline_ms);
    let pair = RunCredentialPair::generate().unwrap();
    let safety = Arc::new(
        RunSafetyProjection::new(Arc::new(SharedAdmissionGate::new()), "epoch".into()).unwrap(),
    );
    safety.finish_startup_recovery();
    let verifier = Arc::new(
        RunCredentialVerifier::new(
            RunCredentialRecord {
                task_id: run.task_id.clone(),
                run_id: run.run_id.clone(),
                lease_id: run.lease_id.clone(),
                safety: safety_binding(&run),
                model_alias: "worker-model".into(),
                protocol: AgentIngressProtocolV1::Responses,
                fingerprints: RunCredentialFingerprints {
                    model: pair.fingerprints().model,
                    self_query: pair.fingerprints().self_query,
                },
            },
            Arc::clone(&safety),
        )
        .unwrap(),
    );
    let temporary = tempfile::tempdir().unwrap();
    let observation = Arc::new(
        LocalObservationStore::open(temporary.path(), DigestAuthority::new([7; 32])).unwrap(),
    );
    let runtime = Arc::new(MemoryRuntime::new(run.clone()));
    let journal = PersistentWorkerRunJournal::new(
        runtime.clone(),
        safety,
        verifier,
        observation,
        Arc::new(DelegationFinalization::default()),
        run,
        hiroute_diagnostics::runtime::DiagnosticsPort::default()
            .handle()
            .root_context(),
    )
    .unwrap();

    (temporary, runtime, journal)
}

#[test]
fn prelaunch_dependency_failure_keeps_host_reason_in_progress_after_reopen() {
    use hiroute_domain::delegation::{
        NativeDependencyCheckV1, NativeDependencyFailureReasonV1, NativeDependencyFailureV1,
    };
    use hiroute_observation::managed_text::{ManagedTextProgressRead, ManagedTextProgressTarget};
    let (temporary, _runtime, journal) = cancellation_fixture();
    let failure = NativeDependencyFailureV1 {
        check: NativeDependencyCheckV1::ClaudeVersion,
        reason: NativeDependencyFailureReasonV1::Timeout,
    };
    journal
        .execution_failed(DelegationErrorV1::DependencyCheckFailed(failure))
        .unwrap();
    journal
        .execution_failed(DelegationErrorV1::DependencyCheckFailed(failure))
        .unwrap();
    let run = journal.run().unwrap();
    assert_eq!(run.progress.state, RunStateV1::Failed);
    assert!(run.progress.workspace_releasable());
    assert!(run.process.is_none());
    assert!(run.result_body.is_none());
    let target = ManagedTextProgressTarget {
        scope: ManagedTextScope {
            workspace_id: run.workspace_id,
            task_id: run.task_id,
            run_id: run.run_id,
        },
        created_at_ms: run.accepted_at_ms.unwrap() as i64,
    };
    let reopened =
        LocalObservationStore::open(temporary.path(), DigestAuthority::new([7; 32])).unwrap();
    let ManagedTextProgressRead::Available(page) = reopened
        .managed_text_progress_read(&target, None, 4096)
        .unwrap()
    else {
        panic!("host failure must be available through ordinary Worker progress");
    };
    assert_eq!(page.text.matches("[HiRoute]").count(), 1);
    assert!(page.text.contains("claude_version: timeout"));
    assert!(page.text.contains("Retry the same dependency selection"));
}

#[test]
fn cancelled_preparation_refreshes_without_restoring_launch_authority() {
    let (_temporary, runtime, journal) = cancellation_fixture();
    runtime.cancel_outside_journal();
    assert!(journal.before_launch().is_err());
    journal
        .execution_failed(DelegationErrorV1::Conflict)
        .unwrap();
    let cancelled = journal.run().unwrap();
    assert_eq!(cancelled.progress.state, RunStateV1::Cancelled);
    assert!(cancelled.progress.workspace_releasable());
    assert!(cancelled.lease_revoked);
    assert!(cancelled.process.is_none());
}

#[test]
fn cancellation_racing_unrecorded_spawn_retains_unknown_cleanup() {
    let (_temporary, runtime, journal) = cancellation_fixture();
    journal.before_launch().unwrap();
    runtime.cancel_outside_journal();
    journal.spawned_unrecorded().unwrap();
    journal
        .execution_failed(DelegationErrorV1::Conflict)
        .unwrap();
    let run = journal.run().unwrap();
    assert_eq!(run.progress.cleanup, RunCleanupV1::Unknown);
    assert!(!run.progress.workspace_releasable());
    assert!(run.lease_revoked);
}

#[test]
fn stop_evidence_reconciles_the_exact_lease_after_formal_cancel_advances_revision() {
    let (_temporary, runtime, journal) = cancellation_fixture();

    journal.before_launch().unwrap();
    journal
        .process_spawned(&WorkerProcessIdentity {
            launch_nonce: "launch/one".into(),
            handle_id: "held/one".into(),
            creation_identity: "created/one".into(),
        })
        .unwrap();
    let session = DelegationSessionBindingV1 {
        acp_session_id: "session/one".into(),
        native_session_id: Some("native/one".into()),
    };
    journal.session_bound(&session).unwrap();
    journal.before_prompt().unwrap();

    runtime.cancel_outside_journal();
    journal
        .record_stop(Some(RunStopEvidenceV1 {
            scope: RunStopScopeV1::ProcessGroup,
            observation: RunProcessObservationV1::Exited { code: None },
            scope_stopped: true,
            residual_unknown: false,
        }))
        .unwrap();

    let reconciled = journal.run().unwrap();
    assert!(reconciled.lease_revoked);
    assert_eq!(reconciled.progress.state, RunStateV1::Cancelled);
    assert_eq!(reconciled.progress.cleanup, RunCleanupV1::Complete);
    assert!(!reconciled.progress.process_running);
}

/// The worker and task lifecycle events follow the real journal branches; the end state is the
/// product run state the checkpoint produced, not a diagnostic-specific guess.
#[test]
fn worker_and_task_lifecycle_follow_the_real_journal_branches() {
    use hiroute_diagnostics::event::ProcessRole;
    use hiroute_diagnostics::level::DiagnosticLevel;
    use hiroute_diagnostics::record::Component;
    use hiroute_diagnostics::runtime::{DiagnosticRuntime, RuntimeConfig};

    let deadline_ms = now_ms().unwrap() + 30_000;
    let run = run(deadline_ms);
    let pair = RunCredentialPair::generate().unwrap();
    let safety = Arc::new(
        RunSafetyProjection::new(Arc::new(SharedAdmissionGate::new()), "epoch".into()).unwrap(),
    );
    safety.finish_startup_recovery();
    let verifier = Arc::new(
        RunCredentialVerifier::new(
            RunCredentialRecord {
                task_id: run.task_id.clone(),
                run_id: run.run_id.clone(),
                lease_id: run.lease_id.clone(),
                safety: safety_binding(&run),
                model_alias: "worker-model".into(),
                protocol: AgentIngressProtocolV1::Responses,
                fingerprints: RunCredentialFingerprints {
                    model: pair.fingerprints().model,
                    self_query: pair.fingerprints().self_query,
                },
            },
            Arc::clone(&safety),
        )
        .unwrap(),
    );
    let temporary = crate::test_support::private_tempdir();
    let observation = Arc::new(
        LocalObservationStore::open(temporary.path(), DigestAuthority::new([8; 32])).unwrap(),
    );
    let diagnostics_root = temporary.path().join("diagnostics");
    let diagnostics = DiagnosticRuntime::start(RuntimeConfig {
        root: diagnostics_root.clone(),
        role: ProcessRole::Daemon,
        component: Component::Worker,
        parent_session_id: None,
        level_override: Some(DiagnosticLevel::Info),
    });
    let journal = PersistentWorkerRunJournal::new(
        Arc::new(MemoryRuntime::new(run.clone())),
        safety,
        verifier,
        observation,
        Arc::new(DelegationFinalization::default()),
        run,
        diagnostics.handle().root_context(),
    )
    .unwrap();

    journal.before_launch().unwrap();
    journal
        .process_spawned(&WorkerProcessIdentity {
            launch_nonce: "launch/one".into(),
            handle_id: "held/one".into(),
            creation_identity: "created/one".into(),
        })
        .unwrap();
    journal
        .session_bound(&DelegationSessionBindingV1 {
            acp_session_id: "session/one".into(),
            native_session_id: Some("native/one".into()),
        })
        .unwrap();
    journal.before_prompt().unwrap();
    journal.stop_intent().unwrap();
    journal
        .execution_failed(DelegationErrorV1::Cancelled)
        .unwrap();
    journal
        .record_stop(Some(RunStopEvidenceV1 {
            scope: RunStopScopeV1::ProcessGroup,
            observation: RunProcessObservationV1::Exited { code: Some(137) },
            scope_stopped: true,
            residual_unknown: false,
        }))
        .unwrap();
    diagnostics.shutdown();

    let log =
        std::fs::read_to_string(diagnostics_root.join("daemon").join("current.jsonl")).unwrap();
    assert!(log.contains("\"worker_lifecycle\""), "{log}");
    assert!(log.contains("\"phase\":\"spawn\""), "{log}");
    assert!(log.contains("\"phase\":\"exit\""), "{log}");
    assert!(log.contains("\"exit_code\":137"), "{log}");
    assert!(log.contains("\"task_lifecycle\""), "{log}");
    assert!(log.contains("\"phase\":\"cancel\""), "{log}");
    assert!(log.contains("\"phase\":\"end\""), "{log}");
    // Cancellation after a sent prompt leaves the product run explicitly unknown.
    assert!(log.contains("\"state\":\"unknown\""), "{log}");
}

/// S1: two interleaved runs are reconstructable from their own tokens and spans without any
/// time-based guesswork, and the raw task/run identifiers never reach the log.
#[test]
fn interleaved_runs_are_reconstructable_from_their_own_tokens() {
    use hiroute_diagnostics::event::{DiagnosticEvent, ProcessRole};
    use hiroute_diagnostics::level::DiagnosticLevel;
    use hiroute_diagnostics::record::{Component, DiagnosticRecordV1};
    use hiroute_diagnostics::runtime::{DiagnosticRuntime, RuntimeConfig};

    let temporary = crate::test_support::private_tempdir();
    let diagnostics_root = temporary.path().join("diagnostics");
    let diagnostics = DiagnosticRuntime::start(RuntimeConfig {
        root: diagnostics_root.clone(),
        role: ProcessRole::Daemon,
        component: Component::Worker,
        parent_session_id: None,
        level_override: Some(DiagnosticLevel::Debug),
    });
    let deadline_ms = now_ms().unwrap() + 30_000;

    let build = |task: &str, run_id: &str, marker: char| {
        let mut record = run(deadline_ms);
        record.task_id = task.to_owned();
        record.run_id = run_id.to_owned();
        record.lease_id = format!("lease/{run_id}");
        let pair = RunCredentialPair::generate().unwrap();
        let safety = Arc::new(
            RunSafetyProjection::new(Arc::new(SharedAdmissionGate::new()), "epoch".into()).unwrap(),
        );
        safety.finish_startup_recovery();
        let verifier = Arc::new(
            RunCredentialVerifier::new(
                RunCredentialRecord {
                    task_id: record.task_id.clone(),
                    run_id: record.run_id.clone(),
                    lease_id: record.lease_id.clone(),
                    safety: safety_binding(&record),
                    model_alias: "worker-model".into(),
                    protocol: AgentIngressProtocolV1::Responses,
                    fingerprints: RunCredentialFingerprints {
                        model: pair.fingerprints().model,
                        self_query: pair.fingerprints().self_query,
                    },
                },
                Arc::clone(&safety),
            )
            .unwrap(),
        );
        let observation = Arc::new(
            LocalObservationStore::open(temporary.path(), DigestAuthority::new([marker as u8; 32]))
                .unwrap(),
        );
        PersistentWorkerRunJournal::new(
            Arc::new(MemoryRuntime::new(record.clone())),
            safety,
            verifier,
            observation,
            Arc::new(DelegationFinalization::default()),
            record,
            diagnostics.handle().root_context(),
        )
        .unwrap()
    };
    let alpha = build("task/alpha", "run/alpha", 'a');
    let beta = build("task/beta", "run/beta", 'b');

    // The two runs interleave in every phase this journal owns.
    for (index, journal) in [&alpha, &beta].into_iter().enumerate() {
        journal.before_launch().unwrap();
        let _ = index;
    }
    for (journal, suffix) in [(&alpha, "alpha"), (&beta, "beta")] {
        journal
            .process_spawned(&WorkerProcessIdentity {
                launch_nonce: "launch/one".into(),
                handle_id: format!("held/{suffix}"),
                creation_identity: format!("created/{suffix}"),
            })
            .unwrap();
    }
    for journal in [&alpha, &beta] {
        journal
            .session_bound(&DelegationSessionBindingV1 {
                acp_session_id: format!("session/{}", journal.run().unwrap().run_id),
                native_session_id: Some("native/one".into()),
            })
            .unwrap();
        journal.before_prompt().unwrap();
    }
    beta.stop_intent().unwrap();
    alpha.stop_intent().unwrap();
    alpha
        .execution_failed(DelegationErrorV1::Cancelled)
        .unwrap();
    beta.execution_failed(DelegationErrorV1::Cancelled).unwrap();
    diagnostics.shutdown();

    let log =
        std::fs::read_to_string(diagnostics_root.join("daemon").join("current.jsonl")).unwrap();
    for raw in ["task/alpha", "run/alpha", "task/beta", "run/beta"] {
        assert!(!log.contains(raw), "raw identifier leaked: {raw}");
    }

    let mut runs: Vec<(
        Option<hiroute_diagnostics::identity::CorrelationToken>,
        Option<CorrelationToken>,
        Option<hiroute_diagnostics::identity::SpanId>,
    )> = Vec::new();
    for line in log.lines() {
        let record = DiagnosticRecordV1::parse_line(line.as_bytes()).expect("record");
        let tokens = match &record.event {
            DiagnosticEvent::WorkerLifecycle(event) => (event.task, event.run),
            DiagnosticEvent::TaskLifecycle(event) => (event.task, event.run),
            _ => continue,
        };
        assert!(
            tokens.0.is_some() && tokens.1.is_some(),
            "a lifecycle event must carry both tokens: {line}"
        );
        runs.push((tokens.0, tokens.1, record.span_id));
    }
    assert_eq!(
        runs.len(),
        6,
        "two runs produce three lifecycle events each"
    );

    let alpha_tokens = &runs[0];
    let beta_tokens = runs
        .iter()
        .find(|(task, _, _)| *task != alpha_tokens.0)
        .expect("the second run carries its own task token");
    assert_ne!(alpha_tokens.1, beta_tokens.1, "run tokens differ");
    for (task, run_token, span) in &runs {
        if *task == alpha_tokens.0 {
            assert_eq!(run_token, &alpha_tokens.1, "one token per run");
            assert_eq!(span, &alpha_tokens.2, "all events of a run share one span");
        } else {
            assert_eq!(task, &beta_tokens.0);
            assert_eq!(run_token, &beta_tokens.1);
            assert_eq!(span, &beta_tokens.2);
        }
    }
    assert_ne!(
        alpha_tokens.2, beta_tokens.2,
        "interleaved runs never share a span"
    );
}
