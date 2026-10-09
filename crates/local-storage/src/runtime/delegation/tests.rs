use super::*;
use hiroute_domain::{AgentPlanId, CanonicalDigest};

mod listing;
mod native_lifecycle;

fn sample(task_id: &str, run_id: &str, root: &str) -> DelegationAcceptanceV1 {
    let workspace_id = WorkspaceId::parse("workspace").unwrap();
    let execution = WorkerExecutionIntentV1 {
        root_identity: root.into(),
        access: WorkspaceAccessV1::TrustedNative,
        tools: vec![WorkerToolV1::Read, WorkerToolV1::Shell],
        network: WorkerNetworkV1::Allowed,
        duration_ms: 1000,
        delegation_depth: 1,
    };
    let body = DelegationBodyRefV1 {
        opaque_id: format!("body-{task_id}-{run_id}"),
        scope_run_id: run_id.into(),
        visibility_generation: 0,
        original_retention_deadline_ms: 10_000,
    };
    DelegationAcceptanceV1 {
        task: DelegationTaskV1 {
            workspace_id: workspace_id.clone(),
            task_id: task_id.into(),
            parent_task_ref: None,
            plan: DelegationPlanBindingV1 {
                authority_id: "authority".into(),
                plan_id: AgentPlanId::parse("plan").unwrap(),
                plan_revision: 1,
                plan_digest: CanonicalDigest::of_bytes(b"plan"),
                publication_revision: 1,
                publication_digest: CanonicalDigest::of_bytes(b"publication"),
                exact_reference: "exact".into(),
                model_alias: "alias".into(),
                harness: WorkerHarnessV1::CodexCli,
                harness_configuration_digest: CanonicalDigest::of_bytes(b"profile"),
            },
            workspace: DelegationWorkspaceV1 {
                root_identity: root.into(),
                volume_identity: "volume".into(),
                ancestry: vec!["root".into(), root.into()],
            },
            created_at_ms: 1,
            latest_run_id: run_id.into(),
            latest_admission_sequence: 0,
            title: Some(DelegationTaskTitleV1 {
                value: format!("Task {task_id}"),
                source: DelegationTaskTitleSourceV1::Explicit,
                initial_body_ref: body.clone(),
            }),
            session: None,
            resume_until_ms: 0,
            required_body_ids: vec![body.opaque_id.clone()],
            body_refs: vec![body],
            native_history_paths: vec![],
        },
        run: DelegationRunV1 {
            workspace_id,
            task_id: task_id.into(),
            run_id: run_id.into(),
            ordinal: 1,
            continued_from: None,
            idempotency_key: run_id.into(),
            request_digest: CanonicalDigest::of_bytes(run_id.as_bytes()),
            admission_sequence: 0,
            accepted_at_ms: None,
            execution_owner_ref: run_id.into(),
            lease_id: run_id.into(),
            daemon_epoch: "epoch".into(),
            permit_id: format!("run-config/{run_id}"),
            permit_generation: 1,
            configuration: DelegationRunConfigurationV1 {
                format_version: DELEGATION_RUN_CONFIGURATION_VERSION_V1,
                scope_id: format!("run-config/{run_id}"),
                generation: 1,
                canonical_workspace_path: "/workspace".into(),
                permission_policy: WorkerPermissionPolicyV1::ApproveAll,
            },
            execution,
            deadline_ms: 1001,
            lease_revoked: false,
            launch_nonce: run_id.into(),
            process: None,
            session: None,
            progress: RunProgressV1::default(),
            stop_evidence: None,
            result_body: None,
            result_incomplete: false,
        },
        title_lookup_key: Some(format!("title-{task_id}")),
        expected_latest_run_id: None,
        admitted_at_ms: 1,
    }
}

fn open(path: &std::path::Path) -> RuntimeStore {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    RuntimeStore::open(
        &crate::test_storage_authority(),
        path.join("runtime.db"),
        path.join("backups"),
    )
    .unwrap()
}

