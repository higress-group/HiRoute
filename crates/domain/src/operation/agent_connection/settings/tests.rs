use super::*;
use serde_json::json;

fn spec(model: Value, collaboration: Value) -> ChangeSpecV1 {
    ChangeSpecV1 {
        schema_version: crate::CHANGE_SPEC_SCHEMA_V1,
        command_id: AgentConnectionTransactionKindV1::Settings
            .command_id()
            .into(),
        resource_id: Some("agent-context/test".into()),
        desired_state: json!({
            "schema_version": {"major":2,"minor":0},
            "context_id":"agent-context/test",
            "model":model, "collaboration":collaboration,
        }),
    }
}
fn control(spec: &ChangeSpecV1, file_change: bool) -> AgentConnectionControlIntentV1 {
    AgentConnectionControlIntentV1::from_settings_planner(
        AgentConnectionTransactionSubjectV1::from_registered_profile(
            "agent.codex",
            "default",
            "codex.profile.v1",
        )
        .unwrap(),
        spec,
        file_change,
        &json!({"revision":1}),
    )
    .unwrap()
}
fn effects(
    control: &AgentConnectionControlIntentV1,
    roles: &[AgentConnectionEffectRoleV1],
) -> Vec<ExternalEffectIntentV1> {
    roles
        .iter()
        .map(|role| {
            ExternalEffectIntentV1::from_agent_connection_planner(
                control,
                *role,
                None,
                &json!({"content_digest":CanonicalDigest::of_bytes(b"fixture"), "context_id":"agent-context/test", "action":"install"}),
                0o644,
            )
            .unwrap()
        })
        .collect()
}
fn rebuild(
    plan: &TransactionPlanV1,
    effects: Vec<ExternalEffectIntentV1>,
) -> Result<TransactionPlanV1, OperationValidationError> {
    // The journal decoder uses this same registered seam after decoding protected durable input.
    TransactionPlanV1::from_registered_typed_planner(
        plan.spec().clone(),
        plan.control().clone(),
        None,
        vec![],
        vec![],
        effects,
    )
}

#[test]
fn claude_settings_target_is_the_native_user_configuration() {
    let spec = spec(
        json!({"intent":"keep"}),
        json!({"intent":"configure","settings":{"trigger_mode":"explicit"}}),
    );
    let subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
        "agent_claude_default",
        "claude-messages-v1",
        "builtin/claude-messages/v1",
    )
    .unwrap();
    let role = AgentConnectionEffectRoleV1::ManagedConfiguration;
    let native_target = role.target_for(&subject).unwrap();
    let settings_target = role.settings_target_for(&subject).unwrap();
    assert_eq!(native_target, settings_target);
    let control = AgentConnectionControlIntentV1::from_settings_planner(
        subject,
        &spec,
        true,
        &json!({"revision": 1}),
    )
    .unwrap();
    let intent = ExternalEffectIntentV1::from_agent_connection_planner(
        &control,
        role,
        None,
        &json!({"context_id": "agent-context/test"}),
        0o600,
    )
    .unwrap();
    assert_eq!(intent.target(), settings_target);
    assert!(
        ExternalEffectIntentV1::from_registered_adapter(
            intent.effect_id(),
            intent.kind(),
            format!("{native_target}/launch-snapshot"),
            None,
            intent.desired().clone(),
            intent.desired_mode(),
            intent.sensitive(),
        )
        .is_err()
    );
}

#[test]
fn token_action_requires_model_configure_and_exact_protected_input_slot() {
    let invalid = ChangeSpecV1 {
        desired_state: json!({
            "schema_version": {"major": 2, "minor": 0},
            "context_id": "agent-context/test",
            "model": {"intent": "keep"},
            "collaboration": {"intent": "configure", "settings": {"trigger_mode": "explicit"}},
            "access_token": {"intent": "regenerate"},
        }),
        ..spec(
            json!({"intent":"keep"}),
            json!({"intent":"configure","settings":{"trigger_mode":"explicit"}}),
        )
    };
    assert!(decode_spec(&invalid).is_err());
    let mut configured = model_spec(codex_selection());
    configured.desired_state["access_token"] =
        json!({"intent":"set","input_slot":"candidate/native/agent-token-one"});
    let settings = decode_spec(&configured).unwrap();
    assert!(matches!(
        settings.access_token,
        AgentAccessTokenIntentV1::Set { .. }
    ));
    configured.desired_state["access_token"] = json!({"intent":"set","input_slot":"../other"});
    assert!(decode_spec(&configured).is_err());
}

#[test]
fn settings_catalog_target_is_bound_to_payload_digest() {
    let spec = model_spec(codex_selection());
    let control = control(&spec, false);
    let make = |content: &str| {
        ExternalEffectIntentV1::from_agent_connection_planner(
            &control,
            AgentConnectionEffectRoleV1::ModelCatalog,
            None,
            &json!({"content_digest": CanonicalDigest::of_bytes(content.as_bytes())}),
            0o600,
        )
        .unwrap()
    };
    let first = make("first");
    let second = make("second");
    assert_ne!(first.target(), second.target());
    assert_eq!(first.target(), make("first").target());
    assert!(
        ExternalEffectIntentV1::from_registered_adapter(
            first.effect_id(),
            first.kind(),
            second.target(),
            None,
            first.desired().clone(),
            first.desired_mode(),
            first.sensitive(),
        )
        .is_err()
    );
}

