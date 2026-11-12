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
printf '%s\n' '- id: llm-pi-ai' '- id: skill-filesystem' '- id: tool-skill' '- id: skill'
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
            .dsh_desktop_model_configuration_is_current(&proof.0)
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
            .dsh_desktop_model_configuration_is_current(&proof.0)
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
            .dsh_desktop_model_configuration_is_current(&proof.0)
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

#[test]
fn desktop_plugin_profile_requires_the_bundled_independent_provider_api() {
    let directory = tempfile::tempdir().unwrap();
    let layout = desktop_layout(directory.path());
    let executable = layout.dsh_desktop_executable.clone().unwrap();
    let scanner = FilesystemAgentScannerV1::new(layout, registry());
    let manifest = scanner
        .dsh_desktop_models_target()
        .with_file_name("package.json");
    fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    fs::write(&manifest, br#"{"dependencies":{"fixture":"1.0.0"},"dsh":{"profile":{"bundles":["@deepseek-ai/dsh-base","@deepseek-ai/dsh-web-app","fixture"]}}}"#).unwrap();
    assert_eq!(
        scanner.check_dsh_desktop_model_configuration(),
        Err(DshDesktopModelCheckError::MissingAdapter)
    );
    fs::write(&executable, b"#!/bin/sh\n[ \"$2\" = --experimental-import-meta-resolve ] || exit 1\nprintf '%s\\n' 'HIRoute Desktop provider ready'\n").unwrap();
    let (proof, independent) = scanner.check_dsh_desktop_model_configuration().unwrap();
    assert!(independent);
    assert!(
        scanner
            .dsh_desktop_model_configuration_is_current(&proof)
            .unwrap()
    );
    assert!(!scanner.dsh_desktop_models_target().exists());
}

#[test]
fn desktop_management_requires_its_shared_skill_roots_and_never_proves_delegation() {
    use hiroute_domain::AgentAction;
    let directory = tempfile::tempdir().unwrap();
    let mut selected = desktop_layout(directory.path());
    selected.codex_desktop_executable = selected.dsh_desktop_executable.clone();
    let root = selected.dsh_config_root.clone();
    let executable = selected.dsh_desktop_executable.clone().unwrap();
    let scanner = FilesystemAgentScannerV1::new(selected, registry());
    let target = root.join("skills/hiroute-management/SKILL.md");
    for agent in ["agent_codex_default", "agent_dsh_default"] {
        let installation = scanner
            .desktop_management_installation(agent, &target, true)
            .unwrap();
        assert!(
            installation
                .require_action(AgentAction::InstallManagementSkill)
                .is_ok()
        );
        assert!(
            installation
                .require_action(AgentAction::InstallCollaborationSkill)
                .is_err()
        );
        assert!(!target.exists(), "discovery installed the Skill");
    }
    let patch = scanner.dsh_desktop_models_target();
    fs::create_dir_all(patch.parent().unwrap()).unwrap();
    for bytes in [
        b"- id: skill-filesystem\n  config: {includeDefaultRoots: false}\n".as_slice(),
        b"- id: skill-filesystem\n  config: {dshHome: /other/home}\n".as_slice(),
        b"- id: skill-filesystem\n  name: custom-module\n".as_slice(),
        b"- id: tool-skill\n  disabled: true\n".as_slice(),
        b"- insert: {id: skill-filesystem, name: custom-module}\n".as_slice(),
    ] {
        fs::write(&patch, bytes).unwrap();
        assert!(
            scanner
                .desktop_management_installation("agent_dsh_default", &target, true)
                .is_none()
        );
    }
    fs::remove_file(&patch).unwrap();
    let before = scanner
        .desktop_management_installation("agent_dsh_default", &target, true)
        .unwrap();
    fs::write(&patch, b"- id: llm-pi-ai\n  config: {providers: {}}\n").unwrap();
    let after = scanner
        .desktop_management_installation("agent_dsh_default", &target, true)
        .unwrap();
    assert_eq!(
        before.observation_digest, after.observation_digest,
        "model-only changes revoke a Skill proof"
    );
    fs::remove_file(executable).unwrap();
    for agent in ["agent_codex_default", "agent_dsh_default"] {
        assert!(
            scanner
                .desktop_management_installation(agent, &target, true)
                .is_none()
        );
        let restore = scanner
            .desktop_management_installation(agent, &target, false)
            .unwrap();
        assert!(
            restore
                .require_action(AgentAction::RemoveManagementSkill)
                .is_ok()
        );
        assert!(
            restore
                .require_action(AgentAction::InstallManagementSkill)
                .is_err()
        );
    }
}