fn commit_ready_root(store: &RuntimeStore, workspace: &WorkspaceId, task_id: &str) {
    let snapshot = store.native_root(workspace, task_id).unwrap().unwrap();
    store
        .commit_native_root_ready(&DelegationNativeRootReadyV1 {
            workspace_id: workspace.clone(),
            task_id: task_id.to_owned(),
            root_generation: snapshot.root.root_generation,
            creation_nonce: snapshot.root.creation_nonce,
            managed_base_path: "/managed/sessions".into(),
            filesystem_identity: DelegationNativeFilesystemIdentityV1 {
                scheme: "unix-dev-inode-v1".into(),
                base_device: 1,
                base_inode: 2,
                root_device: 1,
                root_inode: 3,
                marker_device: 1,
                marker_inode: 4,
            },
        })
        .unwrap();
}

#[test]
fn legacy_run_json_without_accepted_time_remains_readable() {
    let run = sample("legacy-task", "legacy-run", "/legacy").run;
    let mut value = serde_json::to_value(run).unwrap();
    value.as_object_mut().unwrap().remove("accepted_at_ms");
    let decoded: DelegationRunV1 = serde_json::from_value(value).unwrap();
    assert!(decoded.accepted_at_ms.is_none());
}

fn advance(store: &RuntimeStore, run: &DelegationRunV1, event: RunEventV1) -> DelegationRunV1 {
    store
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            &format!("event-{}", run.progress.revision),
            &DelegationCheckpointV1::Progress { event },
        )
        .unwrap()
}

fn release_slot(store: &RuntimeStore, run: &DelegationRunV1) {
    let released = advance(store, run, RunEventV1::LaunchFailedBeforeSpawn);
    assert!(released.progress.workspace_releasable());
}

#[test]
fn acceptance_preserves_initial_zero_visibility_generation() {
    let root = crate::test_tempdir().unwrap();
    let store = open(root.path());
    let mut input = sample("task-zero", "run-zero", "zero");
    let body = DelegationBodyRefV1 {
        opaque_id: "text-initial".into(),
        scope_run_id: input.run.run_id.clone(),
        visibility_generation: 0,
        original_retention_deadline_ms: 10000,
    };
    input.task.required_body_ids = vec![body.opaque_id.clone()];
    input.task.body_refs = vec![body.clone()];
    input.task.title.as_mut().unwrap().initial_body_ref = body.clone();
    store.accept(&input).unwrap();
    let persisted = store
        .task(&input.task.workspace_id, &input.task.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted.body_refs, vec![body]);
}

#[test]
fn atomic_acceptance_deduplicates_conflicts_and_survives_reopen() {
    let dir = crate::test_tempdir().unwrap();
    let first = sample("task-a", "run-a", "a");
    let run = {
        let store = open(dir.path());
        let run = store.accept(&first).unwrap();
        assert_eq!(store.accept(&first).unwrap(), run);
        let mut changed = first.clone();
        changed.run.request_digest = CanonicalDigest::of_bytes(b"different");
        assert_eq!(store.accept(&changed), Err(DelegationErrorV1::Conflict));
        // cwd/root identity is execution context, not a scheduling lock.
        assert!(store.accept(&sample("task-b", "run-b", "a")).is_ok());
        run
    };
    let store = open(dir.path());
    assert_eq!(
        store.run(&run.workspace_id, &run.run_id).unwrap(),
        Some(run)
    );
}

