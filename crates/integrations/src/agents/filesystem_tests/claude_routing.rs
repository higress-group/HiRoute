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
        AgentDiscoveryOutcomeV1::Supported { installation } if installation.agent_id == "agent_claude_default"
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

#[test]
fn claude_context_policy_detects_overrides_and_restores_owned_values() {
    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    let original = json!({"env": {"CLAUDE_CODE_AUTO_COMPACT_WINDOW":"500000", "CLAUDE_CODE_MAX_CONTEXT_TOKENS":"900000", "UNRELATED":"keep"}});
    write_secret_settings(&layout.claude_user_settings, original.clone());
    let scanner = FilesystemAgentScannerV1::new(layout.clone(), registry());
    assert!(!scanner.claude_context_override().unwrap());
    let before = scanner.claude_user_config_document().unwrap();
    let desired = hiroute_domain::claude_context_environment(272_000)
        .unwrap()
        .into_iter()
        .map(|(key, value)| (format!("env.{key}"), Some(json!(value))))
        .collect();
    let change = hiroute_domain::AgentConfigChangeV1::preview(&before, desired).unwrap();
    let rendered = scanner.render_claude_user_config_change(&change).unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&rendered).unwrap();
    assert_eq!(parsed["env"]["CLAUDE_CODE_AUTO_COMPACT_WINDOW"], "272000");
    assert_eq!(parsed["env"]["UNRELATED"], "keep");
    let restored = crate::agents::filesystem_config::restore_claude_change_bytes(
        &rendered,
        &serde_json::to_vec(&original).unwrap(),
        &change,
    )
    .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&restored).unwrap(),
        original
    );
    for key in hiroute_domain::CLAUDE_CONTEXT_CONFLICT_ENVIRONMENT {
        let scanner =
            FilesystemAgentScannerV1::new(layout.clone().with_process_value(key, "1"), registry());
        assert!(scanner.claude_context_override().unwrap(), "{key}");
    }
    write_secret_settings(
        &directory.path().join("project/.claude/settings.json"),
        json!({"env":{"DISABLE_COMPACT":"1"}}),
    );
    assert!(scanner.claude_context_override().unwrap());
}

#[test]
fn claude_context_policy_release_rebases_to_original_preferences() {
    use crate::agents::filesystem_config::{
        rebase_claude_change_bytes, render_claude_change_bytes,
    };
    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    let original = json!({"env":{"CLAUDE_CODE_AUTO_COMPACT_WINDOW":"500000"}});
    write_secret_settings(&layout.claude_user_settings, original.clone());
    let scanner = FilesystemAgentScannerV1::new(layout.clone(), registry());
    let before = scanner.claude_user_config_document().unwrap();
    let first = hiroute_domain::AgentConfigChangeV1::preview(
        &before,
        hiroute_domain::claude_context_environment(272_000)
            .unwrap()
            .into_iter()
            .map(|(key, value)| (format!("env.{key}"), Some(json!(value))))
            .collect(),
    )
    .unwrap();
    let original_bytes = serde_json::to_vec(&original).unwrap();
    let current = render_claude_change_bytes(&original_bytes, &first).unwrap();
    fs::write(&layout.claude_user_settings, current.as_slice()).unwrap();
    let release = hiroute_domain::AgentConfigChangeV1::preview(
        &scanner.claude_user_config_document().unwrap(),
        hiroute_domain::CLAUDE_CONTEXT_ENVIRONMENT
            .into_iter()
            .map(|key| (format!("env.{key}"), None))
            .collect(),
    )
    .unwrap();
    let (rendered, rebased) =
        rebase_claude_change_bytes(&current, &original_bytes, &first, &release).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&rendered).unwrap(),
        original
    );
    rebased.validate().unwrap();
}

#[test]
fn claude_version_diagnostics_do_not_change_admission_or_dependency_identity() {
    let root = tempfile::tempdir().unwrap();
    let layout = layout(root.path());
    let diagnostic = root.path().join("diagnostic");
    // Change output without changing the executable inode, content or permissions.
    fs::write(
        &layout.claude_executable,
        format!("#!/bin/sh\ncat '{}'\n", diagnostic.display()),
    )
    .unwrap();
    let scanner = FilesystemAgentScannerV1::new(layout.clone(), registry());
    let mut digest = None;
    let mut legacy = scanner.clone();
    legacy.legacy_recovery_version = Some("2.1.231".into());
    legacy.legacy_process_format = true;
    fs::write(&diagnostic, "2.1.231").unwrap();
    let legacy_digest = legacy
        .scan()
        .into_iter()
        .find_map(|found| match found.outcome {
            AgentDiscoveryOutcomeV1::Supported { installation }
                if installation.profile.kind == AgentKindV1::ClaudeCode =>
            {
                Some(installation.observation_digest)
            }
            _ => None,
        })
        .unwrap();
    for label in [
        Some("0.0.1"),
        Some("99.1.2"),
        Some(""),
        Some("not a version"),
        None,
    ] {
        if let Some(label) = label {
            fs::write(&diagnostic, label).unwrap();
        } else {
            fs::remove_file(&diagnostic).unwrap();
        }
        let found = scanner
            .scan()
            .into_iter()
            .find(|item| outcome_agent_id(&item.outcome) == "agent_claude_default")
            .unwrap();
        let AgentDiscoveryOutcomeV1::Supported { installation } = found.outcome else {
            panic!("diagnostic output cannot deny admission");
        };
        assert!(found.managed_launch.unwrap().is_launchable());
        assert!(scanner.matches_legacy_claude_observation("2.1.231", &legacy_digest));
        if let Some(expected) = &digest {
            assert_eq!(&installation.observation_digest, expected);
        } else {
            digest = Some(installation.observation_digest);
        }
    }
    fs::write(&layout.claude_executable, "#!/bin/sh\nexit 0\n").unwrap();
    let found = scanner
        .scan()
        .into_iter()
        .find(|item| outcome_agent_id(&item.outcome) == "agent_claude_default")
        .unwrap();
    let AgentDiscoveryOutcomeV1::Supported { installation } = found.outcome else {
        panic!("executable remains installed");
    };
    assert_ne!(Some(installation.observation_digest), digest);
    assert!(!scanner.matches_legacy_claude_observation("2.1.231", &legacy_digest));
}

#[test]
fn legacy_process_digest_matches_the_pre_window_contract_fixture() {
    use crate::agents::filesystem_config::{ClaudeSource, recover_pre_context_process_digest};
    let mut observation = ClaudeSource::Process
        .read(&BTreeMap::new(), &BTreeSet::new())
        .unwrap();
    recover_pre_context_process_digest(&mut observation);
    // Frozen hash of the original five-field representation from 9e1ddbb:
    // four empty strings followed by the helper-presence boolean.
    assert_eq!(
        observation.digest.as_str(),
        "sha256:2e119288169c0e9f0d13cc0bf91861599ef9958a275c54dd87fdabe72e5d5d9d"
    );
    let original = observation.digest.clone();
    observation
        .settings
        .env
        .context_environment
        .insert("DISABLE_COMPACT".into(), "1".into());
    observation.digest = CanonicalDigest::of_bytes(b"current format with context override");
    let current = observation.digest.clone();
    recover_pre_context_process_digest(&mut observation);
    assert_eq!(observation.digest, current);
    assert_ne!(observation.digest, original);
}
