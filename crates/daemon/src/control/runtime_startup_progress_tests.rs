use super::*;

#[test]
fn ordinary_startup_and_reopen_do_not_claim_a_history_upgrade() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::startup_progress_tests::ordinary_startup_and_reopen_do_not_claim_a_history_upgrade",
    ) {
        return;
    }
    let directory = crate::test_support::private_tempdir();
    for _ in 0..2 {
        let (sender, receiver) = std::sync::mpsc::channel();
        let runtime = ProductionControlRuntime::open_inner_with_cpa_and_overrides(
            directory.path(),
            crate::release_catalog::fixture_catalog(),
            None,
            None,
            true,
            RuntimeOpenOverrides {
                storage_startup: hiroute_local_storage::StorageStartupOptions {
                    upgrade_progress: Some(sender.into()),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            receiver.try_recv().is_err(),
            "ordinary startup must retain its normal readiness deadline"
        );
        drop(runtime);
    }
}

#[cfg(unix)]
#[test]
fn startup_primes_exact_secret_descriptors_without_informational_native_processes() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::startup_progress_tests::startup_primes_exact_secret_descriptors_without_informational_native_processes",
    ) {
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let directory = crate::test_support::private_tempdir();
    let root = directory.path();
    drop(hiroute_local_storage::LocalStorageSet::open_for_daemon_startup(root).unwrap());
    let home = root.join("native-home");
    std::fs::create_dir(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    let marker = root.join("version-probes");
    let executable = root.join("native-fixture");
    std::fs::write(
        &executable,
        format!(
            "#!/bin/sh\nprintf 'probe\\n' >> '{}'\nprintf '2.1.231\\n'\n",
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut layout = hiroute_integrations::AgentFilesystemLayoutV1::from_process(&home, root);
    layout.codex_executable = executable.clone();
    layout.codex_desktop_executable = None;
    layout.claude_executable = executable.clone();
    layout.qoder_executable = executable.clone();
    layout.pi_executable = executable.clone();
    layout.dsh_executable = executable.clone();
    layout.opencode_executable = executable;
    layout.claude_managed_settings.clear();
    layout.claude_project_settings.clear();
    layout.claude_launch_settings = None;
    let settings = layout.claude_user_settings.clone();
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::set_permissions(
        settings.parent().unwrap(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let original = br#"{"env":{"ANTHROPIC_BASE_URL":"https://open.bigmodel.cn/api/anthropic","ANTHROPIC_MODEL":"claude-opus-5","ANTHROPIC_DEFAULT_OPUS_MODEL":"glm-5.3[1m]","ANTHROPIC_AUTH_TOKEN":"test-only-startup-secret"}}"#;
    std::fs::write(&settings, original).unwrap();
    std::fs::set_permissions(&settings, std::fs::Permissions::from_mode(0o600)).unwrap();
    let catalog = crate::release_catalog::fixture_catalog();
    let scanner = hiroute_integrations::FilesystemAgentScannerV1::new(
        layout,
        hiroute_integrations::ClaudeRegistrationIndexV1::from_verified_model_data(
            catalog.registry(),
            &catalog.current_release_model_data().data,
        )
        .unwrap(),
    );
    let runtime =
        ProductionControlRuntime::prepare_for_role_all_with_scanner(root, catalog, None, scanner)
            .unwrap();
    assert!(
        !marker.exists(),
        "startup launched informational --version probes"
    );
    let descriptor = runtime
        .adapter
        .protected_inputs
        .lock()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    assert_eq!(
        runtime
            .adapter
            .scanner
            .read_discovered_secret(&descriptor)
            .unwrap()
            .expose(),
        b"test-only-startup-secret"
    );
    assert_eq!(std::fs::read(&settings).unwrap(), original);
    std::fs::write(&settings, b"{}").unwrap();
    assert!(
        runtime
            .adapter
            .scanner
            .read_discovered_secret(&descriptor)
            .is_err(),
        "changed native configuration retained stale secret authority"
    );
    runtime.adapter.refresh_discovery().unwrap();
    assert!(
        marker.exists(),
        "explicit Agent inventory must retain native version discovery"
    );
    assert!(runtime.adapter.protected_inputs.lock().unwrap().is_empty());
}