#[test]
fn worker_concurrency_setting_is_strict_last_write_wins_and_survives_reopen() {
    let dir = crate::test_tempdir().unwrap();
    {
        let store = open(dir.path());
        assert_eq!(
            store.worker_concurrency_settings().unwrap(),
            WorkerConcurrencySettingsV1::default()
        );
        assert_eq!(
            store
                .set_worker_concurrency_settings(WorkerConcurrencySettingsV1 { max_concurrent: 42 })
                .unwrap(),
            WorkerConcurrencySettingsV1 { max_concurrent: 42 }
        );
        assert_eq!(
            store
                .set_worker_concurrency_settings(WorkerConcurrencySettingsV1 {
                    max_concurrent: 1_000,
                })
                .unwrap(),
            WorkerConcurrencySettingsV1 {
                max_concurrent: 1_000,
            }
        );
        assert_eq!(
            store
                .set_worker_concurrency_settings(WorkerConcurrencySettingsV1 { max_concurrent: 0 }),
            Err(DelegationErrorV1::InvalidArguments)
        );
    }
    let store = open(dir.path());
    assert_eq!(
        store.worker_concurrency_settings().unwrap(),
        WorkerConcurrencySettingsV1 {
            max_concurrent: 1_000,
        }
    );
}

#[test]
fn admission_counts_durable_occupancy_and_replay_precedes_capacity() {
    let dir = crate::test_tempdir().unwrap();
    let store = open(dir.path());
    store
        .set_worker_concurrency_settings(WorkerConcurrencySettingsV1 { max_concurrent: 1 })
        .unwrap();
    let first = sample("task-a", "run-a", "same-root");
    let accepted = store.accept(&first).unwrap();
    assert_eq!(store.accept(&first).unwrap(), accepted);
    let mut other_workspace = sample("task-b", "run-b", "same-root");
    let workspace = WorkspaceId::parse("other-workspace").unwrap();
    other_workspace.task.workspace_id = workspace.clone();
    other_workspace.run.workspace_id = workspace;
    assert_eq!(
        store.accept(&other_workspace),
        Err(DelegationErrorV1::CapacityExceeded)
    );
}

#[test]
fn default_capacity_is_ten() {
    let dir = crate::test_tempdir().unwrap();
    let store = open(dir.path());
    for index in 0..10 {
        store
            .accept(&sample(
                &format!("task-{index}"),
                &format!("run-{index}"),
                &format!("root-{index}"),
            ))
            .unwrap();
    }
    assert_eq!(
        store.accept(&sample("task-eleven", "run-eleven", "root-eleven")),
        Err(DelegationErrorV1::CapacityExceeded)
    );
}

#[test]
fn lowering_capacity_keeps_existing_runs_and_only_changes_later_admission() {
    let dir = crate::test_tempdir().unwrap();
    let store = open(dir.path());
    let first = store.accept(&sample("task-a", "run-a", "a")).unwrap();
    let second = store.accept(&sample("task-b", "run-b", "b")).unwrap();
    store.accept(&sample("task-c", "run-c", "c")).unwrap();
    store
        .set_worker_concurrency_settings(WorkerConcurrencySettingsV1 { max_concurrent: 2 })
        .unwrap();
    assert_eq!(
        store.accept(&sample("task-d", "run-d", "d")),
        Err(DelegationErrorV1::CapacityExceeded)
    );
    release_slot(&store, &first);
    assert_eq!(
        store.accept(&sample("task-d", "run-d", "d")),
        Err(DelegationErrorV1::CapacityExceeded)
    );
    release_slot(&store, &second);
    assert!(store.accept(&sample("task-d", "run-d", "d")).is_ok());
}

