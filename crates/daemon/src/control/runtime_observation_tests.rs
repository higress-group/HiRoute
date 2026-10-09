use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hiroute_application::ApplicationService;
use hiroute_application_api::{
    DelegationAcceptedTimeStateV1, DelegationContentAvailabilityV1, DelegationGetV1,
    DelegationListV1, LOCAL_CONTROL_SCHEMA_V2, LocalControlRequestV2, PrincipalV1,
    WorkerExecutorPresentationBasisV1, WorkerPermissionPolicyV1, WorkerReadContentStateV1,
    WorkerReadDataV1,
};
use hiroute_domain::delegation::{
    DELEGATION_RUN_CONFIGURATION_VERSION_V1, DelegationAcceptanceV1, DelegationCheckpointV1,
    DelegationPlanBindingV1, DelegationRunConfigurationV1, DelegationRunV1, DelegationRuntimePort,
    DelegationTaskV1, DelegationWorkspaceV1, RunEventV1, RunProgressV1, WorkerExecutionIntentV1,
    WorkerHarnessV1, WorkerNetworkV1, WorkerToolV1, WorkspaceAccessV1,
};
use hiroute_domain::{
    AgentPlanId, AttemptId, CanonicalDigest, CompletenessDeltaV1, CorrelationProvenance,
    EXECUTION_FACT_PORT_DIGEST_V2, EXECUTION_FACT_SCHEMA_V2, EventId, ExecutionCorrelationV1,
    ExecutionFactChannelV1, ExecutionFactEnvelopeV1, ExecutionFactV1, ExecutionProducerComponentV1,
    ExecutionProducerV1, ExecutionRequestOutcomeV1, FactsCompleteness, FrozenExecutionTrustV1,
    FrozenValueFactsV1, IngressProtocolV1, LogicalRequestId, ObservationStreamV1,
    ObservationValueQueryV2, ProducerEpoch, ProducerId, SelectorSourceV1, SessionId,
    SessionScopeV1, StreamId, TrafficKind, TurnId, UsageFactsV1, WorkspaceId,
};
use hiroute_gateway::server::core_runtime::observation::{
    NativeAgentObservationIdentityV1, derive_native_agent_observation_session_id,
};
use hiroute_observation::managed_text::{ManagedTextProgressTarget, ManagedTextScope};
use hiroute_observation::{LocalObservationWriter, OfferOutcome, WriterCycleOutcome};
use serde_json::json;

use super::*;

