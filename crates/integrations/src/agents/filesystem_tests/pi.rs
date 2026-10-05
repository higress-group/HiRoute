//! Passive native API discovery is documentation of the supported product boundary.
use super::*;

fn source_scanner(
    root: &Path,
    models: serde_json::Value,
    auth: serde_json::Value,
) -> FilesystemAgentScannerV1 {
    let layout = layout(root);
    write_secret_settings(&layout.pi_config_root.join("models.json"), models);
    write_secret_settings(&layout.pi_config_root.join("auth.json"), auth);
    FilesystemAgentScannerV1::new(layout, registry())
}
fn models() -> serde_json::Value {
    json!({"providers":{"private-api":{"baseUrl":"http://127.0.0.1:4222/v1","api":"openai-responses",
        "apiKey":"provider-fallback","models":[{"id":"native-model","contextWindow":16384,"maxTokens":4096,"input":["text"]}]}}})
}

#[test]
fn pi_static_api_import_uses_saved_credential_templates_and_rejects_changed_source() {
    let root = tempfile::tempdir().unwrap();
    let scanner = source_scanner(
        root.path(),
        models(),
        json!({"private-api":{"type":"api_key",
        "key":"prefix-${ACCEPTANCE_KEY}","env":{"ACCEPTANCE_KEY":"saved-token"}}}),
    );
    let sources = scanner.pi_api_sources().unwrap();
    assert_eq!(sources.len(), 1);
    let selected = &sources[0];
    assert_eq!(
        selected.protocol,
        Some(hiroute_domain::UpstreamProtocol::Responses)
    );
    let public = serde_json::to_string(selected).unwrap();
    assert!(!public.contains("saved-token") && !public.contains("127.0.0.1"));
    let (endpoint, secret) = scanner.read_pi_api_source(selected).unwrap();
    assert_eq!(endpoint.as_str(), "http://127.0.0.1:4222/v1");
    assert!(secret.expose() == b"prefix-saved-token");
    write_secret_settings(
        &scanner.layout.pi_config_root.join("auth.json"),
        json!({"private-api":{"type":"api_key","key":"rotated"}}),
    );
    assert!(matches!(
        scanner.read_pi_api_source(selected),
        Err(AgentFilesystemScanError::SourceChanged)
    ));
}

#[test]
fn pi_helpers_oauth_and_custom_header_auth_never_become_importable_static_keys() {
    for auth in [
        json!({"private-api":{"type":"api_key","key":"!exit 98"}}),
        json!({"private-api":{"type":"oauth","access":"must-not-import"}}),
    ] {
        let root = tempfile::tempdir().unwrap();
        let scanner = source_scanner(root.path(), models(), auth);
        let source = scanner.pi_api_sources().unwrap().remove(0);
        assert!(source.credential.is_none());
        assert!(scanner.read_pi_api_source(&source).is_err());
    }
    let root = tempfile::tempdir().unwrap();
    let mut declarations = models();
    declarations["providers"]["private-api"]["headers"] = json!({"x-special-auth":"!exit 99"});
    let scanner = source_scanner(root.path(), declarations, json!({}));
    assert!(!scanner.pi_api_sources().unwrap()[0].supported_auth);
}

