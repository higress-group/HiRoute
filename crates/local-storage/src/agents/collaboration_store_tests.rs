use super::*;
use hiroute_domain::{AgentCollaborationCredential, AgentCollaborationGrant, OperationState};
use std::collections::BTreeSet;

fn begin(control: &ControlStore, key: &str) -> OperationV1 {
    let operation = agent_operation(
        agent_transaction_plan(AgentConnectionTransactionKindV1::Apply, None),
        key,
    );
    control
        .grant_apply_capability(key, &operation, i64::MAX)
        .unwrap();
    let authorization = control
        .verify_apply_authorization(
            &ProtectedApplyCapability::new(key.to_owned()).unwrap(),
            &operation.workspace_id,
            &operation.idempotency.principal,
            &operation.idempotency.operation_kind,
            &operation.accepted_digest,
            &operation.expected_revisions,
        )
        .unwrap();
    assert_eq!(
        control.begin_operation(&operation, &authorization).unwrap(),
        BeginOperationOutcome::Created
    );
    operation
}
fn issued() -> AgentCollaborationGrant {
    AgentCollaborationGrant::issue(
        WorkspaceId::default(),
        "agent-context/one".into(),
        "collaboration-grant/one".into(),
        1,
        BTreeSet::new(),
        &AgentCollaborationCredential::from_csprng_entropy([9; 32]),
    )
    .unwrap()
}

#[test]
fn collaboration_store_reopens_revocation_with_same_operation_and_no_old_grant() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("data/control.db");
    let backups = directory.path().join("backups");
    let control =
        ControlStore::open(&crate::test_storage_authority(), &database, &backups).unwrap();
    let mut enable = begin(&control, "collab-enable");
    let grant = issued();
    control
        .store_collaboration_grant(&enable.operation_id, 0, &grant)
        .unwrap();
    control
        .store_collaboration_grant(&enable.operation_id, 0, &grant)
        .unwrap();
    enable.state = OperationState::Succeeded;
    add_component_file_receipts(&mut enable);
    control.finish_operation(&mut enable).unwrap();
    let revoke = begin(&control, "collab-revoke");
    let checkpoint = grant
        .plan_revocation(revoke.operation_id.clone(), 1)
        .unwrap();
    control
        .record_collaboration_revocation(&checkpoint)
        .unwrap();
    drop(control);
    let reopened =
        ControlStore::open(&crate::test_storage_authority(), &database, &backups).unwrap();
    reopened
        .record_collaboration_revocation(&checkpoint)
        .unwrap();
    assert_eq!(
        reopened
            .collaboration_grant(&WorkspaceId::default(), &grant.grant_id)
            .unwrap(),
        Some(checkpoint.after.clone())
    );
    assert_eq!(
        reopened
            .collaboration_revocations(&WorkspaceId::default())
            .unwrap(),
        vec![(checkpoint.clone(), None, false)]
    );
    reopened
        .acknowledge_collaboration_cancel(&checkpoint, "cancel-receipt/original")
        .unwrap();
    reopened
        .complete_collaboration_file_cleanup(&checkpoint)
        .unwrap();
    assert_eq!(
        reopened
            .collaboration_revocations(&WorkspaceId::default())
            .unwrap(),
        vec![(checkpoint, Some("cancel-receipt/original".into()), true)]
    );
    assert!(
        reopened
            .store_collaboration_grant(&revoke.operation_id, 0, &grant)
            .is_err()
    );
}

#[test]
fn collaboration_store_transaction_failure_never_persists_half_a_revocation() {
    let directory = tempdir().unwrap();
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        directory.path().join("data/control.db"),
        directory.path().join("backups"),
    )
    .unwrap();
    let operation = begin(&control, "collab-atomic");
    let grant = issued();
    control
        .store_collaboration_grant(&operation.operation_id, 0, &grant)
        .unwrap();
    control.with_connection(|connection| connection.execute_batch(
        "CREATE TRIGGER fail_collaboration_update BEFORE UPDATE ON agent_collaboration_grants BEGIN SELECT RAISE(ABORT, 'injected'); END;"
    ).unwrap());
    let checkpoint = grant
        .plan_revocation(operation.operation_id.clone(), 1)
        .unwrap();
    assert!(
        control
            .record_collaboration_revocation(&checkpoint)
            .is_err()
    );
    assert!(
        control
            .collaboration_revocations(&WorkspaceId::default())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        control
            .collaboration_grant(&WorkspaceId::default(), &grant.grant_id)
            .unwrap(),
        Some(grant)
    );
    control.with_connection(|connection| {
        connection
            .execute_batch("DROP TRIGGER fail_collaboration_update;")
            .unwrap()
    });
    control
        .record_collaboration_revocation(&checkpoint)
        .unwrap();
    assert_eq!(
        control
            .collaboration_revocations(&WorkspaceId::default())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn collaboration_skill_refs_survive_reopen_and_stale_cleanup_cannot_restore_them() {
    use hiroute_domain::{MANAGED_COLLABORATION_SKILL_SCHEMA, ManagedCollaborationSkill};
    let directory = tempdir().unwrap();
    let database = directory.path().join("data/control.db");
    let backups = directory.path().join("backups");
    let control =
        ControlStore::open(&crate::test_storage_authority(), &database, &backups).unwrap();
    let workspace = WorkspaceId::default();
    let mut install = begin(&control, "skill-install");
    let first = ManagedCollaborationSkill {
        schema: MANAGED_COLLABORATION_SKILL_SCHEMA.into(),
        root_ref: "skill-root/shared".into(),
        template_revision: "test/1".into(),
        content_digest: CanonicalDigest::of_bytes(b"skill"),
        file_ownership: hiroute_domain::CollaborationSkillFileOwnership::Managed,
        contexts: BTreeSet::from(["context/one".into(), "context/two".into()]),
        revision: 1,
        file_effect: Some(hiroute_domain::SkillFileEffectRef {
            operation_id: install.operation_id.clone(),
            effect_id: "agent-connection-routing-skill".into(),
            target: "agents/skill-root/shared/routing-skill".into(),
            intent_digest: CanonicalDigest::of_bytes(b"protected-file-effect-reference-fixture"),
        }),
    };
    control
        .store_skill_installation(&workspace, &install.operation_id, 0, &first)
        .unwrap();
    install.state = OperationState::Succeeded;
    add_component_file_receipts(&mut install);
    control.finish_operation(&mut install).unwrap();
    let remove = begin(&control, "skill-remove");
    let mut next = first.clone();
    next.revision = 2;
    next.contexts.remove("context/one");
    control
        .store_skill_installation(&workspace, &remove.operation_id, 1, &next)
        .unwrap();
    drop(control);
    let reopened =
        ControlStore::open(&crate::test_storage_authority(), &database, &backups).unwrap();
    reopened
        .store_skill_installation(&workspace, &remove.operation_id, 1, &next)
        .unwrap();
    assert_eq!(
        reopened
            .skill_installation(&workspace, &first.root_ref)
            .unwrap(),
        Some(next)
    );
    assert!(
        reopened
            .store_skill_installation(&workspace, &remove.operation_id, 0, &first)
            .is_err()
    );
}
