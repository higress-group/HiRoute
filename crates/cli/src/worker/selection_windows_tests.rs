//! Real native input identities and the existing canonical transaction/replay boundary.
use super::*;
use hiroute_application_api::{WorkerHarnessV1, plan_worker_dependency_selection};
use hiroute_platform_test_support::junction;

struct Fixture {
    root: PathBuf,
    _cleanup: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let cleanup = tempfile::tempdir().unwrap();
        let root = cleanup.path().join("private 中文 % ! ' &");
        security::create_owner_only_directory(&root).unwrap();
        Self {
            root,
            _cleanup: cleanup,
        }
    }

    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.root.join(name);
        hiroute_platform::durable::write_private_new(&path, bytes).unwrap();
        path
    }
}

fn request(path: &Path) -> WorkerDependenciesSelectRequestV1 {
    WorkerDependenciesSelectRequestV1 {
        harness: WorkerHarnessV1::DeepseekHarness,
        adapter_path: None,
        cli_path: path.to_str().unwrap().into(),
        node_path: None,
        expected_selection_revision: 7,
    }
}

#[test]
fn ordinary_inputs_bind_the_same_native_files_and_canonical_digest_without_mutation() {
    let fixture = Fixture::new();
    let cli = fixture.file("client 中文 % ! ' &.exe", b"manual executable is never run");
    let adapter = fixture.file("adapter.mjs", b"export {}; // never run");
    let node = fixture.file("node.exe", b"manual node is never run");
    let source = request(&cli);
    let source = WorkerDependenciesSelectRequestV1 {
        harness: WorkerHarnessV1::CodexCli,
        adapter_path: Some(adapter.to_str().unwrap().into()),
        node_path: Some(node.to_str().unwrap().into()),
        ..source
    };
    let original = security::open_identity(&cli, false).unwrap();
    let permissions = hiroute_platform::file_security::capture(&original).unwrap();
    let prepared = prepare(source.clone()).unwrap();
    let expected = WorkerDependenciesSelectRequestV1 {
        cli_path: std::fs::canonicalize(&cli)
            .unwrap()
            .to_str()
            .unwrap()
            .into(),
        adapter_path: Some(
            std::fs::canonicalize(&adapter)
                .unwrap()
                .to_str()
                .unwrap()
                .into(),
        ),
        node_path: Some(
            std::fs::canonicalize(&node)
                .unwrap()
                .to_str()
                .unwrap()
                .into(),
        ),
        ..source.clone()
    };
    assert_eq!(prepared.request, expected);
    let canonical_plan = plan_worker_dependency_selection(&expected).unwrap();
    let ordinary_plan = plan_worker_dependency_selection(&source).unwrap();
    // Before this CLI projection the old valid request had a different digest
    // and failed Application's exact normalized-tuple admission.
    assert_ne!(ordinary_plan.accept_digest, canonical_plan.accept_digest);
    assert_eq!(
        plan_worker_dependency_selection(&prepared.request)
            .unwrap()
            .accept_digest,
        canonical_plan.accept_digest
    );
    assert_eq!(
        plan_worker_dependency_selection(&prepared.request)
            .unwrap()
            .idempotency_key,
        canonical_plan.idempotency_key
    );
    for (path, projected) in [
        (&cli, prepared.request.cli_path.as_str()),
        (&adapter, prepared.request.adapter_path.as_deref().unwrap()),
        (&node, prepared.request.node_path.as_deref().unwrap()),
    ] {
        let before = security::open_identity(path, false).unwrap();
        let after = security::open_identity(Path::new(projected), false).unwrap();
        assert_eq!(
            security::file_identity(&before).unwrap(),
            security::file_identity(&after).unwrap()
        );
    }
    assert!(std::fs::rename(&cli, fixture.root.join("replaced.exe")).is_err());
    assert!(std::fs::rename(&fixture.root, fixture.root.with_file_name("renamed")).is_err());
    assert!(OpenOptions::new().write(true).open(&cli).is_err());
    assert_eq!(
        std::fs::read(&cli).unwrap(),
        b"manual executable is never run"
    );
    assert_eq!(
        hiroute_platform::file_security::capture(&original).unwrap(),
        permissions
    );
    drop(original);
    drop(prepared);
    std::fs::rename(&cli, fixture.root.join("replaced.exe")).unwrap();
}

#[test]
fn canonical_completed_selection_replays_before_missing_file_inspection() {
    canonical_selection_replays(WorkerHarnessV1::CodexCli);
    canonical_selection_replays(WorkerHarnessV1::OpenCode);
}

