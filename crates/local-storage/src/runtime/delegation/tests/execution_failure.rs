use super::*;
use hiroute_domain::delegation::{WorkerExecutionFailureV1, WorkerExecutionStageV1};

/// Frozen by the v0.3.1 RuntimeStore acceptance/checkpoint producer at
/// 13856abd873da5b4373d39322493b4cc105c4867, using its existing sample fixture and
/// LaunchFailedBeforeSpawn. The fixture bytes never pass through the current producer.
#[cfg(unix)]
#[test]
fn released_run_without_failure_cause_retains_its_facts_through_reopen_and_replay() {
    let bytes = include_str!("fixtures/v0.3.1-failed-run.json");
    let expected: DelegationRunV1 = serde_json::from_str(bytes).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    let run = store
        .accept(&sample("released-task", "released-run", "/released"))
        .unwrap();
    advance(&store, &run, RunEventV1::LaunchFailedBeforeSpawn);
    store
        .connection
        .borrow()
        .execute(
            "UPDATE delegation_runs SET record_json=?1 WHERE run_id=?2",
            params![bytes, expected.run_id],
        )
        .unwrap();
    drop(store);
    let store = open(dir.path());
    let actual = store
        .run(&expected.workspace_id, &expected.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual.progress.state, RunStateV1::Failed);
    assert!(!actual.progress.prompt_may_have_executed);
    assert!(actual.execution_failure.is_none());
    assert_eq!(
        store
            .find_submission(&actual.workspace_id, false, &actual.idempotency_key)
            .unwrap(),
        Some(actual.clone())
    );
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::from_str::<serde_json::Value>(bytes).unwrap()
    );
}

#[test]
fn failure_cause_is_atomic_immutable_and_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    let run = store
        .accept(&sample("failure-task", "failure-run", "/failure"))
        .unwrap();
    let failure = WorkerExecutionFailureV1 {
        stage: WorkerExecutionStageV1::Preparation,
        error: DelegationErrorV1::CapabilityUnavailable,
    };
    let checkpoint = DelegationCheckpointV1::ExecutionFailed {
        event: RunEventV1::LaunchFailedBeforeSpawn,
        failure,
    };
    let failed = store
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "failed",
            &checkpoint,
        )
        .unwrap();
    assert_eq!(failed.progress.state, RunStateV1::Failed);
    assert_eq!(failed.execution_failure, Some(failure));
    assert_eq!(
        store
            .checkpoint(
                &run.workspace_id,
                &run.run_id,
                run.progress.revision,
                "failed",
                &checkpoint
            )
            .unwrap(),
        failed
    );
    let contradictory = DelegationCheckpointV1::ExecutionFailed {
        event: RunEventV1::LaunchFailedBeforeSpawn,
        failure: WorkerExecutionFailureV1 {
            error: DelegationErrorV1::Cancelled,
            ..failure
        },
    };
    assert_eq!(
        store.checkpoint(
            &run.workspace_id,
            &run.run_id,
            failed.progress.revision,
            "replacement",
            &contradictory
        ),
        Err(DelegationErrorV1::Conflict)
    );
    drop(store);
    let store = open(dir.path());
    assert_eq!(
        store
            .run(&run.workspace_id, &run.run_id)
            .unwrap()
            .unwrap()
            .execution_failure,
        Some(failure)
    );
}
