//! Two native Codex homes through public settings Operations, grants and restart recovery.
use super::*;

#[test]
fn desktop_and_cli_profile_have_independent_lifecycles() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::settings_codex_desktop_tests::desktop_and_cli_profile_have_independent_lifecycles",
    ) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let storage = dir.path().join("state");
    fs::create_dir(&storage).unwrap();
    let (runtime, _) = open_with_codex_fixture_surfaces(&storage);
    let ordinary = runtime.adapter.scanner.codex_user_config_target();
    fs::create_dir_all(ordinary.parent().unwrap()).unwrap();
    fs::write(&ordinary, "# user's subscription\nuser_option = true\n").unwrap();
    fs::set_permissions(&ordinary, fs::Permissions::from_mode(0o600)).unwrap();
    let auth = ordinary.with_file_name("auth.json");
    fs::write(&auth, "native-auth-placeholder").unwrap();
    let original = fs::read(&ordinary).unwrap();
    runtime.adapter.reconcile_startup_and_open().unwrap();
    runtime
        .adapter
        .finish_startup_publication_recovery()
        .unwrap();
    let service = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()));
    let scan = service
        .dispatch_wire(request("ScanAgents", json!({}), None))
        .data
        .unwrap();
    let agents = scan["agents"].as_array().unwrap();
    let cli = agents
        .iter()
        .find(|a| a["agent_id"] == "agent_codex_default")
        .unwrap();
    let desktop = agents
        .iter()
        .find(|a| a["agent_id"] == "agent_codex_desktop")
        .unwrap();
    let cli_context = cli["codex_access"]["profile_context_id"].clone();
    let desktop_context = desktop["context_id"].clone();
    assert_ne!(cli_context, desktop_context);
    assert_eq!(desktop["available_surfaces"], json!(["codex_desktop"]));
    assert!(desktop.get("codex_access").is_none());
    let paths = runtime
        .adapter
        .scanner
        .codex_desktop_instance_paths()
        .unwrap();
    assert!(!paths.root.starts_with(&storage));
    let plan = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap()
        .verify()
        .unwrap()
        .published_agent_plans()
        .unwrap()
        .into_iter()
        .find(|p| {
            p.active
                && p.supported_ingress
                    .contains(&AgentIngressProtocolV1::Responses)
        })
        .unwrap();
    let configure = |context: &serde_json::Value| {
        json!({
        "schema_version":{"major":2,"minor":0},"context_id":context,
        "model":{"intent":"configure","settings":{"mode":"codex_default",
            "native_model_mode":"hiroute_only","fixed_models":[],"allowed_plan_ids":[plan.agent_plan_id],
            "default_selection":{"kind":"plan","plan_id":plan.agent_plan_id}}}})
    };
    apply(&service, &runtime, configure(&cli_context), "enable-cli");
    let profile = runtime.adapter.scanner.codex_profile_config_target();
    let cli_bytes = fs::read(&profile).unwrap();
    apply(
        &service,
        &runtime,
        configure(&desktop_context),
        "enable-desktop",
    );
    assert_eq!(fs::read(&profile).unwrap(), cli_bytes);
    assert_eq!(fs::read(&ordinary).unwrap(), original);
    assert_eq!(
        fs::read_to_string(&auth).unwrap(),
        "native-auth-placeholder"
    );
    assert!(!paths.codex_home.join("auth.json").exists());
    let desktop_config = paths.codex_home.join("config.toml");
    let desktop_bytes = fs::read(&desktop_config).unwrap();
    assert_ne!(
        desktop_bytes, cli_bytes,
        "each connection needs its own scoped grant/catalog"
    );
    fs::write(paths.codex_home.join("native-session"), "retain history").unwrap();
    let status = |context: &serde_json::Value| {
        let response = service.dispatch_wire(request(
            "GetAgentConnectionStatus",
            json!({"schema_version":{"major":2,"minor":0},"context_id":context}),
            None,
        ));
        assert!(response.error.is_none(), "{response:?}");
        response.data.unwrap()
    };
    let restore = |context: &serde_json::Value, restore_point: &serde_json::Value| {
        json!({"schema_version":{"major":2,"minor":0},"context_id":context,
        "model":{"intent":"restore","restore_point_ref":restore_point}})
    };
    let desktop_before = status(&desktop_context);
    let cli_before = status(&cli_context);
    assert_eq!(desktop_before["state"], "configured");
    assert_ne!(
        desktop_before["restore_point_ref"],
        cli_before["restore_point_ref"]
    );
    // Reconfigure/restore the CLI while Desktop remains enabled, including its grant.
    apply(&service, &runtime, configure(&cli_context), "edit-cli");
    apply(
        &service,
        &runtime,
        restore(&cli_context, &status(&cli_context)["restore_point_ref"]),
        "disable-cli",
    );
    assert_eq!(status(&desktop_context)["state"], "configured");
    assert_eq!(fs::read(&desktop_config).unwrap(), desktop_bytes);
    apply(&service, &runtime, configure(&cli_context), "reenable-cli");
    let cli_bytes = fs::read(&profile).unwrap();
    // Native edits remain preserved and prevent unsafe restore, without touching the CLI.
    let desktop_text = String::from_utf8(desktop_bytes.clone()).unwrap();
    fs::write(
        &desktop_config,
        desktop_text.replace("name = \"HiRoute\"", "name = \"User gateway\""),
    )
    .unwrap();
    assert_eq!(status(&desktop_context)["state"], "drift");
    assert_eq!(status(&cli_context)["state"], "configured");
    fs::write(&desktop_config, &desktop_bytes).unwrap();
    let desktop_restore = restore(
        &desktop_context,
        &status(&desktop_context)["restore_point_ref"],
    );
    apply(&service, &runtime, desktop_restore, "disable-desktop");
    assert_eq!(status(&cli_context)["state"], "configured");
    assert_eq!(fs::read(&profile).unwrap(), cli_bytes);
    assert_eq!(
        fs::read_to_string(paths.codex_home.join("native-session")).unwrap(),
        "retain history"
    );
    assert_eq!(fs::read(&ordinary).unwrap(), original);
    apply(
        &service,
        &runtime,
        configure(&desktop_context),
        "reenable-desktop",
    );
    let original_scanner = runtime.adapter.scanner.clone();
    drop(service);
    drop(runtime);
    let restarted = open_with_scanner(&storage, original_scanner.clone());
    restarted.adapter.reconcile_startup_and_open().unwrap();
    restarted
        .adapter
        .finish_startup_publication_recovery()
        .unwrap();
    let service = LocalControlDaemon::new(ApplicationService::new(restarted.application_ports()));
    for context in [&cli_context, &desktop_context] {
        let response = service.dispatch_wire(request(
            "GetAgentConnectionStatus",
            json!({"schema_version":{"major":2,"minor":0},"context_id":context}),
            None,
        ));
        assert!(response.error.is_none(), "{response:?}");
        assert_eq!(response.data.unwrap()["state"], "configured");
    }

    // A process started inside the private Codex instance can inherit its CODEX_HOME.
    // Startup must explain the conflict without rebinding existing managed targets.
    let desktop_status = service
        .dispatch_wire(request(
            "GetAgentConnectionStatus",
            json!({"schema_version":{"major":2,"minor":0},"context_id":desktop_context}),
            None,
        ))
        .data
        .unwrap();
    drop(service);
    drop(restarted);
    let home = storage.join("home");
    let mut layout = AgentFilesystemLayoutV1::from_process(&home, &storage);
    layout.codex_user_config = desktop_config.clone();
    layout.codex_executable = storage.join("codex-cli-fixture");
    layout.codex_desktop_executable = Some(storage.join("codex-desktop-fixture"));
    layout.claude_executable = storage.join("missing-claude");
    let registry = serde_json::from_slice(include_bytes!(
        "../../../../../assets/connector-registry/current/registry-seed.json"
    ))
    .unwrap();
    let models: ReleaseModelDataBundleV2 = serde_json::from_slice(include_bytes!(
        "../../../../../assets/release-facts/current/bundle/model-data.json"
    ))
    .unwrap();
    let scanner = FilesystemAgentScannerV1::new(
        layout,
        ClaudeRegistrationIndexV1::from_verified_model_data(&registry, &models.data).unwrap(),
    );
    let private_before_overlap = fs::read(&desktop_config).unwrap();
    let key = storage.join("observation-hmac-key");
    let original_key = fs::read(&key).unwrap();
    let failure = ProductionControlRuntime::prepare_for_role_all_with_scanner(
        &storage,
        crate::release_catalog::fixture_catalog(),
        None,
        scanner,
    );
    let message = match failure {
        Err(message) => message,
        Ok(_) => panic!("overlapping homes must not register two native targets"),
    };
    assert_eq!(
        message,
        crate::control::runtime::native_context_guidance::CODEX_HOME_CONFLICT
    );
    assert_eq!(fs::read(&desktop_config).unwrap(), private_before_overlap);
    assert_eq!(fs::read(&key).unwrap(), original_key);
    assert_eq!(fs::read(&profile).unwrap(), cli_bytes);
    assert_eq!(fs::read(&ordinary).unwrap(), original);
    assert_eq!(
        fs::read_to_string(&auth).unwrap(),
        "native-auth-placeholder"
    );

    // Returning to the original environment preserves both connections and their ownership.
    let recovered = open_with_scanner(&storage, original_scanner);
    recovered.adapter.reconcile_startup_and_open().unwrap();
    recovered
        .adapter
        .finish_startup_publication_recovery()
        .unwrap();
    let service = LocalControlDaemon::new(ApplicationService::new(recovered.application_ports()));
    let wrong = service.dispatch_wire(request(
        "PreviewAgentConnectionRestore",
        json!({"spec":restore(&desktop_context, &cli_before["restore_point_ref"])}),
        None,
    ));
    assert!(
        wrong.error.is_some()
            || wrong
                .data
                .as_ref()
                .is_some_and(|data| data["applicable"] != true),
        "another connection's restore point must not acquire Desktop ownership: {wrong:?}"
    );
    apply(
        &service,
        &recovered,
        restore(&desktop_context, &desktop_status["restore_point_ref"]),
        "disable-desktop-after-home-conflict",
    );
    let restored = service.dispatch_wire(request(
        "GetAgentConnectionStatus",
        json!({"schema_version":{"major":2,"minor":0},"context_id":desktop_context}),
        None,
    ));
    assert!(restored.error.is_none(), "{restored:?}");
    assert_eq!(restored.data.unwrap()["state"], "restored");
    assert!(
        recovered
            .adapter
            .stores_lock()
            .unwrap()
            .secrets()
            .inspect_agent_access_grant(
                WorkspaceId::DEFAULT,
                &format!("agent-connection/{}", desktop_context.as_str().unwrap())
            )
            .unwrap()
            .is_none()
    );
    let cli_after = service.dispatch_wire(request(
        "GetAgentConnectionStatus",
        json!({"schema_version":{"major":2,"minor":0},"context_id":cli_context}),
        None,
    ));
    assert!(cli_after.error.is_none(), "{cli_after:?}");
    assert_eq!(cli_after.data.unwrap()["state"], "configured");
    assert_eq!(fs::read(&profile).unwrap(), cli_bytes);
    assert_eq!(fs::read(&ordinary).unwrap(), original);
    assert_eq!(
        fs::read_to_string(paths.codex_home.join("native-session")).unwrap(),
        "retain history"
    );
}
