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
    for root in roots {
        std::fs::create_dir_all(&root).unwrap();
        let executable = root.join("DeepSeek Harness.exe");
        std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        assert_eq!(
            std::fs::canonicalize(windows_installed_executable(root).unwrap()).unwrap(),
            std::fs::canonicalize(executable).unwrap()
        );
    }
    assert!(windows_installed_executable(PathBuf::from("relative")).is_none());
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
    let command = scanner.dsh_desktop_command().unwrap();
    let args: Vec<_> = command.get_args().collect();
    assert_eq!(args[0], "--expose-internals");
    // GetTempPath can use an 8.3 account name. Production admission returns the
    // actual ordinary drive path rather than retaining that alternate spelling.
    let expected_archive = crate::agents::managed_launch::windows_executable_path(&archive).unwrap();
    assert_eq!(
        Path::new(args[1]),
        expected_archive.join("dsh/node_modules/@deepseek-ai/dsh-desktop-host/lib/cli.js")
    );
    let proof = scanner.dsh_desktop_contract().unwrap();
    std::fs::write(&archive, b"a changed bundled runtime identity").unwrap();
    assert_ne!(proof, scanner.dsh_desktop_contract().unwrap());
    std::fs::remove_file(&archive).unwrap();
    assert!(scanner.dsh_desktop_contract().is_err());
    std::fs::remove_dir(install.join("resources")).unwrap();
    let foreign = directory.path().join("foreign-resources");
    std::fs::create_dir(&foreign).unwrap();
    std::fs::write(foreign.join("app.asar"), b"foreign runtime").unwrap();
    hiroute_platform_test_support::junction::create(&install.join("resources"), &foreign).unwrap();
    assert!(
        scanner.dsh_desktop_contract().is_err(),
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
    let home = hiroute_platform::windows::background::profile().unwrap();
    let layout = AgentFilesystemLayoutV1::from_process(&home, &home);
    assert!(
        layout.dsh_desktop_executable.is_some(),
        "official Desktop must be independently discovered"
    );
    let root = layout.dsh_config_root.clone();
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
    let scanner = FilesystemAgentScannerV1::new(layout, super::super::tests::registry());
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
            .dsh_desktop_model_configuration_is_current(&proof)
            .unwrap()
    );
    assert!(
        before == read(),
        "version/composition probes must not initialize or edit native profiles"
    );
}
