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
    let pending = pending.data.unwrap();
    assert_eq!(pending["state"], "activating", "{pending}");
    let access = runtime.adapter.codex_access_view().unwrap();
    assert!(access.slot_occupied && access.access_revoked);
    assert_eq!(
        access.pending_operation.as_deref(),
        pending["operation_id"].as_str()
    );
    assert!(!access.conflict_fields.is_empty());
    // A damaged target must not hide all Agents or the original cleanup operation.
    fs::write(&root, original).unwrap();
    ensure_target_cache(&runtime.adapter);
    let pending_bytes = fs::read(&profile).unwrap();
    fs::write(&profile, [0xff, 0xfe]).unwrap();
    let damaged = service.dispatch_wire(request("ScanAgents", json!({}), None));
    assert!(damaged.error.is_none(), "{damaged:?}");
    let damaged = damaged.data.unwrap();
    let damaged_access = &damaged["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["agent_id"] == "agent_codex_default")
        .unwrap()["codex_access"];
    assert_eq!(damaged_access["pending_operation"], pending["operation_id"]);
    assert_eq!(damaged_access["access_revoked"], true);
    assert_eq!(
        damaged_access["conflict_fields"],
        json!(["configuration_encoding"])
    );
    fs::remove_file(&profile).unwrap();
    std::os::unix::fs::symlink(&root, &profile).unwrap();
    let unsafe_target = service.dispatch_wire(request("ScanAgents", json!({}), None));
    assert!(unsafe_target.error.is_none(), "{unsafe_target:?}");
    let unsafe_target = unsafe_target.data.unwrap();
    let unsafe_access = &unsafe_target["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["agent_id"] == "agent_codex_default")
        .unwrap()["codex_access"];
    assert_eq!(unsafe_access["pending_operation"], pending["operation_id"]);
    assert_eq!(
        unsafe_access["conflict_fields"],
        json!(["configuration_unreadable"])
    );
    assert_eq!(fs::read_to_string(&root).unwrap(), original);
    fs::remove_file(&profile).unwrap();
    fs::write(&profile, pending_bytes).unwrap();
    fs::set_permissions(&profile, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&root, unavailable_root).unwrap();
    fs::remove_file(root.parent().unwrap().join("models_cache.json")).unwrap();
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
    // Startup keeps the same home reserved and does not overwrite the concurrent edit.
    runtime.adapter.reconcile_startup_and_open().unwrap();
    assert!(
        fs::read_to_string(&profile)
            .unwrap()
            .starts_with("# user's concurrent edit")
    );
    let retry = json!({"schema":"hiroute.agent-settings-retry/v1","context_id":context,"operation_id":pending["operation_id"]});
    let mut wrong = retry.clone();
    wrong["context_id"] = root_spec["context_id"].clone();
    assert!(
        service
            .dispatch_wire(request("ApplyAgentConnectionChange", wrong, None))
            .error
            .is_some()
    );
    // The publication is pinned while the original cleanup is pending.
    use hiroute_domain::ExternalEffectPort;
    let pending_id: OperationId = serde_json::from_value(pending["operation_id"].clone()).unwrap();
    let pending_op = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .load_operation(&pending_id)
        .unwrap()
        .unwrap();
    let publication_intent = pending_op
        .plan
        .external()
        .iter()
        .find(|i| i.kind() == hiroute_domain::OwnedEffectKind::Publication)
        .unwrap();
    assert!(
        runtime
            .adapter
            .validate_external_admission(publication_intent)
            .is_err()
    );
    assert!(runtime.adapter.guard_codex_pending_change(None).is_err());
    assert!(
        runtime
            .adapter
            .guard_codex_pending_change(Some(&pending_id))
            .is_ok()
    );
    // Temporarily restore unrelated root discovery dependencies for the independent writer.
    fs::write(&root, original).unwrap();
    ensure_target_cache(&runtime.adapter);
    // A real collaboration-only write has no publication but owns shared control state.
    let collaboration =
        LocalControlDaemon::new(ApplicationService::new(
            runtime.application_ports().with_agent_connection(Arc::new(
                FixtureFacts::collaboration(runtime.adapter.clone()),
            )),
        ));
    let skill_spec = json!({"schema_version":{"major":2,"minor":0},"context_id":root_spec["context_id"],
        "collaboration":{"intent":"configure","settings":{"trigger_mode":"delegate_by_default"}}});
    let skill_preview = collaboration.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":skill_spec}),
        None,
    ));
    assert!(skill_preview.error.is_none(), "{skill_preview:?}");
    let skill_preview = skill_preview.data.unwrap();
    assert_eq!(skill_preview["applicable"], true, "{skill_preview}");
    let denied_skill = collaboration.dispatch_wire(request("ApplyAgentConnectionChange", json!({"spec":skill_spec,"accept_digest":skill_preview["accept_digest"],"dependency_digest":skill_preview["dependency_digest"],"expected_revisions":skill_preview["expected_revisions"],"idempotency_key":"pending-tail-skill"}), None));
    assert!(denied_skill.error.is_some(), "{denied_skill:?}");
    fs::write(&root, unavailable_root).unwrap();
    fs::remove_file(root.parent().unwrap().join("models_cache.json")).unwrap();
    // User cleans the owned fields but keeps new comments and unrelated settings.
    let cleaned = "# user's concurrent edit\nuser_edit = true\nanother_user_edit = 'retained'\n";
    fs::write(&profile, cleaned).unwrap();
    let completed = service.dispatch_wire(request("ApplyAgentConnectionChange", retry, None));
    assert!(completed.error.is_none(), "{completed:?}");
    let completed = completed.data.unwrap();
    assert_eq!(completed["operation_id"], pending["operation_id"]);
    assert_eq!(completed["state"], "succeeded", "{completed}");
    assert_eq!(fs::read_to_string(&root).unwrap(), unavailable_root);
    let remaining = fs::read_to_string(&profile).unwrap();
    assert_eq!(remaining, cleaned);
    assert!(runtime.adapter.guard_codex_pending_change(None).is_ok());
    runtime.adapter.reconcile_startup_and_open().unwrap();
    assert!(!remaining.contains("X-HiRoute-Token"));
    assert!(!remaining.contains("model_provider ="));
    fs::write(&root, original).unwrap();
    ensure_target_cache(&runtime.adapter);
    // Switch is a fresh root configuration only after restoration has finished.
    apply(&service, &runtime, root_spec, "switch-after-revoke");
}

