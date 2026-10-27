//! Public Desktop carrier and independent profile admission, with no installed npm CLI.
use super::*;
use std::os::unix::fs::PermissionsExt;

fn desktop_layout(root: &Path) -> AgentFilesystemLayoutV1 {
    let mut selected = layout(root);
    let contents = root.join("DeepSeek Harness.app/Contents");
    let executable = contents.join("MacOS/DeepSeek Harness");
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::create_dir_all(contents.join("Resources")).unwrap();
    fs::write(contents.join("Resources/app.asar"), b"controlled runtime").unwrap();
    fs::write(&executable, br#"#!/bin/sh
[ "$ELECTRON_RUN_AS_NODE" = 1 ] || exit 90
[ "$HOME" = "$DSH_HOME" ] && [ "$PWD" = "$HOME" ] || exit 91
[ "$1" = --expose-internals ] || exit 92
case "$2" in */app.asar/dsh/node_modules/@deepseek-ai/dsh-desktop-host/lib/cli.js) ;; *) exit 93;; esac
shift 2
if [ "$1" = --version ]; then echo 0.2.1-rc.3; exit; fi
[ "$1" = --profile ] && [ "$2" = web ] && [ "$3" = --dump-default-config ] || exit 94
printf '%s\n' '- id: llm-pi-ai'
"#).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    selected.dsh_desktop_executable = Some(executable);
    selected
}

#[test]
fn desktop_only_installation_discovers_and_proves_the_shipped_composition() {
    let directory = tempfile::tempdir().unwrap();
    let selected = desktop_layout(directory.path());
    let scanner = FilesystemAgentScannerV1::new(selected, registry());
    assert!(scanner.dsh_executable_target().is_none());
    let result = scanner.scan_agent("agent_dsh_desktop").unwrap();
    let AgentDiscoveryOutcomeV1::Supported { installation } = result.outcome else {
        panic!("Desktop discovery");
    };
    assert_eq!(installation.agent_id, "agent_dsh_desktop");
    assert_eq!(installation.version, "0.2.1-rc.3");
    assert!(
        scanner
            .available_model_surfaces("agent_dsh_desktop")
            .contains(&hiroute_domain::AgentModelSurfaceV2::DshDesktop)
    );
    assert_ne!(
        scanner.dsh_desktop_models_target(),
        scanner.dsh_user_models_target()
    );
    let proof = scanner.check_dsh_desktop_model_configuration().unwrap();
    assert!(
        scanner
            .dsh_desktop_model_configuration_is_current(&proof)
            .unwrap()
    );
    assert!(
        !scanner.dsh_desktop_models_target().exists(),
        "probe must not initialize the real profile"
    );
}

#[test]
fn desktop_proof_tracks_only_its_actual_dependencies_and_rejects_custom_composition() {
    let directory = tempfile::tempdir().unwrap();
    let selected = desktop_layout(directory.path());
    let root = selected.dsh_config_root.clone();
    let executable = selected.dsh_desktop_executable.clone().unwrap();
    let scanner = FilesystemAgentScannerV1::new(selected, registry());
    let manifest = root.join("profiles/desktop/package.json");
    fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    fs::write(&manifest, br#"{"name":"desktop","private":true,"dependencies":{},"dsh":{"profile":{"bundles":["@deepseek-ai/dsh-base","@deepseek-ai/dsh-web-app"]}}}"#).unwrap();
    let proof = scanner.check_dsh_desktop_model_configuration().unwrap();
    fs::create_dir_all(scanner.dsh_user_models_target().parent().unwrap()).unwrap();
    fs::write(
        scanner.dsh_user_models_target(),
        b"!!js Web-only dynamic configuration",
    )
    .unwrap();
    assert!(
        scanner
            .dsh_desktop_model_configuration_is_current(&proof)
            .unwrap(),
        "unrelated Web profile is not a Desktop dependency"
    );
    fs::write(
        scanner.dsh_desktop_models_target(),
        b"# new Desktop content\n[]\n",
    )
    .unwrap();
    assert!(
        !scanner
            .dsh_desktop_model_configuration_is_current(&proof)
            .unwrap()
    );
    fs::write(
        &manifest,
        br#"{"dependencies":{},"dsh":{"profile":{"bundles":["custom-bundle"]}}}"#,
    )
    .unwrap();
    assert!(scanner.check_dsh_desktop_model_configuration().is_err());
    fs::write(&manifest, b"{}").unwrap();
    assert!(scanner.check_dsh_desktop_model_configuration().is_err());
    fs::remove_file(&executable).unwrap();
    assert!(
        matches!(
            scanner.dsh_desktop_settings_discovery().outcome,
            AgentDiscoveryOutcomeV1::Supported { .. }
        ),
        "owned file recovery must not require a running installation"
    );
    assert!(
        scanner.check_dsh_desktop_model_configuration().is_err(),
        "recovery facts cannot authorize Configure"
    );
}

#[test]
fn desktop_probe_failure_does_not_fall_back_to_web_cli_or_mutate_native_files() {
    let directory = tempfile::tempdir().unwrap();
    let selected = desktop_layout(directory.path());
    fs::write(
        selected.dsh_desktop_executable.as_ref().unwrap(),
        b"#!/bin/sh\nexit 23\n",
    )
    .unwrap();
    let scanner = FilesystemAgentScannerV1::new(selected, registry());
    assert!(scanner.check_dsh_desktop_model_configuration().is_err());
    assert!(!scanner.dsh_desktop_models_target().exists());
    assert!(!scanner.dsh_user_models_target().exists());
}
