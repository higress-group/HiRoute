use super::*;
use std::os::unix::fs::MetadataExt;

const RELEASED_EXPLICIT_SKILL: &str =
    include_str!("../../../tests/fixtures/collaboration-skill-v0.3.1/explicit.md");
const RELEASED_DEFAULT_SKILL: &str =
    include_str!("../../../tests/fixtures/collaboration-skill-v0.3.1/default.md");

#[test]
fn v2_settings_skill_only_enable_change_and_disable_keep_model_untouched() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::facets::collaboration::v2_settings_skill_only_enable_change_and_disable_keep_model_untouched",
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    drop(hiroute_local_storage::LocalStorageSet::open_for_daemon_startup(root.path()).unwrap());
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
    let runtime = open_with_scanner(root.path(), scanner);
    let model = runtime.adapter.scanner.codex_user_config_target();
    fs::create_dir_all(model.parent().unwrap()).unwrap();
    fs::set_permissions(model.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    let original_model = b"# keep me\nuser_option = true\n".to_vec();
    fs::write(&model, &original_model).unwrap();
    fs::set_permissions(&model, fs::Permissions::from_mode(0o600)).unwrap();
    runtime.adapter.reconcile_startup_and_open().unwrap();

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
        .unwrap()["codex_access"]["root_context_id"]
        .as_str()
        .unwrap()
        .to_owned();
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
                    .contains(&AgentIngressProtocolV1::Responses)
        })
        .unwrap();
    let enable = json!({
        "schema_version":{"major":2,"minor":0}, "context_id":context,
        "collaboration":{"intent":"configure","settings":{
            "trigger_mode":"delegate_by_default"}}
    });
    let blocked = ordinary.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":enable}),
        None,
    ));
    assert!(blocked.error.is_none(), "{blocked:?}");
    let blocked = blocked.data.unwrap();
    assert_eq!(blocked["applicable"], false);
    assert!(blocked["blockers"].as_array().is_some_and(|blockers| {
        blockers
            .iter()
            .any(|blocker| blocker["reason"] == "capability_unavailable")
    }));

    let service =
        LocalControlDaemon::new(ApplicationService::new(
            runtime.application_ports().with_agent_connection(Arc::new(
                FixtureFacts::collaboration(runtime.adapter.clone()),
            )),
        ));
    assert_worker_plans_empty(&service, "worker-plans-before-enable");
    let legacy = home.join(".hiroute/credential-artifacts/codex.sealed");
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    fs::set_permissions(legacy.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    let material = AgentCollaborationCredential::from_csprng_entropy([76; 32]);
    let grant = AgentCollaborationGrant::issue(
        WorkspaceId::default(),
        context.clone(),
        format!("collaboration-grant/{context}"),
        1,
        BTreeSet::from([plan.agent_plan_id.clone()]),
        &material,
    )
    .unwrap();
    let sealed = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .secrets()
        .seal_collaboration_bootstrap(&grant, &material, &context)
        .unwrap();
    fs::write(&legacy, sealed.as_bytes()).unwrap();
    fs::set_permissions(&legacy, fs::Permissions::from_mode(0o600)).unwrap();

    let skill = home.join(".agents/skills/hiroute-collaboration/SKILL.md");
    let install_released = |spec: serde_json::Value,
                            revision: &'static str,
                            content: &'static str,
                            key: &str| {
        let released = LocalControlDaemon::new(ApplicationService::new(
            runtime.application_ports().with_agent_connection(Arc::new(
                FixtureFacts::released_collaboration(runtime.adapter.clone(), revision, content),
            )),
        ));
        let accepted = apply(&released, &runtime, spec, key);
        assert_eq!(fs::read_to_string(&skill).unwrap(), content);
        let id: OperationId = serde_json::from_value(accepted["operation_id"].clone()).unwrap();
        let operation = runtime
            .adapter
            .stores_lock()
            .unwrap()
            .control()
            .load_operation(&id)
            .unwrap()
            .unwrap();
        (id, serde_json::to_vec(&operation).unwrap())
    };
    let (released_id, released_operation) = install_released(
        enable.clone(),
        "worker-instance-default-v1",
        RELEASED_DEFAULT_SKILL,
        "released-default-skill",
    );
    let enabled = apply(&service, &runtime, enable, "skill-enable");
    assert!(!legacy.exists());
    assert_local_worker_collaboration_management(&service, &context, "delegate_by_default");
    let content = fs::read_to_string(&skill).unwrap();
    assert!(content.contains("Delegate suitable executable work by default"));
    assert!(content.contains("hiroute worker plans"));
    assert!(content.contains("literal value in `--submission-key`"));
    assert!(content.contains("An empty list or unclear match"));
    assert!(!content.contains("--agent"));
    assert!(content.contains(
        "use a restricted policy only when the user asks for one and the selected harness supports it"
    ));
    assert_eq!(fs::read(&model).unwrap(), original_model);
    assert_skill_upgrade(
        &runtime,
        &released_id,
        &released_operation,
        "default",
        &content,
    );

    fs::write(&legacy, b"not a sealed HiRoute artifact").unwrap();
    fs::set_permissions(&legacy, fs::Permissions::from_mode(0o600)).unwrap();
    let enabled_id: OperationId = serde_json::from_value(enabled["operation_id"].clone()).unwrap();
    let enabled_operation = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .load_operation(&enabled_id)
        .unwrap()
        .unwrap();
    assert!(
        runtime
            .adapter
            .finish_settings_collaboration(&enabled_operation)
            .is_err()
    );
    assert_eq!(fs::read(&legacy).unwrap(), b"not a sealed HiRoute artifact");
    fs::remove_file(&legacy).unwrap();

    let change = json!({
        "schema_version":{"major":2,"minor":0}, "context_id":context,
        "collaboration":{"intent":"configure","settings":{"trigger_mode":"explicit"}}
    });
    let (released_id, released_operation) = install_released(
        change.clone(),
        "worker-instance-explicit-v1",
        RELEASED_EXPLICIT_SKILL,
        "released-explicit-skill",
    );
    let changed = apply(&service, &runtime, change.clone(), "skill-change");
    let changed_id: OperationId = serde_json::from_value(changed["operation_id"].clone()).unwrap();
    let changed_operation = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .load_operation(&changed_id)
        .unwrap()
        .unwrap();
    assert!(
        !changed_operation.plan.external().is_empty(),
        "changing the template revision rewrites the owned Skill content"
    );
    let changed_content = fs::read_to_string(&skill).unwrap();
    assert_ne!(changed_content, content);
    assert!(changed_content.contains("only when the user explicitly asks"));
    assert!(changed_content.contains("literal value in `--submission-key`"));
    assert_skill_upgrade(
        &runtime,
        &released_id,
        &released_operation,
        "explicit",
        &changed_content,
    );
    let inode = fs::metadata(&skill).unwrap().ino();
    let reapplied = apply(&service, &runtime, change, "skill-reapply-identical");
    let reapplied_id: OperationId =
        serde_json::from_value(reapplied["operation_id"].clone()).unwrap();
    assert_eq!(fs::read_to_string(&skill).unwrap(), changed_content);
    assert_eq!(fs::metadata(&skill).unwrap().ino(), inode);
    assert_local_worker_collaboration_management(&service, &context, "explicit");
    assert_eq!(fs::read(&model).unwrap(), original_model);
    assert_worker_plans_empty(&service, "worker-plans-after-change");

    let disable = json!({
        "schema_version":{"major":2,"minor":0}, "context_id":context,
        "collaboration":{"intent":"restore","restore_point_ref":codex_model_restore_point_ref(&reapplied_id)}
    });
    apply(&service, &runtime, disable, "skill-disable");
    assert!(!skill.exists());
    let record = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .skill_installation(&WorkspaceId::default(), "skill-root/agent_codex_default")
        .unwrap()
        .unwrap();
    assert!(record.contexts.is_empty());
    assert_eq!(fs::read(&model).unwrap(), original_model);
    assert_worker_plans_empty(&service, "worker-plans-after-disable");
}

