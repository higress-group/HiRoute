use super::hermes_sources::*;
use hiroute_domain::{AgentNativeTargetV1, UpstreamProtocol};
use std::{fs, path::Path};

fn write(root: &Path, name: &str, content: &str) {
    let path = root.join(name);
    fs::write(&path, content).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}
fn target(root: &Path) -> AgentNativeTargetV1 {
    AgentNativeTargetV1::Hermes {
        config_root: root.canonicalize().unwrap().to_str().unwrap().into(),
    }
}
const CONFIG: &str = r#"model:
  provider: coding-plan
  default: declared-first
  context_length: 64000
providers:
  coding-plan:
    api: https://example.invalid/api/coding/v4
    url: https://ignored.invalid
    api_mode: chat_completions
    transport: codex_responses
    key_env: HERMES_SOURCE_TEST_KEY
    models:
      declared-first:
        context_length: 128000
      declared-second:
        context_length: 64000
"#;

#[test]
fn bare_custom_main_chat_is_discovered_with_native_inline_key_precedence() {
    let root = tempfile::tempdir().unwrap();
    let config = "model:\n  provider: custom\n  default: declared-first\n  base_url: https://example.invalid/v1\n  api_mode: chat_completions\n  api_key: inline-fixture-key\n  key_env: OTHER_KEY\n  context_length: 64000\n";
    write(root.path(), "config.yaml", config);
    write(root.path(), ".env", "OTHER_KEY=env-fixture-key\n");
    let selected_target = target(root.path());
    let discovery = hermes_api_sources(&selected_target).unwrap();
    assert!(discovery.diagnostics.is_empty());
    let point = crate::native_api_connection_points(discovery.sources).remove(0);
    assert_eq!(point.sources.len(), 1);
    let source = &point.sources[0];
    assert_eq!(source.protocol, Some(UpstreamProtocol::ChatCompletions));
    let binding = source.point_binding.as_ref().unwrap();
    assert_eq!(binding.endpoint_selector, "model/base_url");
    assert_eq!(
        binding.configured_default_model_id.as_deref(),
        Some("declared-first")
    );
    let (_, secret) = read_hermes_api_source(&selected_target, source).unwrap();
    assert_eq!(secret.expose(), b"inline-fixture-key");
    assert!(
        !serde_json::to_string(source)
            .unwrap()
            .contains("fixture-key")
    );
    assert_eq!(
        fs::read_to_string(root.path().join("config.yaml")).unwrap(),
        config
    );
    write(
        root.path(),
        "config.yaml",
        &config.replace("inline-fixture-key", "changed-fixture-key"),
    );
    assert!(read_hermes_api_source(&selected_target, source).is_err());
}

#[test]
fn bare_custom_main_keeps_unresolved_protocol_and_endpoint_overrides_unavailable() {
    let root = tempfile::tempdir().unwrap();
    let config = "model: {provider: custom, default: declared, base_url: 'https://example.invalid/v1', api_mode: chat_completions, key_env: MAIN_KEY}\n";
    write(root.path(), "config.yaml", config);
    write(root.path(), ".env", "MAIN_KEY=profile-fixture-key\n");
    let selected_target = target(root.path());
    let source = hermes_api_sources(&selected_target)
        .unwrap()
        .sources
        .remove(0);
    let (_, secret) = read_hermes_api_source(&selected_target, &source).unwrap();
    assert_eq!(secret.expose(), b"profile-fixture-key");
    write(
        root.path(),
        ".env",
        "MAIN_KEY=profile-fixture-key\nCUSTOM_BASE_URL=https://other.invalid\n",
    );
    let overridden = hermes_api_sources(&selected_target).unwrap();
    assert!(
        overridden
            .diagnostics
            .iter()
            .any(|d| d.reason == "unsupported_native_composition")
    );
    assert!(read_hermes_api_source(&selected_target, &overridden.sources[0]).is_err());
    write(root.path(), ".env", "MAIN_KEY=profile-fixture-key\n");
    write(
        root.path(),
        "config.yaml",
        &config.replace("chat_completions", "responses"),
    );
    let unresolved = hermes_api_sources(&selected_target).unwrap();
    assert!(
        unresolved
            .diagnostics
            .iter()
            .any(|d| d.reason == "native_protocol_unresolved")
    );
    assert!(read_hermes_api_source(&selected_target, &unresolved.sources[0]).is_err());
}

