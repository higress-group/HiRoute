use super::*;

#[test]
fn claude_settings_discovery_does_not_gate_save_on_version_probe() {
    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    fs::write(&layout.claude_executable, b"#!/bin/sh\nexit 9\n").unwrap();
    write_secret_settings(&layout.claude_user_settings, json!({}));
    let scanner = FilesystemAgentScannerV1::new(layout, registry());
    let ordinary = scanner.scan();
    assert!(ordinary.iter().any(|item| matches!(
        &item.outcome,
        AgentDiscoveryOutcomeV1::ReportOnly {
            agent_id,
            reason: AgentReportOnlyReasonV1::ExecutableProbeUnavailable,
            ..
        } if agent_id == "agent_claude_default"
    )));
    let settings = scanner.claude_settings_discovery(false);
    assert!(matches!(
        settings.outcome,
        AgentDiscoveryOutcomeV1::Supported { .. }
    ));
    assert!(
        settings
            .managed_launch
            .as_ref()
            .is_some_and(|launch| launch.is_launchable())
    );
}

#[test]
fn claude_settings_discovery_allows_unregistered_native_endpoint() {
    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    write_secret_settings(
        &layout.claude_user_settings,
        json!({"env": {
            "ANTHROPIC_BASE_URL": "https://unknown.example.invalid/anthropic",
            "ANTHROPIC_MODEL": "custom-model"
        }}),
    );
    let scanner = FilesystemAgentScannerV1::new(layout, registry());
    let ordinary = scanner.scan();
    assert!(ordinary.iter().any(|item| matches!(
        &item.outcome,
        AgentDiscoveryOutcomeV1::ReportOnly {
            agent_id,
            reason: AgentReportOnlyReasonV1::UnregisteredEndpoint,
            ..
        } if agent_id == "agent_claude_default"
    )));
    let settings = scanner.claude_settings_discovery(false);
    assert!(matches!(
        settings.outcome,
        AgentDiscoveryOutcomeV1::Supported { .. }
    ));
    assert_eq!(
        settings.configuration_issue,
        Some(AgentReportOnlyReasonV1::UnregisteredEndpoint)
    );
}

#[test]
fn ordinary_claude_requires_user_file_to_own_routing_and_auth() {
    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    write_secret_settings(
        &layout.claude_user_settings,
        json!({
            "apiKeyHelper": "untrusted-original --never-run",
            "env": {"ANTHROPIC_AUTH_TOKEN":"private-token", "ANTHROPIC_MODEL":"opus"}
        }),
    );
    let user_only = FilesystemAgentScannerV1::new(layout.clone(), registry());
    assert!(
        !user_only.claude_native_routing_conflict().unwrap(),
        "the owned user file can replace its own old auth without executing it"
    );

    let process = FilesystemAgentScannerV1::new(
        layout
            .clone()
            .with_process_value("ANTHROPIC_AUTH_TOKEN", "process-secret"),
        registry(),
    );
    assert!(
        process.claude_native_routing_conflict().unwrap(),
        "a user-file edit cannot remove an inherited process token"
    );

    let project = directory.path().join("project/.claude/settings.json");
    write_secret_settings(&project, json!({"env":{"ANTHROPIC_MODEL":"project-model"}}));
    let project_override = FilesystemAgentScannerV1::new(layout, registry());
    assert!(
        project_override.claude_native_routing_conflict().unwrap(),
        "a project override must not be reported as a working ordinary connection"
    );
}

#[test]
fn claude_explicit_selection_keeps_top_level_model_distinct_from_presets() {
    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    let settings = &layout.claude_user_settings;
    write_secret_settings(
        settings,
        json!({"model":"opus", "env":{"ANTHROPIC_MODEL":"sonnet",
            "ANTHROPIC_DEFAULT_OPUS_MODEL":"provider-opus"}}),
    );
    let scanner = FilesystemAgentScannerV1::new(layout.clone(), registry());
    assert_eq!(
        scanner
            .claude_explicit_model_selection()
            .unwrap()
            .as_deref(),
        Some("sonnet")
    );
    assert!(
        !scanner
            .claude_user_config_document()
            .unwrap()
            .fields
            .contains_key("model"),
        "the native top-level selection is observed but never an owned write field"
    );

    write_secret_settings(
        settings,
        json!({"model":"opus", "env":{
        "ANTHROPIC_DEFAULT_OPUS_MODEL":"provider-opus"}}),
    );
    assert_eq!(
        scanner
            .claude_explicit_model_selection()
            .unwrap()
            .as_deref(),
        Some("opus")
    );

    write_secret_settings(
        settings,
        json!({"env":{
        "ANTHROPIC_DEFAULT_OPUS_MODEL":"provider-opus"}}),
    );
    assert_eq!(scanner.claude_explicit_model_selection().unwrap(), None);

    write_secret_settings(
        &layout.claude_project_settings[0],
        json!({"model":"project-opus"}),
    );
    assert!(scanner.claude_native_routing_conflict().unwrap());
}

#[test]
fn claude_managed_helper_detects_auth_added_after_a_token_free_save() {
    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    write_secret_settings(
        &layout.claude_user_settings,
        json!({"env": {"UNRELATED": "keep-me"}}),
    );
    let scanner = FilesystemAgentScannerV1::new(layout.clone(), registry());
    let before = scanner.claude_user_config_document().unwrap();
    let change = hiroute_domain::AgentConfigChangeV1::preview(
        &before,
        BTreeMap::from([
            (
                "apiKeyHelper".to_owned(),
                Some(json!({
                    "executable": "/opt/hiroute/bin/hiroute",
                    "argv": [hiroute_domain::HIDDEN_AGENT_GRANT_HELPER_VERB_V1,
                        "agent-connection/agent_claude_default/claude-messages-v1"]
                })),
            ),
            ("hiroute.auth_environment".to_owned(), None),
            (
                "env.ANTHROPIC_BASE_URL".to_owned(),
                Some(json!("http://127.0.0.1:35837")),
            ),
        ]),
    )
    .unwrap();
    assert!(
        change
            .fields
            .iter()
            .all(|field| field.path != "hiroute.auth_environment")
    );
    let rendered = scanner.render_claude_user_config_change(&change).unwrap();
    write_secret_settings(
        &layout.claude_user_settings,
        serde_json::from_slice(&rendered).unwrap(),
    );
    assert!(
        scanner
            .claude_user_config_change_is_applied(&change)
            .unwrap()
    );

    for name in [
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_CUSTOM_HEADERS",
    ] {
        let mut modified: serde_json::Value =
            serde_json::from_slice(&fs::read(&layout.claude_user_settings).unwrap()).unwrap();
        modified["env"][name] = json!("unexpected-credential");
        write_secret_settings(&layout.claude_user_settings, modified);
        assert!(
            !scanner
                .claude_user_config_change_is_applied(&change)
                .unwrap(),
            "{name} must invalidate a managed helper"
        );
        write_secret_settings(
            &layout.claude_user_settings,
            serde_json::from_slice(&rendered).unwrap(),
        );
    }
}
