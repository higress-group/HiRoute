//! Native profile lifecycle through the production wire entry and protected stores.
use super::*;

#[test]
fn profile_lifecycle_inheritance_slot_and_safe_restore() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::settings_profile_tests::profile_lifecycle_inheritance_slot_and_safe_restore",
    ) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (runtime, _) = open_with_codex_fixture_surfaces(dir.path());
    ensure_target_cache(&runtime.adapter);
    let root = runtime.adapter.scanner.codex_user_config_target();
    fs::create_dir_all(root.parent().unwrap()).unwrap();
    fs::set_permissions(root.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    let original = "# ordinary entry remains native\nuser_option = true\n";
    fs::write(&root, original).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o600)).unwrap();
    runtime.adapter.reconcile_startup_and_open().unwrap();
    // Complete the same Gateway handoff as the production startup entry.
    runtime
        .adapter
        .finish_startup_publication_recovery()
        .unwrap();
    let service = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()));
    let scan = service
        .dispatch_wire(request("ScanAgents", json!({}), None))
        .data
        .unwrap();
    let codex = scan["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["agent_id"] == "agent_codex_default")
        .unwrap();
    let access = &codex["codex_access"];
    assert_eq!(access["selected_mode"], "profile");
    assert_eq!(codex["context_id"], access["profile_context_id"]);
    assert_ne!(access["root_context_id"], access["profile_context_id"]);
    let context = access["profile_context_id"].clone();
    let profile = runtime.adapter.scanner.codex_profile_config_target();
    let publication = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap()
        .verify()
        .unwrap();
    let plans = publication.published_agent_plans().unwrap();
    let plan = plans
        .iter()
        .find(|p| {
            p.active
                && p.supported_ingress
                    .contains(&AgentIngressProtocolV1::Responses)
        })
        .unwrap();
    let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":context,"model":{"intent":"configure","settings":{
        "mode":"codex_default","native_model_mode":"hiroute_only","fixed_models":[],"allowed_plan_ids":[plan.agent_plan_id],"default_selection":{"kind":"plan","plan_id":plan.agent_plan_id}}}});
    // A filename never establishes ownership, even for an empty preexisting file.
    fs::write(&profile, "").unwrap();
    fs::set_permissions(&profile, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(
        service
            .dispatch_wire(request(
                "PreviewAgentConnectionChange",
                json!({"spec":spec}),
                None
            ))
            .error
            .is_some()
    );
    fs::remove_file(&profile).unwrap();
    let preview = service.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert!(preview.error.is_none(), "{preview:?}");
    let preview = preview.data.unwrap();
    assert_eq!(preview["applicable"], true, "{preview}");
    // Root changes invalidate preview even though the target profile is still absent.
    fs::write(&root, format!("{original}[model_providers.hiroute]\nhttp_headers = {{ Authorization = 'user-owned' }}\n")).unwrap();
    let stale = service.dispatch_wire(request("ApplyAgentConnectionChange", json!({"spec":spec,"accept_digest":preview["accept_digest"],"dependency_digest":preview["dependency_digest"],"expected_revisions":preview["expected_revisions"],"idempotency_key":"stale-profile"}), None));
    assert!(stale.error.is_some(), "{stale:?}");
    assert!(!profile.exists());
    fs::write(&root, original).unwrap();
    apply(&service, &runtime, spec.clone(), "create-profile");
    assert_eq!(fs::read_to_string(&root).unwrap(), original);
    let configured = fs::read_to_string(&profile).unwrap();
    assert!(configured.contains("requires_openai_auth = false"));
    // Root and profile have distinct context ids but share the same home slot.
    let mut root_spec = spec.clone();
    root_spec["context_id"] = access["root_context_id"].clone();
    assert!(
        service
            .dispatch_wire(request(
                "PreviewAgentConnectionChange",
                json!({"spec":root_spec}),
                None
            ))
            .error
            .is_some()
    );
    fs::write(&profile, format!("user_edit = true\n{configured}")).unwrap();
    // Configure/edit tails must pin the publication as well as revoke tails.
    let before_edit = fs::read(&profile).unwrap();
    let mut edit_ports = runtime.application_ports();
    edit_ports.mutation = Some(Arc::new(fault::FileRace {
        adapter: runtime.adapter.clone(),
        path: profile.clone(),
        fail_after_file: false,
        replacement: None,
    }));
    let editing = LocalControlDaemon::new(ApplicationService::new(edit_ports));
    let edit_preview = editing
        .dispatch_wire(request(
            "PreviewAgentConnectionChange",
            json!({"spec":spec}),
            None,
        ))
        .data
        .unwrap();
    let parked_edit = editing.dispatch_wire(request("ApplyAgentConnectionChange", json!({"spec":spec,"accept_digest":edit_preview["accept_digest"],"dependency_digest":edit_preview["dependency_digest"],"expected_revisions":edit_preview["expected_revisions"],"idempotency_key":"edit-profile"}), None));
    assert!(parked_edit.error.is_none(), "{parked_edit:?}");
    let parked_edit = parked_edit.data.unwrap();
    assert_eq!(parked_edit["state"], "activating");
    assert!(runtime.adapter.guard_codex_pending_change(None).is_err());
    fs::write(&profile, before_edit).unwrap();
    let edited = service.dispatch_wire(request("ApplyAgentConnectionChange", json!({"schema":"hiroute.agent-settings-retry/v1","context_id":context,"operation_id":parked_edit["operation_id"]}), None));
    assert!(edited.error.is_none(), "{edited:?}");
    let edited = edited.data.unwrap();
    assert_eq!(edited["state"], "succeeded");
    assert!(runtime.adapter.guard_codex_pending_change(None).is_ok());
    assert!(
        fs::read_to_string(&profile)
            .unwrap()
            .contains("user_edit = true")
    );
    let operation: OperationId = serde_json::from_value(edited["operation_id"].clone()).unwrap();
    let restore = json!({"schema_version":{"major":2,"minor":0},"context_id":context,"model":{"intent":"restore","restore_point_ref":codex_model_restore_point_ref(&operation)}});
    // A profile revoke needs neither the original metadata directory nor a valid root file.
    let unavailable_root = "[invalid root TOML";
    fs::write(&root, unavailable_root).unwrap();
    fs::remove_file(root.parent().unwrap().join("models_cache.json")).unwrap();
    let mut ports = runtime.application_ports();
    ports.mutation = Some(Arc::new(fault::FileRace {
        adapter: runtime.adapter.clone(),
        path: profile.clone(),
        fail_after_file: false,
        replacement: None,
    }));
    let racing = LocalControlDaemon::new(ApplicationService::new(ports));
    let preview = racing
        .dispatch_wire(request(
            "PreviewAgentConnectionRestore",
            json!({"spec":restore}),
            None,
        ))
        .data
        .unwrap();
    assert_eq!(preview["applicable"], true, "{preview}");
    let pending = racing.dispatch_wire(request("ApplyAgentConnectionRestore", json!({"spec":restore,"accept_digest":preview["accept_digest"],"dependency_digest":preview["dependency_digest"],"expected_revisions":preview["expected_revisions"],"idempotency_key":"revoke-profile"}), None));
    assert!(pending.error.is_none(), "{pending:?}");
    let failed = pending.data.unwrap();
    assert_eq!(failed["state"], "rolled_back", "{failed}");
    let access = runtime.adapter.codex_access_view().unwrap();
    assert!(access.slot_occupied && !access.access_revoked);
    assert!(access.pending_operation.is_none());
    assert!(runtime.adapter.guard_codex_pending_change(None).is_ok());
    assert!(
        fs::read_to_string(&profile)
            .unwrap()
            .starts_with("# user's concurrent edit")
    );
    // A concurrent edit is preserved while the unused staged restoration is discarded.
    runtime.adapter.reconcile_startup_and_open().unwrap();
    let status = service
        .dispatch_wire(request("GetClientServiceStatus", json!({}), None))
        .data
        .unwrap();
    assert_eq!(status["mutation_available"], true, "{status}");
    fs::write(&root, original).unwrap();
    ensure_target_cache(&runtime.adapter);
    let preserved = fs::read(&profile).unwrap();
    fs::write(&profile, [0xff, 0xfe]).unwrap();
    let damaged = service.dispatch_wire(request("ScanAgents", json!({}), None));
    assert!(damaged.error.is_none(), "{damaged:?}");
    assert_eq!(
        runtime.adapter.codex_access_view().unwrap().conflict_fields,
        ["configuration_encoding"]
    );
    fs::remove_file(&profile).unwrap();
    std::os::unix::fs::symlink(&root, &profile).unwrap();
    let unsafe_target = service.dispatch_wire(request("ScanAgents", json!({}), None));
    assert!(unsafe_target.error.is_none(), "{unsafe_target:?}");
    assert_eq!(
        runtime.adapter.codex_access_view().unwrap().conflict_fields,
        ["configuration_unreadable"]
    );
    fs::remove_file(&profile).unwrap();
    fs::write(&profile, &preserved).unwrap();
    fs::set_permissions(&profile, fs::Permissions::from_mode(0o600)).unwrap();
    // An unrelated real collaboration save remains available after the failed disable.
    let collaboration =
        LocalControlDaemon::new(ApplicationService::new(
            runtime.application_ports().with_agent_connection(Arc::new(
                FixtureFacts::collaboration(runtime.adapter.clone()),
            )),
        ));
    let skill_spec = json!({"schema_version":{"major":2,"minor":0},"context_id":root_spec["context_id"],
        "collaboration":{"intent":"configure","settings":{"trigger_mode":"delegate_by_default"}}});
    apply(
        &collaboration,
        &runtime,
        skill_spec,
        "skill-after-failed-disable",
    );
    // A fresh preview/operation completes restoration without metadata or a valid root.
    fs::write(&root, unavailable_root).unwrap();
    fs::remove_file(root.parent().unwrap().join("models_cache.json")).unwrap();
    apply(&service, &runtime, restore, "retry-disable-after-race");
    assert!(!runtime.adapter.codex_access_view().unwrap().slot_occupied);
    let remaining = fs::read_to_string(&profile).unwrap();
    assert!(
        remaining.contains("# user's concurrent edit") && remaining.contains("user_edit = true")
    );
    assert!(!remaining.contains("X-HiRoute-Token") && !remaining.contains("model_provider ="));
    assert_eq!(fs::read_to_string(&root).unwrap(), unavailable_root);
    fs::write(&root, original).unwrap();
    ensure_target_cache(&runtime.adapter);
    apply(&service, &runtime, root_spec, "switch-after-revoke");
}