#[test]
fn production_writer_store_drives_application_owned_defaults_and_all_plan_scope() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::tests::production_writer_store_drives_application_owned_defaults_and_all_plan_scope",
    ) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let runtime = ProductionControlRuntime::open_with_release_catalog(
        temp.path(),
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    let revisions = runtime
        .adapter
        .stores
        .lock()
        .unwrap()
        .control()
        .current_revisions(&WorkspaceId::default())
        .unwrap();
    assert_eq!(revisions.dependencies.get("release.registry"), Some(&1));
    assert_eq!(revisions.dependencies.get("release.model_data"), Some(&1));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    ingest_value(&runtime.observation, "alpha", "plan/alpha", now_ms - 2_000);
    ingest_value(&runtime.observation, "beta", "plan/beta", now_ms - 1_000);

    let application = ApplicationService::new(runtime.application_ports());
    let grant = |operation: &str, scope: CanonicalDigest| {
        let capability = format!(
            "observation-test-capability-{operation}-{}",
            &scope.as_str()[..16]
        );
        runtime
            .register_apply_capability(
                ApplyCapabilityRegistrationV1::from_protected_launcher(
                    capability.clone(),
                    "interactive-user",
                    WorkspaceId::default(),
                    operation,
                    scope,
                    revisions.clone(),
                    now_ms / 1000 + 120,
                )
                .unwrap(),
            )
            .unwrap();
        hiroute_application_api::ProtectedClientGrantV2 {
            principal_kind: hiroute_application_api::PrincipalKind::InteractiveUser,
            capability,
        }
    };
    let mut request = LocalControlRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: "production-sessions".to_owned(),
        principal: PrincipalV1::ambient_local_peer(),
        operation_id: "ListSessions".to_owned(),
        payload: json!({}),
        protected_grant: None,
    };
    let same_uid_sessions = application.dispatch(request.clone());
    assert!(same_uid_sessions.error.is_none(), "{same_uid_sessions:?}");
    assert_eq!(
        same_uid_sessions.data.unwrap()["sessions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let scope: hiroute_application_api::SessionListRequestV1 =
        serde_json::from_value(json!({})).unwrap();
    request.protected_grant = Some(grant("ListSessions", CanonicalDigest::of(&scope).unwrap()));
    let sessions = application.dispatch(request);
    assert_eq!(
        sessions.data.unwrap()["sessions"].as_array().unwrap().len(),
        2
    );

    let v2 = hiroute_application_api::ObservationReadRequestV2::new(
        hiroute_application_api::ObservationReadIntentV2::Sessions(
            hiroute_application_api::ObservationRequestQuery {
                from_ms: now_ms - 7 * 86_400_000,
                to_ms: now_ms + 1000,
                session_id: None,
                request_id: None,
                limit: 50,
                cursor: None,
                agent_id: None,
                plan_id: None,
                native_model: None,
                outcome: None,
                only_model_switch: false,
            },
        ),
    );
    let mut v2_wire = LocalControlRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: "production-observation-v2".into(),
        principal: PrincipalV1::ambient_local_peer(),
        operation_id: v2.operation().into(),
        payload: serde_json::to_value(&v2).unwrap(),
        protected_grant: Some(grant(
            v2.protected_operation(),
            CanonicalDigest::of(&v2).unwrap(),
        )),
    };
    let listed = application.dispatch(v2_wire.clone());
    let sessions = listed.data.unwrap()["sessions"].as_array().unwrap().clone();
    assert_eq!(sessions.len(), 2);
    assert!(
        sessions
            .iter()
            .all(|session| session["correlation_kind"] == "agent_supplied")
    );
    v2_wire.payload["intent"]["query"]["limit"] = json!(100);
    assert_eq!(
        application.dispatch(v2_wire).error.unwrap().code,
        hiroute_application_api::ErrorCode::CapabilityDenied
    );

    let home = hiroute_application_api::ObservationReadRequestV2::new(
        hiroute_application_api::ObservationReadIntentV2::HomeValue(
            hiroute_application_api::ObservationHomeValueQueryV2 {
                period: Some(hiroute_application_api::ValuePeriodV1::SevenDays),
                session_id: None,
                currency: Some("USD".into()),
            },
        ),
    );
    let home_response = application.dispatch(LocalControlRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: "production-home-value".into(),
        principal: PrincipalV1::ambient_local_peer(),
        operation_id: home.operation().into(),
        payload: serde_json::to_value(&home).unwrap(),
        protected_grant: Some(grant(
            home.protected_operation(),
            CanonicalDigest::of(&home).unwrap(),
        )),
    });
    let home_data = home_response
        .data
        .expect("typed Home value through real Application");
    assert_eq!(home_data["amounts"][0]["known_sum_micros"], 160);
    assert_eq!(home_data["archive_boundary_partial"], false);

    let scoped_home = hiroute_application_api::ObservationReadRequestV2::new(
        hiroute_application_api::ObservationReadIntentV2::HomeValue(
            hiroute_application_api::ObservationHomeValueQueryV2 {
                period: Some(hiroute_application_api::ValuePeriodV1::SevenDays),
                session_id: Some("session-alpha".into()),
                currency: Some("USD".into()),
            },
        ),
    );
    let scoped_response = application.dispatch(LocalControlRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: "production-home-value-scoped".into(),
        principal: PrincipalV1::ambient_local_peer(),
        operation_id: scoped_home.operation().into(),
        payload: serde_json::to_value(&scoped_home).unwrap(),
        protected_grant: Some(grant(
            scoped_home.protected_operation(),
            CanonicalDigest::of(&scoped_home).unwrap(),
        )),
    });
    let scoped_data = scoped_response
        .data
        .expect("fresh session value through real Application");
    assert_eq!(scoped_data["retention_boundary_partial"], false);
    assert_eq!(
        scoped_data["to_ms"].as_i64().unwrap() - scoped_data["from_ms"].as_i64().unwrap(),
        7 * 86_400_000 - 1
    );
    assert_eq!(scoped_data["amounts"][0]["known_sum_micros"], 80);

    let scope: hiroute_application_api::ValueRequestV1 = serde_json::from_value(json!({})).unwrap();
    let values = application.dispatch(LocalControlRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: "production-values".to_owned(),
        principal: PrincipalV1::ambient_local_peer(),
        operation_id: "GetValue".to_owned(),
        payload: json!({}),
        protected_grant: Some(grant("GetValue", CanonicalDigest::of(&scope).unwrap())),
    });
    let values = values.data.unwrap();
    assert_eq!(
        values["to_ms"].as_i64().unwrap() - values["from_ms"].as_i64().unwrap(),
        7 * 86_400_000
    );
    let plans = values["plans"].as_array().unwrap();
    assert_eq!(plans.len(), 2);
    assert_eq!(plans[0]["agent_plan_id"], "plan/alpha");
    assert_eq!(plans[1]["agent_plan_id"], "plan/beta");
}

