use super::*;
use serde_json::json;

const PROVIDER: &str =
    "hiroute-main-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const ENDPOINT: &str = "http://127.0.0.1:4321/_hiroute/qoder/v1";

fn models() -> Vec<AdditionalAgentModelV1> {
    ["plan-branch-cheap", "hiroute/0011223344556677"]
        .into_iter()
        .map(|alias| AdditionalAgentModelV1 {
            protocol: hiroute_domain::AgentIngressProtocolV1::Responses,
            alias: alias.into(),
            context_window_tokens: 32_768,
            max_output_tokens: 4096,
        })
        .collect()
}

fn provider(alias: &str) -> String {
    let model = models().into_iter().find(|m| m.alias == alias).unwrap();
    hiroute_domain::additional_model_provider_for(PROVIDER, &models(), &model)
}
fn first_provider() -> String {
    provider("plan-branch-cheap")
}

fn token(byte: u8) -> AgentAccessGrantMaterial {
    AgentAccessGrantMaterial::from_csprng_entropy([byte; 32])
}

fn install(bytes: Option<&[u8]>) -> Edit {
    configure(
        hiroute_domain::AgentKindV1::Qoder,
        bytes,
        PROVIDER,
        ENDPOINT,
        &models(),
        &token(3),
        None,
    )
    .unwrap()
}

fn value(bytes: &[u8]) -> Value {
    Document::parse(bytes).unwrap().root.value
}

#[test]
fn two_additional_routes_keep_native_default_extensions_and_foreign_provider_bytes() {
    let original = br#"{
// user's preferred native model
"model": { "name": "auto", "reasoningEffort": "high" },
"providers": { "user-models": {"apiKey":"native-secret", "unknown": [1,2]} },
"hooks":{"SessionEnd":[]},"mcpServers":{"local":{"command":"my-tool"}},
"unknown": "keep // this string"
}"#;
    let edit = install(Some(original));
    let configured = value(&edit.bytes);
    let before = value(original);
    for key in ["model", "hooks", "mcpServers", "unknown"] {
        assert_eq!(configured[key], before[key]);
    }
    assert_eq!(
        configured["providers"]["user-models"],
        before["providers"]["user-models"]
    );
    let text = std::str::from_utf8(&edit.bytes).unwrap();
    assert!(text.contains("// user's preferred native model"));
    assert!(text.contains("\"user-models\": {\"apiKey\":\"native-secret\", \"unknown\": [1,2]}"));
    for model in models() {
        let provider = &configured["providers"][&provider(&model.alias)];
        assert_eq!(provider["protocol"], "openai-responses");
        assert_eq!(provider["baseUrl"], ENDPOINT);
        assert_eq!(
            provider["apiKey"],
            std::str::from_utf8(token(3).expose()).unwrap()
        );
        assert_eq!(provider["models"].as_array().unwrap().len(), 1);
        let native = &provider["models"][0];
        assert_eq!(native["model"], model.alias);
        assert_eq!(native["contextWindow"], 32_768);
        assert_eq!(native["maxOutputTokens"], 4096);
        assert!(provider.get("routing").is_none());
    }
    assert!(configured.get("modelConfigs").is_none());
    assert_eq!(
        edit.restore
            .restore(Some(&edit.bytes))
            .unwrap()
            .unwrap()
            .as_slice(),
        original
    );
}

#[test]
fn unrelated_edits_survive_rotation_and_final_restore_without_adopting_owned_drift() {
    let original = br#"{"model":{"name":"auto"},"unrelated":1}"#;
    let first = install(Some(original));
    let edited = std::str::from_utf8(&first.bytes)
        .unwrap()
        .replace("\"unrelated\":1", "\"unrelated\":2 /* user changed this */");
    assert!(first.restore.applied(Some(edited.as_bytes())).unwrap());
    let rotated = configure(
        hiroute_domain::AgentKindV1::Qoder,
        Some(edited.as_bytes()),
        PROVIDER,
        ENDPOINT,
        &models()[..1],
        &token(4),
        Some(&first.restore),
    )
    .unwrap();
    assert_ne!(
        value(&rotated.bytes)["providers"][&first_provider()]["apiKey"],
        value(&first.bytes)["providers"][&first_provider()]["apiKey"]
    );
    let restored = rotated
        .restore
        .restore(Some(&rotated.bytes))
        .unwrap()
        .unwrap();
    assert_eq!(value(&restored)["unrelated"], 2);
    assert!(
        std::str::from_utf8(&restored)
            .unwrap()
            .contains("/* user changed this */")
    );
    assert!(
        value(&restored)["providers"]
            .get(first_provider())
            .is_none()
    );
    // A user edit within the owned node remains a conflict even if all declared routes survive.
    let mut drift = value(&rotated.bytes);
    drift["providers"][&first_provider()]["routing"] = json!({"compact":"user-selected"});
    let drift = serde_json::to_vec(&drift).unwrap();
    assert!(!rotated.restore.applied(Some(&drift)).unwrap());
    assert!(rotated.restore.restore(Some(&drift)).is_err());
    assert!(
        configure(
            hiroute_domain::AgentKindV1::Qoder,
            Some(&drift),
            PROVIDER,
            ENDPOINT,
            &models(),
            &token(5),
            Some(&rotated.restore)
        )
        .is_err()
    );
}

