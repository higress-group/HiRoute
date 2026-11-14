use super::*;
use hiroute_platform::desktop_engine::DesktopEngineLocation;

#[test]
fn codex_desktop_location_refresh_observes_install_update_and_uninstall_without_rebinding_home() {
    let root = tempfile::tempdir().unwrap();
    let mut layout = layout(root.path());
    layout.codex_executable = root.path().join("codex-not-installed.exe");
    let scanner = FilesystemAgentScannerV1::new(layout.clone(), registry())
        .with_codex_desktop_instance(crate::agents::CodexDesktopInstancePaths::at_root(
            root.path().join("instance"),
        ))
        .with_dynamic_codex_desktop_location();
    let clone = scanner.clone();
    let original = scanner.codex_user_config_target();
    let private = scanner.codex_desktop_config_target();
    let first = root.path().join("first-engine");
    let second = root.path().join("updated-engine");
    write_executable(&first, "1.0.0");
    write_executable(&second, "1.0.1");
    let surface = hiroute_domain::AgentModelSurfaceV2::CodexDesktop;
    assert!(scanner.codex_engine_target(surface).is_none());
    let assert_reason = |reason| {
        for id in ["agent_codex_default", crate::agents::CODEX_DESKTOP_AGENT_ID] {
            assert!(
                matches!(
                    clone.scan_agent(id).unwrap().outcome,
                    AgentDiscoveryOutcomeV1::ReportOnly { reason: actual, .. } if actual == reason
                ),
                "{id} must report {reason:?}"
            );
        }
    };
    assert_reason(AgentReportOnlyReasonV1::NotFoundInScope);
    for engine in [&first, &second] {
        scanner.refresh_codex_desktop_location(|| {
            Ok(Some(DesktopEngineLocation::File(engine.clone())))
        });
        assert_eq!(clone.codex_engine_target(surface).as_ref(), Some(engine));
        assert!(
            clone
                .available_model_surfaces(crate::agents::CODEX_DESKTOP_AGENT_ID)
                .contains(&surface)
        );
        assert_eq!(
            clone
                .codex_desktop_scanner()
                .unwrap()
                .layout
                .codex_executable,
            *engine
        );
        assert_eq!(clone.codex_user_config_target(), original);
        assert_eq!(clone.codex_desktop_config_target(), private);
    }
    scanner.refresh_codex_desktop_location(|| Err(std::io::ErrorKind::TimedOut.into()));
    assert_reason(AgentReportOnlyReasonV1::ExecutableProbeUnavailable);
    scanner.refresh_codex_desktop_location(|| Ok(Some(DesktopEngineLocation::File(second))));
    assert!(matches!(
        clone
            .scan_agent(crate::agents::CODEX_DESKTOP_AGENT_ID)
            .unwrap()
            .outcome,
        AgentDiscoveryOutcomeV1::Supported { .. }
    ));
    scanner.refresh_codex_desktop_location(|| Ok(None));
    assert_reason(AgentReportOnlyReasonV1::NotFoundInScope);
    assert!(clone.codex_engine_target(surface).is_none());
    assert!(
        clone
            .available_model_surfaces(crate::agents::CODEX_DESKTOP_AGENT_ID)
            .is_empty()
    );
    // Supplied fixture/selected targets remain static unless their owner enables discovery.
    let pinned = FilesystemAgentScannerV1::new(layout, registry());
    pinned.refresh_codex_desktop_location(|| panic!("Static target must not query installations"));
}
