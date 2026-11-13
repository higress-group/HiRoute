use super::additional_native::{Restore, configure};
use super::dsh_config::Patch;
use hiroute_domain::{
    AdditionalAgentModelV1, AgentAccessGrantMaterial, AgentIngressProtocolV1, AgentKindV1,
};
const OWNER: &str = "hiroute-main-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn dsh_desktop_restore_preserves_comments_after_removing_the_only_patch_row() {
    use super::additional_native::configure_dsh_desktop;
    let directory = tempfile::tempdir().unwrap();
    let edit = configure_dsh_desktop(
        &directory.path().join("cordis.patch.yml"),
        None,
        OWNER,
        "http://127.0.0.1:4321/v1",
        &[AdditionalAgentModelV1 {
            alias: "hiroute-plan".into(),
            protocol: AgentIngressProtocolV1::Responses,
            context_window_tokens: 32768,
            max_output_tokens: 4096,
        }],
        &AgentAccessGrantMaterial::from_csprng_entropy([5; 32]),
        None,
    )
    .unwrap();
    let restore = Restore::decode(&edit.restore.encode().unwrap()).unwrap();
    assert!(restore.restore(Some(&edit.bytes)).unwrap().is_none());

    let changed = [
        b"# note before the route\n".as_slice(),
        edit.bytes.as_slice(),
        b"  # note after the route\n",
    ]
    .concat();
    let restored = restore.restore(Some(&changed)).unwrap().unwrap();
    assert_eq!(
        restored.as_slice(),
        b"# note before the route\n[]\n  # note after the route\n"
    );
    let edited = String::from_utf8(changed)
        .unwrap()
        .replace("hiroute-routing.mjs", "edited-adapter.mjs");
    assert!(restore.restore(Some(edited.as_bytes())).is_err());
    let restored = restore
        .render_restoration(Some(edited.as_bytes()), true)
        .unwrap()
        .unwrap();
    assert_eq!(
        restored.as_slice(),
        b"# note before the route\n[]\n  # note after the route\n"
    );
}

#[test]
fn dsh_desktop_save_moves_only_released_owned_providers_and_restores_the_profile() {
    use super::additional_native::{attach_routing_adapter, configure_dsh_desktop};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("cordis.patch.yml");
    let baseline = b"# baseline\n- id: original\n  config: {keep: true}\n";
    let user_edit = b"- id: user-plugin\n  config: {new: true}\n";
    let models = [AdditionalAgentModelV1 {
        alias: "hiroute-plan".into(),
        protocol: AgentIngressProtocolV1::Responses,
        context_window_tokens: 32768,
        max_output_tokens: 4096,
    }];
    let grant = AgentAccessGrantMaterial::from_csprng_entropy([5; 32]);
    let endpoint = "http://127.0.0.1:4321/v1";
    let mut legacy = configure(
        AgentKindV1::DeepseekHarness,
        Some(baseline),
        OWNER,
        endpoint,
        &models,
        &grant,
        None,
    )
    .unwrap();
    legacy.bytes = attach_routing_adapter(&legacy.bytes, &path, None).unwrap();
    legacy.restore.rendered_digest = hiroute_domain::CanonicalDigest::of_bytes(&legacy.bytes);
    let old = Restore::decode(&legacy.restore.encode().unwrap()).unwrap();
    let current = [legacy.bytes.as_slice(), user_edit.as_slice()].concat();
    let new = configure_dsh_desktop(
        &path,
        Some(&current),
        OWNER,
        endpoint,
        &models,
        &grant,
        Some(&old),
    )
    .unwrap();
    let patch = Patch::parse(Some(&new.bytes)).unwrap();
    assert!(patch.providers().unwrap().as_object().unwrap().is_empty());
    let id = hiroute_domain::additional_model_provider_for(OWNER, &models, &models[0]);
    let insert = patch.routing_adapter().unwrap().unwrap();
    assert_eq!(
        insert["insert"][0]["config"]["providers"][id]["models"][0]["id"],
        models[0].alias
    );
    let restore = Restore::decode(&new.restore.encode().unwrap()).unwrap();
    assert!(restore.applied(Some(&new.bytes)).unwrap());
    let additional = b"\n# keep this comment\n- id: another-user-plugin\n  config: {keep: later}\n";
    let drift = [new.bytes.as_slice(), additional.as_slice()].concat();
    assert_eq!(
        restore.restore(Some(&drift)).unwrap().unwrap().as_slice(),
        [
            baseline.as_slice(),
            user_edit.as_slice(),
            additional.as_slice()
        ]
        .concat()
    );
}

