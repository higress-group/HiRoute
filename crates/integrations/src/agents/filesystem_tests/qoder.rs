use super::*;

#[test]
fn qoder_discovery_does_not_launch_or_read_model_and_auth_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let mut layout = layout(directory.path());
    layout.qoder_executable = directory.path().join("bin/qoder");
    write_executable(&layout.qoder_executable, "should never execute");
    let launched = directory.path().join("unexpected-launch");
    fs::write(
        &layout.qoder_executable,
        format!(
            "#!/bin/sh\nprintf launched > '{}'\nexit 91\n",
            launched.display()
        ),
    )
    .unwrap();
    fs::create_dir_all(&layout.qoder_config_root).unwrap();
    // Deliberately invalid settings and an auth-shaped file cannot prevent collaboration
    // discovery or become a model/source import. Neither needs parsing for this capability.
    fs::write(
        layout.qoder_config_root.join("settings.json"),
        b"invalid model/provider secret-sentinel",
    )
    .unwrap();
    fs::write(
        layout.qoder_config_root.join("auth.json"),
        b"auth-secret-sentinel",
    )
    .unwrap();
    let expected_target = layout
        .qoder_config_root
        .join("skills/hiroute-collaboration/SKILL.md");
    let scanner = FilesystemAgentScannerV1::new(layout, registry());
    let found = scanner.qoder_settings_discovery(true);
    assert!(
        !launched.exists(),
        "discovery must not run a behavior probe"
    );
    let AgentDiscoveryOutcomeV1::Supported { installation } = &found.outcome else {
        panic!("Qoder path should be discovered: {:?}", found.outcome)
    };
    assert_eq!(installation.profile.kind, AgentKindV1::Qoder);
    assert_eq!(
        installation.profile.ingress_protocol,
        Some(hiroute_domain::AgentIngressProtocolV1::Responses)
    );
    assert!(installation.effective_config.is_empty());
    // A registered model adapter and a located CLI are not proof of configured access or
    // model verification. Collaboration proof is independently absent until its actual probe.
    for action in [
        hiroute_domain::AgentAction::ConfigureModel,
        hiroute_domain::AgentAction::VerifyModel,
    ] {
        assert!(installation.require_action(action).is_err());
    }
    assert!(
        installation
            .require_action(hiroute_domain::AgentAction::InstallCollaborationSkill)
            .is_err()
    );
    assert!(found.discovered_credential.is_none());
    assert_eq!(scanner.qoder_user_skill_target(), expected_target);
    assert_eq!(
        scanner.available_model_surfaces("agent_qoder_default"),
        [hiroute_domain::AgentModelSurfaceV2::QoderCli]
            .into_iter()
            .collect()
    );
    assert!(
        scanner
            .codex_engine_target(hiroute_domain::AgentModelSurfaceV2::QoderCli)
            .is_none()
    );
    assert!(!launched.exists());
    assert!(
        !serde_json::to_string(&found)
            .unwrap()
            .contains("secret-sentinel")
    );
}

#[test]
fn missing_selected_qoder_never_switches_to_a_different_installed_cli() {
    let directory = tempfile::tempdir().unwrap();
    let mut layout = layout(directory.path());
    layout.qoder_executable = directory.path().join("missing-selected-qoder");
    let alternate = layout.qoder_home.join(".local/bin");
    fs::create_dir_all(&alternate).unwrap();
    write_executable(&alternate.join("qodercli"), "not selected");
    let scanner = FilesystemAgentScannerV1::new(layout, registry());
    assert!(scanner.qoder_executable_target().is_none());
    assert!(
        scanner
            .available_model_surfaces("agent_qoder_default")
            .is_empty()
    );
    assert!(matches!(
        scanner.qoder_settings_discovery(false).outcome,
        AgentDiscoveryOutcomeV1::ReportOnly {
            reason: AgentReportOnlyReasonV1::NotFoundInScope,
            ..
        }
    ));
}

#[cfg(windows)]
#[test]
fn windows_qoder_scan_distinguishes_absence_from_unusable_path() {
    const CASE: &str = "agents::filesystem::tests::qoder::windows_qoder_scan_distinguishes_absence_from_unusable_path";
    const SEARCH_ROOT: &str = "HIROUTE_TEST_QODER_SEARCH_ROOT";
    let Some(root) = std::env::var_os(SEARCH_ROOT).map(PathBuf::from) else {
        let directory = tempfile::tempdir().unwrap();
        let search = directory.path().join("search");
        fs::create_dir(&search).unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CASE, "--nocapture"])
            .env(SEARCH_ROOT, directory.path())
            .env("PATH", search)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&child.stdout);
        assert!(
            child.status.success() && stdout.contains("1 passed;"),
            "{stdout}\n{}",
            String::from_utf8_lossy(&child.stderr)
        );
        return;
    };
    let mut layout = layout(&root);
    layout.qoder_executable = PathBuf::from("qoder");
    let scanner = FilesystemAgentScannerV1::new(layout, registry());
    let reason = || {
        let outcome = scanner.scan_agent("agent_qoder_default").unwrap().outcome;
        let AgentDiscoveryOutcomeV1::ReportOnly { reason, .. } = outcome else {
            panic!("unexpected Qoder installation");
        };
        reason
    };
    assert_eq!(reason(), AgentReportOnlyReasonV1::NotFoundInScope);

    fs::create_dir(root.join("search/qoder.exe")).unwrap();
    assert_eq!(
        reason(),
        AgentReportOnlyReasonV1::ExecutableProbeUnavailable
    );
}

#[test]
#[cfg(unix)]
fn failed_explicit_reverification_invalidates_previous_installed_skill_proof() {
    use crate::agents::{QoderCollaborationEvidence, QoderCollaborationProbeTarget};
    let directory = tempfile::tempdir().unwrap();
    let mut layout = layout(directory.path());
    layout.qoder_executable = directory.path().join("bin/qoder");
    write_executable(&layout.qoder_executable, "unused");
    fs::write(&layout.qoder_executable, "#!/bin/sh\nexit 1\n").unwrap();
    let cli = directory.path().join("bin/hiroute");
    write_executable(
        &cli,
        r#"{"status":"succeeded","data":{"commands":[{"command_id":"schema.list"}]}}"#,
    );
    let target = layout
        .qoder_config_root
        .join("skills/hiroute-collaboration/SKILL.md");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    let content =
        b"---\nname: hiroute-collaboration\ndescription: fixture\n---\nconfirmed fixture\n";
    fs::write(&target, content).unwrap();
    let scanner = FilesystemAgentScannerV1::new(layout, registry());
    let target = QoderCollaborationProbeTarget::InstalledUserSkill {
        expected_content: CanonicalDigest::of_bytes(content),
    };
    let evidence = QoderCollaborationEvidence::fixture(
        &scanner.qoder_executable_target().unwrap(),
        &scanner.qoder_native_context().unwrap(),
        &cli,
        target.clone(),
    );
    scanner.qoder_collaboration.lock().unwrap().evidence = Some(evidence);
    assert!(scanner.qoder_installed_collaboration_verified());
    assert!(scanner.check_qoder_collaboration(&cli, target).is_err());
    assert!(!scanner.qoder_installed_collaboration_verified());
}
