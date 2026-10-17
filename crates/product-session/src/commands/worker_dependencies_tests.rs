//! Native installation selection before protected Worker admission.
use super::*;
use hiroute_domain::delegation::WorkerHarnessV1;

fn selection(root: &Path) -> WorkerDependenciesSelectRequestV1 {
    WorkerDependenciesSelectRequestV1 {
        harness: WorkerHarnessV1::CodexCli,
        adapter_path: Some(root.join("adapter.js").to_string_lossy().into_owned()),
        cli_path: root.join("codex").to_string_lossy().into_owned(),
        node_path: Some(root.join("node").to_string_lossy().into_owned()),
        expected_selection_revision: 3,
    }
}

fn make_file(path: &Path, mode: u32) {
    fs::write(path, b"fixture").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }
}

fn native_selection(root: &Path) -> WorkerDependenciesSelectRequestV1 {
    WorkerDependenciesSelectRequestV1 {
        harness: WorkerHarnessV1::QoderCli,
        adapter_path: None,
        cli_path: root.join("./qodercli").to_string_lossy().into_owned(),
        node_path: None,
        expected_selection_revision: 3,
    }
}

#[test]
fn qoder_confirmation_retains_only_the_canonical_cli_and_revision() {
    let root = tempfile::tempdir().unwrap();
    make_file(&root.path().join("qodercli"), 0o700);
    let normalized = normalize_worker_dependency_selection(&native_selection(root.path())).unwrap();
    assert_eq!(
        Path::new(&normalized.cli_path),
        fs::canonicalize(root.path().join("qodercli")).unwrap()
    );
    let state = WorkerDependencyConfirmationState::default();
    let confirmation = state.prepare("main", normalized.clone(), 1).unwrap();
    let confirmed = state
        .take("main", &confirmation.confirmation_id, 2)
        .unwrap();
    assert_eq!(confirmed, normalized);
    let wire = serde_json::to_value(&confirmed).unwrap();
    assert!(wire.get("adapter_path").is_none() && wire.get("node_path").is_none());
    let plan = hiroute_application_api::plan_worker_dependency_selection(&confirmed).unwrap();
    assert_eq!(plan.expected_revisions.target, 3);
    assert_eq!(plan.change.after_selection.cli_path, normalized.cli_path);
    assert!(
        state
            .take("main", &confirmation.confirmation_id, 3)
            .is_err()
    );
}

#[test]
fn native_and_adapter_installations_cannot_borrow_each_others_shape() {
    let root = tempfile::tempdir().unwrap();
    let qoder = native_selection(root.path());
    let absent = root
        .path()
        .join("not-installed")
        .to_string_lossy()
        .into_owned();
    for request in [
        WorkerDependenciesSelectRequestV1 {
            adapter_path: Some(absent.clone()),
            ..qoder.clone()
        },
        WorkerDependenciesSelectRequestV1 {
            node_path: Some(absent),
            ..qoder.clone()
        },
        WorkerDependenciesSelectRequestV1 {
            harness: WorkerHarnessV1::CodexCli,
            ..qoder.clone()
        },
        WorkerDependenciesSelectRequestV1 {
            harness: WorkerHarnessV1::ClaudeCode,
            ..qoder
        },
    ] {
        assert!(matches!(
            normalize_worker_dependency_selection(&request),
            Err(DesktopFailure::Native { code }) if code == "REQUEST_INVALID"
        ));
    }
}

#[test]
fn qoder_requires_an_existing_executable_file_before_confirmation() {
    let root = tempfile::tempdir().unwrap();
    let request = native_selection(root.path());
    assert!(matches!(
        normalize_worker_dependency_selection(&request),
        Err(DesktopFailure::Native { code }) if code == "WORKER_DEPENDENCIES_MISSING"
    ));
    fs::create_dir(root.path().join("qodercli")).unwrap();
    assert!(matches!(
        normalize_worker_dependency_selection(&request),
        Err(DesktopFailure::Native { code }) if code == "WORKER_DEPENDENCIES_INVALID"
    ));
    #[cfg(unix)]
    {
        fs::remove_dir(root.path().join("qodercli")).unwrap();
        make_file(&root.path().join("qodercli"), 0o600);
        assert!(matches!(
            normalize_worker_dependency_selection(&request),
            Err(DesktopFailure::Native { code }) if code == "WORKER_DEPENDENCIES_INVALID"
        ));
    }
}

#[test]
fn preparation_canonicalizes_and_checks_each_selected_component() {
    let root = tempfile::tempdir().unwrap();
    make_file(&root.path().join("adapter.js"), 0o600);
    make_file(&root.path().join("codex"), 0o700);
    make_file(&root.path().join("node"), 0o700);
    for harness in [WorkerHarnessV1::CodexCli, WorkerHarnessV1::ClaudeCode] {
        let request = WorkerDependenciesSelectRequestV1 {
            harness,
            ..selection(root.path())
        };
        let normalized = normalize_worker_dependency_selection(&request).unwrap();
        assert_eq!(
            normalized.adapter_path,
            Some(
                fs::canonicalize(root.path().join("adapter.js"))
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            )
        );
        let native_adapter = WorkerDependenciesSelectRequestV1 {
            node_path: None,
            ..normalized
        };
        assert!(normalize_worker_dependency_selection(&native_adapter).is_err());
    }
}

#[test]
fn confirmation_is_window_bound_single_use_and_expiring() {
    let root = tempfile::tempdir().unwrap();
    make_file(&root.path().join("adapter.js"), 0o600);
    make_file(&root.path().join("codex"), 0o700);
    make_file(&root.path().join("node"), 0o700);
    let state = WorkerDependencyConfirmationState::default();
    let first = state.prepare("main", selection(root.path()), 1).unwrap();
    assert!(state.take("other", &first.confirmation_id, 2).is_err());
    let second = state.prepare("main", selection(root.path()), 3).unwrap();
    assert!(state.take("main", &first.confirmation_id, 4).is_err());
    assert_eq!(
        state.take("main", &second.confirmation_id, 5).unwrap(),
        selection(root.path())
    );
    assert!(state.take("main", &second.confirmation_id, 6).is_err());
    let expired = state.prepare("main", selection(root.path()), 10).unwrap();
    assert!(
        state
            .take(
                "main",
                &expired.confirmation_id,
                10 + WORKER_DEPENDENCY_CONFIRMATION_TTL_MS + 1,
            )
            .is_err()
    );
}

#[test]
fn cancellation_is_idempotent_for_the_owning_window() {
    let root = tempfile::tempdir().unwrap();
    let state = WorkerDependencyConfirmationState::default();
    let confirmation = state.prepare("main", selection(root.path()), 1).unwrap();
    state.cancel("main", &confirmation.confirmation_id).unwrap();
    state.cancel("main", &confirmation.confirmation_id).unwrap();
}
