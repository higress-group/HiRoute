//! OpenCode uses the same edit/restore machinery, with its native SDK and JSONC policy.
use super::additional_native::{Restore, configure};
use super::opencode::parse_jsonc;
use hiroute_domain::{
    AdditionalAgentModelV1, AgentAccessGrantMaterial, AgentIngressProtocolV1, AgentKindV1,
};
const OWNER: &str = "hiroute-main-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
fn models() -> Vec<AdditionalAgentModelV1> {
    [
        AgentIngressProtocolV1::Responses,
        AgentIngressProtocolV1::Messages,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, protocol)| AdditionalAgentModelV1 {
        alias: format!("hiroute/{i:016x}"),
        protocol,
        context_window_tokens: 16384,
        max_output_tokens: 4096,
    })
    .collect()
}
#[test]
fn opencode_routes_preserve_jsonc_defaults_and_restore_after_unowned_edits() {
    let original = br#"{
      // Daily default remains native.
      "model":"daily/first",
      "small_model":"daily/small",
      "provider":{"daily":{"npm":"@ai-sdk/openai","models":{"first":{},"small":{},},},},
      "extension":{"array":[1,2,],},
    }"#;
    let models = models();
    let token = AgentAccessGrantMaterial::from_csprng_entropy([7; 32]);
    let edit = configure(
        AgentKindV1::OpenCode,
        Some(original),
        OWNER,
        "http://127.0.0.1:4321/v1",
        &models,
        &token,
        None,
    )
    .unwrap();
    let value = parse_jsonc(&edit.bytes).unwrap();
    assert_eq!(value["model"], "daily/first");
    assert_eq!(value["small_model"], "daily/small");
    assert!(value.get("providers").is_none());
    for m in &models {
        let id = hiroute_domain::additional_model_provider_for(OWNER, &models, m);
        let p = &value["provider"][&id];
        assert_eq!(
            p["npm"],
            if m.protocol == AgentIngressProtocolV1::Responses {
                "@ai-sdk/openai"
            } else {
                "@ai-sdk/anthropic"
            }
        );
        assert_eq!(p["models"].as_object().unwrap().len(), 1);
        assert_eq!(p["models"][&m.alias]["id"], m.alias);
        assert_eq!(
            p["options"]["headers"]["X-HiRoute-Token"],
            std::str::from_utf8(token.expose()).unwrap()
        );
        assert_eq!(p["options"]["baseURL"], "http://127.0.0.1:4321/v1");
        assert!(p["models"][&m.alias]["limit"].get("input").is_none());
    }
    let restore = Restore::decode(&edit.restore.encode().unwrap()).unwrap();
    assert_eq!(
        restore
            .restore(Some(&edit.bytes))
            .unwrap()
            .unwrap()
            .as_slice(),
        original
    );
    let edited = String::from_utf8(edit.bytes.to_vec())
        .unwrap()
        .replace("\"array\":[1,2,]", "\"array\":[1,2,3,]");
    let restored = restore.restore(Some(edited.as_bytes())).unwrap().unwrap();
    assert!(
        std::str::from_utf8(&restored)
            .unwrap()
            .contains("\"array\":[1,2,3,]")
    );
    assert_eq!(
        parse_jsonc(&restored).unwrap()["provider"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn opencode_referenced_selector_can_rotate_but_cannot_disappear() {
    let models = models();
    let token = AgentAccessGrantMaterial::from_csprng_entropy([7; 32]);
    let edit = configure(
        AgentKindV1::OpenCode,
        None,
        OWNER,
        "http://127.0.0.1:4321/v1",
        &models,
        &token,
        None,
    )
    .unwrap();
    let mut value = parse_jsonc(&edit.bytes).unwrap();
    let id = hiroute_domain::additional_model_provider_for(OWNER, &models, &models[0]);
    value["agent"] = serde_json::json!({"custom":{"model":format!("{id}/{}",models[0].alias)}});
    let current = serde_json::to_vec(&value).unwrap();
    assert!(edit.restore.restore(Some(&current)).is_err());
    let rotated = AgentAccessGrantMaterial::from_csprng_entropy([8; 32]);
    assert!(
        configure(
            AgentKindV1::OpenCode,
            Some(&current),
            OWNER,
            "http://127.0.0.1:4321/v1",
            &models,
            &rotated,
            Some(&edit.restore)
        )
        .is_ok()
    );
    assert!(
        configure(
            AgentKindV1::OpenCode,
            Some(&current),
            OWNER,
            "http://127.0.0.1:4321/v1",
            &models[1..],
            &rotated,
            Some(&edit.restore)
        )
        .is_err()
    );
}

#[test]
fn opencode_new_native_file_has_schema_without_adopting_an_existing_user_file() {
    let models = models();
    let token = AgentAccessGrantMaterial::from_csprng_entropy([3; 32]);
    let edit = configure(
        AgentKindV1::OpenCode,
        None,
        OWNER,
        "http://127.0.0.1:4321/v1",
        &models,
        &token,
        None,
    )
    .unwrap();
    assert_eq!(
        parse_jsonc(&edit.bytes).unwrap()["$schema"],
        "https://opencode.ai/config.json"
    );
    assert!(edit.restore.restore(Some(&edit.bytes)).unwrap().is_none());
    let existing = br#"{"extension":true}"#;
    let edit = configure(
        AgentKindV1::OpenCode,
        Some(existing),
        OWNER,
        "http://127.0.0.1:4321/v1",
        &models,
        &token,
        None,
    )
    .unwrap();
    assert!(parse_jsonc(&edit.bytes).unwrap().get("$schema").is_none());
}

#[test]
fn opencode_format_metadata_is_bounded_and_does_not_execute_the_selected_binary() {
    let root = tempfile::tempdir().unwrap();
    let platform = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    };
    let cpu = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    let name = format!(
        "opencode-{platform}-{cpu}{}",
        if cfg!(target_env = "musl") {
            "-musl"
        } else {
            ""
        }
    );
    let package = root.path().join(&name);
    let bin = package.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let native = bin.join(if cfg!(windows) {
        "opencode.exe"
    } else {
        "opencode"
    });
    std::fs::write(&native, b"unexecuted native metadata fixture").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut metadata = serde_json::json!({"name":name,"version":"1.19.3","os":[if cfg!(windows){"win32"}else{platform}],"cpu":[cpu]});
    if cfg!(target_env = "musl") {
        metadata["libc"] = serde_json::json!(["musl"]);
    }
    let path = package.join("package.json");
    std::fs::write(&path, metadata.to_string()).unwrap();
    assert_eq!(super::opencode_native_format(&native).unwrap().len(), 38);
    metadata["version"] = "unaudited".into();
    std::fs::write(&path, metadata.to_string()).unwrap();
    assert!(
        super::opencode_native_executable(&native)
            .unwrap()
            .is_some()
    );
    assert!(super::opencode_native_format(&native).is_err());
}

#[test]
fn opencode_symbolic_model_choices_block_removal_but_allow_retained_token_rotation() {
    let models = models();
    let token = AgentAccessGrantMaterial::from_csprng_entropy([7; 32]);
    let edit = configure(
        AgentKindV1::OpenCode,
        None,
        OWNER,
        "http://127.0.0.1:4321/v1",
        &models,
        &token,
        None,
    )
    .unwrap();
    for choice in [
        serde_json::json!({"model":"{env:NATIVE_MODEL}"}),
        serde_json::json!({"small_model":"{file:small-model.txt}"}),
        serde_json::json!({"agent":{"daily":{"model":"{env:NATIVE_MODEL}"}}}),
        serde_json::json!({"mode":{"daily":{"model":"{file:agent-model.txt}"}}}),
    ] {
        let mut value = parse_jsonc(&edit.bytes).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .extend(choice.as_object().unwrap().clone());
        let current = serde_json::to_vec(&value).unwrap();
        assert!(
            edit.restore.restore(Some(&current)).is_err(),
            "an unresolved native selector cannot authorize removal"
        );
        let rotated = AgentAccessGrantMaterial::from_csprng_entropy([8; 32]);
        assert!(
            configure(
                AgentKindV1::OpenCode,
                Some(&current),
                OWNER,
                "http://127.0.0.1:4321/v1",
                &models,
                &rotated,
                Some(&edit.restore),
            )
            .is_ok(),
            "retaining every selector can rotate the grant without interpreting native choices"
        );
    }
}
