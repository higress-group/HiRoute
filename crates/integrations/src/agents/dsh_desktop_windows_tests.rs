use super::*;

#[test]
fn windows_dsh_desktop_resolves_standard_user_and_machine_installations_without_path() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("用户");
    let local = directory.path().join("local");
    let programs = directory.path().join("programs");
    let roots = windows_installation_roots(&home, |key| match key {
        "LOCALAPPDATA" => Some(local.clone().into_os_string()),
        "ProgramW6432" | "ProgramFiles" => Some(programs.clone().into_os_string()),
        _ => None,
    });
    assert_eq!(
        roots,
        vec![
            local.join("Programs/DeepSeek Harness"),
            programs.join("DeepSeek Harness")
        ]
    );
    assert_eq!(
        windows_installation_roots(&home, |_| None),
        vec![home.join("AppData/Local/Programs/DeepSeek Harness")]
    );
    let mut layout = AgentFilesystemLayoutV1::from_process(&home, directory.path());
    layout.dsh_desktop_installation_roots = roots.clone();
    layout.dsh_config_root = directory.path().join(".dsh");
    let scanner = FilesystemAgentScannerV1::new(layout, super::super::tests::registry());
    let initial = scanner.dsh_desktop_executable_target();
    // Install the machine copy first, then the preferred per-user copy, using
    // the same scanner throughout. The test machine may have a registry fallback.
    for root in roots.iter().rev() {
        std::fs::create_dir_all(root).unwrap();
        let executable = root.join("DeepSeek Harness.exe");
        std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        let discovered = scanner.dsh_desktop_executable_target();
        assert_ne!(initial, discovered);
        assert_eq!(
            std::fs::canonicalize(discovered.unwrap()).unwrap(),
            std::fs::canonicalize(executable).unwrap()
        );
        assert!(matches!(
            scanner.scan_agent("agent_dsh_desktop").unwrap().outcome,
            AgentDiscoveryOutcomeV1::Supported { .. }
        ));
    }
    assert!(windows_installed_executable(Path::new("relative")).is_none());
}

#[test]
fn windows_dsh_desktop_uses_sibling_resources_and_rejects_archive_aliases() {
    let directory = tempfile::tempdir().unwrap();
    let install = directory.path().join("DeepSeek Harness");
    std::fs::create_dir_all(install.join("resources")).unwrap();
    let executable = install.join("DeepSeek Harness.exe");
    std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    let archive = install.join("resources/app.asar");
    std::fs::write(&archive, b"runtime identity").unwrap();
    let mut layout = AgentFilesystemLayoutV1::from_process(directory.path(), directory.path());
    layout.dsh_desktop_executable = Some(executable);
    layout.dsh_config_root = directory.path().join(".dsh");
    let registry = super::super::tests::registry();
    let scanner = FilesystemAgentScannerV1::new(layout, registry);
    let carrier = scanner.dsh_desktop_executable_target().unwrap();
    let admitted_archive = FilesystemAgentScannerV1::dsh_desktop_archive(&carrier).unwrap();
    let command = FilesystemAgentScannerV1::dsh_desktop_command(&carrier, &admitted_archive);
    let args: Vec<_> = command.get_args().collect();
    assert_eq!(args[0], "--expose-internals");
    // GetTempPath can use an 8.3 account name. Production admission returns the
    // actual ordinary drive path rather than retaining that alternate spelling.
    let expected_archive =
        crate::agents::managed_launch::windows_executable_path(&archive).unwrap();
    assert_eq!(
        Path::new(args[1]),
        expected_archive.join("dsh/node_modules/@deepseek-ai/dsh-desktop-host/lib/cli.js")
    );
    let proof = scanner.dsh_desktop_contract(false).unwrap();
    std::fs::write(&archive, b"a changed bundled runtime identity").unwrap();
    assert_ne!(proof, scanner.dsh_desktop_contract(false).unwrap());
    std::fs::remove_file(&archive).unwrap();
    assert!(scanner.dsh_desktop_contract(false).is_err());
    std::fs::remove_dir(install.join("resources")).unwrap();
    let foreign = directory.path().join("foreign-resources");
    std::fs::create_dir(&foreign).unwrap();
    std::fs::write(foreign.join("app.asar"), b"foreign runtime").unwrap();
    hiroute_platform_test_support::junction::create(&install.join("resources"), &foreign).unwrap();
    assert!(
        [false, true]
            .into_iter()
            .all(|skills| scanner.dsh_desktop_contract(skills).is_err()),
        "archive aliases cannot establish composition authority"
    );
    assert!(
        matches!(
            scanner.dsh_desktop_settings_discovery().outcome,
            AgentDiscoveryOutcomeV1::Supported { .. }
        ),
        "Restore facts survive a removed runtime"
    );
}

#[test]
#[ignore = "requires the official DSH Desktop installed in the existing Windows account"]
fn windows_installed_dsh_desktop_discovers_and_checks_the_bundled_cli_without_changing_profiles() {
    const CHILD: &str = "HIROUTE_DSH_DESKTOP_ISOLATION_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let (_, module) = module_path!().split_once("::").unwrap();
        let case = format!(
            "{module}::windows_installed_dsh_desktop_discovers_and_checks_the_bundled_cli_without_changing_profiles"
        );
        // Set the ASAR override only in the test child.
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &case, "--ignored"])
            .env(CHILD, "1")
            .env("ELECTRON_NO_ASAR", "1")
            .output()
            .unwrap();
        assert!(
            child.status.success()
                && String::from_utf8_lossy(&child.stdout).contains("1 passed; 0 failed"),
            "the installed bundled CLI must work with ASAR disabled in the scanner's parent environment"
        );
        return;
    }
    let home = hiroute_platform::windows::background::profile().unwrap();
    let layout = AgentFilesystemLayoutV1::from_process(&home, &home);
    let root = layout.dsh_config_root.clone();
    let scanner = FilesystemAgentScannerV1::new(layout, super::super::tests::registry());
    assert!(
        scanner.dsh_desktop_executable_target().is_some(),
        "official Desktop must be independently discovered"
    );
    let profiles = [
        "cordis.patch.yml",
        "profiles/web/cordis.patch.yml",
        "profiles/desktop/cordis.patch.yml",
        "profiles/desktop/package.json",
    ];
    let read = || {
        profiles.map(|relative| match std::fs::read(root.join(relative)) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("existing native profile is unreadable: {error}"),
        })
    };
    let before = read();
    let result = scanner.scan_dsh_desktop();
    let AgentDiscoveryOutcomeV1::Supported { installation } = result.outcome else {
        panic!("the installed Windows Desktop must be discovered");
    };
    assert!(
        !installation.version.is_empty(),
        "the actual bundled CLI must report its version"
    );
    let proof = scanner.check_dsh_desktop_model_configuration().unwrap();
    assert!(
        scanner
            .dsh_desktop_model_configuration_is_current(&proof.0)
            .unwrap()
    );
    assert!(
        before == read(),
        "version/composition probes must not initialize or edit native profiles"
    );
}
