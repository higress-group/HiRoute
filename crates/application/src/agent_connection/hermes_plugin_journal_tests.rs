use super::*;
use hiroute_domain::{AgentConnectionTransactionSubjectV1, CHANGE_SPEC_SCHEMA_V1, ChangeSpecV1};
use serde_json::json;

fn fixture(
    agent: &str,
    role: AgentConnectionEffectRoleV1,
    content: &'static str,
) -> (
    AgentConnectionControlIntentV1,
    String,
    NativePluginFileFacts,
) {
    let root = std::env::temp_dir().join("hiroute-plugin-journal-test");
    let root = root.to_str().unwrap();
    let digest = CanonicalDigest::of_bytes(root.as_bytes());
    let suffix = digest.as_str().strip_prefix("sha256:").unwrap();
    let (context, profile, integration, selection) = if agent == "hermes" {
        (
            format!("agent-context/hermes/sha256:{suffix}"),
            format!("hermes-main-{suffix}"),
            "builtin/hermes-main/v1".to_owned(),
            json!({"mode":"hermes_default","plan_id":"plan/plugin-journal","protocol":"responses"}),
        )
    } else {
        (
            format!("agent-context/{agent}/plugin-journal"),
            format!("{agent}-responses-v1"),
            format!("builtin/{agent}-responses/v1"),
            json!({"mode":format!("{agent}_additional"),"allowed_plan_ids":["plan/plugin-journal"]}),
        )
    };
    let mut desired = json!({"schema_version":{"major":2,"minor":0},
        "context_id":context,"model":{"intent":"configure","settings":selection}});
    if agent == "hermes" {
        desired["native_target"] = json!({"kind":"hermes","config_root":root});
    }
    let spec = ChangeSpecV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        command_id: "agents.settings.apply".into(),
        resource_id: Some(context.clone()),
        desired_state: desired,
    };
    let control = AgentConnectionControlIntentV1::from_settings_planner(
        AgentConnectionTransactionSubjectV1::from_registered_profile(
            format!("agent_{agent}_default"),
            profile,
            integration,
        )
        .unwrap(),
        &spec,
        false,
        &json!({"accept_digest":CanonicalDigest::of_bytes(b"native-plugin-accept")}),
    )
    .unwrap();
    (
        control,
        context,
        NativePluginFileFacts {
            role,
            before_fingerprint: None,
            expected_content: CanonicalDigest::of_bytes(b""),
            content,
        },
    )
}

#[test]
fn every_bundled_native_plugin_fits_the_current_constructor_and_decoder() {
    for (agent, role, content) in [
        (
            "hermes",
            AgentConnectionEffectRoleV1::HermesPluginModule,
            include_str!("../../../../assets/agents/hermes/__init__.py"),
        ),
        (
            "hermes",
            AgentConnectionEffectRoleV1::HermesPluginManifest,
            include_str!("../../../../assets/agents/hermes/plugin.yaml"),
        ),
        (
            "pi",
            AgentConnectionEffectRoleV1::NativeRoutingAdapter,
            include_str!("../../../../assets/agents/pi/hiroute-routing.mjs"),
        ),
        (
            "dsh",
            AgentConnectionEffectRoleV1::NativeRoutingAdapter,
            include_str!("../../../../assets/agents/dsh/hiroute-routing.mjs"),
        ),
        (
            "opencode",
            AgentConnectionEffectRoleV1::NativeRoutingAdapter,
            include_str!("../../../../assets/agents/opencode/hiroute-routing.mjs"),
        ),
    ] {
        let (control, context, facts) = fixture(agent, role, content);
        let intent = settings_native_plugin_file_intent(
            &control,
            &context,
            &facts,
            &NativePluginFileAction::Configure {
                previous_operation: None,
                content: facts.content.into(),
            },
        )
        .unwrap_or_else(|error| panic!("{agent} {role:?}: {} bytes: {error:?}", content.len()));
        let decoded = decode_settings_native_plugin_file(&intent).unwrap();
        let NativePluginFileAction::Configure {
            content: actual, ..
        } = decoded.change
        else {
            panic!("bundled resource must remain a Configure action");
        };
        assert_eq!(actual, content);
        assert_eq!(decoded.context_id, context);
        assert_eq!(decoded.expected_content, facts.expected_content);
        assert_eq!(intent.desired_mode(), 0o644);
        assert!(!intent.sensitive());
    }
}

#[test]
fn native_plugin_content_accepts_old_sizes_and_the_32_kib_byte_boundary() {
    let (control, context, facts) = fixture(
        "hermes",
        AgentConnectionEffectRoleV1::HermesPluginModule,
        "",
    );
    for content in [
        "x".into(),
        "x".repeat(16 * 1024),
        "x".repeat(32 * 1024),
        "é".repeat(16 * 1024),
    ] {
        let intent = settings_native_plugin_file_intent(
            &control,
            &context,
            &facts,
            &NativePluginFileAction::Configure {
                previous_operation: None,
                content: content.clone(),
            },
        )
        .unwrap();
        let decoded = decode_settings_native_plugin_file(&intent).unwrap();
        assert!(
            matches!(decoded.change, NativePluginFileAction::Configure { content: actual, .. } if actual == content)
        );
    }
}

#[test]
fn native_plugin_constructor_and_decoder_reject_empty_nul_and_over_32_kib() {
    let (control, context, facts) = fixture(
        "hermes",
        AgentConnectionEffectRoleV1::HermesPluginModule,
        "",
    );
    for (case, content, domain_rejects) in [
        ("empty", String::new(), false),
        ("NUL", "native\0plugin".into(), true),
        ("over 32 KiB", "x".repeat(32 * 1024 + 1), false),
    ] {
        let change = NativePluginFileAction::Configure {
            previous_operation: None,
            content,
        };
        assert!(
            settings_native_plugin_file_intent(&control, &context, &facts, &change).is_err(),
            "{case}: plugin constructor must reject invalid content"
        );
        // Domain rejects NUL before a carrier exists. Empty and 32 KiB+1 values
        // remain valid carriers and must independently fail plugin decoding.
        let carrier = ExternalEffectIntentV1::from_agent_connection_planner(
            &control,
            facts.role,
            None,
            &NativePluginFilePayload {
                schema: "hiroute.settings-hermes-plugin-file/v1".into(),
                context_id: context.clone(),
                expected_content: facts.expected_content.clone(),
                change,
            },
            0o644,
        );
        if domain_rejects {
            assert!(
                matches!(
                    &carrier,
                    Err(OperationValidationError::UnregisteredEffectPlan)
                ),
                "{case}: Domain rejects embedded NUL in every registered payload before plugin decoding"
            );
        } else {
            let intent = carrier.unwrap_or_else(|error| {
                panic!("{case}: empty and 32 KiB+1 content fit the Domain carrier: {error:?}")
            });
            assert!(
                decode_settings_native_plugin_file(&intent).is_err(),
                "{case}: a valid registered carrier must still fail the plugin-specific bound"
            );
        }
    }
}
