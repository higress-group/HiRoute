use super::*;

fn fixture(id: &str, previous: Option<&str>, opus: &str, sonnet: &str) -> Node {
    let settings: AgentModelSelectionV2 = serde_json::from_value(serde_json::json!({
        "mode":"claude_launcher", "surfaces":["claude_cli"],"fixed_models":[],
        "preset_mappings":{"opus":{"kind":"plan","plan_id":opus}, "sonnet":{"kind":"plan","plan_id":sonnet},"haiku":{"kind":"preserve_native"}}
    })).unwrap();
    Node {
        id: OperationId::parse(id).unwrap(),
        previous: previous.map(|id| OperationId::parse(id).unwrap()),
        resolved: BTreeMap::from([
            ("opus".into(), serde_json::json!(opus)),
            ("sonnet".into(), serde_json::json!(sonnet)),
            ("haiku".into(), serde_json::json!("native-haiku")),
        ]),
        spec: AgentSettingsSpecV2 {
            schema_version: AGENT_SETTINGS_SCHEMA_V2,
            context_id: "context".into(),
            model: AgentFacetIntent::Configure {
                settings: settings.clone(),
            },
            collaboration: Default::default(),
            management_skill: Default::default(),
            native_target: None,
            restore_native_model: None,
            protected_native_model_ids: vec![],
            access_token: AgentAccessTokenIntentV1::Regenerate,
        },
        settings,
    }
}

#[test]
fn previous_routing_skips_unchanged_opus_and_preserves_later_sonnet() {
    let current = fixture(
        "op_00000000000000000000000000000003",
        Some("op_00000000000000000000000000000002"),
        "plan/trial",
        "plan/new-sonnet",
    );
    let mut chain = BTreeMap::from([
        (
            "op_00000000000000000000000000000002",
            fixture(
                "op_00000000000000000000000000000002",
                Some("op_00000000000000000000000000000001"),
                "plan/trial",
                "plan/sonnet",
            ),
        ),
        (
            "op_00000000000000000000000000000001",
            fixture(
                "op_00000000000000000000000000000001",
                None,
                "plan/opus",
                "plan/sonnet",
            ),
        ),
    ]);
    let result = recover(current, &BTreeSet::from(["opus".into()]), |id| {
        chain.remove(id.as_str()).ok_or("history_missing")
    });
    let Preview::Available {
        spec,
        restored_from,
        ..
    } = result
    else {
        panic!("history should be recoverable")
    };
    let AgentFacetIntent::Configure {
        settings: AgentModelSelectionV2::ClaudeLauncher {
            preset_mappings, ..
        },
    } = spec.model
    else {
        panic!()
    };
    assert_eq!(
        preset_mappings.opus,
        AgentClaudePresetSelectionV2::Plan {
            plan_id: AgentPlanId::parse("plan/opus").unwrap()
        }
    );
    assert_eq!(
        preset_mappings.sonnet,
        AgentClaudePresetSelectionV2::Plan {
            plan_id: AgentPlanId::parse("plan/new-sonnet").unwrap()
        }
    );
    assert_eq!(
        restored_from["opus"].as_str(),
        "op_00000000000000000000000000000001"
    );
    assert_eq!(spec.collaboration, AgentFacetIntent::Keep);
    assert_eq!(spec.access_token, AgentAccessTokenIntentV1::Keep);
}

#[test]
fn previous_routing_never_infers_native_from_missing_or_cyclic_history() {
    for (current, reason) in [
        (
            fixture(
                "op_00000000000000000000000000000001",
                None,
                "plan/trial",
                "plan/sonnet",
            ),
            "no_previous_configuration",
        ),
        (
            fixture(
                "op_00000000000000000000000000000001",
                Some("op_00000000000000000000000000000001"),
                "plan/trial",
                "plan/sonnet",
            ),
            "history_cycle",
        ),
        (
            fixture(
                "op_00000000000000000000000000000001",
                Some("op_00000000000000000000000000000000"),
                "plan/trial",
                "plan/sonnet",
            ),
            "history_missing",
        ),
    ] {
        let result = recover(current, &BTreeSet::from(["opus".into()]), |_| {
            Err("history_missing")
        });
        assert!(matches!(result, Preview::Unavailable { reason: actual, .. } if actual == reason));
    }
}

#[test]
fn previous_routing_bound_is_explicit_even_when_every_mapping_is_unchanged() {
    let result = recover(
        fixture(
            "op_00000000000000000000000000000000",
            Some("op_00000000000000000000000000000001"),
            "plan/trial",
            "plan/sonnet",
        ),
        &BTreeSet::from(["opus".into()]),
        |id| {
            let n = u64::from_str_radix(id.as_str().strip_prefix("op_").unwrap(), 16).unwrap();
            Ok(fixture(
                id.as_str(),
                Some(&format!("op_{:032x}", n + 1)),
                "plan/trial",
                "plan/sonnet",
            ))
        },
    );
    assert!(
        matches!(result, Preview::Unavailable { reason, .. } if reason == "history_bound_reached")
    );
}

#[test]
fn recovery_targets_only_include_presets_currently_mapped_to_the_plan() {
    let value = fixture(
        "op_00000000000000000000000000000001",
        None,
        "plan/trial",
        "plan/separate",
    );
    assert_eq!(mapped_presets(&value.settings, "plan/trial"), vec!["opus"]);
    assert_eq!(
        mapped_presets(&value.settings, "plan/separate"),
        vec!["sonnet"]
    );
    assert!(mapped_presets(&value.settings, "plan/unmapped").is_empty());
}

#[test]
fn opencode_previous_routing_recovers_exact_additional_mapping() {
    let entry = |id: &str, previous: Option<&str>, plan: &str, protocol: &str| {
        let mut value = fixture(id, previous, "plan/opus", "plan/sonnet");
        value.settings = serde_json::from_value(serde_json::json!({
            "mode": "opencode_additional", "allowed_plan_ids": [plan],
            "plan_protocols": BTreeMap::from([(plan, protocol)])
        }))
        .unwrap();
        value.spec.model = AgentFacetIntent::Configure {
            settings: value.settings.clone(),
        };
        value.resolved = BTreeMap::from([("models".into(), serde_json::json!([plan]))]);
        value
    };
    let current = entry(
        "op_00000000000000000000000000000002",
        Some("op_00000000000000000000000000000001"),
        "plan/trial",
        "responses",
    );
    assert_eq!(mapped_presets(&current.settings, "plan/trial"), ["models"]);
    assert!(mapped_presets(&current.settings, "plan/unmapped").is_empty());
    let old = entry(
        "op_00000000000000000000000000000001",
        None,
        "plan/original",
        "messages",
    );
    let expected = old.settings.clone();
    let mut chain = BTreeMap::from([(old.id.clone(), old)]);
    let Preview::Available {
        spec,
        restored_from,
        ..
    } = recover(current, &BTreeSet::from(["models".into()]), |id| {
        chain.remove(id).ok_or("history_missing")
    })
    else {
        panic!("OpenCode must use the existing additional-model history path")
    };
    assert_eq!(
        spec.model,
        AgentFacetIntent::Configure { settings: expected }
    );
    assert_eq!(spec.collaboration, AgentFacetIntent::Keep);
    assert_eq!(spec.access_token, AgentAccessTokenIntentV1::Keep);
    assert_eq!(
        restored_from["models"].as_str(),
        "op_00000000000000000000000000000001"
    );
}