#[test]
fn skill_only_seals_and_reconstructs_without_model_effects() {
    let spec = spec(
        json!({"intent":"keep"}),
        json!({"intent":"configure","settings":{"trigger_mode":"explicit"}}),
    );
    let control = control(&spec, true);
    let plan = TransactionPlanV1::from_agent_connection_planner(
        spec,
        control.clone(),
        effects(&control, &[AgentConnectionEffectRoleV1::RoutingSkill]),
    )
    .unwrap();
    let reconstructed = rebuild(&plan, plan.external().to_vec()).unwrap();
    assert_eq!(
        serde_json::to_value(&plan).unwrap(),
        serde_json::to_value(reconstructed).unwrap()
    );
    assert!(plan.agent_access_grants().is_empty());
    assert!(plan.agent_connection_projection().unwrap().is_none());
    assert!(rebuild(&plan, vec![]).is_err());
    assert!(
        rebuild(
            &plan,
            effects(
                &control,
                &[
                    AgentConnectionEffectRoleV1::RoutingSkill,
                    AgentConnectionEffectRoleV1::ManagedConfiguration,
                    AgentConnectionEffectRoleV1::GrantScopedPublication,
                ]
            )
        )
        .is_err()
    );
    let mut foreign = control.clone();
    foreign.subject.profile_id = "other".into();
    assert!(
        rebuild(
            &plan,
            effects(&foreign, &[AgentConnectionEffectRoleV1::RoutingSkill])
        )
        .is_err()
    );
    for payload in [
        json!({"context_id":"agent-context/other", "action":"install"}),
        json!({"context_id":"agent-context/test", "action":"remove"}),
    ] {
        let wrong = ExternalEffectIntentV1::from_agent_connection_planner(
            &control,
            AgentConnectionEffectRoleV1::RoutingSkill,
            None,
            &payload,
            0o644,
        )
        .unwrap();
        assert!(rebuild(&plan, vec![wrong]).is_err());
    }
}

#[test]
fn shared_skill_reference_change_has_no_file_and_cannot_add_one_on_recovery() {
    let spec = spec(
        json!({"intent":"keep"}),
        json!({"intent":"restore","restore_point_ref":"restore/context"}),
    );
    let control = control(&spec, false);
    let plan =
        TransactionPlanV1::from_agent_connection_planner(spec, control.clone(), vec![]).unwrap();
    assert!(rebuild(&plan, vec![]).is_ok());
    assert!(
        rebuild(
            &plan,
            vec![
                ExternalEffectIntentV1::from_agent_connection_planner(
                    &control,
                    AgentConnectionEffectRoleV1::RoutingSkill,
                    None,
                    &json!({"context_id":"agent-context/test", "action":"remove"}),
                    0o644,
                )
                .unwrap()
            ]
        )
        .is_err()
    );
}

#[test]
fn model_restore_requires_exact_revoke_and_both_model_effects() {
    let spec = spec(
        json!({"intent":"restore","restore_point_ref":"restore/model"}),
        json!({"intent":"keep"}),
    );
    let control = control(&spec, false);
    let external = effects(
        &control,
        &[
            AgentConnectionEffectRoleV1::GrantScopedPublication,
            AgentConnectionEffectRoleV1::ManagedConfiguration,
        ],
    );
    assert!(
        TransactionPlanV1::from_agent_connection_planner(
            spec.clone(),
            control.clone(),
            external.clone()
        )
        .is_err()
    );
    let grant = AgentAccessGrantMutationV1::revoke(
        "principal/local-owner",
        "agent-connection/agent-context/test",
        1,
    )
    .unwrap();
    let plan = TransactionPlanV1::from_agent_connection_planner_with_agent_access_grants(
        spec.clone(),
        control.clone(),
        vec![grant.clone()],
        external,
    )
    .unwrap();
    assert_eq!(
        rebuild(&plan, plan.external().to_vec())
            .unwrap()
            .agent_access_grants(),
        [grant]
    );
    assert!(rebuild(&plan, vec![plan.external()[0].clone()]).is_err());
    let wrong =
        AgentAccessGrantMutationV1::revoke("principal/local-owner", "agent-connection/other", 1)
            .unwrap();
    assert!(
        TransactionPlanV1::from_agent_connection_planner_with_agent_access_grants(
            spec,
            control,
            vec![wrong],
            plan.external().to_vec(),
        )
        .is_err()
    );
}

