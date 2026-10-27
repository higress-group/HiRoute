use hiroute_domain::delegation::WorkerHarnessV1;
use hiroute_domain::delegation::*;
use hiroute_domain::*;
use hiroute_local_storage::{ApplyCapabilityRegistrationV1, LocalStorageSet};
use serde_json::json;
use std::collections::BTreeSet;

fn agent_transaction_plan(
    transaction: AgentConnectionTransactionKindV1,
    before: Option<CanonicalDigest>,
) -> TransactionPlanV1 {
    let spec = ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: transaction.command_id().to_owned(),
        resource_id: Some("agent-connection/codex-default".to_owned()),
        desired_state: match transaction {
            AgentConnectionTransactionKindV1::Settings => {
                panic!("legacy fixture does not model independent settings")
            }
            AgentConnectionTransactionKindV1::Apply => json!({
                "agent_id": "agent.codex",
                "profile_id": "default",
                "default_agent_plan_id": "plan.primary"
            }),
            AgentConnectionTransactionKindV1::Restore => json!({
                "agent_id": "agent.codex",
                "profile_id": "default",
                "restore_point_ref": "restore.codex.1"
            }),
        },
    };
    let subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
        "agent.codex",
        "default",
        "codex.profile.v1",
    )
    .unwrap();
    let control = AgentConnectionControlIntentV1::from_registered_planner(
        transaction,
        subject,
        &spec,
        &json!({"connection_revision": 4, "desired_digest": "connection.digest.v4"}),
    )
    .unwrap();
    let external = [
        AgentConnectionEffectRoleV1::GrantScopedPublication,
        AgentConnectionEffectRoleV1::ManagedConfiguration,
    ]
    .into_iter()
    .map(|role| {
        ExternalEffectIntentV1::from_agent_connection_planner(
            &control,
            role,
            if role == AgentConnectionEffectRoleV1::ManagedConfiguration {
                before.clone()
            } else {
                None
            },
            &json!({"role": format!("{role:?}"), "content_digest": "sha256:fixture"}),
            if role == AgentConnectionEffectRoleV1::ManagedConfiguration {
                0o640
            } else {
                0o644
            },
        )
        .unwrap()
    })
    .collect();
    TransactionPlanV1::from_agent_connection_planner(spec, control, external).unwrap()
}

pub fn begin(stores: &LocalStorageSet, key: &str) -> OperationV1 {
    let workspace = WorkspaceId::default();
    let scope =
        IdempotencyScopeV1::new("interactive-user", "ApplyAgentConnectionChange", key).unwrap();
    let digest = CanonicalDigest::of_bytes(key.as_bytes());
    let revisions = stores.control().current_revisions(&workspace).unwrap();
    let operation = OperationV1::new(
        OperationId::derive(&workspace, &scope, &digest),
        workspace.clone(),
        scope,
        digest.clone(),
        digest.clone(),
        revisions.clone(),
        agent_transaction_plan(AgentConnectionTransactionKindV1::Apply, None),
    )
    .unwrap();
    let capability = format!("fixture-exact-apply-capability-{key}-0123456789");
    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        + 120;
    stores
        .apply_capability_registrar()
        .register(
            ApplyCapabilityRegistrationV1::from_protected_launcher(
                capability.clone(),
                "interactive-user",
                workspace.clone(),
                "ApplyAgentConnectionChange",
                digest,
                revisions,
                expires,
            )
            .unwrap(),
        )
        .unwrap();
    let auth = stores
        .control()
        .verify_apply_authorization(
            &ProtectedApplyCapability::new(capability).unwrap(),
            &workspace,
            &operation.idempotency.principal,
            &operation.idempotency.operation_kind,
            &operation.accepted_digest,
            &operation.expected_revisions,
        )
        .unwrap();
    assert_eq!(
        stores.control().begin_operation(&operation, &auth).unwrap(),
        BeginOperationOutcome::Created
    );
    operation
}
pub fn finish(stores: &LocalStorageSet, mut operation: OperationV1) {
    // This authority-only fixture does not execute native files. Model the exact
    // adapter receipt required by its declared file intent; file authentication
    // is covered by the real artifact and publication-process scenarios.
    let receipts = operation
        .plan
        .external()
        .iter()
        .filter(|intent| intent.kind() == OwnedEffectKind::AgentArtifact)
        .map(|intent| OwnedEffectV1 {
            effect_id: intent.effect_id().into(),
            kind: intent.kind(),
            target: intent.target().into(),
            before_fingerprint: intent.before_fingerprint().cloned(),
            after_fingerprint: Some(CanonicalDigest::of_bytes(b"authority-fixture-file")),
            compensation: json!({"schema":"hiroute.managed-artifact-compensation/v1",
                "operation_id":operation.operation_id,"effect_id":intent.effect_id()})
            .into(),
        })
        .collect();
    let files = operation.step_mut(OperationStepKind::ApplyAgentArtifacts);
    files.effects = receipts;
    files.status = OperationStepStatus::Applied;
    operation.state = OperationState::Succeeded;
    stores.control().finish_operation(&mut operation).unwrap();
}
pub fn permit() -> WorkspaceExecutionPermitV1 {
    WorkspaceExecutionPermitV1 {
        permit_id: "permit".into(),
        generation: 1,
        root_identity: "root-a".into(),
        access: WorkspaceAccessV1::TrustedNative,
        tools: vec![WorkerToolV1::Read],
        network: WorkerNetworkV1::Allowed,
        expires_at_ms: 10000,
        max_run_ms: 1000,
        max_concurrent: 2,
        revoked: false,
    }
}

pub fn configured(
    stores: &LocalStorageSet,
) -> (AgentCollaborationGrant, AgentCollaborationCredential) {
    let op = begin(stores, "enable");
    let material = AgentCollaborationCredential::from_csprng_entropy([7; 32]);
    let grant = AgentCollaborationGrant::issue(
        WorkspaceId::default(),
        "owner".into(),
        "collaboration-grant/one".into(),
        1,
        BTreeSet::from([AgentPlanId::parse("plan").unwrap()]),
        &material,
    )
    .unwrap();
    stores
        .secrets()
        .prepare_collaboration_credential(&op.operation_id, &grant, &material)
        .unwrap();
    stores
        .control()
        .commit_permit(&DelegationPermitMutationV1 {
            workspace: WorkspaceId::default(),
            operation: op.operation_id.clone(),
            before: None,
            after: permit(),
        })
        .unwrap();
    stores
        .control()
        .store_collaboration_grant(&op.operation_id, 0, &grant)
        .unwrap();
    let prepared = stores
        .secrets()
        .recover_prepared_collaboration_credential(&op.operation_id, &grant)
        .unwrap();
    assert_eq!(prepared.expose(), material.expose());
    finish(stores, op);
    (grant, prepared)
}

pub fn sample(task_id: &str, run_id: &str, root: &str) -> DelegationAcceptanceV1 {
    let workspace_id = WorkspaceId::default();
    let body = DelegationBodyRefV1 {
        opaque_id: format!("body-{task_id}-{run_id}"),
        scope_run_id: run_id.into(),
        visibility_generation: 0,
        original_retention_deadline_ms: 10_000,
    };
    let execution = WorkerExecutionIntentV1 {
        root_identity: root.into(),
        access: WorkspaceAccessV1::TrustedNative,
        tools: vec![WorkerToolV1::Read],
        network: WorkerNetworkV1::Allowed,
        duration_ms: 1000,
        delegation_depth: 1,
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
