use super::*;
fn scanner(root: &Path) -> FilesystemAgentScannerV1 {
    let mut layout = layout(root);
    layout.opencode_context =
        Some(crate::agents::OpenCodeNativeContext::from_lookup(root, |_| None).unwrap());
    FilesystemAgentScannerV1::new(layout, registry())
}
fn declaration() -> serde_json::Value {
    json!({"$schema":"https://opencode.ai/config.json","provider":{"ordinary":{"npm":"@ai-sdk/openai","options":{"baseURL":"http://127.0.0.1:4222/v1","apiKey":"static-only"},"models":{"explicit":{"limit":{"context":16384,"output":4096}}}}}})
}

#[test]
fn opencode_routing_resource_root_is_independent_of_an_explicit_config_file() {
    for use_extra in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let home = root.path();
        let xdg = home.join("xdg-config");
        let selected = home.join("selected/custom.jsonc");
        let extra_root = use_extra.then(|| home.join("extra-resources"));
        let context = crate::agents::OpenCodeNativeContext::from_lookup(home, |name| match name {
            "XDG_CONFIG_HOME" => Some(xdg.clone().into_os_string()),
            "OPENCODE_CONFIG" => Some(selected.clone().into_os_string()),
            "OPENCODE_CONFIG_DIR" => extra_root.clone().map(PathBuf::into_os_string),
            _ => None,
        })
        .unwrap();
        let resource = extra_root.unwrap_or_else(|| xdg.join("opencode"));
        let mut configured = layout(home);
        configured.opencode_context = Some(context);
        let scanner = FilesystemAgentScannerV1::new(configured, registry());
        assert_eq!(scanner.opencode_user_models_target(), selected);
        assert_eq!(
            scanner.native_routing_adapter_target(AgentKindV1::OpenCode),
            Some(resource.join("plugins/hiroute-routing.js"))
        );
        assert!(!selected.exists());
        assert!(
            !resource.exists(),
            "passive target selection must not write native resources"
        );
    }
}