#[test]
fn independent_spec_rejects_noop_cross_context_unknown_schema_and_raw_dsl() {
    let noop = spec(json!({"intent":"keep"}), json!({"intent":"keep"}));
    assert!(decode_spec(&noop).is_err());
    let valid = spec(
        json!({"intent":"keep"}),
        json!({"intent":"configure","settings":{"trigger_mode":"explicit"}}),
    );
    let mut cross_context = valid.clone();
    cross_context.resource_id = Some("agent-context/other".into());
    assert!(decode_spec(&cross_context).is_err());
    let mut schema = valid.clone();
    schema.desired_state["schema_version"]["major"] = json!(1);
    assert!(decode_spec(&schema).is_err());
    let mut injected = valid;
    injected.desired_state["external_intent"] = json!({"target":"arbitrary-file"});
    assert!(decode_spec(&injected).is_err());
}

fn model_spec(settings: Value) -> ChangeSpecV1 {
    spec(
        json!({"intent":"configure", "settings":settings}),
        json!({"intent":"keep"}),
    )
}

fn codex_selection() -> Value {
    json!({
        "mode":"codex_default",
        "native_model_mode":"preserve_available",
        "fixed_models":[{"client_model_id":"Vendor/Model.V1[1m]", "candidate":{"binding_id":"binding/native"}}],
        "allowed_plan_ids":[],
        "default_selection":{"kind":"preserve_native"}
    })
}

#[test]
fn codex_fixed_selection_preserves_native_name_and_implicit_default() {
    let selection = codex_selection();
    let decoded = decode_spec(&model_spec(selection.clone())).unwrap();
    let AgentFacetIntent::Configure { settings } = decoded.model else {
        panic!("configure");
    };
    assert_eq!(serde_json::to_value(settings).unwrap(), selection);
    let mut legacy = selection;
    legacy["surfaces"] = json!(["codex_cli"]);
    assert!(decode_spec(&model_spec(legacy)).is_err());
}

#[test]
fn model_selection_rejects_empty_routes_and_uncovered_default() {
    let valid = codex_selection();
    for (field, value) in [
        ("fixed_models", json!([])),
        (
            "default_selection",
            json!({"kind":"fixed_model","client_model_id":"unknown"}),
        ),
        (
            "default_selection",
            json!({"kind":"plan","plan_id":"plan_unknown"}),
        ),
    ] {
        let mut selection = valid.clone();
        selection[field] = value;
        assert!(decode_spec(&model_spec(selection)).is_err(), "{field}");
    }
    let mut explicit = valid;
    explicit["default_selection"] =
        json!({"kind":"fixed_model","client_model_id":"Vendor/Model.V1[1m]"});
    assert!(decode_spec(&model_spec(explicit)).is_ok());
}

#[test]
fn fixed_names_reject_duplicates_and_header_control_characters() {
    for name in ["", "bad\0name", "bad\rname", "bad\nname"] {
        let mut selection = codex_selection();
        selection["fixed_models"][0]["client_model_id"] = json!(name);
        assert!(decode_spec(&model_spec(selection)).is_err());
    }
    let mut selection = codex_selection();
    let first = selection["fixed_models"][0].clone();
    selection["fixed_models"]
        .as_array_mut()
        .unwrap()
        .push(first);
    assert!(decode_spec(&model_spec(selection)).is_err());
}

#[test]
fn claude_three_presets_derive_one_scope_and_reject_extra_permissions() {
    let selection = json!({
        "mode":"claude_launcher",
        "surfaces":["claude_cli"],
        "fixed_models":[],
        "preset_mappings":{
            "opus":{"kind":"plan","plan_id":"plan_shared"},
            "sonnet":{"kind":"plan","plan_id":"plan_shared"},
            "haiku":{"kind":"plan","plan_id":"plan_shared"}
        }
    });
    let decoded = decode_spec(&model_spec(selection.clone())).unwrap();
    let AgentFacetIntent::Configure { settings } = decoded.model else {
        panic!("configure");
    };
    assert_eq!(settings.allowed_plan_ids().len(), 1);
    let mut fourth = selection.clone();
    fourth["preset_mappings"]["small_fast"] = json!({"kind":"preserve_native"});
    assert!(decode_spec(&model_spec(fourth)).is_err());
    for field in ["allowed_plan_ids", "default_selection"] {
        let mut extra = selection.clone();
        extra[field] = json!([]);
        assert!(decode_spec(&model_spec(extra)).is_err());
    }
    let mut missing = selection;
    missing["preset_mappings"]
        .as_object_mut()
        .unwrap()
        .remove("haiku");
    assert!(decode_spec(&model_spec(missing)).is_err());
}

#[test]
fn codex_plan_scope_has_no_legacy_thirty_two_item_limit() {
    let mut selection = codex_selection();
    selection["fixed_models"] = json!([]);
    selection["allowed_plan_ids"] = json!((0..33).map(|i| format!("plan_{i}")).collect::<Vec<_>>());
    selection["default_selection"] = json!({"kind":"plan","plan_id":"plan_0"});
    assert!(decode_spec(&model_spec(selection)).is_ok());
    assert!(
        decode_spec(&model_spec(json!({
            "default_plan_id":"plan_0", "allowed_plan_ids":["plan_0"]
        })))
        .is_err()
    );
}
