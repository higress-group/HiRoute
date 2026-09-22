use super::*;

#[test]
fn codex_explicit_reasoning_reads_the_original_native_value() {
    assert_eq!(
        codex_explicit_reasoning_effort("model_reasoning_effort = 'high'\n").unwrap(),
        Some("high".to_owned())
    );
    assert_eq!(
        codex_explicit_reasoning_effort("model = 'native'\n").unwrap(),
        None
    );
    assert!(codex_explicit_reasoning_effort("model_reasoning_effort = 42\n").is_err());
}

#[test]
fn codex_protected_restore_record_survives_restart_and_keeps_user_changes() {
    let original = "# user's comment\nmodel = 'private-original'\ncustom = 3\n";
    let edit = configure(original, CodexSelectionTarget::Root).unwrap();
    let encoded = edit.restore.encode_protected().unwrap();
    let restored_record = CodexNativeRestore::decode_protected(&encoded).unwrap();
    let current = edit.rendered.replace("custom = 3", "custom = 4");
    let restored = restore_codex_native(&current, &restored_record).unwrap();
    assert_eq!(
        restored.as_str(),
        original.replace("custom = 3", "custom = 4")
    );
    assert!(CodexNativeRestore::decode_protected(b"{}").is_err());
    let mut corrupt: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    corrupt["fields"][0]["path"] = serde_json::json!(["user_private_key"]);
    assert!(CodexNativeRestore::decode_protected(&serde_json::to_vec(&corrupt).unwrap()).is_err());
}

fn configure(
    text: &str,
    target: CodexSelectionTarget,
) -> Result<CodexNativeEdit, CodexNativeError> {
    configure_codex_native(
        text,
        &CanonicalDigest::of_bytes(text.as_bytes()),
        target,
        "hiroute",
        "http://127.0.0.1:5837/v1",
        Some("hiroute/0011223344556677"),
        &AgentAccessGrantMaterial::from_csprng_entropy([3; 32]),
    )
}

#[test]
fn codex_native_preserves_comments_unknown_fields_and_original_provider() {
    let original = "# personal configuration\nmodel = 'old' # keep this\nmodel_provider = 'mine'\nunrelated = 42\n[model_providers.mine]\nbase_url = 'https://original.example'\nenv_key = 'PRIVATE_KEY'\n";
    let edit = configure(original, CodexSelectionTarget::Root).unwrap();
    assert!(edit.rendered.contains("# personal configuration"));
    assert!(edit.rendered.contains("# keep this"));
    assert!(edit.rendered.contains("env_key = 'PRIVATE_KEY'"));
    let parsed = edit.rendered.parse::<DocumentMut>().unwrap();
    assert_eq!(
        parsed["model_providers"]["hiroute"]["wire_api"].as_str(),
        Some("responses")
    );
    let provider = parsed["model_providers"]["hiroute"].as_table().unwrap();
    assert!(
        provider["http_headers"]["X-HiRoute-Token"]
            .as_str()
            .is_some()
    );
    assert_eq!(provider["requires_openai_auth"].as_bool(), Some(true));
    assert_eq!(provider["supports_websockets"].as_bool(), Some(false));
    for forbidden in [
        "experimental_bearer_token",
        "env_key",
        "env_http_headers",
        "auth_command",
    ] {
        assert!(!provider.contains_key(forbidden));
    }
    let user_edited = edit.rendered.replace("unrelated = 42", "unrelated = 43");
    let restored = restore_codex_native(&user_edited, &edit.restore).unwrap();
    assert_eq!(
        restored.as_str(),
        original.replace("unrelated = 42", "unrelated = 43")
    );
    assert_eq!(
        restore_codex_native(&restored, &edit.restore).unwrap(),
        restored
    );
}

#[test]
fn codex_native_profile_changes_are_scoped_and_existing_provider_is_not_claimed() {
    let original =
        "model = 'daily'\n[profiles.work]\nmodel = 'work'\n[profiles.other]\nmodel = 'other'\n";
    let edit = configure(original, CodexSelectionTarget::LegacyProfile("work".into())).unwrap();
    let parsed = edit.rendered.parse::<DocumentMut>().unwrap();
    assert_eq!(parsed["model"].as_str(), Some("daily"));
    assert_eq!(parsed["profiles"]["other"]["model"].as_str(), Some("other"));
    assert_eq!(
        parsed["profiles"]["work"]["model"].as_str(),
        Some("hiroute/0011223344556677")
    );
    assert!(matches!(
        configure(
            "[model_providers.hiroute]\nunknown = 1\n",
            CodexSelectionTarget::Root
        ),
        Err(CodexNativeError::ProviderAlreadyExists)
    ));
}