#[test]
fn installed_opencode_routing_plugin_preserves_passive_import_but_foreign_plugins_do_not() {
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    write_secret_settings(&scanner.opencode_user_models_target(), declaration());
    let plugins = scanner
        .opencode_native_context()
        .unwrap()
        .resource_root()
        .join("plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    let adapter = plugins.join("hiroute-routing.js");
    std::fs::write(&adapter, crate::agents::OPENCODE_ROUTING_ADAPTER).unwrap();
    let sources = scanner.opencode_api_sources().unwrap();
    assert_eq!(sources.len(), 1);
    assert!(sources[0].supported_auth);
    assert_eq!(
        scanner
            .read_opencode_api_source(&sources[0])
            .unwrap()
            .1
            .expose(),
        b"static-only"
    );

    for (path, content) in [
        (adapter.clone(), "user-edited routing plugin"),
        (
            plugins.join("foreign.js"),
            crate::agents::OPENCODE_ROUTING_ADAPTER,
        ),
    ] {
        std::fs::write(&path, content).unwrap();
        let blocked = scanner.opencode_api_sources().unwrap();
        assert_eq!(blocked.len(), 1);
        assert!(!blocked[0].supported_auth && blocked[0].credential.is_none());
        assert!(scanner.read_opencode_api_source(&sources[0]).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&adapter, crate::agents::OPENCODE_ROUTING_ADAPTER).unwrap();
    }
}

#[test]
fn opencode_provider_credential_failures_do_not_hide_independent_sources_or_relax_stale_reads() {
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    let secret = root.path().join("broken.key");
    let mut value = declaration();
    let mut broken = value["provider"]["ordinary"].clone();
    broken["options"]["apiKey"] = json!(format!("{{file:{}}}", secret.display()));
    value["provider"]["a-broken"] = broken;
    write_secret_settings(&scanner.opencode_user_models_target(), value.clone());
    for bytes in [vec![0xff], vec![b'x'; 1024 * 1024 + 1]] {
        std::fs::write(&secret, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert_independent_source(&scanner);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        std::fs::write(&secret, b"native-public-key").unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&secret).is_err() {
            // Root may bypass POSIX read permissions; exercise a real read denial.
            assert_independent_source(&scanner);
        }
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::remove_file(&secret).unwrap();
        let target = root.path().join("target.key");
        std::fs::write(&target, b"native-linked-key").unwrap();
        symlink(target, &secret).unwrap();
        assert_independent_source(&scanner);
    }
    let selected = scanner
        .opencode_api_sources()
        .unwrap()
        .into_iter()
        .find(|s| s.provider_id == "ordinary")
        .unwrap();
    value["provider"]["ordinary"]["options"]["apiKey"] = json!("changed-independent-key");
    write_secret_settings(&scanner.opencode_user_models_target(), value);
    assert!(scanner.read_opencode_api_source(&selected).is_err());
}

fn assert_independent_source(scanner: &FilesystemAgentScannerV1) {
    let sources = scanner.opencode_api_sources().unwrap();
    assert_eq!(sources.len(), 2);
    let broken = sources
        .iter()
        .find(|s| s.provider_id == "a-broken")
        .unwrap();
    assert!(!broken.supported_auth && broken.credential.is_none());
    assert!(scanner.read_opencode_api_source(broken).is_err());
    let valid = sources
        .iter()
        .find(|s| s.provider_id == "ordinary")
        .unwrap();
    assert!(valid.supported_auth && valid.credential.is_some());
    assert_eq!(
        scanner.read_opencode_api_source(valid).unwrap().1.expose(),
        b"static-only"
    );
}

#[test]
fn opencode_discovers_the_official_home_entry_without_executing_or_replacing_a_selection() {
    let root = tempfile::tempdir().unwrap();
    let mut scanner = scanner(root.path());
    let entry = root.path().join(".opencode/bin/opencode");
    std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
    // Invalid process bytes are intentional: passive discovery must not execute them.
    std::fs::write(&entry, b"metadata-only discovery fixture").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&entry, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    scanner.layout.opencode_executable = "opencode".into();
    let primary =
        crate::agents::opencode_native_executable(std::path::Path::new("opencode")).unwrap();
    assert_eq!(
        scanner.opencode_executable_target(),
        primary.or_else(|| Some(std::fs::canonicalize(&entry).unwrap()))
    );
    assert!(matches!(
        scanner.opencode_settings_discovery().outcome,
        AgentDiscoveryOutcomeV1::Supported { .. }
    ));
    // Losing a saved installation cannot switch the user's Agent to this fallback.
    scanner.layout.opencode_executable = root.path().join("explicitly-selected-but-missing");
    assert!(scanner.opencode_executable_target().is_none());
}

#[test]
fn opencode_dynamic_configuration_keys_are_scoped_out_of_models_workers_and_import() {
    let root = tempfile::tempdir().unwrap();
    let search_path = crate::agents::native_ingress_probe::native_probe_path();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .arg(root.path())
            .env_clear()
            .env("HOME", root.path())
            .env("PATH", &search_path)
            .status()
            .unwrap()
            .success()
    );
    let scanner = scanner(root.path());
    for (section, dynamic_key) in [
        ("root", "{env:CONFIG_SECTION}"),
        ("provider", "{file:provider-name.txt}"),
        ("agent", "{env:AGENT_NAME}"),
    ] {
        let mut value = declaration();
        if section == "root" {
            value[dynamic_key] = json!([]);
        } else {
            value[section][dynamic_key] = json!({});
        }
        write_secret_settings(&scanner.opencode_user_models_target(), value);
        let model_rejected = scanner
            .opencode_model_dependencies("hiroute-main-test", &[])
            .is_err();
        let native = scanner.opencode_native_context().unwrap();
        let config = native.read_config(None).unwrap();
        let worker_rejected = crate::agents::validate_opencode_composition(
            native,
            &config,
            root.path(),
            search_path.to_str().unwrap(),
        )
        .is_err();
        let sources = scanner.opencode_api_sources().unwrap();
        let import_rejected = !sources.is_empty()
            && sources
                .iter()
                .all(|source| !source.supported_auth && source.credential.is_none());
        assert_eq!(
            (model_rejected, worker_rejected, import_rejected),
            (true, true, true),
            "{section}"
        );
    }
}
#[test]
fn opencode_static_import_is_secret_free_and_rejects_stale_credential_reads() {
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    write_secret_settings(&scanner.opencode_user_models_target(), declaration());
    let sources = scanner.opencode_api_sources().unwrap();
    assert_eq!(sources.len(), 1);
    let source = &sources[0];
    assert!(source.supported_auth);
    assert!(
        !serde_json::to_string(source)
            .unwrap()
            .contains("static-only")
    );
    let (endpoint, key) = scanner.read_opencode_api_source(source).unwrap();
    assert_eq!(endpoint.as_str(), "http://127.0.0.1:4222/v1");
    assert_eq!(key.expose(), b"static-only");
    let mut changed = declaration();
    changed["provider"]["ordinary"]["options"]["apiKey"] = "rotated".into();
    write_secret_settings(&scanner.opencode_user_models_target(), changed);
    assert!(scanner.read_opencode_api_source(source).is_err());
}
#[test]
fn opencode_dynamic_options_and_oauth_remain_non_importable() {
    for option in ["headers", "fetch", "region"] {
        let root = tempfile::tempdir().unwrap();
        let scanner = scanner(root.path());
        let mut value = declaration();
        value["provider"]["ordinary"]["options"][option] = json!({"unsafe":"must-not-run"});
        write_secret_settings(&scanner.opencode_user_models_target(), value);
        assert!(
            scanner.opencode_api_sources().unwrap()[0]
                .credential
                .is_none()
        );
    }
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    write_secret_settings(&scanner.opencode_user_models_target(), declaration());
    let native = scanner.opencode_native_context().unwrap();
    write_secret_settings(
        &native.data_root.join("auth.json"),
        json!({"ordinary":{"type":"oauth","access":"never-copy"}}),
    );
    assert!(
        scanner.opencode_api_sources().unwrap()[0]
            .credential
            .is_none()
    );
}