#[test]
fn worker_list_rereads_durable_display_facts_and_respects_managed_text_visibility() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::tests::worker_list_rereads_durable_display_facts_and_respects_managed_text_visibility",
    ) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let task_id = "task-worker-list";
    let run_id = "run-worker-list";
    let scope = ManagedTextScope {
        workspace_id: WorkspaceId::default(),
        task_id: task_id.into(),
        run_id: run_id.into(),
    };

    {
        let runtime = ProductionControlRuntime::open_with_release_catalog(
            temp.path(),
            crate::release_catalog::fixture_catalog(),
        )
        .unwrap();
        let goal = format!("{}\n{}", "界".repeat(140), "brief".repeat(120));
        let prompt = hiroute_application_api::DelegationTaskInputV1 {
            goal,
            context: "context must not enter the projection".into(),
            constraints: String::new(),
            acceptance_criteria: String::new(),
        }
        .prompt();
        let body = runtime
            .adapter
            .persist_task_input(&WorkspaceId::default(), task_id, run_id, &prompt, now)
            .unwrap();
        let acceptance = worker_list_acceptance(task_id, run_id, body, now);
        let accepted =
            DelegationRuntimePort::accept(runtime.adapter.as_ref(), &acceptance).unwrap();
        let finished = DelegationRuntimePort::checkpoint(
            runtime.adapter.as_ref(),
            &WorkspaceId::default(),
            run_id,
            accepted.progress.revision,
            "list-fixture-finished",
            &DelegationCheckpointV1::Progress {
                event: RunEventV1::LaunchFailedBeforeSpawn,
            },
        )
        .unwrap();
        assert!(finished.progress.workspace_releasable());

        let service = ApplicationService::new(runtime.application_ports());
        let listed = worker_list(&service, Some(1), None);
        assert!(listed.error.is_none(), "{listed:?}");
        let listed: DelegationListV1 = serde_json::from_value(listed.data.unwrap()).unwrap();
        assert_eq!(listed.tasks.len(), 1);
        assert!(listed.next_cursor.is_none());
        let item = &listed.tasks[0];
        assert_eq!(
            item.content_availability,
            DelegationContentAvailabilityV1::Available
        );
        assert_eq!(item.title.as_ref().unwrap().chars().count(), 140);
        assert_eq!(item.brief.as_ref().unwrap().chars().count(), 512);
        assert!(
            !item
                .brief
                .as_ref()
                .unwrap()
                .contains("context must not enter")
        );
        assert_eq!(item.run.ordinal, 1);
        assert!(item.run.continued_from.is_none());
        assert_eq!(item.created_at_ms, now);
        assert_eq!(item.latest_admission_sequence, accepted.admission_sequence);
        assert_eq!(item.run.admission_sequence, accepted.admission_sequence);
        assert_eq!(item.run.accepted_at_ms, Some(now));
        assert_eq!(
            item.run.accepted_time_state,
            DelegationAcceptedTimeStateV1::Recorded
        );
        assert_eq!(item.run.executor.harness, Some(WorkerHarnessV1::CodexCli));
        assert_eq!(item.run.executor.display_name.as_deref(), Some("Codex"));
        assert_eq!(
            item.run.executor.basis,
            WorkerExecutorPresentationBasisV1::FrozenPlan
        );
        assert_eq!(item.run.scope.canonical_cwd, "/workspace/worker-list");
        assert_eq!(
            item.run.scope.permission_policy,
            WorkerPermissionPolicyV1::DenyAll
        );
        assert_eq!(item.run.scope.deadline_ms, now + 60_000);

        let detail: DelegationGetV1 = serde_json::from_value(
            service
                .dispatch(LocalControlRequestV2 {
                    schema_version: LOCAL_CONTROL_SCHEMA_V2,
                    request_id: "worker-list-detail".into(),
                    principal: PrincipalV1::ambient_local_peer(),
                    operation_id: "WorkerStatus".into(),
                    payload: json!({"task_id":task_id}),
                    protected_grant: None,
                })
                .data
                .unwrap(),
        )
        .unwrap();
        assert_eq!(detail.task, *item);

        let injected = service.dispatch(LocalControlRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "worker-list-injected".into(),
            principal: PrincipalV1::ambient_local_peer(),
            operation_id: "WorkerList".into(),
            payload: json!({"caller":{"context_id":"forged"}}),
            protected_grant: None,
        });
        assert_eq!(
            injected.error.unwrap().code,
            hiroute_application_api::ErrorCode::InvalidArguments
        );
    }

    {
        let runtime = ProductionControlRuntime::open_with_release_catalog(
            temp.path(),
            crate::release_catalog::fixture_catalog(),
        )
        .unwrap();
        let service = ApplicationService::new(runtime.application_ports());
        let restarted: DelegationListV1 =
            serde_json::from_value(worker_list(&service, None, None).data.unwrap()).unwrap();
        assert_eq!(restarted.tasks[0].task_id, task_id);
        assert_eq!(restarted.tasks[0].created_at_ms, now);
        assert_eq!(
            restarted.tasks[0].title.as_deref(),
            Some("界".repeat(140).as_str())
        );
        assert_eq!(restarted.tasks[0].run.accepted_at_ms, Some(now));
        assert_eq!(
            restarted.tasks[0].run.executor.display_name.as_deref(),
            Some("Codex")
        );
        assert_eq!(
            restarted.tasks[0].content_availability,
            DelegationContentAvailabilityV1::Available
        );

        let current = i64::try_from(now + 1).unwrap();
        let preview = runtime
            .observation
            .managed_text_delete_preview(&scope, current, current)
            .unwrap();
        assert_eq!(preview.reference_count, 1);
        runtime
            .observation
            .managed_text_delete_apply(&preview)
            .unwrap();
        let hidden: DelegationListV1 =
            serde_json::from_value(worker_list(&service, None, None).data.unwrap()).unwrap();
        assert_eq!(hidden.tasks.len(), 1);
        assert_eq!(
            hidden.tasks[0].content_availability,
            DelegationContentAvailabilityV1::Unavailable
        );
        assert!(hidden.tasks[0].title.is_none());
        assert!(hidden.tasks[0].brief.is_none());
    }

    let runtime = ProductionControlRuntime::open_with_release_catalog(
        temp.path(),
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    let service = ApplicationService::new(runtime.application_ports());
    let hidden_after_restart: DelegationListV1 =
        serde_json::from_value(worker_list(&service, None, None).data.unwrap()).unwrap();
    assert_eq!(hidden_after_restart.tasks.len(), 1);
    assert_eq!(
        hidden_after_restart.tasks[0].content_availability,
        DelegationContentAvailabilityV1::Unavailable
    );
    assert!(hidden_after_restart.tasks[0].title.is_none());
    assert!(hidden_after_restart.tasks[0].brief.is_none());
}

