use super::*;

#[test]
fn settings_codex_daemon_stages_real_grant_reopens_and_restores_native_file() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::lifecycle::settings_codex_daemon_stages_real_grant_reopens_and_restores_native_file",
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let runtime = open(root.path());
    let path = runtime.adapter.scanner.codex_user_config_target();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    let before = b"# user comment\nmodel = 'user-model'\nuser_setting = true\n";
    fs::write(&path, before).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let mut install = operation(&runtime.adapter, "install", before, None);
    let intent = install
        .plan
        .external()
        .iter()
        .find(|intent| is_settings_codex_model(intent))
        .unwrap()
        .clone();
    let adapter = &runtime.adapter;
    adapter.validate_external_admission(&intent).unwrap();
    assert!(
        adapter.apply_external(&install, &intent).is_err(),
        "grant must be staged first"
    );
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
    let file_effect = adapter.apply_external(&install, &intent).unwrap();
    record_agent_effect(adapter, &mut install, &file_effect);
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "staging does not activate"
    );
    let stores = adapter.stores_lock().unwrap();
    let material = stores
        .secrets()
        .resolve_prepared_agent_access_grant(
            &install.operation_id,
            &install.plan.agent_access_grants()[0],
        )
        .unwrap();
    let reference = AgentAccessGrantRefV1::from_ensure_effect(
        &grant_effect,
        &install.plan.agent_access_grants()[0],
    )
    .unwrap();
    assert!(
        stores
            .secrets()
            .resolve_agent_access_grant(&reference)
            .is_err(),
        "staged grant is not helper authority"
    );
    stores
        .secrets()
        .activate_agent_access_grant(&grant_effect)
        .unwrap();
    drop(stores);
    adapter.activate_external(&install, &file_effect).unwrap();
    let configured = fs::read_to_string(&path).unwrap();
    assert!(configured.contains(std::str::from_utf8(material.expose()).unwrap()));
    assert!(configured.contains("hiroute"));
    assert!(configured.contains("# user comment"));
    assert!(
        !serde_json::to_string(&install)
            .unwrap()
            .contains(std::str::from_utf8(material.expose()).unwrap())
    );
    publish(&runtime.adapter, &mut install);
    let active = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap()
        .verify()
        .unwrap();
    assert!(
        active
            .grants
            .iter()
            .any(|grant| grant.grant_id == reference.grant_id())
    );
    install.state = OperationState::Succeeded;
    adapter
        .stores_lock()
        .unwrap()
        .control()
        .finish_operation(&mut install)
        .unwrap();
    drop(runtime);

    let runtime = open(root.path());
    assert!(matches!(
        runtime.adapter.observe_external(&install, &intent).unwrap(),
        EffectReconciliation::Applied(_)
    ));
    // An unrelated edit is preserved by a new, explicitly previewed restoration.
    let current = format!("new_user_setting = false\n{configured}");
    fs::write(&path, &current).unwrap();
    let mut restore = operation(
        &runtime.adapter,
        "restore",
        current.as_bytes(),
        Some(&install.operation_id),
    );
    let restore_intent = restore
        .plan
        .external()
        .iter()
        .find(|intent| is_settings_codex_model(intent))
        .unwrap()
        .clone();
    let revoke = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .secrets()
        .apply_agent_access_grant(
            &restore.operation_id,
            &restore.plan.agent_access_grants()[0],
            None,
        )
        .unwrap();
    let staged = runtime
        .adapter
        .apply_external(&restore, &restore_intent)
        .unwrap();
    record_agent_effect(&runtime.adapter, &mut restore, &staged);
    runtime
        .adapter
        .activate_external(&restore, &staged)
        .unwrap();
    let restored = fs::read_to_string(&path).unwrap();
    assert!(restored.contains("user-model"));
    assert!(restored.contains("new_user_setting = false"));
    assert!(!restored.contains(std::str::from_utf8(material.expose()).unwrap()));
    assert!(!restored.contains("hiroute"));
    runtime
        .adapter
        .stores_lock()
        .unwrap()
        .secrets()
        .activate_agent_access_grant(&revoke)
        .unwrap();
    publish(&runtime.adapter, &mut restore);
    let active = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap()
        .verify()
        .unwrap();
    assert!(
        !active
            .grants
            .iter()
            .any(|grant| grant.grant_id == reference.grant_id())
    );

    // Lost responses reuse the same file effect after replacement, without comparing the old
    // preview digest to the successfully restored bytes and incorrectly reporting a conflict.
    assert_eq!(
        runtime
            .adapter
            .apply_external(&restore, &restore_intent)
            .unwrap(),
        staged
    );
}
