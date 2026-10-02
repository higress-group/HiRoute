//! Recover the durable publication-compensated / Secret-active crash window.
use super::*;
use hiroute_gateway::server::composition::RuntimePublicationFeed;

#[test]
fn startup_recovers_publication_compensated_before_secret_rollback() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::recovery_admission::startup_recovers_publication_compensated_before_secret_rollback",
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let runtime = open_without_fixture_grants(root.path());
    let adapter = &runtime.adapter;
    let path = adapter.scanner.codex_user_config_target();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    let before = b"# retained user configuration\n";
    fs::write(&path, before).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let mut operation = operation(adapter, "rollback-crash", before, None);
    let mutation = &operation.plan.agent_access_grants()[0];
    let grant = adapter
        .stores_lock()
        .unwrap()
        .secrets()
        .apply_agent_access_grant(&operation.operation_id, mutation, None)
        .unwrap();
    let reference = AgentAccessGrantRefV1::from_ensure_effect(&grant, mutation).unwrap();
    operation.step_mut(OperationStepKind::ApplySecrets).effects = vec![grant.clone()];
    adapter
        .stores_lock()
        .unwrap()
        .control()
        .save_operation(&mut operation)
        .unwrap();
    let intent = operation
        .plan
        .external()
        .iter()
        .find(|intent| intent.kind() == OwnedEffectKind::Publication)
        .unwrap()
        .clone();
    let publication = adapter.apply_external(&operation, &intent).unwrap();
    operation
        .step_mut(OperationStepKind::CompilePublication)
        .effects = vec![publication.clone()];
    adapter
        .stores_lock()
        .unwrap()
        .control()
        .save_operation(&mut operation)
        .unwrap();
    adapter
        .stores_lock()
        .unwrap()
        .secrets()
        .activate_agent_access_grant(&grant)
        .unwrap();
    let aborted = adapter
        .prepare_publication_rollback(&operation, &publication)
        .unwrap();
    operation
        .step_mut(OperationStepKind::CompilePublication)
        .effects = vec![aborted.clone()];
    operation.state = OperationState::RollingBack;
    adapter
        .stores_lock()
        .unwrap()
        .control()
        .save_operation(&mut operation)
        .unwrap();
    assert_eq!(
        adapter.compensate_external(&operation, &aborted).unwrap(),
        CompensationOutcome::Compensated
    );
    operation
        .step_mut(OperationStepKind::CompilePublication)
        .status = OperationStepStatus::Compensated;
    adapter
        .stores_lock()
        .unwrap()
        .control()
        .save_operation(&mut operation)
        .unwrap();
    assert!(
        adapter
            .stores_lock()
            .unwrap()
            .control()
            .prepared_publication(&WorkspaceId::default())
            .unwrap()
            .is_none()
    );
    assert!(
        adapter
            .stores_lock()
            .unwrap()
            .secrets()
            .resolve_agent_access_grant(&reference)
            .is_ok()
    );
    let id = operation.operation_id.clone();
    drop(runtime);

    // Reopen the exact persisted checkpoint through the production startup entry.
    let recovered = ProductionControlRuntime::prepare_for_role_all(
        root.path(),
        crate::release_catalog::fixture_catalog(),
        None,
    )
    .expect("a recoverable rollback must reach journal recovery");
    let target = std::sync::Arc::new(crate::gateway_ports::GatewayPublicationAdapter::new(
        std::sync::Arc::new(
            hiroute_gateway::server::publication::GatewayPublicationInstaller::open(
                root.path().join("gateway-lkg.json"),
            )
            .unwrap(),
        ),
    ));
    *recovered.adapter.publication_target.lock().unwrap() = Some(target.clone());
    assert!(RuntimePublicationFeed::pin(target.as_ref()).is_none());
    // A caller cannot skip recovery and expose the inconsistent grant as serving state.
    assert!(
        recovered
            .adapter
            .finish_startup_publication_recovery()
            .is_err()
    );
    assert!(RuntimePublicationFeed::pin(target.as_ref()).is_none());
    assert!(
        !recovered
            .adapter
            .startup_recovery_complete
            .load(std::sync::atomic::Ordering::Acquire)
    );
    recovered
        .configure_managed_agent_runtime(
            "http://127.0.0.1:5837/v1".into(),
            "/test/hiroute".into(),
            Some(target.clone()),
        )
        .unwrap();
    let stores = recovered.adapter.stores_lock().unwrap();
    assert_eq!(
        stores.control().load_operation(&id).unwrap().unwrap().state,
        OperationState::RolledBack
    );
    assert!(
        stores
            .secrets()
            .resolve_agent_access_grant(&reference)
            .is_err()
    );
    assert!(
        stores
            .control()
            .prepared_publication(&WorkspaceId::default())
            .unwrap()
            .is_none()
    );
    assert!(RuntimePublicationFeed::pin(target.as_ref()).is_some());
    assert_eq!(fs::read(path).unwrap(), before);
}