#[test]
fn legacy_continuation_time_is_unavailable_but_frozen_executor_remains_exact() {
    let now = 1_700_000_000_000_u64;
    let mut acceptance = worker_list_acceptance(
        "task-legacy-continuation",
        "run-legacy-continuation",
        hiroute_domain::delegation::DelegationBodyRefV1 {
            opaque_id: "body-legacy".into(),
            scope_run_id: "run-legacy-continuation".into(),
            visibility_generation: 1,
            original_retention_deadline_ms: i64::try_from(now + 60_000).unwrap(),
        },
        now,
    );
    acceptance.task.plan.harness = WorkerHarnessV1::ClaudeCode;
    acceptance.run.ordinal = 2;
    acceptance.run.continued_from = Some("run-first".into());
    acceptance.run.admission_sequence = 17;
    acceptance.run.accepted_at_ms = None;

    let view = delegation_task_queries::run_view(&acceptance.task, &acceptance.run);
    assert_eq!(view.admission_sequence, 17);
    assert!(view.accepted_at_ms.is_none());
    assert_eq!(
        view.accepted_time_state,
        DelegationAcceptedTimeStateV1::LegacyUnavailable
    );
    assert_eq!(view.executor.harness, Some(WorkerHarnessV1::ClaudeCode));
    assert_eq!(view.executor.display_name.as_deref(), Some("Claude Code"));
    assert_eq!(
        view.executor.basis,
        WorkerExecutorPresentationBasisV1::FrozenPlan
    );
}

