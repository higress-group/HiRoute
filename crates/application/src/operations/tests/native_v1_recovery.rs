use super::*;

#[test]
fn retained_v1_publication_operation_recovers_without_replaying_completed_effects() {
    use hiroute_domain::StoredOperationInputV1;

    let plan = crate::routing::retained_v1_operation_plan();
    let workspace = WorkspaceId::default();
    let scope =
        IdempotencyScopeV1::new("interactive-user", "ApplyAgentPlanChange", "v1-recovery").unwrap();
    let request = CanonicalDigest::of_bytes(b"v1-recovery-request");
    let operation = OperationV1::new(
        OperationId::derive(&workspace, &scope, &request),
        workspace,
        scope,
        request,
        CanonicalDigest::of_bytes(b"v1-recovery-accepted"),
        RevisionSetV1 {
            target: 0,
            dependencies: BTreeMap::new(),
        },
        plan,
    )
    .unwrap();
    let ports = MemoryPorts::default();
    let runtime = TransactionRuntime::default();
    let coordinator = open(&ports, &runtime);
    {
        let mut state = ports.state.borrow_mut();
        state.writer = Some(operation.operation_id.to_string());
        state
            .operations
            .insert(operation.operation_id.to_string(), operation.clone());
    }
    let completed = coordinator.run(&operation.operation_id).unwrap();
    assert_eq!(completed.state, OperationState::Succeeded);
    let activation_before = ports.state.borrow().activation_log.clone();
    assert!(
        activation_before
            .iter()
            .any(|id| id == "routing-publication")
    );

    // Reconstruct immutable inputs from the durable carrier, then model the crash
    // after activation but before the final journal state was committed.
    let stored = StoredOperationInputV1::freeze(&completed.plan).unwrap();
    let stored_bytes = serde_json::to_vec(&stored).unwrap();
    let reopened: StoredOperationInputV1 = serde_json::from_slice(&stored_bytes).unwrap();
    let rebuilt = reopened.build().unwrap();
    let mut pending = OperationV1::new(
        completed.operation_id.clone(),
        completed.workspace_id.clone(),
        completed.idempotency.clone(),
        completed.request_digest.clone(),
        completed.accepted_digest.clone(),
        completed.expected_revisions.clone(),
        rebuilt,
    )
    .unwrap();
    pending
        .restore_durable_state(
            OperationState::Activating,
            completed.generation,
            completed.steps.clone(),
            None,
        )
        .unwrap();
    assert_eq!(
        pending
            .plan
            .plan_content_control()
            .unwrap()
            .unwrap()
            .plan_version
            .configuration
            .native_protocol()
            .unwrap()
            .schema,
        hiroute_domain::SYSTEM_ONE_C_V1
    );
    {
        let mut state = ports.state.borrow_mut();
        state.writer = Some(pending.operation_id.to_string());
        state
            .finalization_pending
            .remove(pending.operation_id.as_str());
        state
            .operations
            .insert(pending.operation_id.to_string(), pending);
    }
    let restarted_runtime = TransactionRuntime::default();
    let restarted =
        TransactionCoordinator::new(&ports, &ports, &ports, &ports, &ports, &restarted_runtime);
    let recovered = restarted.reconcile_startup_and_open().unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].state, OperationState::Succeeded);
    assert_eq!(
        ports.state.borrow().activation_log,
        activation_before,
        "completed effects were replayed"
    );
}
