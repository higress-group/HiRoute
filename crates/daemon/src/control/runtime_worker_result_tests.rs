use super::*;
use crate::delegation::content::RunBodyWriter;
use hiroute_application_api::{ErrorCode, WORKER_CONTENT_UNAVAILABLE_MESSAGE_KEY};
use hiroute_domain::delegation::DelegationBodyRefV1;
use hiroute_observation::managed_text::ManagedTextPurpose;

#[test]
fn deleted_worker_result_is_distinct_from_a_daemon_failure() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::tests::worker_result_tests::deleted_worker_result_is_distinct_from_a_daemon_failure",
    ) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let runtime = ProductionControlRuntime::open_with_release_catalog(
        temp.path(),
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let task_id = "task-worker-result";
    let run_id = "run-worker-result";
    let workspace_id = WorkspaceId::default();
    let input = runtime
        .adapter
        .persist_task_input(&workspace_id, task_id, run_id, "result task", now)
        .unwrap();
    let accepted = DelegationRuntimePort::accept(
        runtime.adapter.as_ref(),
        &worker_list_acceptance(task_id, run_id, input, now),
    )
    .unwrap();
    let mut run = worker_read_empty_tests::start_prompt(&runtime, &accepted);
    let scope = ManagedTextScope {
        workspace_id: workspace_id.clone(),
        task_id: task_id.into(),
        run_id: run_id.into(),
    };
    let timestamp = i64::try_from(now).unwrap();
    let mut writer = RunBodyWriter::create(
        runtime.observation.clone(),
        scope.clone(),
        ManagedTextPurpose::Result,
        "finished-worker-result".into(),
        timestamp,
        timestamp,
    )
    .unwrap();
    writer.append(b"saved result", timestamp).unwrap();
    let body = writer.finish(timestamp).unwrap();
    for event in [
        DelegationCheckpointV1::ResultRecorded {
            body: Some(DelegationBodyRefV1 {
                opaque_id: body.opaque_id,
                scope_run_id: run_id.into(),
                visibility_generation: body.visibility_generation,
                original_retention_deadline_ms: body.original_retention_deadline_ms,
            }),
            incomplete: false,
        },
        DelegationCheckpointV1::Progress {
            event: RunEventV1::PromptCompleted,
        },
    ] {
        run = DelegationRuntimePort::checkpoint(
            runtime.adapter.as_ref(),
            &workspace_id,
            run_id,
            run.progress.revision,
            &format!("result-{}", run.progress.revision),
            &event,
        )
        .unwrap();
    }
    let service = ApplicationService::new(runtime.application_ports());
    let query = |operation: &str, payload| {
        service.dispatch(LocalControlRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "query-worker-result".into(),
            principal: PrincipalV1::ambient_local_peer(),
            operation_id: operation.into(),
            payload,
            protected_grant: None,
        })
    };
    let result = || query("WorkerResult", json!({"run_id":run_id}));
    assert_eq!(result().data.unwrap()["text"], "saved result");
    let preview = runtime
        .observation
        .managed_text_delete_preview(&scope, timestamp, timestamp)
        .unwrap();
    runtime
        .observation
        .managed_text_delete_apply(&preview)
        .unwrap();
    let hidden = result().error.unwrap();
    assert_eq!(hidden.code, ErrorCode::CapabilityUnavailable);
    assert_eq!(hidden.message_key, WORKER_CONTENT_UNAVAILABLE_MESSAGE_KEY);
    let wait = query("WorkerWait", json!({"run_id":run_id,"wait_timeout_secs":1}));
    let view = &wait.data.as_ref().unwrap()["run"];
    assert_eq!(view["state"], "succeeded");
    assert_eq!(
        view["result_available"], true,
        "the run retains the hidden reference"
    );
}