#[test]
fn worker_read_uses_signed_run_cursors_reports_gap_and_preserves_b6_privacy() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::tests::worker_read_uses_signed_run_cursors_reports_gap_and_preserves_b6_privacy",
    ) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let runtime = ProductionControlRuntime::open_with_release_catalog(
        temp.path(),
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let task_id = "task-worker-read";
    let run_id = "run-worker-read";
    let prompt = hiroute_application_api::DelegationTaskInputV1 {
        goal: "observe progress".into(),
        context: String::new(),
        constraints: String::new(),
        acceptance_criteria: String::new(),
    }
    .prompt();
    let goal = runtime
        .adapter
        .persist_task_input(&WorkspaceId::default(), task_id, run_id, &prompt, now)
        .unwrap();
    let acceptance = worker_list_acceptance(task_id, run_id, goal, now);
    let accepted = DelegationRuntimePort::accept(runtime.adapter.as_ref(), &acceptance).unwrap();
    assert_eq!(accepted.accepted_at_ms, Some(now));
    let service = ApplicationService::new(runtime.application_ports());

    let pending = worker_read(&service, run_id, None, Some(32));
    assert!(pending.error.is_none(), "{pending:?}");
    let pending: WorkerReadDataV1 = serde_json::from_value(pending.data.unwrap()).unwrap();
    assert_eq!(pending.content_state, WorkerReadContentStateV1::Pending);
    let pending_cursor = pending.next_cursor.unwrap();
    let wrong_purpose = worker_list(&service, Some(1), Some(&pending_cursor));
    assert_eq!(
        wrong_purpose.error.unwrap().code,
        hiroute_application_api::ErrorCode::InvalidArguments
    );

    let target = ManagedTextProgressTarget {
        scope: ManagedTextScope {
            workspace_id: WorkspaceId::default(),
            task_id: task_id.into(),
            run_id: run_id.into(),
        },
        created_at_ms: i64::try_from(now).unwrap(),
    };
    runtime
        .observation
        .managed_text_progress_write_batch(
            &target,
            "first progress",
            false,
            i64::try_from(now + 1).unwrap(),
        )
        .unwrap();
    let first = worker_read(&service, run_id, Some(&pending_cursor), Some(32));
    let first: WorkerReadDataV1 = serde_json::from_value(first.data.unwrap()).unwrap();
    assert_eq!(first.text.as_deref(), Some("first progress"));
    assert_eq!(first.segment.as_deref(), Some("0"));
    let old_cursor = first.next_cursor.unwrap();

    runtime
        .observation
        .managed_text_progress_write_batch(
            &target,
            "after gap",
            true,
            i64::try_from(now + 2).unwrap(),
        )
        .unwrap();
    let gap = worker_read(&service, run_id, Some(&old_cursor), Some(32));
    assert!(gap.data.is_none());
    assert_eq!(gap.error.unwrap().message_key, "worker.read.cursor_gap");
    assert_eq!(gap.next_actions[0].command_id, "worker.read");
    assert_eq!(gap.next_actions[0].reason_code, "worker.read.resume_gap");
    let recovery_cursor = gap.next_actions[0].input["cursor"].as_str().unwrap();
    assert_ne!(recovery_cursor, old_cursor);
    let recovered = worker_read(&service, run_id, Some(recovery_cursor), Some(32));
    let recovered: WorkerReadDataV1 = serde_json::from_value(recovered.data.unwrap()).unwrap();
    assert_eq!(recovered.text.as_deref(), Some("after gap"));
    assert_eq!(recovered.segment.as_deref(), Some("1"));

    let preview = runtime
        .observation
        .managed_text_delete_preview(
            &target.scope,
            i64::try_from(now + 2).unwrap(),
            i64::try_from(now + 2).unwrap(),
        )
        .unwrap();
    runtime
        .observation
        .managed_text_delete_apply(&preview)
        .unwrap();
    let deleted = worker_read(&service, run_id, None, Some(32));
    let deleted: WorkerReadDataV1 = serde_json::from_value(deleted.data.unwrap()).unwrap();
    assert_eq!(deleted.content_state, WorkerReadContentStateV1::Deleted);
    assert!(deleted.text.is_none());
    assert!(deleted.next_cursor.is_none());
    assert_eq!(deleted.truncated, None);
    let stale = worker_read(&service, run_id, Some(recovery_cursor), Some(32));
    assert_eq!(stale.error.unwrap().message_key, "worker.read.cursor_stale");
}

fn worker_list(
    service: &ApplicationService,
    limit: Option<u16>,
    cursor: Option<&str>,
) -> hiroute_application_api::MachineEnvelopeV2<serde_json::Value> {
    service.dispatch(LocalControlRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: "worker-list".into(),
        principal: PrincipalV1::ambient_local_peer(),
        operation_id: "WorkerList".into(),
        payload: serde_json::to_value(hiroute_application_api::WorkerListRequestV1 {
            title: None,
            cursor: cursor.map(str::to_owned),
            limit,
        })
        .expect("worker list request should serialize"),
        protected_grant: None,
    })
}

fn worker_read(
    service: &ApplicationService,
    run_id: &str,
    cursor: Option<&str>,
    max_bytes: Option<u32>,
) -> hiroute_application_api::MachineEnvelopeV2<serde_json::Value> {
    service.dispatch(LocalControlRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: "worker-read".into(),
        principal: PrincipalV1::ambient_local_peer(),
        operation_id: "WorkerRead".into(),
        payload: serde_json::to_value(hiroute_application_api::WorkerReadRequestV1 {
            run_id: run_id.into(),
            cursor: cursor.map(str::to_owned),
            max_bytes,
        })
        .unwrap(),
        protected_grant: None,
    })
}