#[test]
fn opencode_review_substituted_model_ids_cannot_be_imported_as_literals() {
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    for id in [
        "{env:UPSTREAM_MODEL}",
        "{file:model-id.txt}",
        "prefix-{env:UPSTREAM_MODEL}",
    ] {
        let mut value = declaration();
        value["provider"]["ordinary"]["models"]["explicit"]["id"] = json!(id);
        write_secret_settings(&scanner.opencode_user_models_target(), value);
        let sources = scanner.opencode_api_sources().unwrap();
        assert_eq!(sources.len(), 1);
        assert!(!sources[0].supported_auth, "{id}");
        assert!(sources[0].credential.is_none());
        assert!(scanner.read_opencode_api_source(&sources[0]).is_err());
    }
    write_secret_settings(&scanner.opencode_user_models_target(), declaration());
    let sources = scanner.opencode_api_sources().unwrap();
    assert!(sources[0].supported_auth);
    assert!(scanner.read_opencode_api_source(&sources[0]).is_ok());
}

#[test]
fn opencode_model_target_stays_bound_and_explicit_selectors_have_no_alternative() {
    let root = tempfile::tempdir().unwrap();
    let native = crate::agents::OpenCodeNativeContext::from_lookup(root.path(), |_| None).unwrap();
    let json = native.config_root.join("opencode.json");
    write_secret_settings(&json, declaration());
    let scanner = scanner(root.path());
    write_secret_settings(&json.with_extension("jsonc"), json!({}));
    assert_eq!(scanner.opencode_user_models_target(), json);
    assert_eq!(
        scanner.opencode_model_target_candidates(),
        vec![json.clone(), json.with_extension("jsonc")]
    );

    let explicit_root = tempfile::tempdir().unwrap();
    let mut layout = layout(explicit_root.path());
    let selected = explicit_root.path().join("explicit.json");
    layout.opencode_context = Some(
        crate::agents::OpenCodeNativeContext::from_lookup(explicit_root.path(), |key| {
            (key == "OPENCODE_CONFIG").then(|| selected.clone().into_os_string())
        })
        .unwrap(),
    );
    let mut explicit = FilesystemAgentScannerV1::new(layout, registry());
    assert_eq!(explicit.opencode_model_target_candidates(), vec![selected]);
    let unselected_sibling = explicit
        .opencode_native_context()
        .unwrap()
        .config_root
        .join("opencode.jsonc");
    assert!(
        explicit
            .bind_opencode_model_target(unselected_sibling)
            .is_err()
    );
}
#[test]
fn opencode_settings_dependencies_detect_layer_changes_and_owned_provider_shadowing() {
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    let native = scanner.opencode_native_context().unwrap();
    let a = scanner
        .opencode_model_dependencies("hiroute-main-test", &[])
        .unwrap();
    write_secret_settings(
        &native.config_root.join("config.json"),
        json!({"small_model":"daily/small"}),
    );
    let b = scanner
        .opencode_model_dependencies("hiroute-main-test", &[])
        .unwrap();
    assert_ne!(a, b);
    write_secret_settings(
        &native.config_root.join("config.json"),
        json!({"provider":{"hiroute-main-test":{"options":{"apiKey":"foreign"}}}}),
    );
    assert!(
        scanner
            .opencode_model_dependencies("hiroute-main-test", &[])
            .is_err()
    );
}

