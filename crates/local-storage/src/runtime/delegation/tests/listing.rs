use super::*;

#[test]
fn trust_task_population_includes_tasks_without_model_execution_and_binds_creation_scope() {
    let root = crate::test_tempdir().unwrap();
    let store = open(root.path());
    let input = sample("task-trust", "run-trust", "trust-root");
    store.accept(&input).unwrap();
    let population = store
        .trust_task_population(&input.task.workspace_id, "plan", Some(1), 0, 10)
        .unwrap();
    assert!(!population.over_limit);
    assert_eq!(population.tasks.len(), 1);
    assert_eq!(population.tasks[0].state, RunStateV1::Accepted);
    assert_eq!(population.tasks[0].ordinal, 1);
    assert!(
        store
            .trust_task_population(&input.task.workspace_id, "plan", Some(2), 0, 10)
            .unwrap()
            .tasks
            .is_empty()
    );
    assert!(
        store
            .trust_task_population(&input.task.workspace_id, "plan", None, 2, 10)
            .unwrap()
            .tasks
            .is_empty()
    );
    drop(store);
    let reopened = open(root.path());
    assert_eq!(
        reopened
            .trust_task_population(&input.task.workspace_id, "plan", None, 0, 10)
            .unwrap()
            .tasks
            .len(),
        1
    );
}

#[test]
fn latest_run_listing_pages_the_continued_run_once_and_survives_reopen() {
    let dir = crate::test_tempdir().unwrap();
    let workspace = WorkspaceId::parse("workspace").unwrap();
    let second_sequence;
    {
        let store = open(dir.path());
        let first_body = DelegationBodyRefV1 {
            opaque_id: "text-task-a-initial".into(),
            scope_run_id: "run-a-one".into(),
            visibility_generation: 0,
            original_retention_deadline_ms: 100_000,
        };
        let mut first = sample("task-a", "run-a-one", "a");
        first.task.required_body_ids = vec![first_body.opaque_id.clone()];
        first.task.body_refs = vec![first_body.clone()];
        first.task.title.as_mut().unwrap().initial_body_ref = first_body.clone();
        let mut first_run = store.accept(&first).unwrap();
        commit_ready_root(&store, &workspace, "task-a");
        assert_eq!(first_run.accepted_at_ms, Some(first.task.created_at_ms));
        first_run = advance(&store, &first_run, RunEventV1::Preparing);
        first_run = store
            .checkpoint(
                &workspace,
                &first_run.run_id,
                first_run.progress.revision,
                "spawn-first",
                &DelegationCheckpointV1::ProcessSpawned {
                    binding: DelegationProcessBindingV1 {
                        launch_nonce: first_run.launch_nonce.clone(),
                        handle_id: "handle-first".into(),
                        creation_identity: "creation-first".into(),
                    },
                },
            )
            .unwrap();
        first_run = store
            .checkpoint(
                &workspace,
                &first_run.run_id,
                first_run.progress.revision,
                "session-first",
                &DelegationCheckpointV1::SessionBound {
                    binding: DelegationSessionBindingV1 {
                        acp_session_id: "acp-first".into(),
                        native_session_id: Some("native-first".into()),
                    },
                },
            )
            .unwrap();
        first_run = advance(&store, &first_run, RunEventV1::PromptSendIntent);
        first_run = advance(&store, &first_run, RunEventV1::PromptCompleted);
        first_run = store
            .checkpoint(
                &workspace,
                &first_run.run_id,
                first_run.progress.revision,
                "stop-first",
                &DelegationCheckpointV1::ProcessStopped {
                    evidence: RunStopEvidenceV1 {
                        scope: RunStopScopeV1::ProcessGroup,
                        observation: RunProcessObservationV1::Exited { code: Some(0) },
                        scope_stopped: true,
                        residual_unknown: false,
                    },
                },
            )
            .unwrap();
        assert!(first_run.progress.workspace_releasable());
        store
            .set_resume_materials(
                &workspace,
                "task-a",
                "run-a-one",
                10_000,
                std::slice::from_ref(&first_body),
                &["history.jsonl".into()],
            )
            .unwrap();

        let other = store.accept(&sample("task-b", "run-b", "b")).unwrap();
        release_slot(&store, &other);

        let mut continued = sample("task-a", "run-a-two", "a");
        continued.task = store.task(&workspace, "task-a").unwrap().unwrap();
        continued.task.latest_run_id = continued.run.run_id.clone();
        let next_body = DelegationBodyRefV1 {
            opaque_id: "text-task-a-continue".into(),
            scope_run_id: continued.run.run_id.clone(),
            visibility_generation: 0,
            original_retention_deadline_ms: 100_000,
        };
        continued
            .task
            .required_body_ids
            .push(next_body.opaque_id.clone());
        continued.task.body_refs.push(next_body);
        continued.run.ordinal = 2;
        continued.run.continued_from = Some(first_run.run_id.clone());
        continued.run.deadline_ms = 1_010;
        continued.admitted_at_ms = 10;
        continued.expected_latest_run_id = Some(first_run.run_id.clone());
        continued.title_lookup_key = None;
        let continued_run = store.accept(&continued).unwrap();
        assert_eq!(continued_run.accepted_at_ms, Some(continued.admitted_at_ms));
        second_sequence = continued_run.admission_sequence;
        release_slot(&store, &continued_run);

        let first_page = store.list_latest_runs(&workspace, None, None, 1).unwrap();
        assert_eq!(first_page.len(), 1);
        assert_eq!(first_page[0].run_id, "run-a-two");
        assert_eq!(first_page[0].ordinal, 2);
        assert_eq!(first_page[0].continued_from.as_deref(), Some("run-a-one"));
        let second_page = store
            .list_latest_runs(&workspace, None, Some(first_page[0].admission_sequence), 1)
            .unwrap();
        assert_eq!(second_page.len(), 1);
        assert_eq!(second_page[0].run_id, "run-b");
    }

    let reopened = open(dir.path());
    let latest = reopened
        .list_latest_runs(&workspace, None, None, 10)
        .unwrap();
    assert_eq!(
        latest
            .iter()
            .map(|run| run.run_id.as_str())
            .collect::<Vec<_>>(),
        ["run-a-two", "run-b"]
    );
    assert_eq!(latest[0].admission_sequence, second_sequence);
    assert_eq!(latest[0].accepted_at_ms, Some(10));
}

