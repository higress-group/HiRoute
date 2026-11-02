//! Explicit first-save merges user preferences; the existing protected journal owns only new fields.
use super::*;

#[test]
fn codex_profile_merges_preferences_and_restores_after_reopen() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::settings_profile_tests::merge::codex_profile_merges_preferences_and_restores_after_reopen",
    ) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (runtime, _) = open_with_codex_fixture_surfaces(dir.path());
    ensure_target_cache(&runtime.adapter);
    let root = runtime.adapter.scanner.codex_user_config_target();
    fs::create_dir_all(root.parent().unwrap()).unwrap();
    fs::set_permissions(root.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    let ordinary = "# ordinary entry\nuser_option = true\n";
    fs::write(&root, ordinary).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o600)).unwrap();
    let profile = runtime.adapter.scanner.codex_profile_config_target();
    let preferences = "# existing native preferences\nmodel_reasoning_effort = 'high'\nscreen_reader_detection_done = true\n[custom]\nsetting = 'keep'\n";
    fs::write(&profile, preferences).unwrap();
    fs::set_permissions(&profile, fs::Permissions::from_mode(0o600)).unwrap();
    runtime.adapter.reconcile_startup_and_open().unwrap();
    runtime
        .adapter
        .finish_startup_publication_recovery()
        .unwrap();
    let context = runtime
        .adapter
        .codex_access_view()
        .unwrap()
        .profile_context_id;
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
    let service = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()));
    let configured = apply(&service, &runtime, spec, "merge-existing-profile");
    let operation: OperationId =
        serde_json::from_value(configured["operation_id"].clone()).unwrap();
    let configured = fs::read_to_string(&profile).unwrap();
    assert!(
        configured.contains("# existing native preferences")
            && configured.contains("model_reasoning_effort = 'high'")
            && configured.contains("screen_reader_detection_done = true")
    );
    assert_eq!(fs::read_to_string(&root).unwrap(), ordinary);
    fs::write(
        &profile,
        configured.replace("setting = 'keep'", "setting = 'later edit'"),
    )
    .unwrap();
    drop(service);
    drop(runtime);
    let (reopened, _) = open_with_codex_fixture_surfaces(dir.path());
    reopened.adapter.reconcile_startup_and_open().unwrap();
    reopened
        .adapter
        .finish_startup_publication_recovery()
        .unwrap();
    assert!(reopened.adapter.codex_access_view().unwrap().slot_occupied);
    let service = LocalControlDaemon::new(ApplicationService::new(reopened.application_ports()));
    let restore = json!({"schema_version":{"major":2,"minor":0},"context_id":context,"model":{"intent":"restore","restore_point_ref":codex_model_restore_point_ref(&operation)}});
    apply(&service, &reopened, restore, "restore-existing-profile");
    assert_eq!(
        fs::read_to_string(&profile).unwrap(),
        preferences.replace("setting = 'keep'", "setting = 'later edit'")
    );
    assert_eq!(fs::read_to_string(&root).unwrap(), ordinary);
    assert!(!reopened.adapter.codex_access_view().unwrap().slot_occupied);
}