#[test]
fn removing_selected_native_default_requires_user_switch_but_retaining_it_is_allowed() {
    let first = install(Some(br#"{"model":{"name":"auto"}}"#));
    let mut current = value(&first.bytes);
    current["model"]["name"] = format!("{}/plan-branch-cheap", first_provider()).into();
    let current = serde_json::to_vec(&current).unwrap();
    assert_eq!(
        first
            .restore
            .validate_restoration(Some(&current))
            .unwrap_err()
            .stage,
        "default in use"
    );
    assert_eq!(
        validate_configuration(
            hiroute_domain::AgentKindV1::Qoder,
            Some(&current),
            PROVIDER,
            ENDPOINT,
            &models()[1..],
            Some(&first.restore)
        )
        .unwrap_err()
        .stage,
        "default in use"
    );
    let adjusted = configure(
        hiroute_domain::AgentKindV1::Qoder,
        Some(&current),
        PROVIDER,
        ENDPOINT,
        &models()[..1],
        &token(4),
        Some(&first.restore),
    )
    .unwrap();
    assert_eq!(
        value(&adjusted.bytes)["model"]["name"],
        format!("{}/plan-branch-cheap", first_provider())
    );
    let mut switched = value(&adjusted.bytes);
    switched["model"]["name"] = "user-provider/user-model".into();
    let restored = adjusted
        .restore
        .restore(Some(&serde_json::to_vec(&switched).unwrap()))
        .unwrap()
        .unwrap();
    assert_eq!(
        value(&restored)["model"]["name"],
        "user-provider/user-model"
    );
}

#[test]
fn existing_provider_is_never_claimed_by_name_and_missing_original_is_removed_exactly() {
    let first = install(None);
    assert!(
        configure(
            hiroute_domain::AgentKindV1::Qoder,
            Some(&first.bytes),
            PROVIDER,
            ENDPOINT,
            &models(),
            &token(3),
            None
        )
        .is_err()
    );
    assert!(first.restore.restore(Some(&first.bytes)).unwrap().is_none());
    assert!(first.restore.restore(None).unwrap().is_none());
    let protected = first.restore.encode().unwrap();
    let recovered = Restore::decode(&protected).unwrap();
    assert!(recovered.applied(Some(&first.bytes)).unwrap());
    // Root-level credentials/config cannot be smuggled into the protected ownership baseline.
    let mut invalid = recovered;
    invalid.original_exists = true;
    invalid.original = first.bytes.to_vec();
    assert!(Restore::decode(&invalid.encode().unwrap()).is_err());
}

#[test]
fn restore_only_removes_owned_member_at_every_position_preserving_other_jsonc_bytes() {
    for original in [
        br#"{"providers":{},"note":"keep"}"#.as_slice(),
        br#"{/* leading */"providers":{"a":{"v":1},"z":{"v":2}},"note":"keep"}"#,
    ] {
        let first = install(Some(original));
        let mut current = value(&first.bytes);
        current["unrelated"] = true.into();
        let rendered = serde_json::to_vec_pretty(&current).unwrap();
        let restored = first.restore.restore(Some(&rendered)).unwrap().unwrap();
        let mut expected = current;
        for model in models() {
            expected["providers"]
                .as_object_mut()
                .unwrap()
                .remove(&provider(&model.alias));
        }
        assert_eq!(value(&restored), expected);
    }
    // A changed file must take the conditional removal path, preserving the complete foreign
    // byte sequence even when the user reordered the owned provider among other members.
    let first = configure(
        AgentKindV1::Qoder,
        Some(br#"{"model":{"name":"auto"},"providers":{}}"#),
        PROVIDER,
        ENDPOINT,
        &models()[..1],
        &token(3),
        None,
    )
    .unwrap();
    let provider =
        serde_json::to_string(&value(&first.bytes)["providers"][&first_provider()]).unwrap();
    let owned = format!("\"{}\" : {provider}", first_provider());
    for (members, expected) in [
        (
            "  $OWNED /* own */ ,\n \"a\" : 1 ,\t\"z\":2 ",
            "   /* own */ \n \"a\" : 1 ,\t\"z\":2 ",
        ),
        (
            " \"a\" : 1 , /* middle */ $OWNED ,\t\"z\":2 ",
            " \"a\" : 1 , /* middle */  \t\"z\":2 ",
        ),
        (
            " \"a\" : 1 ,\t\"z\":2 , /* last */ $OWNED /* end */ ",
            " \"a\" : 1 ,\t\"z\":2  /* last */  /* end */ ",
        ),
    ] {
        let wrap = |members: &str| {
            format!("{{\r\n  \"providers\" : {{{members}}}, // user\r\n \"outside\":true\r\n}}")
        };
        let current = wrap(&members.replace("$OWNED", &owned));
        assert_ne!(
            CanonicalDigest::of_bytes(current.as_bytes()),
            first.restore.rendered_digest
        );
        let restored = first
            .restore
            .restore(Some(current.as_bytes()))
            .unwrap()
            .unwrap();
        assert_eq!(restored.as_slice(), wrap(expected).as_bytes());
    }
    // The parser also accepts comments after the last value before its comma.
    let source = br#"{"a":1/*a*/,"b":2/*b*/,"c":3/*c*/}"#;
    for key in ["a", "b", "c"] {
        let doc = Document::parse(source).unwrap();
        let edited = doc.edit(&doc.root, key, None).unwrap();
        assert!(value(&edited).get(key).is_none());
        assert!(std::str::from_utf8(&edited).unwrap().contains("/*b*/"));
    }
}

#[test]
fn ambiguous_or_malformed_documents_fail_without_normalizing_foreign_settings() {
    for bytes in [
        br#"{"providers":{},"providers":{}}"#.as_slice(),
        br#"{"foreign":{"same":1,"same":2}}"#,
        br#"{"foreign":{"same":1,"\u0073ame":2}}"#,
        br#"{"foreign":[1,]}"#,
        br#"{"providers":{ /* unfinished}"#,
        br#"{"providers":[]}"#,
        br#"{"model":{"name":"auto"}} trailing"#,
    ] {
        assert!(
            configure(
                hiroute_domain::AgentKindV1::Qoder,
                Some(bytes),
                PROVIDER,
                ENDPOINT,
                &models(),
                &token(3),
                None
            )
            .is_err()
        );
    }
    for separator in ["\r", "\u{2028}", "\u{2029}"] {
        let ambiguous = format!(
            "{{\"model\":{{\"name\":\"auto\"}} // c{separator},\"providers\":{{\"user\":{{}}}}\n}}"
        );
        assert!(
            configure(
                hiroute_domain::AgentKindV1::Qoder,
                Some(ambiguous.as_bytes()),
                PROVIDER,
                ENDPOINT,
                &models(),
                &token(3),
                None
            )
            .is_err()
        );
    }
    let unicode = "\u{feff}{\"标签\":\"值 \\\" // /*\", /* 注释 */ \"providers\":{}}";
    let edit = install(Some(unicode.as_bytes()));
    assert_eq!(
        edit.restore
            .restore(Some(&edit.bytes))
            .unwrap()
            .unwrap()
            .as_slice(),
        unicode.as_bytes()
    );
    let mut bounded = b"{}".to_vec();
    bounded.resize(LIMIT, b' ');
    assert!(Document::parse(&bounded).is_ok());
    bounded.push(b' ');
    assert!(Document::parse(&bounded).is_err());
}

#[test]
fn declarations_reject_unknown_budget_duplicate_alias_and_unowned_namespace() {
    let mut selected = models();
    selected.push(selected[0].clone());
    assert!(
        validate_declaration(
            hiroute_domain::AgentKindV1::Qoder,
            PROVIDER,
            ENDPOINT,
            &selected
        )
        .is_err()
    );
    assert!(
        validate_declaration(
            hiroute_domain::AgentKindV1::Qoder,
            "hiroute-worker-123",
            ENDPOINT,
            &models()
        )
        .is_err()
    );
    assert!(
        validate_declaration(
            hiroute_domain::AgentKindV1::Qoder,
            PROVIDER,
            "https://external.example/v1",
            &models()
        )
        .is_err()
    );
    selected = models();
    selected[0].max_output_tokens = 32_000; // the exact 32K/32K zero-threshold regression
    assert!(
        validate_declaration(
            hiroute_domain::AgentKindV1::Qoder,
            PROVIDER,
            ENDPOINT,
            &selected
        )
        .is_err()
    );
    assert!(
        validate_declaration(hiroute_domain::AgentKindV1::Qoder, PROVIDER, ENDPOINT, &[]).is_err()
    );
}

#[test]
fn per_plan_protocols_keep_provider_identity_and_restore_both_without_foreign_changes() {
    for kind in [AgentKindV1::Qoder, AgentKindV1::Pi] {
        let endpoint = if kind == AgentKindV1::Pi {
            "http://127.0.0.1:4321/v1"
        } else {
            ENDPOINT
        };
        let original = br#"{"providers":{"foreign":{"apiKey":"keep"}}}"#;
        let mut models = models();
        models[1].protocol = hiroute_domain::AgentIngressProtocolV1::Messages;
        let first = configure(
            kind,
            Some(original),
            PROVIDER,
            endpoint,
            &models,
            &token(3),
            None,
        )
        .unwrap();
        let document = value(&first.bytes);
        assert_eq!(document["providers"].as_object().unwrap().len(), 3);
        let key = if kind == AgentKindV1::Pi {
            "api"
        } else {
            "protocol"
        };
        assert_eq!(
            document["providers"][provider(&models[0].alias)][key],
            "openai-responses"
        );
        assert_eq!(
            document["providers"][provider(&models[1].alias)][key],
            if kind == AgentKindV1::Pi {
                "anthropic-messages"
            } else {
                "anthropic"
            }
        );
        if kind == AgentKindV1::Pi {
            // Native SDK request paths must resolve to the public Gateway endpoints.
            let responses = document["providers"][provider(&models[0].alias)]["baseUrl"]
                .as_str()
                .unwrap();
            let messages = document["providers"][provider(&models[1].alias)]["baseUrl"]
                .as_str()
                .unwrap();
            assert_eq!(
                format!("{responses}/responses"),
                "http://127.0.0.1:4321/v1/responses"
            );
            assert_eq!(
                format!("{messages}/v1/messages"),
                "http://127.0.0.1:4321/v1/messages"
            );
        }
        let untouched = document["providers"][provider(&models[1].alias)].clone();
        models[0].protocol = hiroute_domain::AgentIngressProtocolV1::Messages;
        let next = configure(
            kind,
            Some(&first.bytes),
            PROVIDER,
            endpoint,
            &models,
            &token(3),
            Some(&first.restore),
        )
        .unwrap();
        assert_eq!(
            value(&next.bytes)["providers"][provider(&models[1].alias)],
            untouched
        );
        let recovered = Restore::decode(&next.restore.encode().unwrap()).unwrap();
        assert_eq!(
            recovered
                .restore(Some(&next.bytes))
                .unwrap()
                .unwrap()
                .as_slice(),
            original
        );
    }
}

#[test]
fn historical_single_provider_record_restores_and_upgrades_without_claiming_foreign_nodes() {
    let original = br#"{"providers":{"foreign":{"keep":true}}}"#;
    let old_provider = json!({"type":"openai-compatible","protocol":"openai-responses","apiKey":"historical-owned-token","models":[{"model":"plan-branch-cheap"}]});
    let mut old_file = value(original);
    old_file["providers"][PROVIDER] = old_provider.clone();
    let old_bytes = serde_json::to_vec(&old_file).unwrap();
    // Historical V1 wire layout, independent of the current Restore serializer.
    let header = serde_json::to_vec(&json!({"schema":"hiroute.qoder-native-restore/v1","provider_id":PROVIDER,"provider_digest":digest(&old_provider).unwrap(),"rendered_digest":CanonicalDigest::of_bytes(&old_bytes),"original_exists":true})).unwrap();
    let mut envelope = (header.len() as u32).to_le_bytes().to_vec();
    envelope.extend(header);
    envelope.extend(original);
    let old = Restore::decode(&envelope).unwrap();
    assert_eq!(
        old.restore(Some(&old_bytes)).unwrap().unwrap().as_slice(),
        original
    );
    let upgraded = configure(
        AgentKindV1::Qoder,
        Some(&old_bytes),
        PROVIDER,
        ENDPOINT,
        &models(),
        &token(7),
        Some(&old),
    )
    .unwrap();
    assert!(value(&upgraded.bytes)["providers"].get(PROVIDER).is_none());
    assert_eq!(
        value(&upgraded.bytes)["providers"]
            .as_object()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        upgraded
            .restore
            .restore(Some(&upgraded.bytes))
            .unwrap()
            .unwrap()
            .as_slice(),
        original
    );
    let mut selected = old_file;
    selected["model"] = json!({"name":format!("{PROVIDER}/plan-branch-cheap")});
    assert!(
        configure(
            AgentKindV1::Qoder,
            Some(&serde_json::to_vec(&selected).unwrap()),
            PROVIDER,
            ENDPOINT,
            &models(),
            &token(7),
            Some(&old)
        )
        .is_err()
    );
}
