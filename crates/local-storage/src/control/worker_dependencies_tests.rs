use super::*;
use crate::test_tempdir as tempdir;
use hiroute_domain::{
    BeginOperationOutcome, CHANGE_SPEC_SCHEMA_V1, ChangeSpecV1, ControlRepositoryPort,
    IdempotencyScopeV1, OperationState, OperationV1, ProtectedApplyCapability, RevisionSetV1,
    TransactionPlanV1, WorkerDependencySelectionChangeV1,
};

fn change(
    harness: WorkerHarnessV1,
    before_revision: u64,
    suffix: &str,
) -> WorkerDependencySelectionChangeV1 {
    WorkerDependencySelectionChangeV1::new(
        before_revision,
        WorkerDependencySelectionRecordV1::new(
            harness,
            (harness != WorkerHarnessV1::QoderCli).then(|| format!("/opt/{suffix}/adapter.js")),
            format!("/opt/{suffix}/cli"),
            (harness != WorkerHarnessV1::QoderCli).then(|| format!("/opt/{suffix}/node")),
        )
        .unwrap(),
    )
    .unwrap()
}

fn selection_operation(
    workspace: &WorkspaceId,
    key: &str,
    change: &WorkerDependencySelectionChangeV1,
) -> OperationV1 {
    let harness = match change.after_selection.harness {
        WorkerHarnessV1::CodexCli => "codex_cli",
        WorkerHarnessV1::ClaudeCode => "claude_code",
        WorkerHarnessV1::QoderCli => "qoder_cli",
        WorkerHarnessV1::Pi => "pi",
    };
    let expected_revisions = RevisionSetV1 {
        target: change.before_revision,
        dependencies: Default::default(),
    };
    let spec = ChangeSpecV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        command_id: "worker.dependencies.select".into(),
        resource_id: Some(format!("worker-dependency-selection/{harness}")),
        desired_state: serde_json::to_value(&change.after_selection).unwrap(),
    };
    let accepted_digest = spec.canonical_digest(&expected_revisions).unwrap();
    let scope =
        IdempotencyScopeV1::new("interactive-user", "SelectWorkerDependencies", key).unwrap();
    let request_digest = CanonicalDigest::of_bytes(format!("request-{key}").as_bytes());
    let operation_id = OperationId::derive(workspace, &scope, &request_digest);
    OperationV1::new(
        operation_id,
        workspace.clone(),
        scope,
        request_digest,
        accepted_digest,
        expected_revisions,
        TransactionPlanV1::from_worker_dependency_selection_planner(spec, change.clone()).unwrap(),
    )
    .unwrap()
}

fn begin_selection(
    store: &ControlStore,
    workspace: &WorkspaceId,
    key: &str,
    change: &WorkerDependencySelectionChangeV1,
) -> OperationV1 {
    let operation = selection_operation(workspace, key, change);
    let capability = format!("capability-{key}");
    store
        .grant_apply_capability(&capability, &operation, i64::MAX)
        .unwrap();
    let authorization = store
        .verify_apply_authorization(
            &ProtectedApplyCapability::new(capability).unwrap(),
            workspace,
            &operation.idempotency.principal,
            &operation.idempotency.operation_kind,
            &operation.accepted_digest,
            &operation.expected_revisions,
        )
        .unwrap();
    assert_eq!(
        store.begin_operation(&operation, &authorization).unwrap(),
        BeginOperationOutcome::Created
    );
    operation
}

fn finish_succeeded(store: &ControlStore, operation: &mut OperationV1) {
    for state in [
        OperationState::Preparing,
        OperationState::ApplyingSecrets,
        OperationState::MaterializingSources,
        OperationState::CompilingPublication,
        OperationState::ApplyingAgentArtifacts,
        OperationState::Activating,
        OperationState::Succeeded,
    ] {
        operation.transition(state).unwrap();
    }
    store.finish_operation(operation).unwrap();
}

fn finish_rolled_back(store: &ControlStore, operation: &mut OperationV1) {
    operation.transition(OperationState::RollingBack).unwrap();
    operation.transition(OperationState::RolledBack).unwrap();
    store.finish_operation(operation).unwrap();
}