fn assert_skill_upgrade(
    runtime: &ProductionControlRuntime,
    released_id: &OperationId,
    released_operation: &[u8],
    mode: &str,
    content: &str,
) {
    let stores = runtime.adapter.stores_lock().unwrap();
    let record = stores
        .control()
        .skill_installation(&WorkspaceId::default(), "skill-root/agent_codex_default")
        .unwrap()
        .unwrap();
    assert_eq!(
        record.template_revision,
        format!(
            "worker-instance-{mode}-v{}",
            if mode == "default" { 3 } else { 2 }
        )
    );
    assert!(record.revision > 1);
    assert_eq!(
        record.content_digest,
        CanonicalDigest::of_bytes(content.as_bytes())
    );
    assert_eq!(
        record.file_ownership,
        hiroute_domain::CollaborationSkillFileOwnership::Managed
    );
    let unchanged = stores
        .control()
        .load_operation(released_id)
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::to_vec(&unchanged).unwrap(), released_operation);
}

#[test]
fn v2_settings_releases_borrowed_skill_after_user_file_drift() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::facets::collaboration::v2_settings_releases_borrowed_skill_after_user_file_drift",
    ) {
        return;
    }

    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    drop(hiroute_local_storage::LocalStorageSet::open_for_daemon_startup(root.path()).unwrap());
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
    let runtime = open_with_scanner(root.path(), scanner);
    runtime.adapter.reconcile_startup_and_open().unwrap();
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
        .unwrap()["codex_access"]["root_context_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let enable = json!({
        "schema_version":{"major":2,"minor":0}, "context_id":context,
        "collaboration":{"intent":"configure","settings":{"trigger_mode":"explicit"}}
    });
    let typed: AgentSettingsSpecV2 = serde_json::from_value(enable.clone()).unwrap();
    let template = runtime
        .adapter
        .capture_settings_facts(&typed)
        .unwrap()
        .skill_file
        .template
        .content;
    let skill = home.join(".agents/skills/hiroute-collaboration/SKILL.md");
    fs::create_dir_all(skill.parent().unwrap()).unwrap();
    fs::set_permissions(skill.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(&skill, template).unwrap();
    fs::set_permissions(&skill, fs::Permissions::from_mode(0o600)).unwrap();
    let original = fs::metadata(&skill).unwrap();

    let service =
        LocalControlDaemon::new(ApplicationService::new(
            runtime.application_ports().with_agent_connection(Arc::new(
                FixtureFacts::collaboration(runtime.adapter.clone()),
            )),
        ));
    let enabled = apply(&service, &runtime, enable, "borrowed-skill-enable");
    let enabled_id: OperationId = serde_json::from_value(enabled["operation_id"].clone()).unwrap();
    let enabled_operation = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .load_operation(&enabled_id)
        .unwrap()
        .unwrap();
    assert!(enabled_operation.plan.external().is_empty());
    let record = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .skill_installation(&WorkspaceId::default(), "skill-root/agent_codex_default")
        .unwrap()
        .unwrap();
    assert_eq!(
        record.file_ownership,
        hiroute_domain::CollaborationSkillFileOwnership::BorrowedIdentical
    );
    assert!(record.file_effect.is_none());
    assert!(record.contexts.contains(&context));
    assert_local_worker_collaboration_management(&service, &context, "explicit");
    assert_eq!(fs::read_to_string(&skill).unwrap(), template);
    assert_eq!(fs::metadata(&skill).unwrap().ino(), original.ino());
    assert_eq!(fs::metadata(&skill).unwrap().mode(), original.mode());

    let disable = json!({
        "schema_version":{"major":2,"minor":0}, "context_id":context,
        "collaboration":{"intent":"restore","restore_point_ref":codex_model_restore_point_ref(&enabled_id)}
    });
    let user_content = "# User changed the borrowed Skill after enabling it.\n";
    fs::write(&skill, user_content).unwrap();
    let drifted = service.dispatch_wire(request(
        "GetAgentConnectionStatus",
        json!({"schema_version":{"major":2,"minor":0},"context_id":context}),
        None,
    ));
    assert!(drifted.error.is_none(), "{drifted:?}");
    assert_eq!(drifted.data.unwrap()["collaboration"]["state"], "drift");

    let preview = service.dispatch_wire(request(
        "PreviewAgentConnectionRestore",
        json!({"spec":disable.clone()}),
        None,
    ));
    assert!(preview.error.is_none(), "{preview:?}");
    let preview = preview.data.unwrap();
    assert_eq!(preview["applicable"], true, "{preview}");

    fs::remove_file(&skill).unwrap();
    let payload = json!({
        "spec":disable,
        "accept_digest":preview["accept_digest"],
        "dependency_digest":preview["dependency_digest"],
        "expected_revisions":preview["expected_revisions"],
        "idempotency_key":"borrowed-skill-disable",
    });
    let disabled = service.dispatch_wire(request(
        "ApplyAgentConnectionRestore",
        payload.clone(),
        None,
    ));
    assert!(disabled.error.is_none(), "{disabled:?}");
    let disabled = disabled.data.unwrap();
    assert_eq!(disabled["state"], "succeeded", "{disabled}");
    let replay = service.dispatch_wire(request("ApplyAgentConnectionRestore", payload, None));
    assert!(replay.error.is_none(), "{replay:?}");
    assert_eq!(
        replay.data.unwrap()["operation_id"],
        disabled["operation_id"]
    );
    let disabled_id: OperationId =
        serde_json::from_value(disabled["operation_id"].clone()).unwrap();
    let disabled_operation = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .load_operation(&disabled_id)
        .unwrap()
        .unwrap();
    assert!(disabled_operation.plan.external().is_empty());
    let record = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .skill_installation(&WorkspaceId::default(), "skill-root/agent_codex_default")
        .unwrap()
        .unwrap();
    assert!(record.contexts.is_empty());
    assert!(!skill.exists());
    let restored = service.dispatch_wire(request(
        "GetAgentConnectionStatus",
        json!({"schema_version":{"major":2,"minor":0},"context_id":context}),
        None,
    ));
    assert!(restored.error.is_none(), "{restored:?}");
    assert_eq!(restored.data.unwrap()["collaboration"]["state"], "restored");
}

