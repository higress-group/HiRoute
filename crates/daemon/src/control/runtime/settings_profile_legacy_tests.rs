//! Recovery through current production wire entry from a frozen service-first revoke.
use super::*;
use hiroute_domain::{ControlRepositoryPort, ExternalEffectPort, SecretStorePort};

#[path = "settings_profile_legacy_fixture.rs"]
mod fixture;

#[test]
fn legacy_service_first_revoke_survives_restart_and_same_operation_retry() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::settings_profile_tests::legacy::legacy_service_first_revoke_survives_restart_and_same_operation_retry",
    ) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (runtime, _) = open_with_codex_fixture_surfaces(dir.path());
    ensure_target_cache(&runtime.adapter);
    runtime.adapter.reconcile_startup_and_open().unwrap();
    runtime
        .adapter
        .finish_startup_publication_recovery()
        .unwrap();
    let service = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()));
    let access = runtime.adapter.codex_access_view().unwrap();
    let context = access.profile_context_id;
    let other_context = access.root_context_id;
    let root = runtime.adapter.scanner.codex_user_config_target();
    let original = "# ordinary entry remains native\nuser_option = true\n";
    fs::write(&root, original).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o600)).unwrap();
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
    let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":context,"model":{"intent":"configure","settings":{
        "mode":"codex_default","native_model_mode":"hiroute_only","fixed_models":[],"allowed_plan_ids":[plan.agent_plan_id],"default_selection":{"kind":"plan","plan_id":plan.agent_plan_id}}}});
    let configured = apply(&service, &runtime, spec, "legacy-create-profile");
    let created_id: OperationId =
        serde_json::from_value(configured["operation_id"].clone()).unwrap();
    let profile = runtime.adapter.scanner.codex_profile_config_target();
    let before = fs::read_to_string(&profile).unwrap();
    let restore = json!({"schema_version":{"major":2,"minor":0},"context_id":context,"model":{"intent":"restore","restore_point_ref":codex_model_restore_point_ref(&created_id)}});
    let mut ports = runtime.application_ports();
    ports.mutation = Some(Arc::new(fixture::ServiceFirstRevoke(
        runtime.adapter.clone(),
    )));
    let legacy = LocalControlDaemon::new(ApplicationService::new(ports));
    let preview = legacy.dispatch_wire(request(
        "PreviewAgentConnectionRestore",
        json!({"spec":restore}),
        None,
    ));
    assert!(preview.error.is_none(), "{preview:?}");
    let preview = preview.data.unwrap();
    let result = legacy.dispatch_wire(request("ApplyAgentConnectionRestore", json!({"spec":restore,"accept_digest":preview["accept_digest"],"dependency_digest":preview["dependency_digest"],"expected_revisions":preview["expected_revisions"],"idempotency_key":"legacy-revoke-profile"}), None));
    assert!(result.error.is_none(), "{result:?}");
    let pending = result.data.unwrap();
    assert_eq!(pending["state"], "activating");
    let pending_id: OperationId = serde_json::from_value(pending["operation_id"].clone()).unwrap();
    let op = runtime
        .adapter
        .load_operation(&pending_id)
        .unwrap()
        .unwrap();
    let file = op
        .plan
        .external()
        .iter()
        .find(|i| hiroute_domain::is_settings_managed_configuration(i))
        .unwrap();
    assert!(matches!(
        runtime.adapter.observe_external(&op, file).unwrap(),
        hiroute_domain::EffectReconciliation::Missing
    ));
    assert!(
        runtime
            .adapter
            .stores_lock()
            .unwrap()
            .secrets()
            .inspect_agent_access_grant(
                WorkspaceId::DEFAULT,
                &format!("agent-connection/{context}")
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(fs::read_to_string(&profile).unwrap(), before);
    let conflicted = before.replace("name = \"HiRoute\"", "name = \"My Gateway\"");
    assert_ne!(before, conflicted);
    fs::write(&profile, &conflicted).unwrap();
    drop(legacy);
    drop(service);
    drop(runtime);
    // New process composition: rebuild committed Gateway authority before journal recovery.
    let (runtime, _) = open_with_codex_fixture_surfaces(dir.path());
    runtime.adapter.restore_active_publication().unwrap();
    runtime.adapter.reconcile_startup_and_open().unwrap();
    runtime
        .adapter
        .finish_startup_publication_recovery()
        .unwrap();
    let service = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()));
    assert_eq!(fs::read_to_string(&profile).unwrap(), conflicted);
    let access = runtime.adapter.codex_access_view().unwrap();
    assert!(access.slot_occupied && access.access_revoked);
    assert_eq!(
        access.pending_operation.as_deref(),
        Some(pending_id.as_str())
    );
    assert!(access.conflict_fields.iter().any(|f| f.contains("name")));
    assert!(runtime.adapter.guard_codex_pending_change(None).is_err());
    assert!(
        runtime
            .adapter
            .guard_codex_pending_change(Some(&pending_id))
            .is_ok()
    );
    let op = runtime
        .adapter
        .load_operation(&pending_id)
        .unwrap()
        .unwrap();
    let publication = op
        .plan
        .external()
        .iter()
        .find(|i| i.kind() == OwnedEffectKind::Publication)
        .unwrap();
    assert!(
        runtime
            .adapter
            .validate_external_admission(publication)
            .is_err()
    );
    // Target-local diagnostics must preserve all Agents and the same recovery entry.
    for (bytes, expected) in [
        (Some(vec![0xff, 0xfe]), "configuration_encoding"),
        (None, "configuration_unreadable"),
    ] {
        fs::remove_file(&profile).unwrap();
        if let Some(bytes) = bytes {
            fs::write(&profile, bytes).unwrap();
        } else {
            std::os::unix::fs::symlink(&root, &profile).unwrap();
        }
        let scan = service.dispatch_wire(request("ScanAgents", json!({}), None));
        assert!(scan.error.is_none(), "{scan:?}");
        let scan = scan.data.unwrap();
        let access = &scan["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["agent_id"] == "agent_codex_default")
            .unwrap()["codex_access"];
        assert_eq!(access["pending_operation"], pending["operation_id"]);
        assert_eq!(access["access_revoked"], true);
        assert_eq!(access["conflict_fields"], json!([expected]));
    }
    assert_eq!(fs::read_to_string(&root).unwrap(), original);
    fs::remove_file(&profile).unwrap();
    fs::write(&profile, &conflicted).unwrap();
    fs::set_permissions(&profile, fs::Permissions::from_mode(0o600)).unwrap();
    let retry = json!({"schema":"hiroute.agent-settings-retry/v1","context_id":context,"operation_id":pending["operation_id"]});
    let mut wrong = retry.clone();
    wrong["context_id"] = json!(other_context);
    assert!(
        service
            .dispatch_wire(request("ApplyAgentConnectionChange", wrong, None))
            .error
            .is_some()
    );
    let still_pending =
        service.dispatch_wire(request("ApplyAgentConnectionChange", retry.clone(), None));
    assert!(still_pending.error.is_none(), "{still_pending:?}");
    assert_eq!(still_pending.data.unwrap()["state"], "activating");
    assert_eq!(fs::read_to_string(&profile).unwrap(), conflicted);
    // A collaboration-only writer also cannot invalidate the sealed publication/control facts.
    let collaboration =
        LocalControlDaemon::new(ApplicationService::new(
            runtime.application_ports().with_agent_connection(Arc::new(
                FixtureFacts::collaboration(runtime.adapter.clone()),
            )),
        ));
    let skill_spec = json!({"schema_version":{"major":2,"minor":0},"context_id":other_context,
        "collaboration":{"intent":"configure","settings":{"trigger_mode":"delegate_by_default"}}});
    let preview = collaboration.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":skill_spec}),
        None,
    ));
    assert!(preview.error.is_none(), "{preview:?}");
    let preview = preview.data.unwrap();
    assert_eq!(preview["applicable"], true, "{preview}");
    let denied = collaboration.dispatch_wire(request("ApplyAgentConnectionChange", json!({"spec":skill_spec,"accept_digest":preview["accept_digest"],"dependency_digest":preview["dependency_digest"],"expected_revisions":preview["expected_revisions"],"idempotency_key":"legacy-pending-skill"}), None));
    assert!(denied.error.is_some(), "{denied:?}");
    // User-owned semantic cleanup is accepted without rewriting comments or unrelated fields.
    let cleaned = "# user's concurrent edit\nanother_user_edit = 'retained'\n";
    fs::write(&profile, cleaned).unwrap();
    let invalid_root = "[invalid root TOML";
    fs::write(&root, invalid_root).unwrap();
    fs::remove_file(root.parent().unwrap().join("models_cache.json")).unwrap();
    let done = service.dispatch_wire(request("ApplyAgentConnectionChange", retry, None));
    assert!(done.error.is_none(), "{done:?}");
    let done = done.data.unwrap();
    assert_eq!(done["operation_id"], pending["operation_id"]);
    assert_eq!(done["state"], "succeeded", "{done}");
    assert_eq!(fs::read_to_string(&profile).unwrap(), cleaned);
    assert_eq!(fs::read_to_string(&root).unwrap(), invalid_root);
    assert!(runtime.adapter.guard_codex_pending_change(None).is_ok());
    assert!(!runtime.adapter.codex_access_view().unwrap().slot_occupied);
}