#[test]
fn latest_run_listing_is_instance_scoped_with_exact_title_lookup_and_exclusive_cursor() {
    let dir = crate::test_tempdir().unwrap();
    let store = open(dir.path());
    let run_a = store.accept(&sample("task-a", "run-a", "a")).unwrap();
    store.accept(&sample("task-b", "run-b", "b")).unwrap();
    release_slot(&store, &run_a);
    store.accept(&sample("task-c", "run-c", "c")).unwrap();
    let workspace = WorkspaceId::parse("workspace").unwrap();
    let first = store.list_latest_runs(&workspace, None, None, 2).unwrap();
    assert_eq!(
        first
            .iter()
            .map(|run| run.run_id.as_str())
            .collect::<Vec<_>>(),
        ["run-c", "run-b"]
    );
    let second = store
        .list_latest_runs(
            &workspace,
            None,
            Some(first.last().unwrap().admission_sequence),
            2,
        )
        .unwrap();
    assert_eq!(
        second
            .iter()
            .map(|run| run.run_id.as_str())
            .collect::<Vec<_>>(),
        ["run-a"]
    );
    let exact = store
        .list_latest_runs(&workspace, Some("title-task-b"), None, 2)
        .unwrap();
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].run_id, "run-b");
    assert!(
        store
            .list_latest_runs(&workspace, Some("missing-title"), None, 2)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn cancel_intent_is_durable_idempotent_and_does_not_claim_stopped() {
    let dir = crate::test_tempdir().unwrap();
    let store = open(dir.path());
    let input = sample("task", "run", "a");
    let run = store.accept(&input).unwrap();
    let run = advance(&store, &run, RunEventV1::Preparing);
    let operation = OperationId::parse("op_00000000000000000000000000000001").unwrap();
    let receipt = store
        .request_cancel(&run.workspace_id, &run.run_id, &operation, "user")
        .unwrap();
    assert_eq!(
        store
            .request_cancel(&run.workspace_id, &run.run_id, &operation, "user")
            .unwrap(),
        receipt
    );
    assert!(
        store
            .request_cancel(&run.workspace_id, &run.run_id, &operation, "other")
            .is_err()
    );
    let run = store.run(&run.workspace_id, &run.run_id).unwrap().unwrap();
    assert!(run.lease_revoked);
    assert_eq!(run.progress.state, RunStateV1::Cancelling);
    assert!(!run.progress.workspace_releasable());
}

#[test]
fn cancel_before_preparation_releases_capacity_and_fences_stale_launch_after_reopen() {
    let dir = crate::test_tempdir().unwrap();
    let store = open(dir.path());
    let accepted = store.accept(&sample("task", "run", "root")).unwrap();
    let operation = OperationId::parse("op_00000000000000000000000000000003").unwrap();
    let receipt = store
        .request_cancel(&accepted.workspace_id, &accepted.run_id, &operation, "user")
        .unwrap();
    drop(store);
    let store = open(dir.path());
    assert_eq!(
        store
            .request_cancel(&accepted.workspace_id, &accepted.run_id, &operation, "user")
            .unwrap(),
        receipt
    );
    let cancelled = store
        .run(&accepted.workspace_id, &accepted.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(cancelled.progress.state, RunStateV1::Cancelled);
    assert!(cancelled.progress.workspace_releasable());
    assert!(cancelled.lease_revoked);
    assert!(cancelled.process.is_none());
    assert!(!cancelled.progress.prompt_may_have_executed);
    assert!(
        store
            .checkpoint(
                &accepted.workspace_id,
                &accepted.run_id,
                accepted.progress.revision,
                "stale-launch",
                &DelegationCheckpointV1::Progress {
                    event: RunEventV1::Preparing
                }
            )
            .is_err()
    );
    assert!(
        store
            .accept(&sample("next-task", "next-run", "root"))
            .is_ok()
    );
}

#[test]
fn residual_confirmation_releases_slot_without_completing_or_replaying_run() {
    let dir = crate::test_tempdir().unwrap();
    let store = open(dir.path());
    let mut run = store.accept(&sample("task", "run", "a")).unwrap();
    commit_ready_root(&store, &run.workspace_id, &run.task_id);
    run = advance(&store, &run, RunEventV1::Preparing);
    run = advance(&store, &run, RunEventV1::ProcessRunning);
    run = advance(&store, &run, RunEventV1::ConnectionLost);
    assert!(
        store
            .checkpoint(
                &run.workspace_id,
                &run.run_id,
                run.progress.revision,
                "ack",
                &DelegationCheckpointV1::Progress {
                    event: RunEventV1::ResidualAcknowledged
                }
            )
            .is_err()
    );
    run = advance(&store, &run, RunEventV1::ProcessUnknown);
    run = store
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "confirmed",
            &DelegationCheckpointV1::ResidualConfirmed {
                operation_id: OperationId::parse("op_00000000000000000000000000000002").unwrap(),
                actor_id: "user".into(),
            },
        )
        .unwrap();
    assert_eq!(run.progress.cleanup, RunCleanupV1::ResidualAcknowledged);
    assert_ne!(run.progress.state, RunStateV1::Succeeded);
    assert!(run.lease_revoked);
    assert!(store.accept(&sample("new-task", "new-run", "a")).is_ok());
    assert!(
        store
            .set_resume_materials(
                &run.workspace_id,
                &run.task_id,
                &run.run_id,
                1000,
                &[],
                &["history".into()],
            )
            .is_err()
    );
    assert!(
        store
            .checkpoint(
                &run.workspace_id,
                &run.run_id,
                run.progress.revision,
                "replay",
                &DelegationCheckpointV1::Progress {
                    event: RunEventV1::PromptSendIntent
                }
            )
            .is_err()
    );
}

#[test]
fn checkpoint_cas_and_event_keys_do_not_overwrite_newer_state() {
    let dir = crate::test_tempdir().unwrap();
    let store = open(dir.path());
    let run = store.accept(&sample("task", "run", "a")).unwrap();
    let event = DelegationCheckpointV1::Progress {
        event: RunEventV1::Preparing,
    };
    let next = store.checkpoint(&run.workspace_id, &run.run_id, 0, "a", &event);
    assert!(next.is_err());
    let next = store
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "a",
            &event,
        )
        .unwrap();
    assert_eq!(
        store
            .checkpoint(
                &run.workspace_id,
                &run.run_id,
                run.progress.revision,
                "a",
                &event
            )
            .unwrap(),
        next
    );
    assert!(
        store
            .checkpoint(
                &run.workspace_id,
                &run.run_id,
                next.progress.revision,
                "a",
                &DelegationCheckpointV1::Progress {
                    event: RunEventV1::ProcessUnknown
                }
            )
            .is_err()
    );
}