#[path = "settings_profile_fault.rs"]
mod fault;

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
fn preexisting_profile_conflict_revokes_before_file_preparation() {
    use hiroute_domain::SecretStorePort;
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::settings_profile_tests::preexisting_profile_conflict_revokes_before_file_preparation",
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
        .required_publication_target()
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
    let pending = response.data.unwrap();
    assert_eq!(pending["state"], "activating", "{pending}");
    let access = runtime.adapter.codex_access_view().unwrap();
    assert!(access.access_revoked && access.slot_occupied);
    assert_eq!(
        access.pending_operation.as_deref(),
        pending["operation_id"].as_str()
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
            .is_none()
    );
    assert_eq!(fs::read_to_string(&profile).unwrap(), conflicted);
    // Cleanup is still discoverable after a restart and cannot revive the grant.
    runtime.adapter.reconcile_startup_and_open().unwrap();
    let retry = json!({"schema":"hiroute.agent-settings-retry/v1","context_id":context,"operation_id":pending["operation_id"]});
    // A sealed file tail keeps reads and recovery usable while ordinary writes stay closed.
    let status = service.dispatch_wire(request("GetClientServiceStatus", json!({}), None));
    assert!(status.error.is_none(), "{status:?}");
    let status = status.data.unwrap();
    assert_eq!(status["recovery_ready"], true, "{status}");
    assert_eq!(status["mutation_available"], false, "{status}");
    assert!(
        matches!(status["gateway"].as_str(), Some("ready" | "no_new_calls")),
        "{status}"
    );
    let catalog = service.dispatch_wire(request("ListAgentPlanCatalog", json!({}), None));
    assert!(catalog.error.is_none(), "{catalog:?}");
    assert!(
        !catalog.data.unwrap()["plans"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let blocked = service.dispatch_wire(request("ApplyAgentConnectionChange", retry.clone(), None));
    assert_eq!(blocked.data.unwrap()["state"], "activating");
    let cleaned = "# kept user comments\nuser_option = 'retained'\n";
    fs::write(&profile, cleaned).unwrap();
    let completed = service.dispatch_wire(request("ApplyAgentConnectionChange", retry, None));
    assert!(completed.error.is_none(), "{completed:?}");
    let completed = completed.data.unwrap();
    assert_eq!(completed["state"], "succeeded", "{completed}");
    assert_eq!(completed["operation_id"], pending["operation_id"]);
    assert_eq!(fs::read_to_string(&profile).unwrap(), cleaned);
    assert!(!runtime.adapter.codex_access_view().unwrap().slot_occupied);
}