pub(super) fn worker_list_acceptance(
    task_id: &str,
    run_id: &str,
    body: hiroute_domain::delegation::DelegationBodyRefV1,
    admitted_at_ms: u64,
) -> DelegationAcceptanceV1 {
    let workspace_id = WorkspaceId::default();
    let execution = WorkerExecutionIntentV1 {
        root_identity: "worker-list-root".into(),
        access: WorkspaceAccessV1::TrustedNative,
        tools: vec![WorkerToolV1::Read, WorkerToolV1::Edit, WorkerToolV1::Shell],
        network: WorkerNetworkV1::Allowed,
        duration_ms: 60_000,
        delegation_depth: 1,
    };
    let plan = DelegationPlanBindingV1 {
        authority_id: "authority-worker-list".into(),
        plan_id: AgentPlanId::parse("plan/worker-list").unwrap(),
        plan_revision: 7,
        plan_digest: CanonicalDigest::of_bytes(b"worker-list-plan"),
        publication_revision: 9,
        publication_digest: CanonicalDigest::of_bytes(b"worker-list-publication"),
        exact_reference: "exact-worker-list".into(),
        model_alias: "worker-list-model".into(),
        harness: WorkerHarnessV1::CodexCli,
        harness_configuration_digest: CanonicalDigest::of_bytes(b"worker-list-profile"),
    };
    DelegationAcceptanceV1 {
        task: DelegationTaskV1 {
            workspace_id: workspace_id.clone(),
            task_id: task_id.into(),
            parent_task_ref: None,
            plan,
            workspace: DelegationWorkspaceV1 {
                root_identity: "worker-list-root".into(),
                volume_identity: "worker-list-volume".into(),
                ancestry: vec!["root".into(), "worker-list-root".into()],
            },
            created_at_ms: admitted_at_ms,
            latest_run_id: run_id.into(),
            latest_admission_sequence: 0,
            title: Some(hiroute_domain::delegation::DelegationTaskTitleV1 {
                value: "界".repeat(140),
                source: hiroute_domain::delegation::DelegationTaskTitleSourceV1::Goal,
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
            idempotency_key: "worker-list-start".into(),
            request_digest: CanonicalDigest::of_bytes(b"worker-list-request"),
            admission_sequence: 0,
            accepted_at_ms: None,
            execution_owner_ref: "worker-list-owner".into(),
            lease_id: "worker-list-lease".into(),
            daemon_epoch: "worker-list-epoch".into(),
            permit_id: format!("run-config/{run_id}"),
            permit_generation: 1,
            configuration: DelegationRunConfigurationV1 {
                format_version: DELEGATION_RUN_CONFIGURATION_VERSION_V1,
                scope_id: format!("run-config/{run_id}"),
                generation: 1,
                canonical_workspace_path: "/workspace/worker-list".into(),
                permission_policy: WorkerPermissionPolicyV1::DenyAll,
            },
            execution,
            deadline_ms: admitted_at_ms + 60_000,
            lease_revoked: false,
            launch_nonce: "worker-list-launch".into(),
            process: None,
            session: None,
            progress: RunProgressV1::default(),
            stop_evidence: None,
            result_body: None,
            result_incomplete: false,
        },
        title_lookup_key: Some("worker-list-title-key".into()),
        expected_latest_run_id: None,
        admitted_at_ms,
    }
}

#[test]
fn failed_live_check_with_known_usage_is_excluded_by_real_receipt_identity() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::tests::failed_live_check_with_known_usage_is_excluded_by_real_receipt_identity",
    ) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let runtime = ProductionControlRuntime::open_with_release_catalog(
        temp.path(),
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let stream = ObservationStreamV1 {
        producer_id: ProducerId::parse("producer-failed-probe").unwrap(),
        producer_epoch: ProducerEpoch::parse("epoch-failed-probe").unwrap(),
        stream_id: StreamId::parse("stream-failed-probe").unwrap(),
    };
    let identity = NativeAgentObservationIdentityV1::CodexThread {
        thread_id: "failed-probe-thread".into(),
    };
    let mut started = envelope(
        &stream,
        "failed-probe",
        "plan/alpha",
        1,
        now_ms - 2_000,
        ExecutionFactV1::AttemptStarted {
            ordinal: 1,
            candidate_id: "candidate-failed-probe".into(),
            stable_binding_id: "binding-failed-probe".into(),
            profile_digest: CanonicalDigest::of_bytes(b"failed-probe-profile").into(),
            credential_ref: "credential-failed-probe".into(),
            key_id: "key-failed-probe".into(),
            provider_name: "test".into(),
            request_model: "native-failed-probe".into(),
            upstream_protocol: IngressProtocolV1::Responses,
            model_configuration_id: "hiroute/test".into(),
            adapter_revision: "adapter-v1".into(),
            start_reason: "initial_candidate".into(),
            previous_attempt_id: None,
        },
    );
    let session = derive_native_agent_observation_session_id(
        &WorkspaceId::default(),
        &runtime.observation_workspace_key(),
        &started.trust,
        &identity,
    )
    .unwrap();
    started.correlation.conversation_id = session.clone();
    started.attempt_id = Some(AttemptId::parse("attempt-failed-probe").unwrap());
    let mut usage = envelope(
        &stream,
        "failed-probe",
        "plan/alpha",
        2,
        now_ms - 1_000,
        ExecutionFactV1::UsageAndCache {
            ordinal: 1,
            source: hiroute_domain::UsageSourceV1::AcceptedCanonicalModelEvent,
            input_tokens: Some(100),
            output_tokens: Some(25),
            billable_tokens: Some(125),
            cache_read_tokens: Some(40),
            cache_write_tokens: None,
            reasoning_tokens: None,
            input_provenance: hiroute_domain::UsageProvenanceV1::Reported,
            output_provenance: hiroute_domain::UsageProvenanceV1::Reported,
            billable_provenance: hiroute_domain::UsageProvenanceV1::Reported,
            cache_read_provenance: hiroute_domain::UsageProvenanceV1::Reported,
            cache_write_provenance: hiroute_domain::UsageProvenanceV1::Unknown,
            reasoning_provenance: hiroute_domain::UsageProvenanceV1::Unknown,
            effective_cost_micros: None,
            cost_class: None,
            cache_status: hiroute_domain::CacheStatusV1::ConfirmedUsage,
        },
    );
    usage.correlation.conversation_id = session.clone();
    usage.attempt_id = started.attempt_id.clone();
    let mut failed = envelope(
        &stream,
        "failed-probe",
        "plan/alpha",
        3,
        now_ms,
        ExecutionFactV1::RequestFinished {
            outcome: ExecutionRequestOutcomeV1::Failed,
            attempts_started: 1,
            attempts_finished: 0,
            accepted_attempt_ordinal: None,
            facts_completeness: FactsCompleteness::Partial,
        },
    );
    failed.correlation.conversation_id = session.clone();
    let trust = started.trust.clone();
    let writer = LocalObservationWriter::new(runtime.observation.clone());
    let channel = hiroute_observation::FactChannel::new(stream, 128 * 1024);
    for fact in [started, usage, failed] {
        assert_eq!(channel.offer(fact), OfferOutcome::Accepted);
        assert!(matches!(
            writer.consume_fact(&channel),
            WriterCycleOutcome::Ack(_)
        ));
    }
    runtime.observation.settle_pending_valuations(16).unwrap();
    let reader = hiroute_domain::ObservationReaderContext::local_user(
        WorkspaceId::default(),
        "local".into(),
        1,
        now_ms + 10_000,
        false,
        false,
    )
    .unwrap();
    let query = ObservationValueQueryV2 {
        from_ms: now_ms - 5_000,
        to_ms: now_ms + 1,
        session_id: Some(session.to_string()),
        plan_id: None,
        currency: None,
    };
    let before = runtime
        .observation
        .observed_value_totals(&reader, &query, now_ms)
        .unwrap();
    assert_eq!(before.excluded_requests, 0);
    assert_eq!(before.usage[0].known_sum, Some(100));

    // The human CLI now emits this current report intent, not a ValueSnapshot
    // ledger query. These production facts deliberately contain no legacy value.
    let application = ApplicationService::new(runtime.application_ports());
    let value_request = hiroute_application_api::ObservationReadRequestV2::new(
        hiroute_application_api::ObservationReadIntentV2::ValueReport(
            hiroute_application_api::ObservationValueReportOptionsV2 {
                from_ms: Some(query.from_ms),
                to_ms: Some(query.to_ms),
                session_id: Some(session.clone()),
                group_by: hiroute_domain::ValueGroupByV1::Day,
                ..Default::default()
            },
        ),
    );
    let response = application.dispatch(LocalControlRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: "current-value-report".into(),
        principal: PrincipalV1::ambient_local_peer(),
        operation_id: "GetValue".into(),
        payload: serde_json::to_value(value_request).unwrap(),
        protected_grant: None,
    });
    assert!(response.error.is_none(), "{response:?}");
    let report = response.data.unwrap();
    assert_eq!(report["summary"]["usage"][0]["known_sum"], 100);
    assert_eq!(report["summary"]["usage"][1]["known_sum"], 25);
    assert_eq!(report["day_timezone"], "UTC");
    assert_eq!(
        report["days"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|day| day["usage"][0]["known_sum"].as_u64())
            .sum::<u64>(),
        100
    );

    assert!(
        runtime
            .adapter
            .verify_live_receipt(
                &trust,
                &[identity],
                &CanonicalDigest::of_bytes(b"failed-client-output"),
                Instant::now() + Duration::from_secs(1),
            )
            .is_err()
    );
    let after = runtime
        .observation
        .observed_value_totals(&reader, &query, now_ms)
        .unwrap();
    assert_eq!(after.excluded_requests, 1);
    assert!(after.usage.iter().all(|metric| metric.known_sum.is_none()));
    assert_eq!(after.input_cache_hit.total_attempt_count, 0);
}