fn canonical_selection_replays(harness: WorkerHarnessV1) {
    use hiroute_application::ApplicationService;
    use hiroute_daemon::control::{ProductionControlRuntime, start_control};
    let fixture = Fixture::new();
    let cli = fixture.file("manual-client.exe", b"metadata-only fixture");
    let adapter = fixture.file("manual-adapter.js", b"export {};");
    let node = fixture.file("manual-node.exe", b"metadata-only fixture");
    let ordinary = WorkerDependenciesSelectRequestV1 {
        harness,
        adapter_path: (harness == WorkerHarnessV1::CodexCli)
            .then(|| adapter.to_str().unwrap().into()),
        node_path: (harness == WorkerHarnessV1::CodexCli).then(|| node.to_str().unwrap().into()),
        expected_selection_revision: 0,
        ..request(&cli)
    };
    let prepared = prepare(ordinary.clone()).unwrap();
    let canonical = prepared.request.clone();
    let runtime = ProductionControlRuntime::open(fixture.root.join("storage")).unwrap();
    let runtime_root = fixture.root.join("runtime");
    let mut control = start_control(
        ApplicationService::new(runtime.application_ports()),
        &runtime_root,
    )
    .unwrap();
    let client = hiroute_client_core::Client::new(
        "hiroute-cli",
        hiroute_client_core::LocalEndpoint::from_runtime_root(&runtime_root),
    );
    let sent = |request: &WorkerDependenciesSelectRequestV1| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(client.select_worker_dependencies("windows-input-replay", request))
            .unwrap()
    };
    let rejected = sent(&ordinary);
    assert_eq!(
        rejected.error.as_ref().unwrap().code,
        ErrorCode::InvalidArguments
    );
    assert!(
        rejected.operation.is_none(),
        "the old ordinary tuple must not start an Operation"
    );
    let applied = sent(&prepared.request);
    assert!(
        applied.error.is_none(),
        "canonical selection must be admitted"
    );
    assert_eq!(applied.operation.as_ref().unwrap().state, "succeeded");
    assert_eq!(
        applied.data.as_ref().unwrap().selection_revisions[0].revision,
        1
    );
    let changed_revision = WorkerDependenciesSelectRequestV1 {
        expected_selection_revision: 9,
        ..prepared.request.clone()
    };
    let conflict = sent(&changed_revision);
    assert_eq!(
        conflict.error.as_ref().unwrap().code,
        ErrorCode::RevisionConflict
    );
    assert!(
        conflict.operation.is_none(),
        "projection must not refresh the caller's CAS revision"
    );
    drop(prepared);
    for path in [&cli, &adapter, &node] {
        std::fs::remove_file(path).unwrap();
    }
    let replay_input = prepare(canonical).unwrap();
    assert!(
        replay_input._held.is_empty(),
        "canonical replay must not reopen disappeared files"
    );
    let replay = sent(&replay_input.request);
    assert!(
        replay.error.is_none(),
        "completed Operation replay must precede metadata"
    );
    assert_eq!(replay.operation, applied.operation);
    assert_eq!(
        replay.data.as_ref().unwrap().selection_revisions[0].revision,
        1
    );
    assert!(
        prepare(ordinary).is_err(),
        "a missing ordinary path cannot be projected by guessing"
    );
    let next = WorkerDependenciesSelectRequestV1 {
        expected_selection_revision: 1,
        ..replay_input.request.clone()
    };
    let missing = sent(&next);
    assert_eq!(
        missing.error.as_ref().unwrap().message_key,
        "worker.dependencies.missing"
    );
    assert!(missing.operation.is_none());
    control.shutdown();
    control.join(std::time::Duration::from_secs(10)).unwrap();
}

#[test]
fn selection_input_rejects_device_unc_streams_and_real_junctions_without_adoption() {
    let fixture = Fixture::new();
    let cli = fixture.file("client.exe", b"untouched dependency");
    let held = security::open_identity(&cli, false).unwrap();
    let permissions = hiroute_platform::file_security::capture(&held).unwrap();
    for spelling in [
        r"\\server\share\client.exe",
        r"\\?\UNC\server\share\client.exe",
        r"\\.\C:\client.exe",
        r"\\?\GLOBALROOT\Device\HarddiskVolume1\client.exe",
        r"C:client.exe",
        r"C:\client.exe:stream",
        r"C:\a\..\client.exe",
        "C:\\client\\bad\"name.exe",
    ] {
        assert!(prepare(request(Path::new(spelling))).is_err());
    }
    let alias = fixture.root.join("junction");
    junction::create(&alias, &fixture.root).unwrap();
    assert!(prepare(request(&alias.join("client.exe"))).is_err());
    std::fs::remove_dir(alias).unwrap();
    assert_eq!(std::fs::read(&cli).unwrap(), b"untouched dependency");
    assert_eq!(
        hiroute_platform::file_security::capture(&held).unwrap(),
        permissions
    );
}

#[test]
fn ordinary_npm_shim_uses_the_shared_declared_bin_parser_and_rejects_reparse_bins() {
    let fixture = Fixture::new();
    let shim = fixture.file("dsh.cmd", b"@echo do-not-evaluate-this-shim");
    for name in [
        "node_modules",
        "node_modules/@deepseek-ai",
        "node_modules/@deepseek-ai/dsh",
        "node_modules/@deepseek-ai/dsh/lib",
    ] {
        security::create_owner_only_directory(&fixture.root.join(name)).unwrap();
    }
    let manifest = fixture.file(
        "node_modules/@deepseek-ai/dsh/package.json",
        br#"{"name":"@deepseek-ai/dsh","bin":{"dsh":"./lib/bin.js"}}"#,
    );
    let entry = fixture.file(
        "node_modules/@deepseek-ai/dsh/lib/bin.js",
        b"throw new Error('never execute');",
    );
    let prepared = prepare(request(&shim)).unwrap();
    assert_eq!(
        prepared.request.cli_path,
        std::fs::canonicalize(&entry).unwrap().to_str().unwrap()
    );
    assert_eq!(prepared.request.harness, WorkerHarnessV1::DeepseekHarness);
    assert!(OpenOptions::new().write(true).open(&manifest).is_err());
    assert!(std::fs::rename(&entry, entry.with_extension("old")).is_err());
    assert_eq!(
        std::fs::read(&shim).unwrap(),
        b"@echo do-not-evaluate-this-shim"
    );
    drop(prepared);
    std::fs::rename(
        entry.parent().unwrap(),
        entry.parent().unwrap().with_file_name("real-lib"),
    )
    .unwrap();
    let package = manifest.parent().unwrap();
    junction::create(&package.join("lib"), &package.join("real-lib")).unwrap();
    assert!(
        prepare(request(&shim)).is_err(),
        "raw declared bin traversal must reject the actual junction"
    );
    std::fs::remove_dir(package.join("lib")).unwrap();
}