#[path = "settings_profile_legacy_tests.rs"]
mod legacy;

#[test]
fn codex_discovery_keeps_desktop_in_default_profile_mode() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::settings_profile_tests::codex_discovery_keeps_desktop_in_default_profile_mode",
    ) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (runtime, _) = open_with_codex_fixture_surfaces(dir.path());
    ensure_target_cache(&runtime.adapter);
    runtime.adapter.reconcile_startup_and_open().unwrap();
    let service = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()));
    let scan = service
        .dispatch_wire(request("ScanAgents", json!({}), None))
        .data
        .unwrap();
    let codex = scan["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["agent_id"] == "agent_codex_default")
        .unwrap();
    assert_eq!(codex["codex_access"]["selected_mode"], "profile");
    let surfaces = codex["available_surfaces"].as_array().unwrap();
    assert!(surfaces.contains(&json!("codex_cli")));
    assert!(surfaces.contains(&json!("codex_desktop")));
}

#[test]
fn preexisting_profile_conflict_preserves_connection_and_other_writes() {
    use hiroute_domain::SecretStorePort;
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::settings_profile_tests::preexisting_profile_conflict_preserves_connection_and_other_writes",
    ) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (runtime, _) = open_with_codex_fixture_surfaces(dir.path());
    ensure_target_cache(&runtime.adapter);
    runtime.adapter.reconcile_startup_and_open().unwrap();
    // The fixture composes a real suspended target; complete the production startup handoff.
    runtime
        .adapter
        .publication_target()
        .unwrap()
        .unwrap()
        .resume_requests()
        .unwrap();
    let service = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()));
    let scan = service
        .dispatch_wire(request("ScanAgents", json!({}), None))
        .data
        .unwrap();
    let codex = scan["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["agent_id"] == "agent_codex_default")
        .unwrap();
    let context = codex["codex_access"]["profile_context_id"].clone();
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
        .find(|p| {
            p.active
                && p.supported_ingress
                    .contains(&AgentIngressProtocolV1::Responses)
        })
        .unwrap();
    let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":context,"model":{"intent":"configure","settings":{
        "mode":"codex_default","native_model_mode":"hiroute_only","fixed_models":[],"allowed_plan_ids":[plan.agent_plan_id],"default_selection":{"kind":"plan","plan_id":plan.agent_plan_id}}}});
    apply(
        &service,
        &runtime,
        spec,
        "create-preexisting-conflict-profile",
    );
    let status = service
        .dispatch_wire(request(
            "GetAgentConnectionStatus",
            json!({"schema_version":{"major":2,"minor":0},"context_id":context}),
            None,
        ))
        .data
        .unwrap();
    let profile = runtime.adapter.scanner.codex_profile_config_target();
    let configured = fs::read_to_string(&profile).unwrap();
    let conflicted = configured.replace("name = \"HiRoute\"", "name = \"My Gateway\"");
    assert_ne!(conflicted, configured);
    fs::write(&profile, &conflicted).unwrap();
    let publication_before = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap();
    let grant_before = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .secrets()
        .inspect_agent_access_grant(
            WorkspaceId::DEFAULT,
            &format!("agent-connection/{}", context.as_str().unwrap()),
        )
        .unwrap();
    let restore = json!({"schema_version":{"major":2,"minor":0},"context_id":context,"model":{"intent":"restore","restore_point_ref":status["restore_point_ref"]}});
    let preview = service
        .dispatch_wire(request(
            "PreviewAgentConnectionRestore",
            json!({"spec":restore}),
            None,
        ))
        .data
        .unwrap();
    assert_eq!(preview["applicable"], true);
    let response = service.dispatch_wire(request("ApplyAgentConnectionRestore", json!({"spec":restore,"accept_digest":preview["accept_digest"],"dependency_digest":preview["dependency_digest"],"expected_revisions":preview["expected_revisions"],"idempotency_key":"revoke-preexisting-conflict"}), None));
    assert!(response.error.is_none(), "{response:?}");
    let failed = response.data.unwrap();
    assert_eq!(failed["state"], "rolled_back", "{failed}");
    let access = runtime.adapter.codex_access_view().unwrap();
    assert!(!access.access_revoked && access.slot_occupied);
    assert!(access.pending_operation.is_none());
    assert!(
        access
            .conflict_fields
            .iter()
            .any(|field| field.contains("name")),
        "{:?}",
        access.conflict_fields
    );
    assert!(
        runtime
            .adapter
            .stores_lock()
            .unwrap()
            .secrets()
            .inspect_agent_access_grant(
                WorkspaceId::DEFAULT,
                &format!("agent-connection/{}", context.as_str().unwrap())
            )
            .unwrap()
            .is_some()
    );
    assert_eq!(fs::read_to_string(&profile).unwrap(), conflicted);
    assert_eq!(
        runtime
            .adapter
            .stores_lock()
            .unwrap()
            .control()
            .active_publication(&WorkspaceId::default())
            .unwrap()
            .unwrap()
            .digest,
        publication_before.digest
    );
    assert_eq!(
        runtime
            .adapter
            .stores_lock()
            .unwrap()
            .secrets()
            .inspect_agent_access_grant(
                WorkspaceId::DEFAULT,
                &format!("agent-connection/{}", context.as_str().unwrap())
            )
            .unwrap(),
        grant_before
    );
    runtime.adapter.reconcile_startup_and_open().unwrap();
    let available = service
        .dispatch_wire(request("GetClientServiceStatus", json!({}), None))
        .data
        .unwrap();
    assert_eq!(available["mutation_available"], true, "{available}");
    assert!(runtime.adapter.guard_codex_pending_change(None).is_ok());
    fs::write(
        &profile,
        format!("# kept user comments\nuser_option = 'retained'\n{configured}"),
    )
    .unwrap();
    let repaired = fs::read(&profile).unwrap();
    let mut fault_ports = runtime.application_ports();
    fault_ports.mutation = Some(Arc::new(fault::FileRace {
        adapter: runtime.adapter.clone(),
        path: profile.clone(),
        fail_after_file: true,
        replacement: None,
    }));
    let fault_service = LocalControlDaemon::new(ApplicationService::new(fault_ports));
    let preview = fault_service
        .dispatch_wire(request(
            "PreviewAgentConnectionRestore",
            json!({"spec":restore}),
            None,
        ))
        .data
        .unwrap();
    let failed_service = fault_service.dispatch_wire(request("ApplyAgentConnectionRestore", json!({"spec":restore,"accept_digest":preview["accept_digest"],"dependency_digest":preview["dependency_digest"],"expected_revisions":preview["expected_revisions"],"idempotency_key":"service-failure-after-restoring-file"}), None));
    assert!(failed_service.error.is_none(), "{failed_service:?}");
    assert_eq!(failed_service.data.unwrap()["state"], "rolled_back");
    assert_eq!(fs::read(&profile).unwrap(), repaired);
    assert!(!runtime.adapter.codex_access_view().unwrap().access_revoked);
    assert_eq!(
        service
            .dispatch_wire(request("GetClientServiceStatus", json!({}), None))
            .data
            .unwrap()["mutation_available"],
        true
    );
    apply(&service, &runtime, restore, "retry-preexisting-conflict");
    let cleaned = fs::read_to_string(&profile).unwrap();
    assert!(
        cleaned.contains("# kept user comments") && cleaned.contains("user_option = 'retained'")
    );
    assert!(!cleaned.contains("X-HiRoute-Token"));
    assert!(!runtime.adapter.codex_access_view().unwrap().slot_occupied);
}

