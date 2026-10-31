//! Public reads keep saved content independent of execution and cleanup evidence.

use super::*;
use hiroute_domain::delegation::{
    DelegationProcessBindingV1, DelegationSessionBindingV1, RunCleanupV1, RunStateV1,
};
use rusqlite::{Connection, OpenFlags};

#[test]
fn worker_read_empty_progress_survives_reopen_and_residual_acknowledgement() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::tests::worker_read_empty_tests::worker_read_empty_progress_survives_reopen_and_residual_acknowledgement",
    ) {
        return;
    }
    let root = crate::test_support::private_tempdir();
    let (runtime, mut run) = empty_progress_run(root.path());
    let service = ApplicationService::new(runtime.application_ports());
    let cursor = assert_pending(&runtime, &service, &run, None);
    run = start_prompt(&runtime, &run);
    assert_pending(&runtime, &service, &run, Some(&cursor));
    assert_eq!(run.progress.state, RunStateV1::Running);
    assert!(run.progress.prompt_may_have_executed);
    run = advance(&runtime, &run, RunEventV1::ProcessUnknown);
    assert_eq!(run.progress.state, RunStateV1::Unknown);
    assert_eq!(run.progress.cleanup, RunCleanupV1::Unknown);
    drop(service);
    drop(runtime);

    let reopened = ProductionControlRuntime::open_with_release_catalog(
        root.path(),
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    let saved =
        DelegationRuntimePort::run(reopened.adapter.as_ref(), &run.workspace_id, &run.run_id)
            .unwrap()
            .unwrap();
    assert_eq!(saved, run);
    let service = ApplicationService::new(reopened.application_ports());
    assert_pending(&reopened, &service, &saved, Some(&cursor));
    let acknowledged = checkpoint(
        &reopened,
        &saved,
        DelegationCheckpointV1::ResidualConfirmed {
            operation_id: hiroute_domain::OperationId::parse("op_00112233445566778899aabbccddeeff")
                .unwrap(),
            actor_id: "empty-progress-test".into(),
        },
    );
    assert_eq!(acknowledged.progress.state, RunStateV1::Unknown);
    assert_eq!(
        acknowledged.progress.cleanup,
        RunCleanupV1::ResidualAcknowledged
    );
    assert!(acknowledged.progress.workspace_releasable());
    assert_pending(&reopened, &service, &acknowledged, Some(&cursor));
}

#[test]
fn worker_read_empty_progress_preserves_terminal_execution_states() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::tests::worker_read_empty_tests::worker_read_empty_progress_preserves_terminal_execution_states",
    ) {
        return;
    }
    for (event, expected) in [
        (RunEventV1::PromptCompleted, RunStateV1::Succeeded),
        (RunEventV1::PromptFailed, RunStateV1::Failed),
        (
            RunEventV1::ManagedScopeStopped { success: true },
            RunStateV1::Cancelled,
        ),
    ] {
        let root = crate::test_support::private_tempdir();
        let (runtime, mut run) = empty_progress_run(root.path());
        run = start_prompt(&runtime, &run);
        if expected == RunStateV1::Cancelled {
            run = advance(&runtime, &run, RunEventV1::CancelRequested);
        }
        run = advance(&runtime, &run, event);
        assert_eq!(run.progress.state, expected);
        assert!(run.progress.prompt_may_have_executed);
        let service = ApplicationService::new(runtime.application_ports());
        assert_pending(&runtime, &service, &run, None);
    }
}