#[test]
fn known_channel_models_follow_native_yaml_precedence_without_exposing_channels_or_secrets() {
    let root = tempfile::tempdir().unwrap();
    let config = "model: {default: managed-alias}\nplatforms:\n  discord:\n    channel_overrides: {}\n  telegram:\n    enabled: true\n  slack:\n    channel_overrides:\n      private-channel-id: {model: native-selection, api_key: private-fixture-key}\n";
    write(root.path(), "config.yaml", config);
    write(
        root.path(),
        "gateway.json",
        r#"{"platforms":{"discord":{"channel_overrides":{"private-id":{"model":"suppressed-legacy"}}},"telegram":{"channel_overrides":{"other-private-id":{"model":"legacy-selection"}}}}}"#,
    );
    let models = super::hermes_known_channel_models(&target(root.path())).unwrap();
    assert_eq!(models, ["legacy-selection", "native-selection"]);
    assert_eq!(
        fs::read_to_string(root.path().join("config.yaml")).unwrap(),
        config
    );
}

#[test]
fn selected_profile_retains_native_default_mapping_and_precedence_without_writing() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "config.yaml", CONFIG);
    write(
        root.path(),
        ".env",
        "HERMES_SOURCE_TEST_KEY='profile-test-key' # inline comment\n",
    );
    let facts = hermes_api_sources(&target(root.path())).unwrap();
    assert_eq!(facts.sources.len(), 2);
    assert!(facts.diagnostics.is_empty());
    assert!(
        facts
            .sources
            .iter()
            .all(|s| s.protocol == Some(UpstreamProtocol::ChatCompletions))
    );
    let point = crate::native_api_connection_points(facts.sources).remove(0);
    assert_eq!(point.sources.len(), 2);
    let binding = point.sources[0].point_binding.as_ref().unwrap();
    assert_eq!(
        binding.configured_default_model_id.as_deref(),
        Some("declared-first")
    );
    assert_eq!(binding.endpoint, "https://example.invalid/api/coding/v4");
    let (_, secret) = read_hermes_api_source(&target(root.path()), &point.sources[0]).unwrap();
    assert_eq!(secret.expose(), b"profile-test-key");
    assert_eq!(point.sources[0].context_tokens, Some(128000));
    let public_legacy_leaf = serde_json::to_string(&point.sources[0]).unwrap();
    assert!(
        !public_legacy_leaf.contains("profile-test-key")
            && !public_legacy_leaf.contains("HERMES_SOURCE_TEST_KEY")
    );
    assert_eq!(
        fs::read_to_string(root.path().join("config.yaml")).unwrap(),
        CONFIG
    );
}

#[test]
fn roots_and_equal_material_from_different_native_selectors_do_not_share_authority() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    for root in [&first, &second] {
        write(root.path(), "config.yaml", CONFIG);
        write(
            root.path(),
            ".env",
            "HERMES_SOURCE_TEST_KEY=same-test-key\nOTHER_KEY=same-test-key\n",
        );
    }
    let selected = hermes_api_sources(&target(first.path()))
        .unwrap()
        .sources
        .remove(0);
    let before = crate::native_api_connection_points(vec![selected.clone()]).remove(0);
    let other = crate::native_api_connection_points(
        hermes_api_sources(&target(second.path())).unwrap().sources,
    )
    .remove(0);
    assert_ne!(before.key_digest, other.key_digest);
    write(
        first.path(),
        "config.yaml",
        &CONFIG.replace("key_env: HERMES_SOURCE_TEST_KEY", "key_env: OTHER_KEY"),
    );
    let changed = crate::native_api_connection_points(
        hermes_api_sources(&target(first.path())).unwrap().sources,
    )
    .remove(0);
    assert_ne!(before.key_digest, changed.key_digest);
    assert_ne!(before.snapshot_digest, changed.snapshot_digest);
    assert!(read_hermes_api_source(&target(first.path()), &selected).is_err());
}