#[test]
fn dsh_routing_adapter_insert_is_idempotent_restorable_and_rejects_foreign_ownership() {
    let original = b"# keep this\n- id: other\n  config: {keep: true}\n";
    let original_patch = Patch::parse(Some(original)).unwrap();
    let uri = "file:///synthetic/hiroute-routing.mjs";
    let configured = original_patch.edit_routing_adapter(uri, None).unwrap();
    assert!(configured.starts_with(original));
    let restored_patch = Patch::parse(Some(&configured)).unwrap();
    assert_eq!(
        restored_patch.edit_routing_adapter(uri, None).unwrap(),
        configured
    );
    assert_eq!(
        restored_patch
            .edit_routing_adapter(uri, Some(&original_patch))
            .unwrap()
            .as_slice(),
        original
    );
    let edited = [
        configured.as_slice(),
        b"- id: later\n  config: {keep: later}\n",
    ]
    .concat();
    assert_eq!(
        Patch::parse(Some(&edited))
            .unwrap()
            .edit_routing_adapter(uri, Some(&original_patch))
            .unwrap()
            .as_slice(),
        [
            original.as_slice(),
            b"- id: later\n  config: {keep: later}\n"
        ]
        .concat()
    );
    let foreign = b"- insert: [{id: hiroute-routing, name: 'file:///foreign.mjs'}]\n";
    assert!(
        Patch::parse(Some(foreign))
            .unwrap()
            .edit_routing_adapter(uri, None)
            .is_err()
    );
    assert!(
        Patch::parse(Some(b"[{id: foreign}]\n"))
            .unwrap()
            .edit_routing_adapter(uri, None)
            .is_err()
    );
}

