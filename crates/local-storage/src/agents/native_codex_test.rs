//! Explicit native-binary consumption evidence, never part of ordinary fixture runs.
//! The loopback endpoint has no upstream; its challenge grants no HiRoute authority.
use super::*;
use hiroute_integrations::CodexNativeIngressProbe;
use std::os::unix::fs::MetadataExt;

#[test]
#[ignore = "requires explicitly selected trusted native Codex binary and native platform admission"]
fn native_codex_consumes_managed_authentication_and_restores() {
    let binary = fs::canonicalize(
        std::env::var_os("HIROUTE_NATIVE_CODEX").expect("explicit native binary required"),
    )
    .unwrap();
    let identity = fs::metadata(&binary).unwrap();
    assert!(identity.is_file() && identity.mode() & 0o022 == 0);
    assert!(identity.uid() == 0 || identity.uid() == rustix::process::geteuid().as_raw());
    for ancestor in binary.ancestors().skip(1) {
        let meta = fs::metadata(ancestor).unwrap();
        assert!(meta.is_dir() && (meta.mode() & 0o022 == 0 || meta.mode() & 0o1000 != 0));
        assert!(meta.uid() == 0 || meta.uid() == rustix::process::geteuid().as_raw());
    }
    // Deliberately retained on any panic: no TempDir drop may hide an unverified restore.
    let root = tempfile::Builder::new()
        .prefix("hiroute-native-codex-")
        .tempdir()
        .unwrap()
        .keep();
    let home = root.join("home");
    let config_root = home.join(".codex");
    let workspace = root.join("workspace");
    for directory in [&home, &config_root, &workspace] {
        fs::create_dir(directory).unwrap();
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    // No organization configuration is copied or overridden to manufacture isolation.
    assert!(!Path::new("/etc/codex/config.toml").exists());
    assert!(!Path::new("/etc/codex/managed_config.toml").exists());
    let path = config_root.join("config.toml");
    let probe = CodexNativeIngressProbe::bind().unwrap();
    let install = intent(
        AgentConnectionTransactionKindV1::Apply,
        AgentConnectionEffectRoleV1::ManagedConfiguration,
        None,
    );
    let artifacts = store(&root, install.target(), &path);
    let operation = OperationId::parse("op_88888888888888888888888888888888").unwrap();
    let staged =
        stage_codex_configuration(&artifacts, &operation, &install, probe.configuration()).unwrap();
    artifacts.activate_artifact(&staged).unwrap();
    let result = probe.run(&binary, &home, &workspace, &artifacts, &operation, &install);

    // Restore through the same protected store and native field adapter even on probe failure.
    drop(artifacts);
    let artifacts = store(&root, install.target(), &path);
    let restore = intent(
        AgentConnectionTransactionKindV1::Restore,
        AgentConnectionEffectRoleV1::ManagedConfiguration,
        artifacts
            .current_external_fingerprint(install.target())
            .unwrap(),
    );
    let restore_operation = OperationId::parse("op_99999999999999999999999999999999").unwrap();
    let staged = stage_codex_restoration(
        &artifacts,
        &restore_operation,
        &restore,
        &operation,
        &install,
        None,
    )
    .unwrap();
    artifacts.activate_artifact(&staged).unwrap();
    assert!(
        !path.exists(),
        "native config absence must be restored before cleanup"
    );
    let after = fs::metadata(&binary).unwrap();
    assert_eq!(
        (
            identity.dev(),
            identity.ino(),
            identity.len(),
            identity.mtime(),
            identity.ctime()
        ),
        (
            after.dev(),
            after.ino(),
            after.len(),
            after.mtime(),
            after.ctime()
        )
    );
    assert!(
        result.is_ok(),
        "native probe failed: {:?}; restored materials retained at {}",
        result.as_ref().err(),
        root.display()
    );
    let evidence = result
        .unwrap()
        .after_restoration(&artifacts, &restore_operation, &restore)
        .unwrap();
    let mut layout = hiroute_integrations::AgentFilesystemLayoutV1::from_process(&home, &workspace);
    layout.codex_executable = binary;
    layout.claude_executable = root.join("no-claude");
    let scanner = hiroute_integrations::FilesystemAgentScannerV1::new(
        layout,
        hiroute_integrations::ClaudeRegistrationIndexV1::default(),
    );
    for (scanner, proven) in [
        (scanner.clone(), false),
        (scanner.with_codex_ingress_evidence(evidence), true),
    ] {
        let found = scanner
            .scan()
            .into_iter()
            .find_map(|discovery| match discovery.outcome {
                hiroute_integrations::AgentDiscoveryOutcomeV1::Supported { installation }
                    if installation.profile.kind == hiroute_domain::AgentKindV1::Codex =>
                {
                    Some(installation)
                }
                _ => None,
            })
            .expect("native Codex installation");
        assert_eq!(
            found
                .require_action(hiroute_domain::AgentAction::ConfigureModel)
                .is_ok(),
            proven
        );
    }
    fs::remove_dir_all(root).unwrap();
}