#[test]
fn pi_static_templates_preserve_invalid_closed_references_and_native_escapes() {
    for (template, expected) in [
        ("${NOT-$PI_TEMPLATE_INNER}", "${NOT-$PI_TEMPLATE_INNER}"),
        (
            "${NOT-$PI_TEMPLATE_INNER}/$PI_TEMPLATE_OUTER",
            "${NOT-$PI_TEMPLATE_INNER}/outer",
        ),
        (
            "$$PI_TEMPLATE_INNER/$!literal",
            "$PI_TEMPLATE_INNER/!literal",
        ),
        ("$1/$PI_TEMPLATE_OUTER", "$1/outer"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let scanner = source_scanner(
            root.path(),
            models(),
            json!({"private-api":{
            "type":"api_key","key":template,"env":{
                "PI_TEMPLATE_INNER":"must-not-expand","PI_TEMPLATE_OUTER":"outer"}}}),
        );
        let source = scanner.pi_api_sources().unwrap().remove(0);
        assert_eq!(
            scanner.read_pi_api_source(&source).unwrap().1.expose(),
            expected.as_bytes()
        );
    }
}

#[test]
fn pi_default_reference_is_protected_independently_of_provider_file() {
    let root = tempfile::tempdir().unwrap();
    let scanner = source_scanner(root.path(), models(), json!({}));
    let before = scanner.pi_default_model().unwrap();
    write_secret_settings(
        &scanner.layout.pi_config_root.join("settings.json"),
        json!({"defaultProvider":"hiroute-main-test","defaultModel":"selected","theme":"dark"}),
    );
    let settings = scanner.layout.pi_config_root.join("settings.json");
    let original = fs::read(&settings).unwrap();
    fs::write(&settings, [&[0xef, 0xbb, 0xbf][..], &original].concat()).unwrap();
    let current = scanner.pi_default_model().unwrap();
    assert_ne!(before.content_digest, current.content_digest);
    assert!(current.removes_default("hiroute-main-test", &[]));
    assert!(!current.removes_default("another-provider", &[]));
    assert!(!current.removes_default(
        "hiroute-main-test",
        &[hiroute_domain::AdditionalAgentModelV1 {
            alias: "selected".into(),
            context_window_tokens: 16384,
            max_output_tokens: 4096
        }]
    ));
}

#[test]
fn pi_import_observes_jsonc_bom_and_the_models_effective_endpoint_override() {
    let root = tempfile::tempdir().unwrap();
    let mut declarations = models();
    declarations["providers"]["private-api"]["models"][0]["baseUrl"] =
        json!("http://127.0.0.1:4333/v1");
    let scanner = source_scanner(root.path(), declarations.clone(), json!({}));
    let target = scanner.pi_user_models_target();
    fs::write(
        &target,
        format!(
            "\u{feff}// native Pi models support comments\n{}",
            declarations
        ),
    )
    .unwrap();
    let selected = scanner.pi_api_sources().unwrap().remove(0);
    assert_eq!(
        scanner.read_pi_api_source(&selected).unwrap().0.as_str(),
        "http://127.0.0.1:4333/v1"
    );
    declarations["providers"]["private-api"]["models"][0]["baseUrl"] =
        json!("http://127.0.0.1:4444/v1");
    write_secret_settings(&target, declarations);
    assert!(matches!(
        scanner.read_pi_api_source(&selected),
        Err(AgentFilesystemScanError::SourceChanged)
    ));
}

#[test]
fn pi_cli_release_labels_and_entry_layout_do_not_gate_settings() {
    let root = tempfile::tempdir().unwrap();
    let package = root.path().join("selected-pi");
    let cli = package.join("bin/pi.js");
    fs::create_dir_all(cli.parent().unwrap()).unwrap();
    write_executable(&cli, "native CLI must not run during metadata discovery");
    let mut selected = layout(root.path());
    selected.pi_executable = cli.clone();
    let scanner = FilesystemAgentScannerV1::new(selected, registry());
    for version in ["1.0.2", "1.0.3", "9.0.0-next"] {
        write_secret_settings(
            &package.join("package.json"),
            json!({
                "name":crate::PI_NPM_PACKAGE, "version":version, "bin":{"pi":"bin/pi.js"}
            }),
        );
        let installation = crate::pi_cli_installation(&cli).unwrap();
        assert_eq!(installation.version, version);
        assert_eq!(installation.package_root, package);
        // A missing Worker SDK does not prevent passive static discovery or settings identity.
        assert!(!package.join("dist/index.js").exists());
        assert!(matches!(
            scanner.pi_settings_discovery().outcome,
            AgentDiscoveryOutcomeV1::Supported { .. }
        ));
    }
    write_secret_settings(
        &package.join("package.json"),
        json!({
            "name":crate::PI_NPM_PACKAGE, "version":"1.0.3", "bin":{"pi":"../foreign.js"}
        }),
    );
    write_executable(&root.path().join("foreign.js"), "foreign CLI");
    assert!(crate::pi_cli_installation(&cli).is_err());
}

#[test]
fn pi_disabled_collaboration_skill_does_not_authorize_enable_or_reuse_a_stale_observation() {
    use hiroute_domain::{AgentCapability, CapabilityState};
    let root = tempfile::tempdir().unwrap();
    let mut layout = layout(root.path());
    let package = root.path().join("selected-pi");
    write_secret_settings(
        &package.join("package.json"),
        json!({"name": crate::PI_NPM_PACKAGE,
        "version":"1.0.2","bin":{"pi":"dist/bundle/cli.js"}}),
    );
    fs::create_dir_all(package.join("dist/bundle")).unwrap();
    write_executable(
        &package.join("dist/bundle/cli.js"),
        "must-not-execute-native-cli",
    );
    fs::write(package.join("dist/index.js"), b"test SDK location only").unwrap();
    layout.pi_executable = package.join("dist/bundle/cli.js");
    let scanner = FilesystemAgentScannerV1::new(layout, registry());
    // This case isolates native Skill exclusion after the local interface check.
    // SDK capability acceptance/rejection is exercised by pi_sdk_contract.test.mjs.
    *scanner.pi_collaboration_cli.lock().unwrap() = Some((
        root.path().join("trusted-hiroute"),
        crate::pi_cli_installation(&scanner.layout.pi_executable)
            .unwrap()
            .manifest_digest,
    ));
    let inspect = || {
        let discovery = scanner.pi_settings_discovery();
        let AgentDiscoveryOutcomeV1::Supported { installation } = discovery.outcome else {
            panic!("fixture installation was not located")
        };
        (
            installation.observation_digest,
            installation
                .capability_evidence
                .into_iter()
                .find(|e| e.capability == AgentCapability::SkillLoading)
                .unwrap()
                .state,
        )
    };
    let before = inspect();
    assert_eq!(before.1, CapabilityState::Proven);
    for rule in [
        "!**/hiroute-collaboration/**",
        "-skills",
        "!**/*",
        "!SKILL.md",
        "!skills/hirout[e]-collaboration/SKILL.md",
        "!SKILL\\.md",
        "!!unrelated-receipt",
    ] {
        write_secret_settings(
            &scanner.layout.pi_config_root.join("settings.json"),
            json!({"skills":[rule]}),
        );
        let excluded = inspect();
        assert_ne!(excluded.0, before.0);
        assert_ne!(excluded.1, CapabilityState::Proven);
    }
    write_secret_settings(
        &scanner.layout.pi_config_root.join("settings.json"),
        json!({"skills":["!unrelated-receipt"]}),
    );
    let settings = scanner.layout.pi_config_root.join("settings.json");
    let original = fs::read(&settings).unwrap();
    fs::write(&settings, [&[0xef, 0xbb, 0xbf][..], &original].concat()).unwrap();
    assert_eq!(inspect().1, CapabilityState::Proven);
    let manifest = package.join("package.json");
    let mut upgraded: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    upgraded["version"] = json!("1.0.3");
    write_secret_settings(&manifest, upgraded);
    assert_ne!(
        inspect().1,
        CapabilityState::Proven,
        "an upgraded CLI must recheck its local resource capability"
    );
    fs::write(&settings, b"[]").unwrap();
    assert_ne!(inspect().1, CapabilityState::Proven);
    assert!(scanner.pi_default_model().is_err());
}