#[test]
fn named_provider_transport_takes_precedence_over_stale_main_api_mode() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "config.yaml",
        &CONFIG.replace(
            "  provider: coding-plan",
            "  provider: coding-plan\n  api_mode: codex_responses",
        ),
    );
    write(root.path(), ".env", "HERMES_SOURCE_TEST_KEY=fixture\n");
    let facts = hermes_api_sources(&target(root.path())).unwrap();
    assert!(facts.diagnostics.is_empty());
    assert!(facts.sources.iter().all(|source| source.supported_auth
        && source.protocol == Some(UpstreamProtocol::ChatCompletions)));
    assert_eq!(crate::native_api_connection_points(facts.sources).len(), 1);
}

#[test]
fn unsupported_members_are_local_and_managed_or_disabled_routes_are_excluded() {
    let root = tempfile::tempdir().unwrap();
    let config = format!(
        "{CONFIG}      unsupported-model:\n        base_url: https://other.invalid\n  broken-provider: {{api: https://bad.invalid}}\n  hiroute-owned: {{api: http://127.0.0.1:1234, transport: codex_responses, api_key: placeholder, default_model: route}}\n  disabled-provider: {{enabled: 'false', api: https://disabled.invalid, transport: openai_chat, api_key: fixture, default_model: disabled}}\n"
    );
    write(root.path(), "config.yaml", &config);
    write(root.path(), ".env", "HERMES_SOURCE_TEST_KEY=fixture\n");
    let facts = hermes_api_sources(&target(root.path())).unwrap();
    assert_eq!(facts.sources.len(), 3);
    assert_eq!(facts.sources.iter().filter(|s| s.supported_auth).count(), 2);
    assert!(
        facts
            .diagnostics
            .iter()
            .any(|d| d.model_id.as_deref() == Some("unsupported-model"))
    );
    assert!(facts.sources.iter().all(|s| !s.provider_id.starts_with("hiroute-") && s.provider_id != "disabled-provider"));
    assert_eq!(crate::native_api_connection_points(facts.sources).len(), 2);
}

#[test]
fn absent_interpolated_environment_and_native_pool_remain_unavailable() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "config.yaml", CONFIG);
    for environment in ["", "HERMES_SOURCE_TEST_KEY=${PROCESS_ONLY_SECRET}\n"] {
        write(root.path(), ".env", environment);
        let facts = hermes_api_sources(&target(root.path())).unwrap();
        assert!(facts.sources.iter().all(|s| s.credential.is_none()));
        assert!(
            facts
                .diagnostics
                .iter()
                .all(|d| d.reason == "profile_environment_required")
        );
    }
    write(root.path(), ".env", "HERMES_SOURCE_TEST_KEY=fixture\n");
    write(
        root.path(),
        "auth.json",
        r#"{"credential_pool":{"custom:coding-plan":[{"id":"private-pool-entry"}]}}"#,
    );
    let facts = hermes_api_sources(&target(root.path())).unwrap();
    assert!(facts.sources.iter().all(|s| s.credential.is_none()));
    assert!(
        facts
            .diagnostics
            .iter()
            .all(|d| d.reason == "unsupported_native_composition")
    );
}

#[test]
fn unrelated_builtin_pools_do_not_replace_a_selected_custom_provider_key() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "config.yaml", CONFIG);
    write(root.path(), ".env", "HERMES_SOURCE_TEST_KEY=fixture\n");
    write(
        root.path(),
        "auth.json",
        r#"{"credential_pool":{"zai":[{"source":"env:GLM_API_KEY"}],"copilot":[{"source":"gh_cli"}]}}"#,
    );
    let facts = hermes_api_sources(&target(root.path())).unwrap();
    assert_eq!(facts.sources.len(), 2);
    assert!(facts.diagnostics.is_empty());
    assert!(
        facts
            .sources
            .iter()
            .all(|s| s.supported_auth && s.credential.is_some())
    );
    let (_, secret) = read_hermes_api_source(&target(root.path()), &facts.sources[0]).unwrap();
    assert_eq!(secret.expose(), b"fixture");
}
