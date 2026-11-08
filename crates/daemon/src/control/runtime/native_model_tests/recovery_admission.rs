//! Recover accepted native settings operations at durable interruption points.
use super::*;
use hiroute_gateway::server::composition::RuntimePublicationFeed;

#[test]
fn startup_recovers_released_codex_settings_before_catalog_staging() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::recovery_admission::startup_recovers_released_codex_settings_before_catalog_staging",
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let runtime = open_without_fixture_grants(root.path());
    let adapter = &runtime.adapter;
    ensure_target_cache(adapter);
    let config = adapter.scanner.codex_user_config_target();
    let before = b"# original Codex settings\n";
    fs::write(&config, before).unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let operation = accept_codex_operation(
        adapter,
        "released-catalog-before-stage",
        before,
        None,
        "be6e8eac029b183056b7e4402879f15d2c85f61b",
    );
    let catalog = operation
        .plan
        .external()
        .iter()
        .find(|intent| is_settings_codex_catalog(intent))
        .unwrap();
    assert!(
        adapter
            .artifacts
            .load_native_restore(&operation.operation_id, catalog)
            .unwrap()
            .is_none()
    );
    drop(runtime);

    let recovered = open_without_fixture_grants(root.path());
    recovered.adapter.reconcile_startup_and_open().unwrap();
    let persisted = recovered
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .load_operation(&operation.operation_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        persisted.state,
        OperationState::Succeeded,
        "{:?}",
        persisted.safe_error_code
    );
    let bytes = recovered
        .adapter
        .artifacts
        .read_native_target(catalog.target())
        .unwrap()
        .unwrap();
    let payload = decode_settings_codex_catalog(catalog).unwrap();
    assert_eq!(CanonicalDigest::of_bytes(&bytes), payload.content_digest);
    let written: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(written["models"][0]["default_reasoning_level"].is_null());
    assert_eq!(
        written["models"][0]["supported_reasoning_levels"],
        json!([])
    );
    assert!(
        fs::read_to_string(config)
            .unwrap()
            .contains("model_catalog_json")
    );
}

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
