use super::*;
use crate::delegation::installation::WorkerInstallationSelection;

struct StaticSelection(Option<WorkerInstallationSelection>);

impl WorkerInstallationSelectionSource for StaticSelection {
    fn selection(
        &self,
        harness: WorkerHarnessV1,
    ) -> Result<Option<WorkerInstallationSelection>, DelegationErrorV1> {
        Ok(self
            .0
            .as_ref()
            .filter(|selection| selection.config.harness == harness)
            .cloned())
    }
}

fn make_file(path: &Path, mode: u32) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, b"fixture").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }
}

#[test]
fn pi_discovers_path_node_without_an_adapter() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    make_file(&bin.join("pi"), 0o700);
    make_file(&bin.join("node"), 0o700);
    let view = discover_with_environment(
        &StaticSelection(None),
        &WorkerDependenciesDiscoverRequestV1 {
            harness: Some(WorkerHarnessV1::Pi),
        },
        &ScanEnvironment {
            path: Some(std::env::join_paths([&bin]).unwrap()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(view.candidates.iter().any(|candidate| {
        candidate.component == WorkerDependencyComponentV1::Node
            && candidate.path
                == fs::canonicalize(bin.join("node"))
                    .unwrap()
                    .to_string_lossy()
            && candidate.state == WorkerDependencyCandidateStateV1::Found
    }));
    assert!(
        view.candidates
            .iter()
            .all(|candidate| { candidate.component != WorkerDependencyComponentV1::Adapter })
    );
    assert!(
        view.install_hints
            .iter()
            .all(|hint| { hint.component != WorkerDependencyComponentV1::Node })
    );
    assert!(view.selected.is_empty());
}

#[test]
#[cfg(unix)]
fn pi_resume_checks_its_own_capability_instead_of_new_session_admission() {
    let root = tempfile::tempdir().unwrap();
    let cli = root.path().join("bin/pi.js");
    let node = root.path().join("bin/node");
    make_file(&cli, 0o700);
    make_file(&node, 0o700);
    fs::write(root.path().join("package.json"), br#"{"name":"@earendil-works/pi-coding-agent","version":"9.0.0-fixture","bin":{"pi":"bin/pi.js"}}"#).unwrap();
    // Emulates only the offline runner receipt. The JS contract tests check the SDK itself.
    fs::write(&node, b"#!/bin/sh\nif [ \"$1\" = --version ]; then echo v22.19.0; elif [ \"$2\" = --check ] && [ \"$4\" = continue ]; then echo hiroute.pi-sdk-capability/v1:ok; else exit 97; fi\n").unwrap();
    let config = WorkerInstallationConfig {
        harness: WorkerHarnessV1::Pi,
        harness_binary: cli,
        adapter: None,
        node_binary: Some(node),
    };
    assert!(validate_persisted_installation_for_run(&config, true).is_ok());
    assert!(matches!(
        validate_persisted_installation_for_run(&config, false),
        Err(DelegationErrorV1::DependencyCheckFailed(
            hiroute_domain::delegation::NativeDependencyFailureV1 {
                check: hiroute_domain::delegation::NativeDependencyCheckV1::PiSdk,
                reason: hiroute_domain::delegation::NativeDependencyFailureReasonV1::ProcessFailed
            }
        ))
    ));
    fs::write(
        config.node_binary.as_ref().unwrap(),
        b"#!/bin/sh\nif [ \"$1\" = --version ]; then echo v22.19.0; else exit 97; fi\n",
    )
    .unwrap();
    assert!(matches!(
        validate_persisted_installation_for_run(&config, true),
        Err(DelegationErrorV1::DependencyCheckFailed(
            hiroute_domain::delegation::NativeDependencyFailureV1 {
                check: hiroute_domain::delegation::NativeDependencyCheckV1::PiSdk,
                reason: hiroute_domain::delegation::NativeDependencyFailureReasonV1::ProcessFailed
            }
        ))
    ));
}

#[test]
#[cfg(unix)]
fn qoder_discovers_native_aliases_and_official_root_without_inventing_dependencies_or_replacing_selection()
 {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let bin = root.path().join("bin");
    let cli = bin.join("qodercli");
    make_file(&cli, 0o700);
    let invoked = root.path().join("invoked");
    fs::write(
        &cli,
        format!("#!/bin/sh\nprintf invoked > '{}'\n", invoked.display()),
    )
    .unwrap();
    std::os::unix::fs::symlink(&cli, bin.join("qoder")).unwrap();
    make_file(&bin.join("node"), 0o700);
    make_file(&bin.join("codex-acp"), 0o700);
    let official = home.join(".qoder/bin/qodercli/qodercli-1.2.3");
    make_file(&official, 0o700);
    let missing = root.path().join("explicitly-selected-but-missing");
    let config = WorkerInstallationConfig::qoder_native(&missing);
    let source = StaticSelection(Some(WorkerInstallationSelection {
        config: config.clone(),
        revision: 4,
    }));
    let view = discover_with_environment(
        &source,
        &WorkerDependenciesDiscoverRequestV1 {
            harness: Some(WorkerHarnessV1::QoderCli),
        },
        &ScanEnvironment {
            path: Some(std::env::join_paths([&bin]).unwrap()),
            home: Some(home),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(view.selected.len(), 1);
    assert_eq!(view.selected[0].cli_path, missing.to_string_lossy());
    assert_eq!(view.selected[0].adapter_path, None);
    assert_eq!(view.selected[0].node_path, None);
    assert!(
        view.candidates
            .iter()
            .all(|entry| entry.component == WorkerDependencyComponentV1::Cli)
    );
    assert!(
        view.install_hints
            .iter()
            .all(|hint| hint.component == WorkerDependencyComponentV1::Cli)
    );
    assert_eq!(
        view.candidates
            .iter()
            .filter(|entry| entry.path == cli.to_string_lossy())
            .count(),
        1
    );
    assert!(
        view.candidates
            .iter()
            .any(|entry| entry.path == official.to_string_lossy()
                && entry.state == WorkerDependencyCandidateStateV1::Found)
    );
    assert!(matches!(
        validate_persisted_installation(&config),
        Err(DelegationErrorV1::DependenciesMissing)
    ));
    let request = WorkerDependenciesSelectRequestV1 {
        harness: WorkerHarnessV1::QoderCli,
        cli_path: cli.to_string_lossy().into_owned(),
        adapter_path: None,
        node_path: None,
        expected_selection_revision: 4,
    };
    assert_eq!(validate_selection(&request).unwrap(), request);
    assert!(
        !invoked.exists(),
        "installation discovery and selection must not launch Qoder"
    );
}

#[test]
fn script_adapter_requires_readability_but_not_execute_permission() {
    let root = tempfile::tempdir().unwrap();
    let root_path = fs::canonicalize(root.path()).unwrap();
    let adapter = root_path.join("adapter.js");
    let cli = root_path.join("codex");
    let node = root_path.join("node");
    fs::write(&adapter, b"export {};").unwrap();
    fs::write(&cli, b"cli").unwrap();
    fs::write(&node, b"node").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&adapter, fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&node, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let request = WorkerDependenciesSelectRequestV1 {
        harness: WorkerHarnessV1::CodexCli,
        adapter_path: Some(adapter.to_string_lossy().into_owned()),
        cli_path: cli.to_string_lossy().into_owned(),
        node_path: Some(node.to_string_lossy().into_owned()),
        expected_selection_revision: 0,
    };
    assert_eq!(validate_selection(&request).unwrap(), request);
    let native = WorkerDependenciesSelectRequestV1 {
        node_path: None,
        ..request
    };
    assert_eq!(
        validate_selection(&native),
        Err(DelegationErrorV1::DependenciesInvalid)
    );
}

#[test]
fn package_bin_rejects_escape_and_accepts_the_registered_entry() {
    let root = tempfile::tempdir().unwrap();
    let package = root.path().join("package.json");
    fs::write(&package, br#"{"bin":{"codex-acp":"dist/index.js"}}"#).unwrap();
    assert_eq!(
        package_bin(&package, "codex-acp").unwrap(),
        Some(PathBuf::from("dist/index.js"))
    );
    fs::write(&package, br#"{"bin":{"codex-acp":"../escape.js"}}"#).unwrap();
    assert!(package_bin(&package, "codex-acp").is_err());
}

#[test]
fn path_and_npx_candidates_are_found_without_persisting_a_selection() {
    let root = tempfile::tempdir().unwrap();
    let root_path = fs::canonicalize(root.path()).unwrap();
    let first_bin = root_path.join("first/bin");
    let second_bin = root_path.join("second/bin");
    let cli = first_bin.join("codex");
    let adapter = first_bin.join("codex-acp");
    let node = first_bin.join("node");
    make_file(&cli, 0o700);
    make_file(&adapter, 0o600);
    make_file(&node, 0o700);
    fs::create_dir_all(&second_bin).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&cli, second_bin.join("codex")).unwrap();

    let npx_package = root_path.join("cache/_npx/one/node_modules/@agentclientprotocol/codex-acp");
    make_file(&npx_package.join("dist/index.js"), 0o600);
    fs::write(
        npx_package.join("package.json"),
        br#"{"bin":{"codex-acp":"dist/index.js"}}"#,
    )
    .unwrap();
    let environment = ScanEnvironment {
        path: Some(std::env::join_paths([&first_bin, &second_bin]).unwrap()),
        home: Some(root_path.join("home")),
        npm_prefix: None,
        npm_cache: Some(root_path.join("cache")),
    };
    let view = discover_with_environment(
        &StaticSelection(None),
        &WorkerDependenciesDiscoverRequestV1 {
            harness: Some(WorkerHarnessV1::CodexCli),
        },
        &environment,
    )
    .unwrap();
    assert!(view.selected.is_empty());
    assert_eq!(view.selection_revisions[0].revision, 0);
    assert_eq!(
        view.candidates
            .iter()
            .filter(|candidate| {
                candidate.component == WorkerDependencyComponentV1::Cli
                    && candidate.path == cli.to_string_lossy().as_ref()
            })
            .count(),
        1
    );
    assert!(view.candidates.iter().any(|candidate| {
        candidate.component == WorkerDependencyComponentV1::Adapter
            && candidate.source == WorkerDependencyCandidateSourceV1::NpxCache
            && candidate.path.ends_with("/dist/index.js")
            && candidate.state == WorkerDependencyCandidateStateV1::Found
    }));
}

#[test]
fn missing_saved_selection_is_retained_and_never_replaced_by_discovery() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    make_file(&bin.join("codex"), 0o700);
    make_file(&bin.join("codex-acp"), 0o700);
    make_file(&bin.join("node"), 0o700);
    let missing = root.path().join("removed");
    let source = StaticSelection(Some(WorkerInstallationSelection {
        config: WorkerInstallationConfig::codex_acp(
            missing.join("adapter.js"),
            missing.join("codex"),
            missing.join("node"),
        ),
        revision: 4,
    }));
    let view = discover_with_environment(
        &source,
        &WorkerDependenciesDiscoverRequestV1 {
            harness: Some(WorkerHarnessV1::CodexCli),
        },
        &ScanEnvironment {
            path: Some(std::env::join_paths([&bin]).unwrap()),
            home: Some(root.path().join("home")),
            npm_prefix: None,
            npm_cache: None,
        },
    )
    .unwrap();
    assert_eq!(view.selection_revisions[0].revision, 4);
    assert_eq!(
        view.selected[0].cli_path,
        missing.join("codex").to_string_lossy().as_ref()
    );
    assert!(view.candidates.iter().any(|candidate| {
        candidate.source == WorkerDependencyCandidateSourceV1::Selected
            && candidate.state == WorkerDependencyCandidateStateV1::Missing
    }));
    assert!(view.candidates.iter().any(|candidate| {
        candidate.source == WorkerDependencyCandidateSourceV1::Path
            && candidate.state == WorkerDependencyCandidateStateV1::Found
    }));
}

#[test]
fn shared_scan_budget_marks_the_view_incomplete() {
    let root = tempfile::tempdir().unwrap();
    let npx = root.path().join("cache/_npx");
    for index in 0..=MAX_DIRECTORY_ENTRIES {
        fs::create_dir_all(npx.join(format!("entry-{index:03}"))).unwrap();
    }
    let view = discover_with_environment(
        &StaticSelection(None),
        &WorkerDependenciesDiscoverRequestV1 {
            harness: Some(WorkerHarnessV1::CodexCli),
        },
        &ScanEnvironment {
            path: None,
            home: Some(root.path().join("home")),
            npm_prefix: None,
            npm_cache: Some(root.path().join("cache")),
        },
    )
    .unwrap();
    assert!(
        view.install_hints
            .iter()
            .any(|hint| hint.reason_code == "worker.dependencies.scan_incomplete")
    );
}

#[test]
#[cfg(unix)]
fn metadata_symlink_loop_is_unavailable_not_missing() {
    let root = tempfile::tempdir().unwrap();
    let entry = root.path().join("codex");
    std::os::unix::fs::symlink(&entry, &entry).unwrap();
    let fact = inspect(&entry, true);
    assert_eq!(fact.state, WorkerDependencyCandidateStateV1::Unavailable);
}

#[test]
fn default_discovery_returns_every_supported_harness_without_a_selection() {
    let view = discover_with_environment(
        &StaticSelection(None),
        &WorkerDependenciesDiscoverRequestV1 { harness: None },
        &ScanEnvironment::default(),
    )
    .unwrap();
    assert_eq!(
        view.selection_revisions
            .iter()
            .map(|item| item.harness)
            .collect::<Vec<_>>(),
        WORKER_HARNESSES
    );
    assert!(
        view.selection_revisions
            .iter()
            .all(|item| item.revision == 0)
    );
    assert!(view.selected.is_empty());
    assert!(view.valid());
    let round_trip: WorkerDependenciesViewV1 =
        serde_json::from_value(serde_json::to_value(&view).unwrap()).unwrap();
    assert_eq!(view, round_trip);
    let mut duplicate = view;
    duplicate.selection_revisions[4] = duplicate.selection_revisions[0].clone();
    assert!(!duplicate.valid());
}
