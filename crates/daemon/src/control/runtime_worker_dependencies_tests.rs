use hiroute_application::ApplicationService;
use hiroute_application_api::{
    ErrorCode, LOCAL_CONTROL_SCHEMA_V2, LocalControlWireRequestV2, PrincipalKind,
    ProtectedClientGrantV2, WORKER_DEPENDENCIES_SELECT_OPERATION_V1,
    WorkerDependenciesSelectRequestV1, WorkerHarnessV1,
};

use super::*;

fn executable(path: &std::path::Path) {
    std::fs::write(path, b"fixture").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
}

fn private_root(path: &std::path::Path) {
    #[cfg(windows)]
    hiroute_platform::windows::security::set_owner_only(path, true).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
}

fn select_request(root: &std::path::Path, suffix: &str) -> WorkerDependenciesSelectRequestV1 {
    // Real clients submit paths exactly as the discovery scanner returned them, i.e. already
    // canonicalized; admission compares against that normalized form.
    let directory = root.canonicalize().unwrap().join(suffix);
    std::fs::create_dir_all(&directory).unwrap();
    let adapter = directory.join("adapter.js");
    let cli = directory.join("codex");
    let node = directory.join("node");
    std::fs::write(&adapter, b"export {};").unwrap();
    executable(&cli);
    executable(&node);
    WorkerDependenciesSelectRequestV1 {
        harness: WorkerHarnessV1::CodexCli,
        adapter_path: Some(adapter.to_string_lossy().into_owned()),
        cli_path: cli.to_string_lossy().into_owned(),
        node_path: Some(node.to_string_lossy().into_owned()),
        expected_selection_revision: 0,
    }
}

fn wire(
    request: &WorkerDependenciesSelectRequestV1,
    grant: Option<ProtectedClientGrantV2>,
) -> LocalControlWireRequestV2 {
    LocalControlWireRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: "worker-dependencies-selection-test".into(),
        operation_id: WORKER_DEPENDENCIES_SELECT_OPERATION_V1.into(),
        payload: serde_json::to_value(request).unwrap(),
        protected_grant: grant,
    }
}

#[test]
fn local_worker_dependency_selection_is_cas_durable_and_replays_before_metadata() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::worker_dependencies_tests::local_worker_dependency_selection_is_cas_durable_and_replays_before_metadata",
    ) {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    private_root(directory.path());
    let runtime = ProductionControlRuntime::open_with_release_catalog(
        directory.path().join("storage"),
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    let daemon =
        super::super::LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()))
            .with_released_commands_only();
    let request = select_request(directory.path(), "first");

    let applied = daemon.dispatch_wire(wire(&request, None));
    assert!(applied.error.is_none(), "{applied:?}");
    assert_eq!(applied.operation.as_ref().unwrap().state, "succeeded");
    assert_eq!(
        applied.data.as_ref().unwrap()["selection_revisions"][0]["revision"],
        1
    );
    let selected = runtime
        .adapter
        .selection(WorkerHarnessV1::CodexCli)
        .unwrap()
        .unwrap();
    assert_eq!(selected.revision, 1);
    assert_eq!(
        selected.config.adapter,
        request.adapter_path.as_ref().map(std::path::PathBuf::from)
    );

    std::fs::remove_dir_all(directory.path().join("first")).unwrap();
    let replay = daemon.dispatch_wire(wire(&request, None));
    assert!(replay.error.is_none(), "{replay:?}");
    assert_eq!(replay.operation, applied.operation);
    assert_eq!(
        replay.data.as_ref().unwrap()["selected"][0]["adapter_path"].as_str(),
        request.adapter_path.as_deref()
    );
    assert!(
        replay.data.as_ref().unwrap()["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| candidate["source"] == "selected" && candidate["state"] == "missing")
    );

    let stale = select_request(directory.path(), "stale");
    let stale = daemon.dispatch_wire(wire(&stale, None));
    assert_eq!(stale.error.unwrap().code, ErrorCode::RevisionConflict);
}

