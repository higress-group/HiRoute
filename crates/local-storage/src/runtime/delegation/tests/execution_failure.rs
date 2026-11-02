use super::*;
use hiroute_domain::delegation::{WorkerExecutionFailureV1, WorkerExecutionStageV1};

#[test]
fn failure_cause_is_atomic_immutable_and_survives_reopen_and_legacy_reads() {
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
    let mut old = serde_json::to_value(&failed).unwrap();
    old.as_object_mut().unwrap().remove("execution_failure");
    assert!(
        serde_json::from_value::<DelegationRunV1>(old)
            .unwrap()
            .execution_failure
            .is_none()
    );
}