#[test]
fn result_checkpoint_preserves_a_real_body_or_an_explicit_gap() {
    let dir = crate::test_tempdir().unwrap();
    let store = open(dir.path());
    let mut run = store.accept(&sample("task", "run", "a")).unwrap();
    run = advance(&store, &run, RunEventV1::Preparing);
    run = store
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "spawn",
            &DelegationCheckpointV1::ProcessSpawned {
                binding: DelegationProcessBindingV1 {
                    launch_nonce: run.launch_nonce.clone(),
                    handle_id: "held".into(),
                    creation_identity: "created".into(),
                },
            },
        )
        .unwrap();
    run = store
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "session",
            &DelegationCheckpointV1::SessionBound {
                binding: DelegationSessionBindingV1 {
                    acp_session_id: "session".into(),
                    native_session_id: Some("native-session".into()),
                },
            },
        )
        .unwrap();
    run = advance(&store, &run, RunEventV1::PromptSendIntent);
    assert_eq!(
        store.checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "missing-without-gap",
            &DelegationCheckpointV1::ResultRecorded {
                body: None,
                incomplete: false,
            },
        ),
        Err(DelegationErrorV1::Conflict)
    );
    let result = DelegationBodyRefV1 {
        opaque_id: "result-body".into(),
        scope_run_id: run.run_id.clone(),
        visibility_generation: 0,
        original_retention_deadline_ms: 1_000,
    };
    run = store
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "result",
            &DelegationCheckpointV1::ResultRecorded {
                body: Some(result.clone()),
                incomplete: false,
            },
        )
        .unwrap();
    assert_eq!(run.result_body, Some(result));
    assert!(!run.result_incomplete);
    assert_eq!(
        store
            .checkpoint(
                &run.workspace_id,
                &run.run_id,
                run.progress.revision,
                "result",
                &DelegationCheckpointV1::ResultRecorded {
                    body: run.result_body.clone(),
                    incomplete: false,
                },
            )
            .unwrap(),
        run
    );
}