#[test]
fn orphaned_current_run_blocks_recovery_admission_and_scope_cancellation() {
    let dir = crate::test_tempdir().unwrap();
    let workspace = WorkspaceId::parse("workspace").unwrap();
    let store = open(dir.path());
    let orphan = store.accept(&sample("orphan", "orphan-run", "a")).unwrap();
    store
        .connection
        .borrow()
        .execute("DELETE FROM delegation_tasks WHERE task_id='orphan'", [])
        .unwrap();
    drop(store);
    let store = open(dir.path());
    assert_eq!(
        store.unreconciled(&workspace),
        Err(DelegationErrorV1::StorageUnavailable)
    );
    assert_eq!(
        store.accept(&sample("healthy", "healthy-run", "b")),
        Err(DelegationErrorV1::StorageUnavailable)
    );
    let operation = OperationId::parse("op_00112233445566778899aabbccddeeff").unwrap();
    assert_eq!(
        store.cancel_scope(
            &workspace,
            &operation,
            &DelegationAuthorizationScopeV1::Permit {
                id: orphan.permit_id.clone(),
                through_generation: orphan.permit_generation,
            }
        ),
        Err(DelegationErrorV1::StorageUnavailable)
    );
    assert_eq!(store.run(&workspace, &orphan.run_id).unwrap(), Some(orphan));
    assert!(store.task(&workspace, "healthy").unwrap().is_none());
}