#[test]
fn dsh_additional_routes_preserve_foreign_configuration_and_restore_after_user_edits() {
    let original = b"# user's configuration\n- id: llm-pi-ai\n  config:\n    providers:\n      native:\n        api: openai-responses\n        baseURL: https://native.invalid/v1\n        models: [{id: original}]\n- id: agent-default-model\n  config: {provider: native, model: original}\n- id: unrelated\n  config: {keep: true}\n";
    let models: Vec<_> = [
        AgentIngressProtocolV1::Responses,
        AgentIngressProtocolV1::Messages,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, protocol)| AdditionalAgentModelV1 {
        alias: format!("hiroute/{i:016x}"),
        protocol,
        context_window_tokens: 32768,
        max_output_tokens: 4096,
    })
    .collect();
    let token = AgentAccessGrantMaterial::from_csprng_entropy([4; 32]);
    let edit = configure(
        AgentKindV1::DeepseekHarness,
        Some(original),
        OWNER,
        "http://127.0.0.1:4321/v1",
        &models,
        &token,
        None,
    )
    .unwrap();
    let root = Patch::parse(Some(&edit.bytes))
        .unwrap()
        .model_root()
        .unwrap();
    assert_eq!(
        root["providers"]["native"]["baseURL"],
        "https://native.invalid/v1"
    );
    assert_eq!(root["model"]["name"], "native/original");
    assert!(
        std::str::from_utf8(&edit.bytes)
            .unwrap()
            .contains("- id: unrelated\n  config: {keep: true}\n")
    );
    for model in &models {
        let id = hiroute_domain::additional_model_provider_for(OWNER, &models, model);
        assert_eq!(
            root["providers"][&id]["api"],
            if model.protocol == AgentIngressProtocolV1::Responses {
                "openai-responses"
            } else {
                "anthropic-messages"
            }
        );
        assert_eq!(
            root["providers"][&id]["baseURL"],
            if model.protocol == AgentIngressProtocolV1::Responses {
                "http://127.0.0.1:4321/v1"
            } else {
                "http://127.0.0.1:4321"
            }
        );
        assert!(root["providers"][&id].get("apiKeyEnv").is_none());
        assert_eq!(
            root["providers"][&id]["headers"]["X-HiRoute-Token"],
            std::str::from_utf8(token.expose()).unwrap()
        );
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
    let edited = [
        edit.bytes.as_slice(),
        b"- id: later-edit\n  config: {keep: later}\n",
    ]
    .concat();
    let restored = restore.restore(Some(&edited)).unwrap().unwrap();
    assert!(
        std::str::from_utf8(&restored)
            .unwrap()
            .contains("- id: later-edit\n  config: {keep: later}\n")
    );
    assert_eq!(
        Patch::parse(Some(&restored))
            .unwrap()
            .providers()
            .unwrap()
            .as_object()
            .unwrap()
            .len(),
        1
    );
    let id = hiroute_domain::additional_model_provider_for(OWNER, &models, &models[0]);
    let selected = [
        edit.bytes.as_slice(),
        format!(
            "- id: agent-default-model2\n  config: {{provider: '{id}', model: '{}'}}\n",
            models[0].alias
        )
        .as_bytes(),
    ]
    .concat();
    // A real native default reference blocks removal; it never silently selects a replacement.
    let selected = String::from_utf8(selected)
        .unwrap()
        .replace("- id: agent-default-model\n", "- id: old-default\n")
        .replace("agent-default-model2", "agent-default-model");
    assert!(restore.restore(Some(selected.as_bytes())).is_err());
}

#[test]
fn dsh_dynamic_or_ambiguous_configuration_is_rejected_without_evaluation() {
    for bytes in [
        b"- id: llm-pi-ai\n  config: !!js 'run()'\n".as_slice(),
        b"- id: llm-pi-ai\n  config: {}\n- id: llm-pi-ai\n  config: {}\n",
        b"- id: llm-pi-ai\n  config: {<<: {providers: {}}}\n",
    ] {
        assert!(
            Patch::parse(Some(bytes)).is_err(),
            "accepted unsafe patch: {}",
            String::from_utf8_lossy(bytes)
        );
    }
}

#[test]
fn dsh_flow_configuration_is_readable_but_not_edited_without_safe_row_spans() {
    for bytes in [
        br#"[{"id":"llm-pi-ai","config":{"providers":{"native":{"api":"openai-responses"}}}},{"id":"agent-default-model","config":{"provider":"native","model":"original"}}]"#.as_slice(),
        b"[{id: llm-pi-ai, config: {providers: {native: {api: openai-responses}}}}, {id: agent-default-model, config: {provider: native, model: original}}]\n".as_slice(),
        b"[\n  {id: llm-pi-ai, config: {providers: {native: {api: openai-responses}}}},\n  {id: agent-default-model, config: {provider: native, model: original, note: \"first\n- one\n- two\nlast\"}}\n]\n".as_slice(),
    ] {
        let patch = Patch::parse(Some(bytes)).unwrap();
        assert_eq!(patch.providers().unwrap()["native"]["api"], "openai-responses");
        assert_eq!(patch.model_root().unwrap()["model"]["name"], "native/original");
        assert!(patch.edit_provider("owned", Some(&serde_json::json!({}))).is_err());
        assert!(patch.edit_provider("native", None).is_err());
    }
}

#[test]
fn dsh_native_bootstrap_comment_is_not_an_executable_expression() {
    let patch = b"# overrides and inserts; `!!js` expressions allowed.\n[]\n";
    assert!(
        Patch::parse(Some(patch))
            .unwrap()
            .providers()
            .unwrap()
            .as_object()
            .unwrap()
            .is_empty()
    );
}
