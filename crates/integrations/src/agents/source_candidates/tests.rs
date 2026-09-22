use super::*;

#[test]
fn codex_empty_or_missing_config_keeps_the_builtin_native_source() {
    use crate::agents::{CodexSelectionTarget, DiscoveredAuthSource};
    for empty_file in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let layout = layout(directory.path());
        fs::create_dir_all(layout.codex_user_config.parent().unwrap()).unwrap();
        if empty_file {
            fs::write(&layout.codex_user_config, "").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&layout.codex_user_config, fs::Permissions::from_mode(0o600))
                    .unwrap();
            }
        }
        let scanner = FilesystemAgentScannerV1::new(layout, registry());
        let candidates = scanner
            .codex_source_candidates(&CodexSelectionTarget::Root)
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].authentication,
            DiscoveredAuthSource::NativeSessionNeedsConfirmation
        );
        assert_eq!(
            candidates[0].endpoint_origin.as_deref(),
            Some("https://api.openai.com")
        );
    }
}

#[test]
#[cfg(unix)]
fn codex_source_rechecks_all_selected_layers_and_native_cli_provider_override() {
    use crate::agents::CodexConfigurationScope;
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    fs::create_dir_all(layout.codex_user_config.parent().unwrap()).unwrap();
    fs::write(&layout.codex_user_config,
        "model_provider = 'custom'\nmodel = 'base'\n[model_providers.custom]\nbase_url = 'https://base.example/v1'\nwire_api = 'responses'\nexperimental_bearer_token = 'base-secret'\n").unwrap();
    fs::set_permissions(&layout.codex_user_config, fs::Permissions::from_mode(0o600)).unwrap();
    let profile = directory.path().join("work.config.toml");
    fs::write(&profile, "model = 'work-model'\n").unwrap();
    fs::set_permissions(&profile, fs::Permissions::from_mode(0o600)).unwrap();
    let mut scope = CodexConfigurationScope::user_file(layout.codex_user_config.clone());
    scope.selected_profile_file = Some(profile.clone());
    scope
        .cli_overrides
        .push("model_providers.custom.base_url='https://launch.example/private'".into());
    let scanner = FilesystemAgentScannerV1::new(layout, registry());
    let candidate = scanner
        .codex_source_candidates_in_scope(&scope)
        .unwrap()
        .remove(0);
    assert_eq!(
        candidate.endpoint_origin.as_deref(),
        Some("https://launch.example")
    );
    let selected = scanner
        .read_selected_codex_source_in_scope(&scope, &candidate)
        .unwrap();
    assert_eq!(
        selected.model.as_deref().map(String::as_str),
        Some("work-model")
    );
    assert_eq!(selected.credential.unwrap().expose(), b"base-secret");
    fs::write(&profile, "model = 'changed-after-selection'\n").unwrap();
    assert!(
        scanner
            .read_selected_codex_source_in_scope(&scope, &candidate)
            .is_err()
    );
}

#[test]
fn source_candidate_keeps_custom_endpoint_without_secret_or_path_in_public_facts() {
    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    write_secret_settings(
        &layout.claude_user_settings,
        json!({"env": {
            "ANTHROPIC_BASE_URL": "https://custom.example/private-path?api_key=query-sentinel",
            "ANTHROPIC_MODEL": "custom-model", "ANTHROPIC_AUTH_TOKEN": "credential-sentinel"
        }}),
    );
    let scanner = FilesystemAgentScannerV1::new(layout.clone(), registry());
    let candidates = scanner.claude_source_candidates().unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0].endpoint_origin.as_deref(),
        Some("https://custom.example")
    );
    let public = serde_json::to_string(&candidates).unwrap();
    for hidden in [
        "private-path",
        "query-sentinel",
        "credential-sentinel",
        directory.path().to_str().unwrap(),
    ] {
        assert!(!public.contains(hidden));
    }
    let selected = scanner.read_selected_claude_source(&candidates[0]).unwrap();
    assert_eq!(
        selected.credential.unwrap().expose(),
        b"credential-sentinel"
    );
    assert!(
        selected
            .endpoint
            .contains("private-path?api_key=query-sentinel")
    );
    write_secret_settings(
        &layout.claude_user_settings,
        json!({"env": {
            "ANTHROPIC_BASE_URL": "https://changed.example", "ANTHROPIC_AUTH_TOKEN": "new-secret"
        }}),
    );
    assert!(matches!(
        scanner.read_selected_claude_source(&candidates[0]),
        Err(AgentFilesystemScanError::SourceChanged)
    ));
}