#[test]
fn opencode_default_reference_conflict_is_a_preview_fact_and_a_stage_rejection() {
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    let native = scanner.opencode_native_context().unwrap();
    write_secret_settings(
        &scanner.opencode_user_models_target(),
        json!({"provider":{"hiroute-main-test":{"models":{"route":{}}}}}),
    );
    write_secret_settings(
        &native.config_root.join("config.json"),
        json!({"small_model":"hiroute-main-test/route"}),
    );
    let (_, in_use) = scanner
        .opencode_model_facts("hiroute-main-test", &[])
        .unwrap();
    assert!(
        in_use,
        "Preview must retain a closed, actionable reference conflict"
    );
    assert!(
        scanner
            .opencode_model_dependencies("hiroute-main-test", &[])
            .is_err(),
        "staging must reject the dangling reference"
    );
}

#[test]
fn opencode_restore_file_capabilities_survive_a_missing_executable_and_bad_model_json() {
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    write_secret_settings(
        &scanner.opencode_user_models_target(),
        json!({"provider":{}}),
    );
    std::fs::write(
        scanner.opencode_user_models_target(),
        b"malformed daily model configuration",
    )
    .unwrap();
    let discovery = scanner.opencode_settings_discovery();
    let AgentDiscoveryOutcomeV1::Supported { installation } = discovery.outcome else {
        panic!("existing files must remain recoverable");
    };
    assert!(installation.capability_evidence.iter().any(|p| p.capability
        == hiroute_domain::AgentCapability::AtomicManagedReplace
        && p.state == hiroute_domain::CapabilityState::Proven));
    assert!(scanner.opencode_executable_target().is_none());
}

#[test]
fn opencode_connection_points_group_native_bindings_and_recheck_rotated_secrets() {
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    let mut value = declaration();
    value["model"] = json!("ordinary/explicit");
    value["provider"]["ordinary"]["models"]["second"] =
        json!({"limit":{"context":16384,"output":4096}});
    value["provider"]["ordinary"]["models"]["messages"] =
        json!({"provider":{"npm":"@ai-sdk/anthropic"}});
    write_secret_settings(&scanner.opencode_user_models_target(), value.clone());
    let sources = scanner.opencode_api_sources().unwrap();
    let points = crate::agents::native_api_connection_points(sources);
    assert_eq!(points.len(), 2);
    let responses = points
        .iter()
        .find(|p| p.protocol() == Some(hiroute_domain::UpstreamProtocol::Responses))
        .unwrap();
    assert_eq!(responses.sources.len(), 2);
    assert_eq!(
        responses.sources[0]
            .point_binding
            .as_ref()
            .unwrap()
            .configured_default_model_id
            .as_deref(),
        Some("explicit")
    );
    assert!(points.iter().any(|p| p.protocol()
        == Some(hiroute_domain::UpstreamProtocol::Messages)
        && p.sources.len() == 1));
    value["provider"]["ordinary"]["options"]["apiKey"] = json!("rotated-point-key");
    write_secret_settings(&scanner.opencode_user_models_target(), value);
    let fresh =
        crate::agents::native_api_connection_points(scanner.opencode_api_sources().unwrap());
    let updated = fresh
        .iter()
        .find(|p| p.key_digest == responses.key_digest)
        .unwrap();
    assert_ne!(updated.snapshot_digest, responses.snapshot_digest);
    assert!(
        scanner
            .read_opencode_api_source(&responses.sources[0])
            .is_err()
    );
    assert_eq!(
        scanner
            .read_opencode_api_source(&updated.sources[0])
            .unwrap()
            .1
            .expose(),
        b"rotated-point-key"
    );
}