fn ingest_value(store: &Arc<LocalObservationStore>, suffix: &str, plan: &str, occurred_at_ms: i64) {
    let stream = ObservationStreamV1 {
        producer_id: ProducerId::parse(format!("producer-{suffix}")).unwrap(),
        producer_epoch: ProducerEpoch::parse(format!("epoch-{suffix}")).unwrap(),
        stream_id: StreamId::parse(format!("stream-{suffix}")).unwrap(),
    };
    let writer = LocalObservationWriter::new(store.clone());
    let channel = hiroute_observation::FactChannel::new(stream.clone(), 128 * 1024);
    for envelope in [
        envelope(
            &stream,
            suffix,
            plan,
            1,
            occurred_at_ms,
            ExecutionFactV1::ValueSnapshot {
                traffic_kind: TrafficKind::Normal,
                usage: UsageFactsV1 {
                    input_tokens: 10,
                    output_tokens: 5,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                    reasoning_tokens: 1,
                },
                value: FrozenValueFactsV1 {
                    agent_plan_id: AgentPlanId::parse(plan).unwrap(),
                    currency: "USD".to_owned(),
                    billing_unit: "micro_usd".to_owned(),
                    price_version: "price-v1".to_owned(),
                    price_override_revision: None,
                    baseline_api_equivalent_cost_micros: Some(100),
                    chosen_api_equivalent_cost_micros: Some(80),
                    actual_incremental_cost_micros: Some(20),
                    routing_savings_micros: Some(20),
                    entitlement_savings_micros: Some(60),
                    estimated_total_savings_micros: Some(80),
                },
            },
        ),
        envelope(
            &stream,
            suffix,
            plan,
            2,
            occurred_at_ms + 1,
            ExecutionFactV1::RequestFinished {
                outcome: ExecutionRequestOutcomeV1::Accepted,
                attempts_started: 0,
                attempts_finished: 0,
                accepted_attempt_ordinal: None,
                facts_completeness: FactsCompleteness::Unknown,
            },
        ),
    ] {
        assert_eq!(channel.offer(envelope), OfferOutcome::Accepted);
        assert!(matches!(
            writer.consume_fact(&channel),
            WriterCycleOutcome::Ack(_)
        ));
    }
}