#[test]
fn profile_delete_race_rolls_back_and_releases_writer() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::settings_profile_tests::profile_delete_race_rolls_back_and_releases_writer",
    ) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (runtime, _) = open_with_codex_fixture_surfaces(dir.path());
    ensure_target_cache(&runtime.adapter);
    let root = runtime.adapter.scanner.codex_user_config_target();
    fs::create_dir_all(root.parent().unwrap()).unwrap();
    fs::set_permissions(root.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    let original = "# ordinary entry remains native\nuser_option = true\n";
    fs::write(&root, original).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o600)).unwrap();
    runtime.adapter.reconcile_startup_and_open().unwrap();
    // Complete the same Gateway handoff as the production startup entry.
    runtime
        .adapter
        .finish_startup_publication_recovery()
        .unwrap();
    let service = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()));
    let scan = service
        .dispatch_wire(request("ScanAgents", json!({}), None))
        .data
        .unwrap();
    let codex = scan["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["agent_id"] == "agent_codex_default")
        .unwrap();
    let access = &codex["codex_access"];
    assert_eq!(access["selected_mode"], "profile");
    assert_eq!(codex["context_id"], access["profile_context_id"]);
    assert_ne!(access["root_context_id"], access["profile_context_id"]);
    let context = access["profile_context_id"].clone();
    let profile = runtime.adapter.scanner.codex_profile_config_target();
    let publication = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap()
        .verify()
        .unwrap();
    let plans = publication.published_agent_plans().unwrap();
    let plan = plans
        .iter()
        .find(|p| {
            p.active
                && p.supported_ingress
                    .contains(&AgentIngressProtocolV1::Responses)
        })
        .unwrap();
    let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":context,"model":{"intent":"configure","settings":{
        "mode":"codex_default","native_model_mode":"hiroute_only","fixed_models":[],"allowed_plan_ids":[plan.agent_plan_id],"default_selection":{"kind":"plan","plan_id":plan.agent_plan_id}}}});

    let configured = apply(&service, &runtime, spec.clone(), "create-delete-race");
    let grant = || {
        runtime
            .adapter
            .stores_lock()
            .unwrap()
            .secrets()
            .inspect_agent_access_grant(
                WorkspaceId::DEFAULT,
                &format!("agent-connection/{}", context.as_str().unwrap()),
            )
            .unwrap()
    };
    let grant_before = grant();
    assert!(grant_before.is_some());
    let operation: OperationId =
        serde_json::from_value(configured["operation_id"].clone()).unwrap();
    let restore = json!({"schema_version":{"major":2,"minor":0},"context_id":context,"model":{"intent":"restore","restore_point_ref":codex_model_restore_point_ref(&operation)}});
    let before = fs::read_to_string(&profile).unwrap();
    let mut ports = runtime.application_ports();
    ports.mutation = Some(Arc::new(fault::FileRace {
        adapter: runtime.adapter.clone(),
        path: profile.clone(),
        fail_after_file: false,
        replacement: None,
    }));
    let racing = LocalControlDaemon::new(ApplicationService::new(ports));
    let preview = racing
        .dispatch_wire(request(
            "PreviewAgentConnectionRestore",
            json!({"spec":restore}),
            None,
        ))
        .data
        .unwrap();
    assert_eq!(preview["applicable"], true);
    let failed = racing.dispatch_wire(request("ApplyAgentConnectionRestore", json!({"spec":restore,"accept_digest":preview["accept_digest"],"dependency_digest":preview["dependency_digest"],"expected_revisions":preview["expected_revisions"],"idempotency_key":"delete-race"}), None));
    assert!(failed.error.is_none(), "{failed:?}");
    assert_eq!(failed.data.unwrap()["state"], "rolled_back");
    assert_eq!(grant(), grant_before);
    assert_eq!(
        fs::read_to_string(&profile).unwrap(),
        format!("# user's concurrent edit\n{before}")
    );
    let access = runtime.adapter.codex_access_view().unwrap();
    assert!(access.slot_occupied && !access.access_revoked);
    assert!(access.pending_operation.is_none());
    let status = service
        .dispatch_wire(request("GetClientServiceStatus", json!({}), None))
        .data
        .unwrap();
    assert_eq!(status["mutation_available"], true);
    apply(&service, &runtime, restore, "retry-delete-race");
    assert!(!runtime.adapter.codex_access_view().unwrap().slot_occupied);
    assert!(!profile.exists());
}