#[test]
fn collaboration_grant_cannot_select_worker_dependencies() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::worker_dependencies_tests::collaboration_grant_cannot_select_worker_dependencies",
    ) {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    private_root(directory.path());
    let runtime = ProductionControlRuntime::open_with_release_catalog(
        directory.path().join("storage"),
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    let daemon =
        super::super::LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()))
            .with_released_commands_only();
    let request = select_request(directory.path(), "collaboration");
    let denied = daemon.dispatch_wire(wire(
        &request,
        Some(ProtectedClientGrantV2 {
            principal_kind: PrincipalKind::Skill,
            capability: "not-an-apply-capability".into(),
        }),
    ));
    assert_eq!(denied.error.unwrap().code, ErrorCode::CapabilityDenied);
}

#[test]
fn unavailable_opencode_context_does_not_block_other_agents_or_control_startup() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::worker_dependencies_tests::unavailable_opencode_context_does_not_block_other_agents_or_control_startup",
    ) {
        return;
    }
    let root = crate::test_support::private_tempdir();
    let home = root.path().join("home");
    std::fs::create_dir(&home).unwrap();
    private_root(&home);
    let mut layout =
        hiroute_integrations::AgentFilesystemLayoutV1::from_process(&home, root.path());
    layout.opencode_context = None;
    let registry = serde_json::from_slice(include_bytes!(
        "../../../../assets/connector-registry/current/registry-seed.json"
    ))
    .unwrap();
    let models: hiroute_domain::ReleaseModelDataBundleV2 = serde_json::from_slice(include_bytes!(
        "../../../../assets/release-facts/current/bundle/model-data.json"
    ))
    .unwrap();
    let scanner = hiroute_integrations::FilesystemAgentScannerV1::new(
        layout,
        hiroute_integrations::ClaudeRegistrationIndexV1::from_verified_model_data(
            &registry,
            &models.data,
        )
        .unwrap(),
    );
    assert!(
        scanner
            .native_routing_adapter_target(hiroute_domain::AgentKindV1::OpenCode)
            .is_none()
    );
    let runtime = ProductionControlRuntime::prepare_for_role_all_with_scanner(
        &root.path().join("storage"),
        crate::release_catalog::fixture_catalog(),
        None,
        scanner,
    )
    .unwrap();
    runtime.adapter.reconcile_startup_and_open().unwrap();
    assert!(runtime.adapter.scanner.opencode_native_context().is_err());
    let daemon =
        super::super::LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()))
            .with_released_commands_only();
    let selected = daemon.dispatch_wire(wire(&select_request(root.path(), "codex"), None));
    assert!(selected.error.is_none(), "{selected:?}");
}

#[test]
fn opencode_selected_entry_is_discovered_after_restart() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::worker_dependencies_tests::opencode_selected_entry_is_discovered_after_restart",
    ) {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    private_root(directory.path());
    let root = directory.path().canonicalize().unwrap();
    let cli = root.join("opencode");
    executable(&cli);
    let storage = root.join("storage");
    {
        let runtime = ProductionControlRuntime::open_with_release_catalog(
            &storage,
            crate::release_catalog::fixture_catalog(),
        )
        .unwrap();
        let daemon = super::super::LocalControlDaemon::new(ApplicationService::new(
            runtime.application_ports(),
        ))
        .with_released_commands_only();
        let request = WorkerDependenciesSelectRequestV1 {
            harness: WorkerHarnessV1::OpenCode,
            cli_path: cli.to_string_lossy().into_owned(),
            adapter_path: None,
            node_path: None,
            expected_selection_revision: 0,
        };
        let applied = daemon.dispatch_wire(wire(&request, None));
        assert!(applied.error.is_none(), "{applied:?}");
        assert_eq!(applied.operation.as_ref().unwrap().state, "succeeded");
    }
    let reopened = ProductionControlRuntime::open_with_release_catalog(
        &storage,
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    assert_eq!(
        reopened.adapter.scanner.opencode_executable_target(),
        Some(cli)
    );
    let agents = reopened.adapter.discover().unwrap();
    assert!(agents.iter().any(|agent| {
        agent.agent_id == "agent_opencode_default"
            && agent.supported
            && agent
                .available_surfaces
                .contains(&hiroute_application_api::AgentModelSurfaceV2::OpenCodeCli)
    }));
}