#[test]
fn codex_native_restore_conflict_and_added_user_field_never_get_overwritten() {
    let edit = configure("", CodexSelectionTarget::Root).unwrap();
    let mut changed = edit.rendered.parse::<DocumentMut>().unwrap();
    changed["model"] = toml_edit::value("my-new-choice");
    assert!(matches!(
        restore_codex_native(&changed.to_string(), &edit.restore),
        Err(CodexNativeError::FieldConflict)
    ));
    let mut changed = edit.rendered.parse::<DocumentMut>().unwrap();
    changed["model_providers"]["hiroute"]["user_extension"] = toml_edit::value("keep");
    let restored = restore_codex_native(&changed.to_string(), &edit.restore).unwrap();
    let parsed = restored.parse::<DocumentMut>().unwrap();
    assert_eq!(
        parsed["model_providers"]["hiroute"]["user_extension"].as_str(),
        Some("keep")
    );
    assert!(parsed.get("model").is_none());
}

#[test]
fn codex_native_rejects_duplicate_keys_and_stale_preview() {
    assert!(matches!(
        configure("model = 'one'\nmodel = 'two'\n", CodexSelectionTarget::Root),
        Err(CodexNativeError::InvalidToml)
    ));
    assert!(matches!(
        configure_codex_native(
            "model = 'changed'",
            &CanonicalDigest::of_bytes(b"old"),
            CodexSelectionTarget::Root,
            "hiroute",
            "http://127.0.0.1:5837/v1",
            Some("hiroute/0011223344556677"),
            &AgentAccessGrantMaterial::from_csprng_entropy([4; 32])
        ),
        Err(CodexNativeError::SourceChanged)
    ));
}

#[test]
fn codex_preserve_native_does_not_own_explicit_or_implicit_model() {
    for original in [
        "# implicit default\n",
        "model = 'Vendor/Native[1m]' # preserve\n",
    ] {
        let edit = configure_codex_native(
            original,
            &CanonicalDigest::of_bytes(original.as_bytes()),
            CodexSelectionTarget::Root,
            "hiroute",
            "http://127.0.0.1:5837/v1",
            None,
            &AgentAccessGrantMaterial::from_csprng_entropy([4; 32]),
        )
        .unwrap();
        assert!(
            edit.rendered.contains(original),
            "{}",
            edit.rendered.as_str()
        );
        let before = original.parse::<DocumentMut>().unwrap();
        let after = edit.rendered.parse::<DocumentMut>().unwrap();
        assert_eq!(
            before.get("model").map(Item::to_string),
            after.get("model").map(Item::to_string)
        );
        assert_eq!(
            restore_codex_native(&edit.rendered, &edit.restore)
                .unwrap()
                .as_str(),
            original
        );
        let record =
            CodexNativeRestore::decode_protected(&edit.restore.encode_protected().unwrap())
                .unwrap();
        let mut user_changed = after;
        user_changed["model"] = toml_edit::value("User/NewChoice");
        let restored = restore_codex_native(&user_changed.to_string(), &record).unwrap();
        assert_eq!(
            restored.parse::<DocumentMut>().unwrap()["model"].as_str(),
            Some("User/NewChoice")
        );
    }
}

