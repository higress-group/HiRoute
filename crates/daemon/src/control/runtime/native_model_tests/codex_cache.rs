use super::*;

#[test]
fn settings_codex_hiroute_only_ignores_native_cache_drift_before_configuration_write() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::codex_cache::settings_codex_hiroute_only_ignores_native_cache_drift_before_configuration_write",
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let runtime = open_without_fixture_grants(root.path());
    let adapter = &runtime.adapter;
    let context = adapter
        .settings_context_for_agent("agent_codex_default")
        .unwrap();
    let path = adapter.scanner.codex_user_config_target();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    let before = b"# unchanged on context drift\nuser_setting = true\n";
    fs::write(&path, before).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

    let mut install = settings_operation(adapter, "catalog-cache-drift", &context, before, None, 1);
    let grant_effect = adapter
        .stores_lock()
        .unwrap()
        .secrets()
        .apply_agent_access_grant(
            &install.operation_id,
            &install.plan.agent_access_grants()[0],
            None,
        )
        .unwrap();
    install.step_mut(OperationStepKind::ApplySecrets).effects = vec![grant_effect];
    adapter
        .stores_lock()
        .unwrap()
        .control()
        .save_operation(&mut install)
        .unwrap();
    let catalog_intent = install
        .plan
        .external()
        .iter()
        .find(|intent| is_settings_codex_catalog(intent))
        .unwrap()
        .clone();
    let cache = path.parent().unwrap().join("models_cache.json");
    let mut changed: serde_json::Value =
        serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
    changed["preview_apply_drift"] = json!(true);
    fs::write(&cache, serde_json::to_vec(&changed).unwrap()).unwrap();
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o644)).unwrap();

    // A HiRoute-only catalog is derived from published plans, not the unused native cache.
    let staged = adapter.apply_external(&install, &catalog_intent).unwrap();
    assert_eq!(staged.effect_id, catalog_intent.effect_id());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(
        adapter
            .artifacts
            .load_native_restore(&install.operation_id, &catalog_intent)
            .unwrap()
            .is_some(),
        "the private HiRoute-only catalog is staged without touching native config"
    );
}