#[test]
fn stop_scope_is_preserved_and_only_confirmed_cleanup_allows_exact_continue() {
    let dir = crate::test_tempdir().unwrap();
    let store = open(dir.path());
    let mut run = store.accept(&sample("task", "run", "a")).unwrap();
    commit_ready_root(&store, &run.workspace_id, &run.task_id);
    run = advance(&store, &run, RunEventV1::Preparing);
    run = store
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "spawn",
            &DelegationCheckpointV1::ProcessSpawned {
                binding: DelegationProcessBindingV1 {
                    launch_nonce: run.launch_nonce.clone(),
                    handle_id: "held".into(),
                    creation_identity: "created".into(),
                },
            },
        )
        .unwrap();
    let binding = DelegationSessionBindingV1 {
        acp_session_id: "native-session".into(),
        native_session_id: Some("native-session".into()),
    };
    run = store
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "session",
            &DelegationCheckpointV1::SessionBound {
                binding: binding.clone(),
            },
        )
        .unwrap();
    run = advance(&store, &run, RunEventV1::PromptSendIntent);
    let result = DelegationBodyRefV1 {
        opaque_id: "body".into(),
        scope_run_id: run.run_id.clone(),
        visibility_generation: 1,
        original_retention_deadline_ms: 5000,
    };
    run = store
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "result",
            &DelegationCheckpointV1::ResultRecorded {
                body: Some(result.clone()),
                incomplete: false,
            },
        )
        .unwrap();
    run = advance(&store, &run, RunEventV1::PromptCompleted);
    let mut evidence = RunStopEvidenceV1 {
        scope: RunStopScopeV1::ProcessGroup,
        observation: RunProcessObservationV1::Exited { code: Some(0) },
        scope_stopped: false,
        residual_unknown: true,
    };
    run = store
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "root-exit",
            &DelegationCheckpointV1::ProcessStopped { evidence },
        )
        .unwrap();
    assert_eq!(run.progress.state, RunStateV1::Succeeded);
    assert_eq!(run.progress.cleanup, RunCleanupV1::Unknown);
    assert!(!run.progress.workspace_releasable());
    assert_eq!(run.stop_evidence, Some(evidence));
    store
        .set_worker_concurrency_settings(WorkerConcurrencySettingsV1 { max_concurrent: 1 })
        .unwrap();
    assert_eq!(
        store.accept(&sample("other-task", "other-run", "other-root")),
        Err(DelegationErrorV1::CapacityExceeded)
    );
    evidence.scope_stopped = true;
    evidence.residual_unknown = false;
    run = store
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "scope-exit",
            &DelegationCheckpointV1::ProcessStopped { evidence },
        )
        .unwrap();
    store
        .set_resume_materials(
            &run.workspace_id,
            &run.task_id,
            &run.run_id,
            5000,
            &[result],
            &["history".into()],
        )
        .unwrap();
    assert_eq!(
        store.resumable_tasks(&run.workspace_id, 4_999).unwrap(),
        vec![
            store
                .task(&run.workspace_id, &run.task_id)
                .unwrap()
                .unwrap()
        ]
    );
    let mut continuation = sample("task", "run-next", "a");
    continuation.task = store
        .task(&run.workspace_id, &run.task_id)
        .unwrap()
        .unwrap();
    continuation.task.latest_run_id = "run-next".into();
    continuation
        .task
        .required_body_ids
        .push("next-input".into());
    continuation.task.body_refs.push(DelegationBodyRefV1 {
        opaque_id: "next-input".into(),
        scope_run_id: "run-next".into(),
        visibility_generation: 1,
        original_retention_deadline_ms: 5000,
    });
    continuation.run.ordinal = 2;
    continuation.run.continued_from = Some(run.run_id.clone());
    continuation.expected_latest_run_id = Some(run.run_id.clone());
    continuation.title_lookup_key = None;
    let mut changed = continuation.clone();
    changed.task.plan.model_alias = "latest-alias".into();
    assert_eq!(
        store.accept(&changed),
        Err(DelegationErrorV1::ResumeUnavailable)
    );
    let mut stale = continuation.clone();
    stale.task.required_body_ids = vec!["stale-caller-copy".into()];
    stale.task.body_refs = vec![DelegationBodyRefV1 {
        opaque_id: "stale-caller-copy".into(),
        scope_run_id: run.run_id.clone(),
        visibility_generation: 1,
        original_retention_deadline_ms: 5000,
    }];
    assert_eq!(
        store.accept(&stale),
        Err(DelegationErrorV1::ResumeUnavailable)
    );
    let next = store.accept(&continuation).unwrap();
    assert!(
        store
            .resumable_tasks(&run.workspace_id, 4_999)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .task(&run.workspace_id, &run.task_id)
            .unwrap()
            .unwrap()
            .required_body_ids,
        vec!["body".to_string(), "next-input".to_string()]
    );
    assert_eq!(next.task_id, run.task_id);
    assert_eq!(next.continued_from, Some(run.run_id.clone()));
    let mut competing = continuation;
    competing.run.run_id = "racing-run".into();
    competing.run.idempotency_key = "racing".into();
    competing.task.latest_run_id = "racing-run".into();
    assert!(store.accept(&competing).is_err());
    assert_eq!(
        store
            .task(&run.workspace_id, &run.task_id)
            .unwrap()
            .unwrap()
            .session,
        Some(binding)
    );
}