fn envelope(
    stream: &ObservationStreamV1,
    suffix: &str,
    plan: &str,
    sequence: u64,
    occurred_at_ms: i64,
    fact: ExecutionFactV1,
) -> ExecutionFactEnvelopeV1 {
    ExecutionFactEnvelopeV1 {
        pricing: None,
        schema_version: EXECUTION_FACT_SCHEMA_V2.to_owned(),
        schema_digest: CanonicalDigest::parse(EXECUTION_FACT_PORT_DIGEST_V2).unwrap(),
        channel: ExecutionFactChannelV1::ExecutionFact,
        producer: ExecutionProducerV1 {
            component: ExecutionProducerComponentV1::GatewayExecution,
            revision: "gateway-production-shape".to_owned(),
            stream: stream.clone(),
        },
        sequence,
        event_id: EventId::parse(format!("event-{suffix}-{sequence}")).unwrap(),
        correlation: ExecutionCorrelationV1 {
            workspace_id: WorkspaceId::default(),
            conversation_id: SessionId::parse(format!("session-{suffix}")).unwrap(),
            session_scope: SessionScopeV1::Conversation,
            correlation_provenance: CorrelationProvenance::AgentSupplied,
            turn_id: TurnId::parse(format!("turn-{suffix}")).unwrap(),
            request_id: LogicalRequestId::parse(format!("request-{suffix}")).unwrap(),
        },
        attempt_id: None,
        trust: FrozenExecutionTrustV1 {
            authority_id: "authority-local".to_owned(),
            authority_epoch: 1,
            served_model_id: "hiroute/test".to_owned(),
            selector_source: SelectorSourceV1::TrustedModelAlias,
            agent_plan_id: Some(AgentPlanId::parse(plan).unwrap()),
            route: hiroute_domain::ModelRequestRouteV2::Plan {
                revision: 1,
                semantic_digest: CanonicalDigest::of_bytes(plan.as_bytes()),
            },
            plan_display_name: Some("Test plan".to_owned()),
            gateway_publication_revision: "1".to_owned(),
            gateway_publication_digest: CanonicalDigest::of_bytes(b"publication"),
            grant_id: "grant-test".to_owned(),
            grant_generation: 1,
            ingress_protocol: IngressProtocolV1::Responses,
        },
        occurred_at_unix_nanos: u64::try_from(occurred_at_ms).unwrap() * 1_000_000,
        fact,
        loss_watermark: None,
        completeness_delta: Some(CompletenessDeltaV1::Unknown),
    }
}
