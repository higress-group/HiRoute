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