#[test]
fn source_candidate_rejects_caller_supplied_identity_and_never_executes_helper() {
    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    let sentinel = directory.path().join("helper-must-not-run");
    write_secret_settings(
        &layout.claude_user_settings,
        json!({
            "apiKeyHelper": format!("touch {}", sentinel.display()),
            "env": {"ANTHROPIC_BASE_URL": "https://custom.example"}
        }),
    );
    let scanner = FilesystemAgentScannerV1::new(layout, registry());
    let mut candidates = scanner.claude_source_candidates().unwrap();
    assert_eq!(
        candidates[0].authentication,
        crate::agents::DiscoveredAuthSource::HelperNeedsInput
    );
    assert!(
        scanner
            .read_selected_claude_source(&candidates[0])
            .unwrap()
            .credential
            .is_none()
    );
    assert!(!sentinel.exists());
    candidates[0].candidate_ref = "source/another-context".into();
    assert!(matches!(
        scanner.read_selected_claude_source(&candidates[0]),
        Err(AgentFilesystemScanError::SourceChanged)
    ));
}

#[test]
fn source_candidate_api_key_rereads_only_selected_field_and_rejects_mixed_auth_layers() {
    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    write_secret_settings(
        &layout.claude_user_settings,
        json!({"env":{
            "ANTHROPIC_BASE_URL":"https://custom.example", "ANTHROPIC_API_KEY":"api-key-sentinel"
        }}),
    );
    let scanner = FilesystemAgentScannerV1::new(layout.clone(), registry());
    let selected = scanner.claude_source_candidates().unwrap().remove(0);
    assert_eq!(
        selected.authentication,
        crate::agents::DiscoveredAuthSource::EnvironmentKey
    );
    assert!(
        !serde_json::to_string(&selected)
            .unwrap()
            .contains("api-key-sentinel")
    );
    assert_eq!(
        scanner
            .read_selected_claude_source(&selected)
            .unwrap()
            .credential
            .unwrap()
            .expose(),
        b"api-key-sentinel"
    );
    write_secret_settings(
        &layout.claude_project_settings[0],
        json!({"env":{"ANTHROPIC_AUTH_TOKEN":"another-token"}}),
    );
    let ambiguous = scanner.claude_source_candidates().unwrap().remove(0);
    assert_eq!(
        ambiguous.authentication,
        crate::agents::DiscoveredAuthSource::Ambiguous
    );
    assert!(
        scanner
            .read_selected_claude_source(&ambiguous)
            .unwrap()
            .credential
            .is_none()
    );
    assert!(scanner.read_selected_claude_source(&selected).is_err());
}

#[test]
fn source_candidate_codex_profile_is_explicit_and_session_material_is_never_copied() {
    use crate::agents::{CodexSelectionTarget, DiscoveredAuthSource};
    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    fs::create_dir_all(layout.codex_user_config.parent().unwrap()).unwrap();
    fs::write(
        &layout.codex_user_config,
        r#"
model_provider = "openai"
model = "native-default"
[profiles.work]
model_provider = "custom"
model = "custom-model"
[model_providers.custom]
base_url = "https://custom.example/private?token=query-sentinel"
wire_api = "responses"
experimental_bearer_token = "codex-secret-sentinel"
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&layout.codex_user_config, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let scanner = FilesystemAgentScannerV1::new(layout.clone(), registry());
    let native = scanner
        .codex_source_candidates(&CodexSelectionTarget::Root)
        .unwrap()
        .remove(0);
    assert_eq!(
        native.authentication,
        DiscoveredAuthSource::NativeSessionNeedsConfirmation
    );
    assert!(
        scanner
            .read_selected_codex_source(&CodexSelectionTarget::Root, &native)
            .unwrap()
            .credential
            .is_none()
    );
    let target = CodexSelectionTarget::LegacyProfile("work".into());
    let selected = scanner.codex_source_candidates(&target).unwrap().remove(0);
    assert_eq!(selected.authentication, DiscoveredAuthSource::InlineToken);
    assert_ne!(selected.context_ref, native.context_ref);
    assert_eq!(
        selected.endpoint_origin.as_deref(),
        Some("https://custom.example")
    );
    let public = serde_json::to_string(&selected).unwrap();
    for hidden in [
        "codex-secret-sentinel",
        "query-sentinel",
        "/private",
        directory.path().to_str().unwrap(),
    ] {
        assert!(!public.contains(hidden));
    }
    assert_eq!(
        scanner
            .read_selected_codex_source(&target, &selected)
            .unwrap()
            .credential
            .unwrap()
            .expose(),
        b"codex-secret-sentinel"
    );
    assert!(
        scanner
            .read_selected_codex_source(&CodexSelectionTarget::Root, &selected)
            .is_err()
    );
    fs::write(&layout.codex_user_config, "model_provider = 'openai'\n").unwrap();
    assert!(
        scanner
            .read_selected_codex_source(&target, &selected)
            .is_err()
    );
}

#[test]
fn codex_configuration_path_honors_explicit_native_home_without_changing_other_agent_roots() {
    let home = Path::new("/users/example");
    assert_eq!(
        codex_config_path(home, None),
        home.join(".codex/config.toml")
    );
    assert_eq!(
        codex_config_path(home, Some(std::ffi::OsStr::new("/isolated/codex"))),
        Path::new("/isolated/codex/config.toml")
    );
    assert_eq!(
        codex_config_path(home, Some(std::ffi::OsStr::new(""))),
        home.join(".codex/config.toml")
    );
}