#[test]
fn selection_stage_is_invisible_and_harness_revisions_are_independent_and_durable() {
    let directory = tempdir().unwrap();
    // Let ControlStore create the immediate parent with its required owner-only mode. Some
    // platforms do not give tempfile's outer directory that exact mode.
    let database = directory.path().join("store/control.db");
    let backups = directory.path().join("backups");
    let workspace = WorkspaceId::default();
    let mut completed_operations = Vec::new();
    {
        let store =
            ControlStore::open(&crate::test_storage_authority(), &database, &backups).unwrap();
        let codex = change(WorkerHarnessV1::CodexCli, 0, "codex");
        let mut codex_operation = begin_selection(&store, &workspace, "codex", &codex);
        let effect = store
            .stage_worker_dependency_selection(&codex_operation.operation_id, &workspace, &codex)
            .unwrap();
        assert!(
            store
                .worker_dependency_selection(&workspace, WorkerHarnessV1::CodexCli)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            store
                .worker_dependency_selection_revision(&workspace, WorkerHarnessV1::CodexCli,)
                .unwrap(),
            0
        );
        assert_eq!(
            store
                .stage_worker_dependency_selection(
                    &codex_operation.operation_id,
                    &workspace,
                    &codex,
                )
                .unwrap(),
            effect
        );
        store.activate_worker_dependency_selection(&effect).unwrap();
        store.activate_worker_dependency_selection(&effect).unwrap();
        let (selected, revision) = store
            .worker_dependency_selection(&workspace, WorkerHarnessV1::CodexCli)
            .unwrap()
            .unwrap();
        assert_eq!(revision, 1);
        assert_eq!(selected, codex.after_selection);
        finish_succeeded(&store, &mut codex_operation);
        completed_operations.push(codex_operation);

        for (harness, suffix) in [
            (WorkerHarnessV1::ClaudeCode, "claude"),
            (WorkerHarnessV1::QoderCli, "qoder"),
        ] {
            let selection = change(harness, 0, suffix);
            let mut operation = begin_selection(&store, &workspace, suffix, &selection);
            let effect = store
                .stage_worker_dependency_selection(&operation.operation_id, &workspace, &selection)
                .unwrap();
            store.activate_worker_dependency_selection(&effect).unwrap();
            assert_eq!(
                store
                    .worker_dependency_selection(&workspace, harness)
                    .unwrap(),
                Some((selection.after_selection, 1))
            );
            finish_succeeded(&store, &mut operation);
            completed_operations.push(operation);
        }

        let stale = change(WorkerHarnessV1::CodexCli, 0, "stale");
        let stale_operation = selection_operation(&workspace, "stale", &stale);
        assert_eq!(
            store
                .stage_worker_dependency_selection(
                    &stale_operation.operation_id,
                    &workspace,
                    &stale,
                )
                .unwrap_err()
                .code,
            PortErrorCode::Conflict
        );
    }
    let reopened =
        ControlStore::open(&crate::test_storage_authority(), &database, &backups).unwrap();
    for harness in [
        WorkerHarnessV1::CodexCli,
        WorkerHarnessV1::ClaudeCode,
        WorkerHarnessV1::QoderCli,
    ] {
        let (_, revision) = reopened
            .worker_dependency_selection(&workspace, harness)
            .unwrap()
            .unwrap();
        assert_eq!(revision, 1);
    }
    for operation in completed_operations {
        assert_eq!(
            reopened.load_operation(&operation.operation_id).unwrap(),
            Some(operation)
        );
    }
}

#[test]
fn compensation_restores_only_the_effect_still_owned_by_the_operation() {
    let directory = tempdir().unwrap();
    let store = ControlStore::open(
        &crate::test_storage_authority(),
        directory.path().join("store/control.db"),
        directory.path().join("backups"),
    )
    .unwrap();
    let workspace = WorkspaceId::default();
    let first = change(WorkerHarnessV1::CodexCli, 0, "first");
    let mut first_operation = begin_selection(&store, &workspace, "first", &first);
    let first_effect = store
        .stage_worker_dependency_selection(&first_operation.operation_id, &workspace, &first)
        .unwrap();
    store
        .activate_worker_dependency_selection(&first_effect)
        .unwrap();
    finish_succeeded(&store, &mut first_operation);
    let second = change(WorkerHarnessV1::CodexCli, 1, "second");
    let mut second_operation = begin_selection(&store, &workspace, "second", &second);
    let second_effect = store
        .stage_worker_dependency_selection(&second_operation.operation_id, &workspace, &second)
        .unwrap();
    store
        .activate_worker_dependency_selection(&second_effect)
        .unwrap();
    assert_eq!(
        store
            .compensate_worker_dependency_selection(&first_effect)
            .unwrap(),
        Some(CompensationOutcome::OwnershipLost)
    );
    assert_eq!(
        store
            .compensate_worker_dependency_selection(&second_effect)
            .unwrap(),
        Some(CompensationOutcome::Compensated)
    );
    let (restored, revision) = store
        .worker_dependency_selection(&workspace, WorkerHarnessV1::CodexCli)
        .unwrap()
        .unwrap();
    assert_eq!(revision, 1);
    assert_eq!(restored, first.after_selection);
    finish_rolled_back(&store, &mut second_operation);
}
