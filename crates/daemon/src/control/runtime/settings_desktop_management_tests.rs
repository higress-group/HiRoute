//! Real wire/Operation/file ownership with Desktop carriers and no separate native CLIs.
use super::*;
use std::path::{Path, PathBuf};

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn desktop_management_shares_canonical_status_and_restores_after_reopen_without_cli_or_app() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::facets::desktop_management::desktop_management_shares_canonical_status_and_restores_after_reopen_without_cli_or_app",
    ) {
        return;
    }
    let root = crate::test_support::private_tempdir();
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let mut layout = AgentFilesystemLayoutV1::from_process(&home, root.path());
    layout.codex_executable = root.path().join("missing-codex-cli");
    layout.dsh_executable = root.path().join("missing-dsh-cli");
    let codex_engine = root.path().join("desktop-codex-engine");
    write(&codex_engine, b"#!/bin/sh\nexit 97\n");
    fs::set_permissions(&codex_engine, fs::Permissions::from_mode(0o700)).unwrap();
    layout.codex_desktop_executable = Some(codex_engine.clone());
    let contents = root.path().join("DSH.app/Contents");
    let dsh_engine = contents.join("MacOS/DSH");
    write(
        &contents.join("Resources/app.asar"),
        b"controlled native composition",
    );
    write(
        &dsh_engine,
        br#"#!/bin/sh
[ "$ELECTRON_RUN_AS_NODE" = 1 ] || exit 90
[ "$HOME" = "$DSH_HOME" ] && [ "$PWD" = "$HOME" ] || exit 91
printf '%s\n' '- id: skill-filesystem' '- id: tool-skill' '- id: skill'
"#,
    );
    fs::set_permissions(&dsh_engine, fs::Permissions::from_mode(0o700)).unwrap();
    layout.dsh_desktop_executable = Some(dsh_engine.clone());
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
    let storage = root.path().join("storage");
    let build = || {
        let runtime = ProductionControlRuntime::prepare_for_role_all_with_scanner(
            &storage,
            crate::release_catalog::fixture_catalog(),
            None,
            scanner.clone(),
        )
        .unwrap();
        runtime.adapter.reconcile_startup_and_open().unwrap();
        let service = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()));
        (runtime, service)
    };
    let (mut runtime, mut service) = build();
    let mut saved = Vec::new();
    for (owner, desktop) in [
        ("agent_codex_default", "agent_codex_desktop"),
        ("agent_dsh_default", "agent_dsh_desktop"),
    ] {
        let context = runtime.adapter.settings_context_for_agent(owner).unwrap();
        let desktop_context = runtime.adapter.settings_context_for_agent(desktop).unwrap();
        let native_paths: Vec<_> = [owner, desktop]
            .into_iter()
            .map(|id| {
                let context = runtime.adapter.settings_context_for_agent(id).unwrap();
                runtime
                    .adapter
                    .settings_agent_for_context(&context)
                    .unwrap()
                    .native_path(&runtime.adapter.scanner)
                    .unwrap()
            })
            .collect();
        let original = if owner == "agent_codex_default" {
            b"# native Codex settings\n".as_slice()
        } else {
            b"# native DSH settings\n[]\n".as_slice()
        };
        for path in &native_paths {
            write(path, original);
        }
        let read = |id: &str| {
            let response = service.dispatch_wire(request("GetAgentConnectionStatus", json!({"schema_version":{"major":2,"minor":0},"context_id":id,"include_skills":true}),None));
            assert!(response.error.is_none(), "{response:?}");
            response.data.unwrap()
        };
        let before = read(&desktop_context);
        let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":context,"management_skill":{"intent":"configure","settings":{}}});
        let outcome = apply(
            &service,
            &runtime,
            spec,
            &format!("desktop-management-{owner}"),
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
        assert_eq!(operation.plan.external().len(), 1);
        assert!(operation.plan.agent_access_grants().is_empty());
        for path in &native_paths {
            assert_eq!(fs::read(path).unwrap(), original);
        }
        let status = read(&desktop_context);
        assert_eq!(
            status["management_skill"],
            read(&context)["management_skill"]
        );
        assert_eq!(status["management_skill"]["state"], "configured");
        assert_eq!(
            status["management_skill"]["resource"]["shared_context_count"],
            1
        );
        assert_eq!(
            status["state"], before["state"],
            "management changed Desktop model state"
        );
        assert!(
            status.get("collaboration").is_none(),
            "management invented Desktop delegation"
        );
        let summary = service.dispatch_wire(request(
            "GetAgentSettingsSummary",
            json!({"context_ids":[context,desktop_context]}),
            None,
        ));
        assert!(summary.error.is_none(), "{summary:?}");
        let entries = summary.data.unwrap();
        assert_eq!(
            entries["entries"][0]["management_skill"],
            entries["entries"][1]["management_skill"]
        );
        assert_eq!(
            entries["entries"][1]["management_skill"]["state"],
            "recorded"
        );
        saved.push((
            context,
            desktop_context,
            status["management_skill"].clone(),
            native_paths,
            original.to_vec(),
        ));
    }
    drop(service);
    drop(runtime);
    (runtime, service) = build();
    fs::remove_file(codex_engine).unwrap();
    fs::remove_file(dsh_engine).unwrap();
    for (index, (context, desktop_context, saved_status, native_paths, original)) in
        saved.into_iter().enumerate()
    {
        let read = |id: &str| {
            let response=service.dispatch_wire(request("GetAgentConnectionStatus",json!({"schema_version":{"major":2,"minor":0},"context_id":id,"include_skills":true}),None));
            assert!(response.error.is_none(), "{response:?}");
            response.data.unwrap()
        };
        assert_eq!(read(&desktop_context)["management_skill"], saved_status);
        let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":context,"management_skill":{"intent":"restore","restore_point_ref":saved_status["restore_point_ref"]}});
        apply(
            &service,
            &runtime,
            spec,
            &format!("desktop-management-restore-{index}"),
        );
        let restored = read(&desktop_context);
        assert_eq!(
            restored["management_skill"],
            read(&context)["management_skill"]
        );
        assert_eq!(restored["management_skill"]["state"], "restored");
        for path in &native_paths {
            assert_eq!(fs::read(path).unwrap(), original);
        }
        assert!(
            !Path::new(saved_status["resource"]["directory"].as_str().unwrap())
                .join("SKILL.md")
                .exists()
        );
    }
}