#[test]
fn delegation_permit_scope_cancel_is_exact_and_replayable() {
    let dir = crate::test_tempdir().unwrap();
    let store = open(dir.path());
    for (index, release) in [(1, false), (2, false), (3, true)] {
        let item = sample(
            &format!("task-{index}"),
            &format!("run-{index}"),
            &format!("root-{index}"),
        );
        let run = store.accept(&item).unwrap();
        if release {
            release_slot(&store, &run);
        }
    }
    let workspace = WorkspaceId::parse("workspace").unwrap();
    let operation = OperationId::parse("op_00112233445566778899aabbccddeeff").unwrap();
    let scope = DelegationAuthorizationScopeV1::Permit {
        id: "run-config/run-1".into(),
        through_generation: 1,
    };
    let unaffected = ["run-2", "run-3"].map(|id| {
        let run = store.run(&workspace, id).unwrap().unwrap();
        (id, run.progress.revision, run.lease_revoked)
    });
    store.connection.borrow().execute_batch("CREATE TRIGGER fail_cancel BEFORE UPDATE ON delegation_runs WHEN NEW.run_id='run-1' BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    assert!(store.cancel_scope(&workspace, &operation, &scope).is_err());
    assert!(
        !store
            .run(&workspace, "run-1")
            .unwrap()
            .unwrap()
            .lease_revoked
    );
    store
        .connection
        .borrow()
        .execute_batch("DROP TRIGGER fail_cancel")
        .unwrap();
    let receipt = store.cancel_scope(&workspace, &operation, &scope).unwrap();
    assert!(
        store
            .run(&workspace, "run-1")
            .unwrap()
            .unwrap()
            .lease_revoked
    );
    for (id, revision, lease_revoked) in unaffected {
        let run = store.run(&workspace, id).unwrap().unwrap();
        assert_eq!(run.progress.revision, revision);
        assert_eq!(run.lease_revoked, lease_revoked);
    }
    let revision = store
        .run(&workspace, "run-1")
        .unwrap()
        .unwrap()
        .progress
        .revision;
    drop(store);
    let reopened = open(dir.path());
    assert_eq!(
        reopened
            .cancel_scope(&workspace, &operation, &scope)
            .unwrap(),
        receipt
    );
    assert_eq!(
        reopened
            .run(&workspace, "run-1")
            .unwrap()
            .unwrap()
            .progress
            .revision,
        revision
    );
}
