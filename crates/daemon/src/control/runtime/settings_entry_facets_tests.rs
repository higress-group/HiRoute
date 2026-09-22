//! Claude and Skill-specific public V2 settings entry scenarios.
use super::*;
use crate::control::runtime::native_claude_model::is_settings_claude_model;

#[test]
fn v2_settings_skill_only_works_before_the_first_publication() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::facets::v2_settings_skill_only_works_before_the_first_publication",
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let executable = root.path().join("codex-fixture");
    fs::write(&executable, b"#!/bin/sh\nprintf 'codex-cli 99.99.99\\n'\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let mut layout = AgentFilesystemLayoutV1::from_process(&home, root.path());
    layout.codex_executable = executable;
    layout.claude_executable = root.path().join("missing-claude");
    let registry = serde_json::from_slice(include_bytes!(
        "../../../../../assets/connector-registry/current/registry-seed.json"
    ))
    .unwrap();
    let models: hiroute_domain::ReleaseModelDataBundleV2 = serde_json::from_slice(include_bytes!(
        "../../../../../assets/release-facts/current/bundle/model-data.json"
    ))
    .unwrap();
    let scanner = FilesystemAgentScannerV1::new(
        layout,
        ClaudeRegistrationIndexV1::from_verified_model_data(&registry, &models.data).unwrap(),
    );
    let runtime = ProductionControlRuntime::prepare_for_role_all_with_scanner(
        root.path(),
        crate::release_catalog::fixture_catalog(),
        None,
        scanner,
    )
    .unwrap();
    *runtime.adapter.managed_agent_runtime.lock().unwrap() = Some(ManagedAgentRuntimeV1 {
        gateway_base_url: "http://127.0.0.1:5837/v1".into(),
        trusted_hiroute_executable: "/test/hiroute".into(),
        worker_executor_availability: Arc::new(
            crate::delegation::installation::WorkerExecutorAvailabilityRegistry::unconfigured(),
        ),
        resident_service_ready: false,
    });
    runtime.adapter.reconcile_startup_and_open().unwrap();
    assert!(
        runtime
            .adapter
            .stores_lock()
            .unwrap()
            .control()
            .active_publication(&WorkspaceId::default())
            .unwrap()
            .is_none()
    );

    let ordinary = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()));
    let scan = ordinary
        .dispatch_wire(request("ScanAgents", json!({}), None))
        .data
        .unwrap();
    let context = scan["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["agent_id"] == "agent_codex_default")
        .unwrap()["context_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let spec = json!({
        "schema_version":{"major":2,"minor":0}, "context_id":context,
        "collaboration":{"intent":"configure","settings":{"trigger_mode":"explicit"}}
    });
    let unproven = ordinary.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert!(unproven.error.is_none(), "{unproven:?}");
    assert_eq!(unproven.data.unwrap()["applicable"], false);

    let service =
        LocalControlDaemon::new(ApplicationService::new(
            runtime.application_ports().with_agent_connection(Arc::new(
                FixtureFacts::collaboration(runtime.adapter.clone()),
            )),
        ));
    apply(&service, &runtime, spec, "skill-before-publication");
    assert!(
        home.join(".agents/skills/hiroute-collaboration/SKILL.md")
            .is_file()
    );
    assert!(
        runtime
            .adapter
            .stores_lock()
            .unwrap()
            .control()
            .active_publication(&WorkspaceId::default())
            .unwrap()
            .is_none(),
        "installing the Skill must not manufacture an empty publication"
    );
}

