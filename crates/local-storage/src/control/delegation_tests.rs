use super::*;
use hiroute_domain::OperationState;
use hiroute_domain::delegation::*;

fn begin(control: &ControlStore, key: &str) -> OperationV1 {
    let operation = agent_operation(
        agent_transaction_plan(AgentConnectionTransactionKindV1::Apply, None),
        key,
    );
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
fn permit(generation: u64) -> WorkspaceExecutionPermitV1 {
    WorkspaceExecutionPermitV1 {
        permit_id: "permit".into(),
        generation,
        root_identity: "root".into(),
        access: WorkspaceAccessV1::TrustedNative,
        tools: vec![WorkerToolV1::Read],
        network: WorkerNetworkV1::Allowed,
        expires_at_ms: 10000,
        max_run_ms: 1000,
        max_concurrent: 2,
        revoked: false,
    }
}
#[test]
fn delegation_permit_original_writer_cas_idempotency_and_reopen() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("control.db");
    let backups = dir.path().join("backups");
    let control = ControlStore::open(&crate::test_storage_authority(), &path, &backups).unwrap();
    let mut operation = begin(&control, "permit-create");
    let first = DelegationPermitMutationV1 {
        workspace: operation.workspace_id.clone(),
        operation: operation.operation_id.clone(),
        before: None,
        after: permit(1),
    };
    let mut wrong = first.clone();
    wrong.operation = OperationId::parse("op_00112233445566778899aabbccddeeff").unwrap();
    assert_eq!(
        control.commit_permit(&wrong),
        Err(DelegationErrorV1::PermissionDenied)
    );
    control.commit_permit(&first).unwrap();
    control.commit_permit(&first).unwrap();
    wrong = first.clone();
    wrong.after.max_concurrent = 3;
    assert_eq!(
        control.commit_permit(&wrong),
        Err(DelegationErrorV1::Conflict)
    );
    operation.state = OperationState::Succeeded;
    add_component_file_receipts(&mut operation);
    control.finish_operation(&mut operation).unwrap();
    let next = begin(&control, "permit-change");
    let mut change = DelegationPermitMutationV1 {
        workspace: first.workspace.clone(),
        operation: next.operation_id,
        before: Some(permit(1)),
        after: permit(2),
    };
    change.before.as_mut().unwrap().max_concurrent = 3;
    assert_eq!(
        control.commit_permit(&change),
        Err(DelegationErrorV1::Conflict)
    );
    assert_eq!(
        control.permit(&first.workspace, "permit").unwrap(),
        Some(permit(1))
    );
    change.before = Some(permit(1));
    change.after.revoked = true;
    control.commit_permit(&change).unwrap();
    drop(control);
    let reopened = ControlStore::open(&crate::test_storage_authority(), &path, &backups).unwrap();
    assert_eq!(
        reopened.permit(&first.workspace, "permit").unwrap(),
        Some(change.after.clone())
    );
    assert_eq!(
        reopened.permit_mutations(&first.workspace).unwrap(),
        vec![first, change]
    );
    assert!(
        reopened
            .permit(&WorkspaceId::parse("other").unwrap(), "permit")
            .unwrap()
            .is_none()
    );
}