#[test]
fn worker_read_real_storage_failure_remains_an_error() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::tests::worker_read_empty_tests::worker_read_real_storage_failure_remains_an_error",
    ) {
        return;
    }
    let root = crate::test_support::private_tempdir();
    let (runtime, run) = empty_progress_run(root.path());
    let service = ApplicationService::new(runtime.application_ports());
    let cursor = assert_pending(&runtime, &service, &run, None);
    // Fault injection affects only this isolated fixture's observation database.
    let connection = Connection::open(&runtime.adapter.observation_activity_path).unwrap();
    connection
        .execute("DROP TABLE managed_text_progress", [])
        .unwrap();
    let response = worker_read(&service, &run.run_id, Some(&cursor), Some(32));
    assert!(response.data.is_none());
    let error = response.error.unwrap();
    assert_eq!(
        error.code,
        hiroute_application_api::ErrorCode::ObservationUnavailable
    );
    assert_eq!(error.message_key, "worker.read.storage_unavailable");
    assert_eq!(
        DelegationRuntimePort::run(runtime.adapter.as_ref(), &run.workspace_id, &run.run_id)
            .unwrap()
            .unwrap(),
        run
    );
}

fn empty_progress_run(root: &std::path::Path) -> (ProductionControlRuntime, DelegationRunV1) {
    let runtime = ProductionControlRuntime::open_with_release_catalog(
        root,
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let body = runtime
        .adapter
        .persist_task_input(
            &WorkspaceId::default(),
            "task-empty",
            "run-empty",
            "goal",
            now,
        )
        .unwrap();
    let version = frozen_plan_version();
    // Seed this query fixture's current frozen version; this is not publication evidence.
    let connection = Connection::open(root.join("live/control.db")).unwrap();
    connection
        .execute(
            "INSERT INTO plan_versions(
                workspace_id,plan_id,content_revision,content_digest,version_json,state,
                owner_operation_id
             ) VALUES(?1,?2,?3,?4,?5,'published','empty-progress-fixture')",
            rusqlite::params![
                version.reference.workspace_id.as_str(),
                version.reference.plan_id.as_str(),
                version.reference.content_revision,
                version.reference.content_digest.as_str(),
                serde_json::to_string(&version).unwrap(),
            ],
        )
        .unwrap();
    let mut acceptance = worker_list_acceptance("task-empty", "run-empty", body, now);
    acceptance.task.plan.plan_id = version.reference.plan_id.clone();
    acceptance.task.plan.plan_revision = version.reference.content_revision;
    acceptance.task.plan.plan_digest = version.reference.content_digest.clone();
    acceptance.task.plan.model_alias = version.compiled.model_alias().as_str().into();
    let run = DelegationRuntimePort::accept(runtime.adapter.as_ref(), &acceptance).unwrap();
    (runtime, run)
}

fn frozen_plan_version() -> hiroute_domain::PlanVersionV1 {
    use hiroute_domain::{
        AgentPlanAuthoringV2, AgentPlanStrategyV2, CandidateSelectionV1, CapabilityRequirementsV1,
        ExactNativeReasoningV1, PLAN_AUTHORING_SCHEMA_V2, PlanEditorMode, ReasoningSelectionV1,
    };
    let publication: hiroute_domain::GatewayPublicationV1 = serde_json::from_slice(include_bytes!(
        "../../../../e2e/product/fixtures/routing/current-publication.v3.json"
    ))
    .unwrap();
    let compiled = publication
        .plans
        .into_iter()
        .find(|plan| plan.agent_plan_id().as_str() == "plan/custom")
        .unwrap();
    let candidates = compiled.body.materialized.attempt_owned.groups[0]
        .candidates
        .iter()
        .map(|candidate| CandidateSelectionV1 {
            binding_id: candidate.binding_id.clone(),
            reasoning: match &candidate.exact_reasoning {
                ExactNativeReasoningV1::Fixed { .. } => None,
                ExactNativeReasoningV1::Toggle { enabled, .. } => {
                    Some(ReasoningSelectionV1::Toggle { enabled: *enabled })
                }
                ExactNativeReasoningV1::Profile { profile, .. } => {
                    Some(ReasoningSelectionV1::Profile {
                        profile: profile.clone(),
                    })
                }
                ExactNativeReasoningV1::Budget { tokens, .. } => {
                    Some(ReasoningSelectionV1::Budget { tokens: *tokens })
                }
            },
        })
        .collect();
    hiroute_domain::PlanVersionV1::new(
        WorkspaceId::default(),
        AgentPlanAuthoringV2 {
            follow_request_reasoning: false,
            schema: PLAN_AUTHORING_SCHEMA_V2.into(),
            display_name: compiled.body.identity.display_name.clone(),
            purpose: compiled.body.identity.purpose.clone(),
            mode: PlanEditorMode::FixedModel,
            requirements: CapabilityRequirementsV1::default(),
            limits: compiled.body.materialized.attempt_owned.limits.clone(),
            strategy: AgentPlanStrategyV2::Custom { candidates },
            delegation_enabled: false,
            work: None,
        },
        compiled,
    )
    .unwrap()
}