#[test]
fn v2_settings_dispatch_claude_configures_and_formally_restores_owned_user_file() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::facets::v2_settings_dispatch_claude_configures_and_formally_restores_owned_user_file",
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let home = root.path().join("agent-home");
    fs::create_dir(&home).unwrap();
    let settings = home.join(".claude/settings.json");
    assert!(
        settings.starts_with(root.path()),
        "test fixture must never target daily settings"
    );
    assert!(
        std::env::var_os("CLAUDE_CONFIG_DIR").is_some(),
        "the child must also isolate native Claude's configuration directory"
    );
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::set_permissions(
        settings.parent().unwrap(),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let original = json!({
        "theme": "dark",
        "model": "opus",
        "apiKeyHelper": "user-owned-helper --do-not-run",
        "env": {
            "ANTHROPIC_BASE_URL": "https://open.bigmodel.cn/api/anthropic",
            "ANTHROPIC_DEFAULT_OPUS_MODEL": "glm-5.3[1m]",
            "ANTHROPIC_AUTH_TOKEN": "fixture-user-token",
            "CLAUDE_CODE_AUTO_COMPACT_WINDOW": "500000",
            "CLAUDE_CODE_MAX_CONTEXT_TOKENS": "900000",
            "UNRELATED": "keep-me"
        }
    });
    fs::write(&settings, serde_json::to_vec_pretty(&original).unwrap()).unwrap();
    fs::set_permissions(&settings, fs::Permissions::from_mode(0o600)).unwrap();
    let executable = root.path().join("claude-fixture");
    fs::write(&executable, b"#!/bin/sh\nprintf '2.1.0 (Claude Code)\\n'\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();

    let mut layout = AgentFilesystemLayoutV1::from_process(&home, root.path());
    layout.codex_executable = root.path().join("missing-codex");
    layout.claude_executable = executable.clone();
    layout.claude_launch_settings = None;
    layout.claude_project_settings.clear();
    layout.claude_user_settings = settings.clone();
    layout.claude_managed_settings.clear();
    let registry = serde_json::from_slice(include_bytes!(
        "../../../../../assets/connector-registry/current/registry-seed.json"
    ))
    .unwrap();
    let models: hiroute_domain::ReleaseModelDataBundleV2 = serde_json::from_slice(include_bytes!(
        "../../../../../assets/release-facts/current/bundle/model-data.json"
    ))
    .unwrap();
    let scanner = FilesystemAgentScannerV1::new(
        layout,
        ClaudeRegistrationIndexV1::from_verified_model_data(&registry, &models.data).unwrap(),
    );
    let runtime = open_with_scanner(root.path(), scanner);
    runtime.adapter.reconcile_startup_and_open().unwrap();

    let ordinary = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()));
    let scan = ordinary
        .dispatch_wire(request("ScanAgents", json!({}), None))
        .data
        .unwrap();
    let claude = scan["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["agent_id"] == "agent_claude_default")
        .unwrap();
    assert_eq!(
        claude["supported"], true,
        "the V2 entry does not gate Claude configuration on a version whitelist"
    );
    let context = claude["context_id"].as_str().unwrap().to_owned();
    let plans = runtime
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
        .unwrap();
    let plan = plans
        .iter()
        .find(|plan| {
            plan.active
                && plan
                    .supported_ingress
                    .contains(&AgentIngressProtocolV1::Messages)
        })
        .unwrap();
    let extra_plan = plans
        .iter()
        .find(|candidate| {
            candidate.agent_plan_id != plan.agent_plan_id
                && candidate.active
                && candidate
                    .supported_ingress
                    .contains(&AgentIngressProtocolV1::Messages)
        })
        .unwrap();
    let spec = json!({
        "schema_version":{"major":2,"minor":0}, "context_id":context,
        "model":{"intent":"configure","settings":{
            "mode":"claude_launcher","surfaces":["claude_cli"],"fixed_models":[],
            "preset_mappings":{
                "opus":{"kind":"plan","plan_id":plan.agent_plan_id},
                "sonnet":{"kind":"preserve_native"},
                "haiku":{"kind":"preserve_native"}}}}
    });
    let unproven = ordinary.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert!(unproven.error.is_none(), "{unproven:?}");
    let unproven = unproven.data.unwrap();
    assert_eq!(unproven["model_effect"]["agent_class"], "claude");
    assert_eq!(unproven["applicable"], false);

    // Ordinary discovery cannot register every existing Claude provider. The same
    // installed client must still reach the real settings preview when switching
    // its native configuration over to a saved HiRoute route.
    let mut unknown_endpoint = original.clone();
    unknown_endpoint["env"]["ANTHROPIC_BASE_URL"] =
        json!("https://unknown.example.invalid/anthropic");
    unknown_endpoint["env"]["ANTHROPIC_MODEL"] = json!("glm-5.3");
    fs::write(&settings, serde_json::to_vec(&unknown_endpoint).unwrap()).unwrap();
    let unknown_scan = ordinary.dispatch_wire(request("ScanAgents", json!({}), None));
    assert!(unknown_scan.error.is_none(), "{unknown_scan:?}");
    assert!(unknown_scan.data.unwrap()["agents"]
        .as_array()
        .unwrap()
        .iter()
        .any(|agent| agent["agent_id"] == "agent_claude_default" && agent["supported"] == false));
    let preview = ordinary.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert!(preview.error.is_none(), "{preview:?}");
    fs::write(&settings, serde_json::to_vec_pretty(&original).unwrap()).unwrap();

    // The ordinary discovery is allowed to report a broken diagnostic version probe, but
    // saving the already located Claude settings must not turn that into a false 404.
    let original_executable = fs::read(&executable).unwrap();
    fs::write(&executable, b"#!/bin/sh\nexit 9\n").unwrap();
    let diagnostic_scan = ordinary.dispatch_wire(request("ScanAgents", json!({}), None));
    assert!(diagnostic_scan.error.is_none(), "{diagnostic_scan:?}");
    let diagnostic_agents = diagnostic_scan.data.unwrap();
    assert!(
        diagnostic_agents["agents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|agent| {
                agent["agent_id"] == "agent_claude_default" && agent["supported"] == true
            })
    );
    // Version diagnostics cannot add blockers to Plan-window configuration.
    for script in [
        "#!/bin/sh\nexit 9\n",
        "#!/bin/sh\nprintf 'unknown client version\\n'\n",
        "#!/bin/sh\nprintf '2.1.0 (Claude Code)\\n'\n",
    ] {
        fs::write(&executable, script).unwrap();
        let settings_preview = ordinary.dispatch_wire(request(
            "PreviewAgentConnectionChange",
            json!({"spec":spec}),
            None,
        ));
        assert!(settings_preview.error.is_none(), "{settings_preview:?}");
        let settings_preview = settings_preview.data.unwrap();
        assert_eq!(settings_preview["blockers"], unproven["blockers"]);
        assert_eq!(
            settings_preview["model_effect"]["claude_context_window"],
            unproven["model_effect"]["claude_context_window"]
        );
        assert!(
            settings_preview["model_effect"]["claude_context_window"]
                .as_u64()
                .is_some()
        );
    }
    fs::write(&executable, original_executable).unwrap();

    let service = LocalControlDaemon::new(ApplicationService::new(
        runtime
            .application_ports()
            .with_agent_connection(Arc::new(FixtureFacts::model(runtime.adapter.clone()))),
    ));
    // The first model connection is allowed without a login-item declaration.
    let guard_preview = service.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert!(guard_preview.error.is_none(), "{guard_preview:?}");
    let guard_preview = guard_preview.data.unwrap();
    assert_eq!(
        guard_preview["resident_service"]["login_item_required"],
        false
    );
    let obsolete_host_payload = json!({
        "spec":spec,
        "accept_digest":guard_preview["accept_digest"],
        "dependency_digest":guard_preview["dependency_digest"],
        "expected_revisions":guard_preview["expected_revisions"],
        "idempotency_key":"claude-obsolete-login-item",
        "login_item":{"before":"not_registered","after":"enabled","created":true},
    });
    let obsolete_host = service.dispatch_wire(request(
        "ApplyAgentConnectionChange",
        obsolete_host_payload,
        None,
    ));
    assert_eq!(
        obsolete_host.error.unwrap().code,
        api::ErrorCode::InvalidArguments
    );
    let first = apply(&service, &runtime, spec.clone(), "claude-settings-first");
    let first_operation = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .load_operation(
            &OperationId::parse(first["operation_id"].as_str().unwrap().to_owned()).unwrap(),
        )
        .unwrap()
        .unwrap();
    assert!(
        first_operation
            .steps
            .iter()
            .flat_map(|step| step.effects.iter())
            .all(|effect| effect.kind != hiroute_domain::OwnedEffectKind::LoginItem),
        "new settings saves never create a login-item effect"
    );
    // The actual user settings are the ordinary claude entry point. The Operation owns only
    // the routing/auth fields and keeps the previous credential bytes in a protected record.
    let snapshot_target = first_operation
        .plan
        .external()
        .iter()
        .find(|intent| intent.effect_id() == "agent-connection-managed-configuration")
        .unwrap()
        .target()
        .to_owned();
    let published_native = |adapter: &LocalControlAdapter| {
        let bytes = adapter
            .artifacts
            .read_native_target(&snapshot_target)
            .unwrap()
            .unwrap_or_else(|| panic!("the native settings must be published"));
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
    };
    let native = published_native(&runtime.adapter);
    assert_eq!(native["theme"], "dark");
    assert_eq!(native["model"], "opus");
    assert_eq!(native["env"]["UNRELATED"], "keep-me");
    assert!(native["env"].get("ANTHROPIC_MODEL").is_none());
    assert_eq!(native["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:5837");
    assert!(native["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
    assert!(
        native["apiKeyHelper"]
            .as_str()
            .unwrap()
            .contains("__internal-agent-grant-v1")
    );
    assert!(
        serde_json::to_string(&native)
            .unwrap()
            .find("fixture-user-token")
            .is_none(),
        "the ordinary Claude file never contains both token and managed helper"
    );
    // The optional diagnostic launcher derives its facts from the immutable Operation, not
    // from the native settings file.
    let launch_connection = format!("agent-connection/{context}");
    let launch_descriptor = service.dispatch_wire(request(
        "GetManagedAgentLaunchDescriptor",
        json!({"connection_id": launch_connection}),
        None,
    ));
    assert!(launch_descriptor.error.is_none(), "{launch_descriptor:?}");
    let launch_descriptor = launch_descriptor.data.unwrap();
    assert_eq!(
        launch_descriptor["schema"],
        "hiroute.managed-claude-launch-descriptor/v2"
    );
    assert_eq!(launch_descriptor["connection_id"], launch_connection);
    assert_eq!(
        launch_descriptor["gateway_base_url"],
        "http://127.0.0.1:5837"
    );
    assert_eq!(launch_descriptor["grant_generation"], 1);
    let window = launch_descriptor["context_window_tokens"]
        .as_u64()
        .expect("Claude launch carries plan window");
    assert!((100_000..=272_000).contains(&window));
    assert_eq!(
        native["env"]["CLAUDE_CODE_AUTO_COMPACT_WINDOW"],
        window.to_string()
    );
    assert_eq!(
        native["env"]["CLAUDE_CODE_MAX_CONTEXT_TOKENS"],
        window.to_string()
    );
    assert_eq!(
        launch_descriptor["presets"],
        json!({"opus":native["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"],
            "sonnet":null,"haiku":null})
    );
    assert_eq!(
        launch_descriptor["helper_argv"],
        json!(["__internal-agent-grant-v1", launch_connection])
    );
    assert_eq!(
        launch_descriptor["executable"],
        executable.to_string_lossy().as_ref()
    );
    let stores = runtime.adapter.stores_lock().unwrap();
    let live_revision = stores
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap()
        .publication_revision;
    let live_revisions = stores
        .control()
        .current_revisions(&WorkspaceId::default())
        .unwrap();
    let live_payload = json!({
        "agent_id":"agent_claude_default", "scope":"live", "suite":"quick",
        "allow_model_call":true,
        "target":{
            "context_id":context, "surface":"claude_cli",
            "expected_applied_revision":live_revision,
            "client_model_ids":[launch_descriptor["presets"]["opus"]]
        }
    });
    let live_request: api::AgentCheckRequestV1 =
        serde_json::from_value(live_payload.clone()).unwrap();
    let live_capability = "claude-live-check-one-shot-capability".to_owned();
    stores
        .apply_capability_registrar()
        .register(
            ApplyCapabilityRegistrationV1::from_protected_launcher(
                live_capability.clone(),
                "interactive-user",
                WorkspaceId::default(),
                "CheckAgentConnection",
                CanonicalDigest::of(&live_request).unwrap(),
                live_revisions.clone(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64
                    + 60,
            )
            .unwrap(),
        )
        .unwrap();
    drop(stores);
    for (field, value, expected) in [
        (
            "surface",
            json!("codex_desktop"),
            api::ErrorCode::CapabilityDenied,
        ),
        (
            "context_id",
            json!("agent-context/other"),
            api::ErrorCode::CapabilityDenied,
        ),
        (
            "client_model_ids",
            json!(["unpublished-model"]),
            api::ErrorCode::CapabilityDenied,
        ),
        (
            "expected_applied_revision",
            json!(live_revision.get() + 1),
            api::ErrorCode::ChangePreviewStale,
        ),
    ] {
        let mut invalid_payload = live_payload.clone();
        invalid_payload["target"][field] = value;
        let invalid_request: api::AgentCheckRequestV1 =
            serde_json::from_value(invalid_payload.clone()).unwrap();
        let digest = CanonicalDigest::of(&invalid_request).unwrap();
        let capability = format!("claude-live-check-invalid-target-capability-{field}");
        runtime
            .adapter
            .stores_lock()
            .unwrap()
            .apply_capability_registrar()
            .register(
                ApplyCapabilityRegistrationV1::from_protected_launcher(
                    capability.clone(),
                    "interactive-user",
                    WorkspaceId::default(),
                    "CheckAgentConnection",
                    digest.clone(),
                    live_revisions.clone(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64
                        + 60,
                )
                .unwrap(),
            )
            .unwrap();
        let rejected = ordinary.dispatch_wire(request(
            "CheckAgentConnection",
            invalid_payload,
            Some(capability.clone()),
        ));
        assert_eq!(rejected.error.unwrap().code, expected, "{field}");
        hiroute_application::control::ControlStatePort::validate_protected_capability(
            runtime.adapter.as_ref(),
            &capability,
            &WorkspaceId::default(),
            api::PrincipalKind::InteractiveUser,
            "CheckAgentConnection",
            &digest,
            &live_revisions,
        )
        .expect("invalid targets must be rejected before consuming model-call consent");
    }
    let checked = ordinary.dispatch_wire(request(
        "CheckAgentConnection",
        live_payload.clone(),
        Some(live_capability.clone()),
    ));
    assert!(checked.error.is_none(), "{checked:?}");
    let checked = checked.data.unwrap();
    assert_eq!(checked["state"], "failed");
    assert_eq!(checked["reason_code"], "LIVE_CLIENT_OUTPUT_INVALID");
    assert_eq!(checked["call_count"], 1);
    let replayed = ordinary.dispatch_wire(request(
        "CheckAgentConnection",
        live_payload,
        Some(live_capability),
    ));
    assert_eq!(
        replayed.error.unwrap().code,
        api::ErrorCode::CapabilityDenied
    );
    let surface_checks = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .agent_surface_checks(&WorkspaceId::default(), &context)
        .unwrap();
    assert_eq!(surface_checks.len(), 1);
    assert_eq!(surface_checks[0].surface, AgentModelSurfaceV2::ClaudeCli);
    assert_eq!(surface_checks[0].state, AgentSurfaceCheckStateV1::Failed);
    assert_eq!(
        surface_checks[0].reason_code.as_deref(),
        Some("LIVE_CLIENT_OUTPUT_INVALID")
    );
    let connection_id = format!("agent-connection/{context}");
    let first_material = runtime
        .adapter
        .resolve_active_agent_grant(&connection_id)
        .expect("configured V2 settings must expose their exact grant through the owner socket");
    let executable_bytes = fs::read(&executable).unwrap();
    let user_bytes = fs::read(&settings).unwrap();
    fs::write(&executable, b"#!/bin/sh\nexit 99\n").unwrap();
    fs::write(&settings, b"not valid settings JSON").unwrap();
    let after_drift = service.dispatch_wire(request(
        "GetManagedAgentLaunchDescriptor",
        json!({"connection_id": launch_connection}),
        None,
    ));
    let drift_material = runtime.adapter.resolve_active_agent_grant(&connection_id);
    fs::write(&executable, executable_bytes).unwrap();
    fs::write(&settings, user_bytes).unwrap();
    assert!(
        after_drift.error.is_some(),
        "a broken ordinary entry cannot remain connected"
    );
    assert!(
        drift_material.is_err(),
        "the helper must fail closed on native-file drift"
    );
    let configured = published_native(&runtime.adapter);
    assert_eq!(
        configured["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"],
        launch_descriptor["presets"]["opus"]
    );
    assert!(configured["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
    let reused = apply(&service, &runtime, spec.clone(), "claude-settings-reuse");
    assert_ne!(reused["operation_id"], first["operation_id"]);
    let reused_operation: OperationId =
        serde_json::from_value(reused["operation_id"].clone()).unwrap();
    let reused_material = runtime
        .adapter
        .resolve_active_agent_grant(&connection_id)
        .expect("an identical settings apply must keep exposing the reused grant");
    assert_eq!(reused_material.sha256(), first_material.sha256());
    assert_eq!(
        runtime
            .adapter
            .stores_lock()
            .unwrap()
            .secrets()
            .agent_access_grant_generation(
                WorkspaceId::DEFAULT,
                &format!("agent-connection/{context}")
            )
            .unwrap(),
        1,
        "an identical settings apply reuses the current grant generation"
    );

    let update_spec = json!({
        "schema_version":{"major":2,"minor":0}, "context_id":context,
        "model":{"intent":"configure","settings":{
            "mode":"claude_launcher","surfaces":["claude_cli"],"fixed_models":[],
            "preset_mappings":{
                "opus":{"kind":"plan","plan_id":plan.agent_plan_id},
                "sonnet":{"kind":"plan","plan_id":extra_plan.agent_plan_id},
                "haiku":{"kind":"preserve_native"}}}}
    });
    let updated = apply(&service, &runtime, update_spec, "claude-settings-update");
    let operation: OperationId = serde_json::from_value(updated["operation_id"].clone()).unwrap();
    let update_journal = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .load_operation(&operation)
        .unwrap()
        .unwrap();
    let update_intent = update_journal
        .plan
        .external()
        .iter()
        .find(|intent| is_settings_claude_model(intent))
        .unwrap()
        .clone();
    let update_payload =
        settings_claude_model_file_for_operation(&update_journal, &update_intent).unwrap();
    let ClaudeModelFileAction::Configure {
        previous_operation,
        snapshot,
        ..
    } = update_payload.change
    else {
        panic!("the update must be a Claude model configuration");
    };
    assert_eq!(previous_operation, Some(reused_operation));
    let updated_material = runtime
        .adapter
        .resolve_active_agent_grant(&connection_id)
        .expect("updated V2 settings must preserve their exact grant");
    assert_eq!(first_material.sha256(), updated_material.sha256());
    let updated_native = published_native(&runtime.adapter);
    assert_eq!(
        updated_native["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"].as_str(),
        snapshot.presets.sonnet.as_deref()
    );
    assert!(updated_native["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());

    let restore_spec = json!({
        "schema_version":{"major":2,"minor":0}, "context_id":context,
        "model":{"intent":"restore","restore_point_ref":codex_model_restore_point_ref(&operation)}
    });
    // A new connection did not create a login item, so restoring it needs no host action.
    let removal_preview = service.dispatch_wire(request(
        "PreviewAgentConnectionRestore",
        json!({"spec":restore_spec}),
        None,
    ));
    assert!(removal_preview.error.is_none(), "{removal_preview:?}");
    let removal_preview = removal_preview.data.unwrap();
    assert_eq!(
        removal_preview["resident_service"]["login_item_removal_required"], false,
        "a new connection owns no login item"
    );
    let restore = apply(&service, &runtime, restore_spec, "claude-settings-restore");
    let restore_operation = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .load_operation(
            &OperationId::parse(restore["operation_id"].as_str().unwrap().to_owned()).unwrap(),
        )
        .unwrap()
        .unwrap();
    assert!(
        restore_operation
            .steps
            .iter()
            .flat_map(|step| step.effects.iter())
            .all(|effect| effect.kind != hiroute_domain::OwnedEffectKind::LoginItem),
        "restoring a new connection does not touch login items"
    );
    let restored: serde_json::Value =
        serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
    assert_eq!(
        restored, original,
        "a formal restore recovers the original auth and native mapping"
    );
    assert!(
        runtime
            .adapter
            .resolve_active_agent_grant(&connection_id)
            .is_err(),
        "a formal restore withdraws the exposed grant"
    );
    let withdrawn = service.dispatch_wire(request(
        "GetManagedAgentLaunchDescriptor",
        json!({"connection_id": connection_id}),
        None,
    ));
    assert!(
        withdrawn.error.is_some(),
        "a restored connection without an active grant has no launch descriptor"
    );
    let again = apply(&service, &runtime, spec.clone(), "claude-settings-again");
    let next_material = runtime
        .adapter
        .resolve_active_agent_grant(&connection_id)
        .expect("reconfigured V2 settings must expose the new exact grant");
    assert_ne!(updated_material.sha256(), next_material.sha256());
    let republished = published_native(&runtime.adapter);
    assert!(republished["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
    assert_eq!(
        runtime
            .adapter
            .stores_lock()
            .unwrap()
            .secrets()
            .agent_access_grant_generation(
                WorkspaceId::DEFAULT,
                &format!("agent-connection/{context}")
            )
            .unwrap(),
        4
    );

    // Reconfiguration and a second restore also leave startup under the user's control.
    let again_id: OperationId = serde_json::from_value(again["operation_id"].clone()).unwrap();
    let second_restore_spec = json!({
        "schema_version":{"major":2,"minor":0}, "context_id":context,
        "model":{"intent":"restore","restore_point_ref":codex_model_restore_point_ref(&again_id)}
    });
    let second_restore_preview = service.dispatch_wire(request(
        "PreviewAgentConnectionRestore",
        json!({"spec":second_restore_spec}),
        None,
    ));
    assert!(
        second_restore_preview.error.is_none(),
        "{second_restore_preview:?}"
    );
    assert_eq!(
        second_restore_preview.data.unwrap()["resident_service"]["login_item_removal_required"],
        false,
        "the re-created connection still owns no login item"
    );
    apply(
        &service,
        &runtime,
        second_restore_spec,
        "claude-settings-second-restore",
    );
    assert!(
        runtime
            .adapter
            .resolve_active_agent_grant(&connection_id)
            .is_err()
    );
    // No explicit current selection is also a normal Claude config. Saving a preset must
    // succeed without pretending the account-dependent Default was proven usable.
    let mut no_default = original.clone();
    no_default.as_object_mut().unwrap().remove("model");
    no_default["env"]
        .as_object_mut()
        .unwrap()
        .remove("ANTHROPIC_AUTH_TOKEN");
    fs::write(&settings, serde_json::to_vec_pretty(&no_default).unwrap()).unwrap();
    apply(&service, &runtime, spec, "claude-settings-no-default");
    let without_default = published_native(&runtime.adapter);
    assert!(without_default.get("model").is_none());
    assert!(without_default["env"].get("ANTHROPIC_MODEL").is_none());
    assert!(without_default["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
    let token_free_settings = fs::read(&settings).unwrap();
    let mut unexpected_token = without_default;
    unexpected_token["env"]["ANTHROPIC_AUTH_TOKEN"] = json!("added-after-save");
    fs::write(&settings, serde_json::to_vec(&unexpected_token).unwrap()).unwrap();
    let drift = service.dispatch_wire(request(
        "GetAgentConnectionStatus",
        json!({"schema_version":{"major":2,"minor":0}, "context_id":context}),
        None,
    ));
    assert!(drift.error.is_none(), "{drift:?}");
    assert_eq!(drift.data.unwrap()["state"], "drift");
    assert!(
        runtime
            .adapter
            .resolve_active_agent_grant(&connection_id)
            .is_err(),
        "the helper must not mint a grant beside a late native token"
    );
    fs::write(&settings, token_free_settings).unwrap();
    let status = service.dispatch_wire(request(
        "GetAgentConnectionStatus",
        json!({"schema_version":{"major":2,"minor":0}, "context_id":context}),
        None,
    ));
    assert!(status.error.is_none(), "{status:?}");
    assert_eq!(status.data.unwrap()["model_verified"], false);
}

#[path = "settings_entry_collaboration_tests.rs"]
mod collaboration;