#[test]
fn current_resumable_task_with_missing_run_is_a_storage_error() {
    let dir = crate::test_tempdir().unwrap();
    let workspace = WorkspaceId::parse("workspace").unwrap();
    let store = open(dir.path());
    store.accept(&sample("task", "run", "a")).unwrap();
    store.connection.borrow().execute_batch(
        "UPDATE delegation_tasks SET record_json=json_set(record_json,'$.resume_until_ms',100000); DELETE FROM delegation_runs WHERE run_id='run';",
    ).unwrap();
    assert_eq!(
        store.resumable_tasks(&workspace, 1),
        Err(DelegationErrorV1::StorageUnavailable)
    );
}

#[test]
fn unsupported_worker_records_do_not_poison_scans_but_still_count_for_capacity() {
    let dir = crate::test_tempdir().unwrap();
    let workspace = WorkspaceId::parse("workspace").unwrap();
    let store = open(dir.path());
    store
        .accept(&sample("old-task", "old-task-run", "a"))
        .unwrap();
    let legacy_session = serde_json::json!({
        "acp_session_id": "old-session", "native_session_id": "old-session",
        "identity_profile_digest": CanonicalDigest::of_bytes(b"retired-worker-artifacts"),
    });
    let mut old_task =
        serde_json::to_value(store.task(&workspace, "old-task").unwrap().unwrap()).unwrap();
    old_task["session"] = legacy_session.clone();
    old_task["resume_until_ms"] = serde_json::json!(100_000);
    let task_json = old_task.to_string();
    store
        .connection
        .borrow()
        .execute(
            "UPDATE delegation_tasks SET record_json=?1 WHERE task_id='old-task'",
            params![task_json],
        )
        .unwrap();
    assert_eq!(
        store.task(&workspace, "old-task"),
        Err(DelegationErrorV1::InvalidArguments)
    );

    let old_run = store
        .accept(&sample("old-run-task", "old-run", "b"))
        .unwrap();
    let mut old_run = serde_json::to_value(old_run).unwrap();
    old_run["session"] = legacy_session;
    let run_json = old_run.to_string();
    store
        .connection
        .borrow()
        .execute(
            "UPDATE delegation_runs SET record_json=?1 WHERE run_id='old-run'",
            params![run_json],
        )
        .unwrap();
    assert_eq!(
        store.run(&workspace, "old-run"),
        Err(DelegationErrorV1::InvalidArguments)
    );
    store
        .set_worker_concurrency_settings(WorkerConcurrencySettingsV1 { max_concurrent: 2 })
        .unwrap();
    assert_eq!(
        store.accept(&sample("healthy", "healthy-run", "c")),
        Err(DelegationErrorV1::CapacityExceeded)
    );
    store
        .set_worker_concurrency_settings(WorkerConcurrencySettingsV1 { max_concurrent: 3 })
        .unwrap();
    let healthy = store
        .accept(&sample("healthy", "healthy-run", "c"))
        .unwrap();
    drop(store);

    let reopened = open(dir.path());
    assert_eq!(
        reopened.unreconciled(&workspace).unwrap(),
        vec![healthy.clone()]
    );
    assert_eq!(
        reopened
            .list_latest_runs(&workspace, None, None, 1)
            .unwrap(),
        vec![healthy]
    );
    assert!(reopened.resumable_tasks(&workspace, 1).unwrap().is_empty());
    assert!(reopened.task(&workspace, "healthy").unwrap().is_some());
    let unchanged: String = reopened
        .connection
        .borrow()
        .query_row(
            "SELECT record_json FROM delegation_tasks WHERE task_id='old-task'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(unchanged, task_json);
    let unchanged: String = reopened
        .connection
        .borrow()
        .query_row(
            "SELECT record_json FROM delegation_runs WHERE run_id='old-run'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(unchanged, run_json);
}