pub(super) fn start_prompt(
    runtime: &ProductionControlRuntime,
    accepted: &DelegationRunV1,
) -> DelegationRunV1 {
    let mut run = advance(runtime, accepted, RunEventV1::Preparing);
    for event in [
        DelegationCheckpointV1::ProcessSpawned {
            binding: DelegationProcessBindingV1 {
                launch_nonce: run.launch_nonce.clone(),
                handle_id: "empty-progress-process".into(),
                creation_identity: "empty-progress-created".into(),
            },
        },
        DelegationCheckpointV1::SessionBound {
            binding: DelegationSessionBindingV1 {
                acp_session_id: "empty-progress-session".into(),
                native_session_id: Some("empty-progress-native".into()),
            },
        },
        DelegationCheckpointV1::Progress {
            event: RunEventV1::PromptSendIntent,
        },
    ] {
        run = checkpoint(runtime, &run, event);
    }
    run
}

fn advance(
    runtime: &ProductionControlRuntime,
    run: &DelegationRunV1,
    event: RunEventV1,
) -> DelegationRunV1 {
    checkpoint(runtime, run, DelegationCheckpointV1::Progress { event })
}

fn checkpoint(
    runtime: &ProductionControlRuntime,
    run: &DelegationRunV1,
    event: DelegationCheckpointV1,
) -> DelegationRunV1 {
    DelegationRuntimePort::checkpoint(
        runtime.adapter.as_ref(),
        &run.workspace_id,
        &run.run_id,
        run.progress.revision,
        &format!("empty-progress-{}", run.progress.revision),
        &event,
    )
    .unwrap()
}

fn assert_pending(
    runtime: &ProductionControlRuntime,
    service: &ApplicationService,
    run: &DelegationRunV1,
    cursor: Option<&str>,
) -> String {
    let before = progress_metadata(runtime);
    assert_eq!(before, (0, 0, 0));
    let response = worker_read(service, &run.run_id, cursor, Some(32));
    assert!(response.error.is_none(), "{response:?}");
    let data: WorkerReadDataV1 = serde_json::from_value(response.data.unwrap()).unwrap();
    assert!(data.valid());
    assert_eq!(data.content_state, WorkerReadContentStateV1::Pending);
    assert_eq!(data.run_state, run.progress.state);
    assert_eq!(data.state_revision, run.progress.revision);
    assert_eq!(data.task_id, run.task_id);
    assert_eq!(data.run_id, run.run_id);
    assert_eq!(progress_metadata(runtime), before);
    assert_eq!(
        DelegationRuntimePort::run(runtime.adapter.as_ref(), &run.workspace_id, &run.run_id)
            .unwrap()
            .unwrap(),
        *run
    );
    let next = data.next_cursor.unwrap();
    if let Some(cursor) = cursor {
        assert_eq!(next, cursor);
    }
    next
}

fn progress_metadata(runtime: &ProductionControlRuntime) -> (u64, u64, u64) {
    let connection = Connection::open_with_flags(
        &runtime.adapter.observation_activity_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    connection
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM managed_text_progress),
                (SELECT COUNT(*) FROM managed_text_progress_blocks),
                (SELECT next_order_seq FROM managed_text_progress_meta WHERE singleton=1)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}
