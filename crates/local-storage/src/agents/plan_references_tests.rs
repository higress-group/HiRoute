use super::*;
use hiroute_domain::*;
use std::collections::BTreeSet;

fn model_plan(restoring: bool) -> TransactionPlanV1 {
    let plan_id = AgentPlanId::parse("plan/review").unwrap();
    let published = PublishedAgentPlanV1 {
        agent_plan_id: plan_id.clone(),
        model_alias: ModelAlias::parse("hiroute/0011223344556677").unwrap(),
        display_name: AgentPlanDisplayName::parse("Review").unwrap(),
        purpose: AgentPlanPurpose::parse("Review code").unwrap(),
        agent_plan_revision: 1,
        active: true,
        supported_ingress: BTreeSet::from([AgentIngressProtocolV1::Responses]),
    };
    let grant = AgentPlanGrantV1::derive(
        AgentIngressProtocolV1::Responses,
        plan_id.clone(),
        BTreeSet::from([plan_id]),
        &[published],
    )
    .unwrap();
    let digest = CanonicalDigest::of_bytes(b"reference-fixture");
    let connection = AgentConnectionV1 {
        schema: AGENT_CONNECTION_SCHEMA_V1.into(),
        agent_id: "agent.codex".into(),
        profile_id: "default".into(),
        integration_profile_ref: "codex.profile.v1".into(),
        protocol: AgentIngressProtocolV1::Responses,
        activation_mode: AgentActivationModeV1::ManagedConfiguration,
        grant,
        native_subagent_routing: false,
        catalog_digest: None,
        overlay_digest: None,
        revision: 7,
    };
    let kind = if restoring {
        AgentConnectionTransactionKindV1::Restore
    } else {
        AgentConnectionTransactionKindV1::Apply
    };
    let spec = ChangeSpecV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        command_id: kind.command_id().into(),
        resource_id: Some("agent-connection/agent.codex/default".into()),
        desired_state: json!({
            "agent_id": connection.agent_id, "profile_id": connection.profile_id,
            "integration_profile_ref": connection.integration_profile_ref,
            "installed_version": "diagnostic", "observation_digest": digest,
            "grant_digest": connection.grant.digest, "publication_digest": digest, "config_change_digest": digest,
            "restore_point_ref": "restore/one"
        }),
    };
    let subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
        "agent.codex",
        "default",
        "codex.profile.v1",
    )
    .unwrap();
    let control = AgentConnectionControlIntentV1::from_registered_planner(
        kind,
        subject,
        &spec,
        &json!({
            "connection": connection, "publication_digest": digest, "config_change_digest": digest,
            "guidance_disposition": "no_registered_tool"
        }),
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
            None,
            &json!({"fixture": true}),
            0o644,
        )
        .unwrap()
    })
    .collect();
    TransactionPlanV1::from_agent_connection_planner(spec, control, external).unwrap()
}
fn begin_plan(control: &ControlStore, restoring: bool, key: &str) -> OperationV1 {
    let plan = model_plan(restoring);
    let scope = IdempotencyScopeV1::new(
        "interactive-user",
        if restoring {
            "ApplyAgentConnectionRestore"
        } else {
            "ApplyAgentConnectionChange"
        },
        key,
    )
    .unwrap();
    let digest = CanonicalDigest::of_bytes(key.as_bytes());
    let operation = OperationV1::new(
        OperationId::derive(&WorkspaceId::default(), &scope, &digest),
        WorkspaceId::default(),
        scope,
        digest.clone(),
        digest,
        RevisionSetV1 {
            target: 0,
            dependencies: BTreeMap::new(),
        },
        plan,
    )
    .unwrap();
    control
        .grant_apply_capability(key, &operation, i64::MAX)
        .unwrap();
    let auth = control
        .verify_apply_authorization(
            &ProtectedApplyCapability::new(key.into()).unwrap(),
            &operation.workspace_id,
            &operation.idempotency.principal,
            &operation.idempotency.operation_kind,
            &operation.accepted_digest,
            &operation.expected_revisions,
        )
        .unwrap();
    assert_eq!(
        control.begin_operation(&operation, &auth).unwrap(),
        BeginOperationOutcome::Created
    );
    operation
}
#[test]
fn agent_plan_references_join_independent_grants_restore_and_workspace() {
    let dir = tempdir().unwrap();
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        dir.path().join("data/control.db"),
        dir.path().join("backups"),
    )
    .unwrap();
    let workspace = WorkspaceId::default();
    let plan = AgentPlanId::parse("plan/review").unwrap();
    assert!(
        control
            .agent_plan_references(&workspace, &plan)
            .unwrap()
            .references
            .is_empty()
    );
    let mut operation = begin_plan(&control, false, "references-enable");
    let grant = AgentCollaborationGrant::issue(
        workspace.clone(),
        "context/claude".into(),
        "collaboration-grant/one".into(),
        1,
        BTreeSet::from([plan.clone()]),
        &AgentCollaborationCredential::from_csprng_entropy([9; 32]),
    )
    .unwrap();
    control
        .store_collaboration_grant(&operation.operation_id, 0, &grant)
        .unwrap();
    assert!(control.agent_plan_references(&workspace, &plan).is_err());
    for step in &mut operation.steps {
        step.status = OperationStepStatus::Applied;
    }
    operation.state = OperationState::Succeeded;
    control.finish_operation(&mut operation).unwrap();
    let initial = control.agent_plan_references(&workspace, &plan).unwrap();
    assert_eq!(initial.references.len(), 3);
    assert_eq!(
        initial,
        control.agent_plan_references(&workspace, &plan).unwrap()
    );
    assert!(
        initial
            .references
            .iter()
            .any(|r| r.kind == AgentPlanReferenceKind::DefaultModel && r.revision == 7)
    );
    assert!(
        control
            .agent_plan_references(&WorkspaceId::parse("workspace/other").unwrap(), &plan)
            .unwrap()
            .references
            .is_empty()
    );
    let mut restore = begin_plan(&control, true, "references-restore");
    for step in &mut restore.steps {
        step.status = OperationStepStatus::Applied;
    }
    restore.state = OperationState::Succeeded;
    control.finish_operation(&mut restore).unwrap();
    let after = control.agent_plan_references(&workspace, &plan).unwrap();
    assert_eq!(after.references.len(), 1);
    assert_eq!(
        after.references[0].kind,
        AgentPlanReferenceKind::CollaborationAllowed
    );
    assert_ne!(initial.facts_digest, after.facts_digest);
}
#[test]
fn agent_plan_references_corrupt_projection_is_not_no_references() {
    let dir = tempdir().unwrap();
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        dir.path().join("data/control.db"),
        dir.path().join("backups"),
    )
    .unwrap();
    let mut operation = begin_plan(&control, false, "references-corrupt");
    for step in &mut operation.steps {
        step.status = OperationStepStatus::Applied;
    }
    operation.state = OperationState::Succeeded;
    control.finish_operation(&mut operation).unwrap();
    control.with_connection(|db| {
        db.execute("UPDATE operations SET operation_json='{}'", [])
            .unwrap()
    });
    assert!(
        control
            .agent_plan_references(
                &WorkspaceId::default(),
                &AgentPlanId::parse("plan/review").unwrap()
            )
            .is_err()
    );
}
