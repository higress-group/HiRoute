//! Wire/Operation/native-file coverage. Capability fixtures are not native Agent execution.
use super::*;
use crate::control::runtime::settings_facts::SettingsAgentClass;
use std::path::{Path, PathBuf};

fn private_file(path: &Path, bytes: &[u8]) {
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent).unwrap();
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn shared_management_and_collaboration_coexist_for_all_existing_agent_families() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::facets::shared_skills::shared_management_and_collaboration_coexist_for_all_existing_agent_families",
    ) {
        return;
    }
    let root = crate::test_support::private_tempdir();
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let executable = root.path().join("native-entry");
    private_file(&executable, b"#!/bin/sh\nprintf 'native fixture 1.0\\n'\n");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let mut layout = AgentFilesystemLayoutV1::from_process(&home, root.path());
    layout.codex_executable = executable.clone();
    layout.claude_executable = executable.clone();
    layout.qoder_executable = executable.clone();
    layout.dsh_executable = executable.clone();
    layout.opencode_executable = executable;
    layout.opencode_context =
        Some(hiroute_integrations::OpenCodeNativeContext::from_lookup(&home, |_| None).unwrap());
    let package = root.path().join("pi-package");
    private_file(&package.join("package.json"),br#"{"name":"@earendil-works/pi-coding-agent","version":"1.0.2","bin":{"pi":"dist/bundle/cli.js"}}"#);
    private_file(&package.join("dist/index.js"), b"fixture resource SDK");
    private_file(&package.join("dist/bundle/cli.js"), b"#!/bin/sh\nexit 97\n");
    fs::set_permissions(
        package.join("dist/bundle/cli.js"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    layout.pi_executable = package.join("dist/bundle/cli.js");
    private_file(&layout.pi_config_root.join("settings.json"), b"{}");
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
        &root.path().join("storage"),
        crate::release_catalog::fixture_catalog(),
        None,
        scanner,
    )
    .unwrap();
    runtime.adapter.reconcile_startup_and_open().unwrap();
    let service =
        LocalControlDaemon::new(ApplicationService::new(
            runtime.application_ports().with_agent_connection(Arc::new(
                FixtureFacts::collaboration(runtime.adapter.clone()),
            )),
        ));
    for class in [
        SettingsAgentClass::Codex,
        SettingsAgentClass::Claude,
        SettingsAgentClass::Qoder,
        SettingsAgentClass::Pi,
        SettingsAgentClass::Dsh,
        SettingsAgentClass::OpenCode,
    ] {
        let context = runtime.adapter.settings_context(class);
        let model = class.native_path(&runtime.adapter.scanner).unwrap();
        // An unrelated malformed model document cannot block a Skill-only save.
        let original = b"unrelated malformed model configuration [\n";
        private_file(&model, original);
        let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":context,
            "management_skill":{"intent":"configure","settings":{}},
            "collaboration":{"intent":"configure","settings":{"trigger_mode":"explicit"}}});
        let outcome = apply(
            &service,
            &runtime,
            spec,
            &format!("shared-skills-both-{}", class.agent_id()),
        );
        let operation_id: OperationId =
            serde_json::from_value(outcome["operation_id"].clone()).unwrap();
        let operation = runtime
            .adapter
            .stores_lock()
            .unwrap()
            .control()
            .load_operation(&operation_id)
            .unwrap()
            .unwrap();
        assert_eq!(operation.plan.external().len(), 2);
        assert!(operation.plan.agent_access_grants().is_empty());
        let read = || {
            let response=service.dispatch_wire(request("GetAgentConnectionStatus",json!({"schema_version":{"major":2,"minor":0},"context_id":context,"include_skills":true}),None));
            assert!(response.error.is_none(), "{class:?}: {response:?}");
            response.data.unwrap()
        };
        let status = read();
        assert_eq!(
            status["management_skill"]["state"], "configured",
            "{class:?}"
        );
        assert_eq!(status["collaboration"]["state"], "configured", "{class:?}");
        let (_, management_target) = runtime
            .adapter
            .settings_skill_resource(&context, None, AgentSettingsFacet::ManagementSkill)
            .unwrap();
        let (_, collaboration_target) = runtime
            .adapter
            .settings_skill_resource(&context, None, AgentSettingsFacet::Collaboration)
            .unwrap();
        let management = runtime
            .adapter
            .artifacts
            .native_target_path(&management_target)
            .unwrap();
        let collaboration = runtime
            .adapter
            .artifacts
            .native_target_path(&collaboration_target)
            .unwrap();
        assert!(management.is_file() && collaboration.is_file());
        assert_eq!(read(), status, "warm history changed status for {class:?}");
        let management_bytes = fs::read(&management).unwrap();
        fs::write(&management, b"externally modified Skill\n").unwrap();
        let drift = read();
        assert_eq!(
            drift["management_skill"]["state"], "drift",
            "warm history hid native drift for {class:?}"
        );
        assert_eq!(drift["collaboration"]["state"], "configured");
        fs::write(&management, management_bytes).unwrap();
        assert_eq!(
            read(),
            status,
            "restored bytes did not restore current status for {class:?}"
        );
        let restore = json!({"schema_version":{"major":2,"minor":0},"context_id":context,
            "management_skill":{"intent":"restore","restore_point_ref":status["management_skill"]["restore_point_ref"]}});
        let remove_key = format!("shared-skills-restore-management-{}", class.agent_id());
        let preview = service.dispatch_wire(request(
            "PreviewAgentConnectionRestore",
            json!({"spec":restore}),
            None,
        ));
        assert!(preview.error.is_none(), "{class:?}: {preview:?}");
        let preview = preview.data.unwrap();
        let remove_payload = json!({"spec":restore, "accept_digest":preview["accept_digest"],
            "dependency_digest":preview["dependency_digest"], "expected_revisions":preview["expected_revisions"],
            "idempotency_key":remove_key});
        let removed = apply(&service, &runtime, restore, &remove_key);
        assert!(!management.exists());
        assert!(collaboration.is_file());
        assert_eq!(read()["collaboration"]["state"], "configured");
        let status = read();
        let restore = json!({"schema_version":{"major":2,"minor":0},"context_id":context,
            "collaboration":{"intent":"restore","restore_point_ref":status["collaboration"]["restore_point_ref"]}});
        apply(
            &service,
            &runtime,
            restore,
            &format!("shared-skills-restore-collaboration-{}", class.agent_id()),
        );
        assert!(!collaboration.exists());
        // Either resource can be prepared first; a joint restore retains distinct references.
        for facet in ["collaboration", "management_skill"] {
            let value = if facet == "collaboration" {
                json!({"trigger_mode":"explicit"})
            } else {
                json!({})
            };
            let mut spec = json!({"schema_version":{"major":2,"minor":0},"context_id":context});
            spec[facet] = json!({"intent":"configure","settings":value});
            apply(
                &service,
                &runtime,
                spec,
                &format!("ordered-{}-{facet}", class.agent_id()),
            );
        }
        // Retrying a settled removal after a fresh installation returns the old
        // result; it must not replay cleanup against the new installation record.
        let before_retry = read();
        let management_before_retry = fs::read(&management).unwrap();
        let collaboration_before_retry = fs::read(&collaboration).unwrap();
        let replay =
            service.dispatch_wire(request("ApplyAgentConnectionRestore", remove_payload, None));
        assert!(
            replay.error.is_none(),
            "settled removal replay after reinstall {class:?}: {replay:?}"
        );
        assert_eq!(
            replay.data.unwrap()["operation_id"],
            removed["operation_id"]
        );
        // Ordinary Apply takes the early replay read. Explicit retry enters the
        // coordinator and must also respect durable completion.
        let retry = service.dispatch_wire(request(
            "ApplyAgentConnectionChange",
            json!({
            "schema":"hiroute.agent-settings-retry/v1", "context_id":context,
            "operation_id":removed["operation_id"]}),
            None,
        ));
        assert!(
            retry.error.is_none(),
            "settled explicit retry after reinstall {class:?}: {retry:?}"
        );
        assert_eq!(retry.data.unwrap()["operation_id"], removed["operation_id"]);
        assert_eq!(read(), before_retry);
        assert_eq!(fs::read(&management).unwrap(), management_before_retry);
        assert_eq!(
            fs::read(&collaboration).unwrap(),
            collaboration_before_retry
        );
        let status = read();
        let restore = json!({"schema_version":{"major":2,"minor":0},"context_id":context,
            "management_skill":{"intent":"restore","restore_point_ref":status["management_skill"]["restore_point_ref"]},
            "collaboration":{"intent":"restore","restore_point_ref":status["collaboration"]["restore_point_ref"]}});
        apply(
            &service,
            &runtime,
            restore,
            &format!("restore-both-{}", class.agent_id()),
        );
        assert!(!management.exists() && !collaboration.exists());
        assert_eq!(fs::read(&model).unwrap(), original);
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
    }
    let stores = runtime.adapter.stores_lock().unwrap();
    let kinds = &["ApplyAgentConnectionChange", "ApplyAgentConnectionRestore"];
    let first = stores
        .control()
        .verified_agent_history_for_kinds(&WorkspaceId::default(), kinds)
        .unwrap();
    let second = stores
        .control()
        .verified_agent_history_for_kinds(&WorkspaceId::default(), kinds)
        .unwrap();
    assert!(
        first.len() >= 36,
        "six families must retain independent configure/restore histories"
    );
    assert_eq!(first.len(), second.len());
    assert!(first.iter().zip(second).all(|(a, b)| Arc::ptr_eq(a, &b)));
}
