use super::additional_native::{Restore, configure};
use super::dsh_config::Patch;
use hiroute_domain::{
    AdditionalAgentModelV1, AgentAccessGrantMaterial, AgentIngressProtocolV1, AgentKindV1,
};
const OWNER: &str = "hiroute-main-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

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