fn worker_plans(
    service: &LocalControlDaemon,
    request_id: &str,
) -> api::MachineEnvelopeV2<serde_json::Value> {
    service.dispatch_wire(LocalControlWireRequestV2 {
        schema_version: api::LOCAL_CONTROL_SCHEMA_V2,
        request_id: request_id.into(),
        operation_id: "WorkerPlans".into(),
        payload: json!({}),
        protected_grant: None,
    })
}

fn assert_worker_plans_empty(service: &LocalControlDaemon, request_id: &str) {
    let listed = worker_plans(service, request_id);
    assert!(listed.error.is_none(), "{listed:?}");
    assert_eq!(listed.data.unwrap()["plans"], json!([]));
}

fn assert_local_worker_collaboration_management(
    service: &LocalControlDaemon,
    context: &str,
    trigger_mode: &str,
) {
    let status = service.dispatch_wire(request(
        "GetAgentConnectionStatus",
        json!({"schema_version":{"major":2,"minor":0},"context_id":context}),
        None,
    ));
    assert!(status.error.is_none(), "{status:?}");
    let collaboration = status.data.unwrap()["collaboration"].clone();
    assert_eq!(collaboration["state"], "configured");
    assert_eq!(
        collaboration["current_selection"]["trigger_mode"],
        trigger_mode
    );

    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    assert!(
        !home
            .join(".hiroute/credential-artifacts/codex.sealed")
            .exists()
    );
    let protected = api::ProtectedClientGrantV2 {
        principal_kind: api::PrincipalKind::SealedCollaboration,
        capability: "obsolete-sealed-material".into(),
    };
    let listed = worker_plans(service, "worker-plans");
    assert!(listed.error.is_none(), "{listed:?}");
    let listed = listed.data.unwrap();
    assert_eq!(listed["schema"], "hiroute.work-plan-list/v1");
    assert!(listed["plans"].is_array());

    let injected = service.dispatch_wire(LocalControlWireRequestV2 {
        schema_version: api::LOCAL_CONTROL_SCHEMA_V2,
        request_id: "worker-plans-injected".into(),
        operation_id: "WorkerPlans".into(),
        payload: json!({}),
        protected_grant: Some(protected),
    });
    assert_eq!(
        injected.error.unwrap().code,
        api::ErrorCode::CapabilityDenied
    );
}