#[test]
fn codex_preserve_native_restores_only_a_hiroute_route_alias() {
    for original in ["# implicit default\n", "model = 'native-original' # keep\n"] {
        let edit = configure_codex_native(
            original,
            &CanonicalDigest::of_bytes(original.as_bytes()),
            CodexSelectionTarget::Root,
            "hiroute",
            "http://127.0.0.1:5837/v1",
            None,
            &AgentAccessGrantMaterial::from_csprng_entropy([4; 32]),
        )
        .unwrap()
        .with_managed_aliases(&["hiroute-coding".into()])
        .unwrap();
        let restore =
            CodexNativeRestore::decode_protected(&edit.restore.encode_protected().unwrap())
                .unwrap();
        let mut active = edit.rendered.parse::<DocumentMut>().unwrap();
        active["model"] = toml_edit::value("hiroute-coding");
        let restored = restore_codex_native(&active.to_string(), &restore).unwrap();
        let restored = restored.parse::<DocumentMut>().unwrap();
        let initial = original.parse::<DocumentMut>().unwrap();
        assert_eq!(
            restored.get("model").and_then(Item::as_str),
            initial.get("model").and_then(Item::as_str)
        );
        assert!(restored.get("model_providers").is_none());

        let mut unrelated_alias = edit.rendered.parse::<DocumentMut>().unwrap();
        unrelated_alias["model"] = toml_edit::value("hiroute-other");
        let restored = restore_codex_native(&unrelated_alias.to_string(), &restore).unwrap();
        assert_eq!(
            restored.parse::<DocumentMut>().unwrap()["model"].as_str(),
            Some("hiroute-other")
        );

        let mut native_choice = edit.rendered.parse::<DocumentMut>().unwrap();
        native_choice["model"] = toml_edit::value("native-other");
        let restored = restore_codex_native(&native_choice.to_string(), &restore).unwrap();
        assert_eq!(
            restored.parse::<DocumentMut>().unwrap()["model"].as_str(),
            Some("native-other")
        );
    }
}

#[test]
fn stale_original_alias_can_be_repaired_with_an_explicit_native_model() {
    let original = "model = 'hiroute-fanyi'\nuser_option = true\n";
    let edit = configure_codex_native(
        original,
        &CanonicalDigest::of_bytes(original.as_bytes()),
        CodexSelectionTarget::Root,
        "hiroute",
        "http://127.0.0.1:5837/v1",
        None,
        &AgentAccessGrantMaterial::from_csprng_entropy([4; 32]),
    )
    .unwrap();
    let restore =
        CodexNativeRestore::decode_protected(&edit.restore.encode_protected().unwrap()).unwrap();
    let repaired =
        restore_codex_native_with_model(&edit.rendered, &restore, Some("gpt-5.6-sol")).unwrap();
    let parsed = repaired.parse::<DocumentMut>().unwrap();
    assert_eq!(parsed["model"].as_str(), Some("gpt-5.6-sol"));
    assert!(parsed.get("model_provider").is_none());
    assert!(parsed.get("model_providers").is_none());
    assert_eq!(parsed["user_option"].as_bool(), Some(true));
}

#[test]
fn codex_explicit_fixed_name_is_not_rewritten_as_a_plan_alias() {
    let edit = configure_codex_native(
        "",
        &CanonicalDigest::of_bytes(b""),
        CodexSelectionTarget::Root,
        "hiroute",
        "http://127.0.0.1:5837/v1",
        Some("Vendor/Native[1m]"),
        &AgentAccessGrantMaterial::from_csprng_entropy([4; 32]),
    )
    .unwrap();
    assert_eq!(
        edit.rendered.parse::<DocumentMut>().unwrap()["model"].as_str(),
        Some("Vendor/Native[1m]")
    );
}

#[test]
fn codex_catalog_pointer_restores_original_and_preserves_unrelated_changes() {
    let original = "model_catalog_json = '/user/original.json' # own catalog\ncustom = 1\n";
    let edit = configure(original, CodexSelectionTarget::Root)
        .unwrap()
        .with_model_catalog(std::path::Path::new("/owned/catalog one.json"))
        .unwrap();
    let parsed = edit.rendered.parse::<DocumentMut>().unwrap();
    assert_eq!(
        parsed["model_catalog_json"].as_str(),
        Some("/owned/catalog one.json")
    );
    assert!(edit.rendered.contains("# own catalog"));
    let encoded = edit.restore.encode_protected().unwrap();
    let record = CodexNativeRestore::decode_protected(&encoded).unwrap();
    let changed = edit.rendered.replace("custom = 1", "custom = 2");
    assert_eq!(
        restore_codex_native(&changed, &record).unwrap().as_str(),
        original.replace("custom = 1", "custom = 2")
    );
}

