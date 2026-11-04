use super::*;

fn running(store: &RuntimeStore, input: &DelegationAcceptanceV1) -> DelegationRunV1 {
    let mut run = store.accept(input).unwrap();
    commit_ready_root(store, &run.workspace_id, &run.task_id);
    run = advance(store, &run, RunEventV1::Preparing);
    for (id, event) in [
        (
            "spawn",
            DelegationCheckpointV1::ProcessSpawned {
                binding: DelegationProcessBindingV1 {
                    launch_nonce: run.launch_nonce.clone(),
                    handle_id: "held-worker".into(),
                    creation_identity: "owned-worker".into(),
                },
            },
        ),
        (
            "session",
            DelegationCheckpointV1::SessionBound {
                binding: DelegationSessionBindingV1 {
                    acp_session_id: "exact-session".into(),
                    native_session_id: Some("exact-session".into()),
                },
            },
        ),
    ] {
        run = store
            .checkpoint(
                &run.workspace_id,
                &run.run_id,
                run.progress.revision,
                id,
                &event,
            )
            .unwrap();
    }
    advance(store, &run, RunEventV1::PromptSendIntent)
}

fn stopped(store: &RuntimeStore, run: &DelegationRunV1) -> DelegationRunV1 {
    store
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "owned-scope-stopped",
            &DelegationCheckpointV1::ProcessStopped {
                evidence: RunStopEvidenceV1 {
                    scope: RunStopScopeV1::ProcessGroup,
                    observation: RunProcessObservationV1::Exited { code: None },
                    scope_stopped: true,
                    residual_unknown: false,
                },
            },
        )
        .unwrap()
}

#[test]
fn cancel_of_clean_unknown_is_idempotent_after_reopen_and_submission_replay() {
    let dir = crate::test_tempdir().unwrap();
    let input = sample("interrupted-task", "interrupted-run", "same-root");
    let store = open(dir.path());
    store
        .set_worker_concurrency_settings(WorkerConcurrencySettingsV1 { max_concurrent: 1 })
        .unwrap();
    let run = running(&store, &input);
    let run = advance(&store, &run, RunEventV1::ConnectionLost);
    let closed = stopped(&store, &run);
    assert_eq!(closed.progress.state, RunStateV1::Unknown);
    assert!(closed.progress.workspace_releasable());
    drop(store);

    let store = open(dir.path());
    let operation = OperationId::parse("op_00000000000000000000000000000343").unwrap();
    let first = store
        .request_cancel(&closed.workspace_id, &closed.run_id, &operation, "user")
        .unwrap();
    drop(store);
    let store = open(dir.path());
    assert_eq!(
        store
            .request_cancel(&closed.workspace_id, &closed.run_id, &operation, "user")
            .unwrap(),
        first
    );
    let another = OperationId::parse("op_00000000000000000000000000000344").unwrap();
    store
        .request_cancel(&closed.workspace_id, &closed.run_id, &another, "user")
        .unwrap();
    assert_eq!(
        store.run(&closed.workspace_id, &closed.run_id).unwrap(),
        Some(closed.clone())
    );
    assert_eq!(store.accept(&input).unwrap(), closed);
    assert_eq!(
        store
            .find_submission(&closed.workspace_id, false, &closed.idempotency_key)
            .unwrap(),
        Some(closed.clone())
    );
    assert!(store.unreconciled(&closed.workspace_id).unwrap().is_empty());
    // An ended uncertain outcome must not reacquire the instance's only execution slot.
    assert!(
        store
            .accept(&sample("next-task", "next-run", "same-root"))
            .is_ok()
    );
}

#[test]
fn complete_scope_proof_rejects_late_observations_for_every_terminal_outcome() {
    for end in [
        RunStateV1::Unknown,
        RunStateV1::Succeeded,
        RunStateV1::Failed,
        RunStateV1::Cancelled,
    ] {
        let dir = crate::test_tempdir().unwrap();
        let store = open(dir.path());
        let run = running(&store, &sample("task", "run", "root"));
        let run = match end {
            RunStateV1::Unknown => advance(&store, &run, RunEventV1::ConnectionLost),
            RunStateV1::Succeeded => advance(&store, &run, RunEventV1::PromptCompleted),
            RunStateV1::Failed => advance(&store, &run, RunEventV1::PromptFailed),
            RunStateV1::Cancelled => advance(&store, &run, RunEventV1::CancelRequested),
            _ => unreachable!(),
        };
        let revision_before_stop = run.progress.revision;
        let closed = stopped(&store, &run);
        assert_eq!(closed.progress.state, end);
        drop(store);
        let store = open(dir.path());
        for event in [
            DelegationCheckpointV1::ProcessObserved {
                observation: RunProcessObservationV1::Unknown,
            },
            DelegationCheckpointV1::ProcessObserved {
                observation: RunProcessObservationV1::Running,
            },
            DelegationCheckpointV1::ProcessObserved {
                observation: RunProcessObservationV1::Exited { code: None },
            },
            DelegationCheckpointV1::ProcessStopped {
                evidence: RunStopEvidenceV1 {
                    scope: RunStopScopeV1::Root,
                    observation: RunProcessObservationV1::Unknown,
                    scope_stopped: false,
                    residual_unknown: true,
                },
            },
            DelegationCheckpointV1::Progress {
                event: RunEventV1::ProcessUnknown,
            },
        ] {
            assert_eq!(
                store.checkpoint(
                    &closed.workspace_id,
                    &closed.run_id,
                    closed.progress.revision,
                    "late-observation",
                    &event
                ),
                Err(DelegationErrorV1::Conflict)
            );
            assert_eq!(
                store.run(&closed.workspace_id, &closed.run_id).unwrap(),
                Some(closed.clone())
            );
        }
        assert!(store.unreconciled(&closed.workspace_id).unwrap().is_empty());
        let checkpoint = DelegationCheckpointV1::ProcessStopped {
            evidence: closed.stop_evidence.unwrap(),
        };
        assert_eq!(
            store
                .checkpoint(
                    &closed.workspace_id,
                    &closed.run_id,
                    revision_before_stop,
                    "owned-scope-stopped",
                    &checkpoint
                )
                .unwrap(),
            closed
        );
    }
}

#[test]
fn missing_scope_proof_still_requires_stop_or_explicit_residual_confirmation() {
    let dir = crate::test_tempdir().unwrap();
    let store = open(dir.path());
    let run = running(&store, &sample("task", "run", "root"));
    let run = advance(&store, &run, RunEventV1::ConnectionLost);
    let unknown = store
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "lost-held-object",
            &DelegationCheckpointV1::ProcessStopped {
                evidence: RunStopEvidenceV1 {
                    scope: RunStopScopeV1::Root,
                    observation: RunProcessObservationV1::Unknown,
                    scope_stopped: false,
                    residual_unknown: true,
                },
            },
        )
        .unwrap();
    assert!(!unknown.progress.workspace_releasable());
    assert!(
        !store
            .unreconciled(&unknown.workspace_id)
            .unwrap()
            .is_empty()
    );
    let cancelled = advance(&store, &unknown, RunEventV1::CancelRequested);
    assert_eq!(cancelled.progress.state, RunStateV1::Cancelling);
    assert!(!cancelled.progress.workspace_releasable());
    let stopped = stopped(&store, &cancelled);
    assert_eq!(stopped.progress.state, RunStateV1::Cancelled);
    assert!(stopped.progress.workspace_releasable());
}