#[test]
fn codex_catalog_pointer_is_scoped_to_the_selected_profile() {
    let original = "model_catalog_json = '/root.json'\n[profiles.work]\nmodel = 'work'\n[profiles.other]\nmodel_catalog_json = '/other.json'\n";
    let edit = configure(original, CodexSelectionTarget::LegacyProfile("work".into()))
        .unwrap()
        .with_model_catalog(std::path::Path::new("/owned/work.json"))
        .unwrap();
    let parsed = edit.rendered.parse::<DocumentMut>().unwrap();
    assert_eq!(parsed["model_catalog_json"].as_str(), Some("/root.json"));
    assert_eq!(
        parsed["profiles"]["other"]["model_catalog_json"].as_str(),
        Some("/other.json")
    );
    assert_eq!(
        parsed["profiles"]["work"]["model_catalog_json"].as_str(),
        Some("/owned/work.json")
    );
    let record =
        CodexNativeRestore::decode_protected(&edit.restore.encode_protected().unwrap()).unwrap();
    assert_eq!(
        restore_codex_native(&edit.rendered, &record)
            .unwrap()
            .as_str(),
        original
    );
}

#[test]
fn codex_catalog_update_keeps_first_restore_point() {
    let original = "model_catalog_json = '/user/original.json'\n";
    let first = configure(original, CodexSelectionTarget::Root)
        .unwrap()
        .with_model_catalog(std::path::Path::new("/owned/first.json"))
        .unwrap();
    let second = reconfigure_codex_native(
        &first.rendered,
        &CanonicalDigest::of_bytes(first.rendered.as_bytes()),
        &first.restore,
        CodexSelectionTarget::Root,
        "hiroute",
        "http://127.0.0.1:5837/v1",
        None,
        &AgentAccessGrantMaterial::from_csprng_entropy([5; 32]),
    )
    .unwrap()
    .with_model_catalog(std::path::Path::new("/owned/second.json"))
    .unwrap();
    assert_eq!(
        second.rendered.parse::<DocumentMut>().unwrap()["model_catalog_json"].as_str(),
        Some("/owned/second.json")
    );
    let record =
        CodexNativeRestore::decode_protected(&second.restore.encode_protected().unwrap()).unwrap();
    assert_eq!(
        restore_codex_native(&second.rendered, &record)
            .unwrap()
            .as_str(),
        original
    );
}

#[test]
fn codex_catalog_user_change_is_not_overwritten_by_restore() {
    let edit = configure("", CodexSelectionTarget::Root)
        .unwrap()
        .with_model_catalog(std::path::Path::new("/owned/catalog.json"))
        .unwrap();
    let changed = edit
        .rendered
        .replace("/owned/catalog.json", "/user/new.json");
    assert!(matches!(
        restore_codex_native(&changed, &edit.restore),
        Err(CodexNativeError::FieldConflict)
    ));
}

#[test]
fn codex_catalog_relative_path_is_rejected() {
    assert!(matches!(
        configure("", CodexSelectionTarget::Root)
            .unwrap()
            .with_model_catalog(std::path::Path::new("relative.json")),
        Err(CodexNativeError::InvalidIntent)
    ));
}

#[test]
fn codex_catalog_parent_path_is_rejected() {
    assert!(matches!(
        configure("", CodexSelectionTarget::Root)
            .unwrap()
            .with_model_catalog(std::path::Path::new("/owned/../catalog.json")),
        Err(CodexNativeError::InvalidIntent)
    ));
}

#[test]
fn codex_catalog_repeated_attachment_is_rejected() {
    let edit = configure("", CodexSelectionTarget::Root)
        .unwrap()
        .with_model_catalog(std::path::Path::new("/owned/first.json"))
        .unwrap();
    assert!(matches!(
        edit.with_model_catalog(std::path::Path::new("/owned/second.json")),
        Err(CodexNativeError::InvalidIntent)
    ));
}

#[test]
fn codex_native_never_produces_a_config_too_large_for_its_restore_reader() {
    let near_limit = format!("#{}\n", "x".repeat(MAX_BYTES - 12));
    assert!(near_limit.len() < MAX_BYTES);
    assert!(matches!(
        configure(&near_limit, CodexSelectionTarget::Root),
        Err(CodexNativeError::InvalidToml)
    ));
}
