use super::super::{CodexConfigurationScope, sample_codex_hiroute_only_catalog_plan};
use super::*;
use hiroute_domain::{CanonicalDigest, GatewayCriticalFactV1, GatewayFidelityV1, UpstreamProtocol};

fn plan() -> CompiledAgentPlanV1 {
    let fixture: Value = serde_json::from_slice(include_bytes!(
        "../../../../e2e/product/golden/routing/compiled-publication.v2.json"
    ))
    .unwrap();
    serde_json::from_value::<CompiledAgentPlanV1>(fixture["plans"][0].clone())
        .unwrap()
        .into_current()
        .unwrap()
}

fn original() -> CodexCatalogSelection {
    let value: Value =
        serde_json::from_slice(include_bytes!("codex_bundled_catalog.json")).unwrap();
    CodexCatalogSelection::parse(value).unwrap()
}

fn policy() -> CodexDefaultPolicy<'static> {
    CodexDefaultPolicy {
        explicit_model: None,
        uses_codex_backend: true,
        allow_provider_model_fallback: false,
    }
}

#[test]
fn hiroute_only_catalog_uses_published_plan_without_native_cache() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.toml");
    let scope = CodexConfigurationScope::user_file(config.clone());
    let plan = plan();
    let first = sample_codex_hiroute_only_catalog_plan(
        &scope,
        std::slice::from_ref(&plan),
        plan.model_alias().as_str(),
    )
    .unwrap();
    let second = sample_codex_hiroute_only_catalog_plan(
        &scope,
        std::slice::from_ref(&plan),
        plan.model_alias().as_str(),
    )
    .unwrap();
    assert_eq!(
        first.producer.metadata_source,
        CodexCatalogMetadataSourceV1::HirouteGenerated
    );
    assert_eq!(first.producer.path, config);
    assert_eq!(first.content_digest, second.content_digest);
    assert_eq!(
        first.producer.dependency_digest,
        second.producer.dependency_digest
    );
    assert_eq!(
        first.selection.original()["models"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        first.selection.original()["models"][0]["slug"],
        plan.model_alias().as_str()
    );
    assert!(!config.exists());
    assert!(matches!(
        sample_codex_hiroute_only_catalog_plan(&scope, &[plan], "different-default"),
        Err(CodexCatalogError::MissingDefault)
    ));
}

#[test]
fn private_worker_catalog_uses_the_same_plan_entry_as_the_user_target_merge() {
    let plan = plan();
    let private: Value =
        serde_json::from_slice(&codex_private_worker_catalog(&plan).unwrap()).unwrap();
    let merged = original()
        .append_plans(
            std::slice::from_ref(&plan),
            policy(),
            CodexCatalogMetadataSourceV1::UserConfigured,
            None,
        )
        .unwrap();
    assert_eq!(private["models"].as_array().unwrap().len(), 1);
    assert_eq!(private["models"][0]["slug"], plan.model_alias().as_str());
    assert_eq!(private["models"][0]["supports_parallel_tool_calls"], false);
    let mut private_entry = private["models"][0].clone();
    let mut merged_entry = merged["models"].as_array().unwrap().last().unwrap().clone();
    private_entry.as_object_mut().unwrap().remove("priority");
    merged_entry.as_object_mut().unwrap().remove("priority");
    assert_eq!(private_entry, merged_entry);
    CodexCatalogSelection::for_current_adapter(private).unwrap();
}

fn reseal(mut plan: CompiledAgentPlanV1) -> CompiledAgentPlanV1 {
    let body = std::sync::Arc::make_mut(&mut plan.body);
    for candidate in body
        .materialized
        .attempt_owned
        .groups
        .iter_mut()
        .flat_map(|group| &mut group.candidates)
    {
        candidate.protocol_profile_digest =
            CanonicalDigest::of(&candidate.protocol_profiles).unwrap();
    }
    body.materialized_route_digest = body.materialized.route_digest().unwrap();
    CompiledAgentPlanV1::seal_current(plan.body.as_ref().clone()).unwrap()
}

#[test]
fn exact_original_entries_remain_unchanged_when_plan_is_appended() {
    let source = original();
    let plan = plan();
    let merged = source
        .append_plans(
            std::slice::from_ref(&plan),
            policy(),
            CodexCatalogMetadataSourceV1::UserConfigured,
            None,
        )
        .unwrap();
    let before = source.original()["models"].as_array().unwrap();
    let after = merged["models"].as_array().unwrap();
    assert_eq!(before.len(), 8);
    assert_eq!(&after[..before.len()], before);
    assert_eq!(after.len(), 9);
    let entry = &after[8];
    assert_eq!(entry["slug"], plan.model_alias().as_str());
    assert_eq!(entry["shell_type"], "shell_command");
    assert_eq!(
        entry["model_messages"]["instructions_template"],
        generic_prompt()
    );
    assert_eq!(
        entry["truncation_policy"],
        json!({"mode":"bytes","limit":10000})
    );
    assert_eq!(entry["context_window"], entry["max_context_window"]);
    assert!(entry["context_window"].as_i64().unwrap() > 0);
    assert!(
        entry["priority"].as_i64().unwrap()
            > before
                .iter()
                .map(|entry| entry["priority"].as_i64().unwrap())
                .max()
                .unwrap()
    );
    for key in [
        "used_fallback_model_metadata",
        "base_instructions",
        "max_output",
        "available_in_plans",
        "minimal_client_version",
    ] {
        assert!(entry.get(key).is_none(), "must not fabricate {key}");
    }
    let updated = CodexCatalogSelection::parse(merged).unwrap();
    source
        .require_preserved_default(&updated, policy())
        .unwrap();
}

#[test]
fn managed_catalog_exposes_only_authorized_names_from_full_metadata() {
    let source = original();
    let before = source.original()["models"].as_array().unwrap().clone();
    assert!(
        before.len() > 1,
        "fixture must retain the full client metadata catalog"
    );
    let retained_name = before[1]["slug"].as_str().unwrap().to_owned();
    let retained = BTreeSet::from([retained_name.clone()]);
    let plan = plan();
    let merged = source
        .append_plans(
            std::slice::from_ref(&plan),
            CodexDefaultPolicy {
                explicit_model: Some(plan.model_alias().as_str()),
                uses_codex_backend: true,
                allow_provider_model_fallback: false,
            },
            CodexCatalogMetadataSourceV1::TargetCache,
            Some(&retained),
        )
        .unwrap();
    let models = merged["models"].as_array().unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0]["slug"], retained_name);
    assert_eq!(models[1]["slug"], plan.model_alias().as_str());
    assert_eq!(
        source.original()["models"].as_array().unwrap(),
        &before,
        "source metadata remains untouched"
    );

    let fixed_only = source
        .append_plans(
            &[],
            CodexDefaultPolicy {
                explicit_model: Some(&retained_name),
                uses_codex_backend: true,
                allow_provider_model_fallback: false,
            },
            CodexCatalogMetadataSourceV1::UserConfigured,
            Some(&retained),
        )
        .unwrap();
    assert_eq!(fixed_only["models"].as_array().unwrap().len(), 1);
    assert_eq!(fixed_only["models"][0], before[1]);
}

#[test]
fn target_cache_missing_parallel_flag_defaults_true_without_mutating_source() {
    let mut raw = original().original().clone();
    raw["models"][0]
        .as_object_mut()
        .unwrap()
        .remove("supports_parallel_tool_calls");
    raw["models"][0]["future_native_field"] = json!({"preserve": [null, 1]});
    raw["models"][1]["supports_parallel_tool_calls"] = json!(false);
    let source = CodexCatalogSelection::parse(raw.clone()).unwrap();
    source.validate_schema().unwrap();

    let merged = source
        .append_plans(
            &[plan()],
            policy(),
            CodexCatalogMetadataSourceV1::TargetCache,
            None,
        )
        .unwrap();
    let before = raw["models"].as_array().unwrap();
    let after = merged["models"].as_array().unwrap();
    assert_eq!(source.original(), &raw);
    assert_eq!(after.len(), before.len() + 1);
    for (before, after) in before.iter().zip(after) {
        let mut expected = before.clone();
        if expected.get("supports_parallel_tool_calls").is_none() {
            expected["supports_parallel_tool_calls"] = json!(true);
        }
        assert_eq!(after, &expected);
    }
    assert_eq!(after[0]["supports_parallel_tool_calls"], true);
    assert_eq!(
        after[0]["future_native_field"],
        json!({"preserve": [null, 1]})
    );
    assert_eq!(after[1]["supports_parallel_tool_calls"], false);
    assert_eq!(after.last().unwrap()["supports_parallel_tool_calls"], false);
}

#[test]
fn user_configured_catalog_does_not_infer_missing_parallel_flag() {
    let mut raw = original().original().clone();
    raw["models"][0]
        .as_object_mut()
        .unwrap()
        .remove("supports_parallel_tool_calls");
    let source = CodexCatalogSelection::parse(raw.clone()).unwrap();
    let merged = source
        .append_plans(
            &[plan()],
            policy(),
            CodexCatalogMetadataSourceV1::UserConfigured,
            None,
        )
        .unwrap();
    let before = raw["models"].as_array().unwrap();
    let after = merged["models"].as_array().unwrap();
    assert_eq!(&after[..before.len()], before);
    assert!(after[0].get("supports_parallel_tool_calls").is_none());
    assert_eq!(after.last().unwrap()["supports_parallel_tool_calls"], false);
}

#[test]
fn plan_catalog_does_not_offer_effort_override_or_mutate_mixed_candidates() {
    let plan = plan();
    let before = serde_json::to_value(&plan).unwrap();
    let choices: Vec<_> = plan
        .body
        .materialized
        .attempt_owned
        .groups
        .iter()
        .flat_map(|group| &group.candidates)
        .map(|candidate| &candidate.exact_reasoning)
        .collect();
    assert_ne!(choices[0], choices[1]);
    let entry = plan_entry(&plan, 99).unwrap();
    assert!(entry["default_reasoning_level"].is_null());
    assert_eq!(entry["supported_reasoning_levels"], json!([]));
    assert!(
        entry["description"]
            .as_str()
            .unwrap()
            .contains("推理强度由计划配置决定")
    );
    assert_eq!(serde_json::to_value(&plan).unwrap(), before);
}

#[test]
fn missing_function_tools_rejects_plan_without_pruning_candidates() {
    let mut value = plan();
    let candidate = &mut std::sync::Arc::make_mut(&mut value.body)
        .materialized
        .attempt_owned
        .groups[0]
        .candidates[1];
    for profile in &mut candidate.protocol_profiles {
        profile.capability.request.function_tools = GatewayFidelityV1::Unsupported;
    }
    let value = reseal(value);
    assert_eq!(
        original().append_plans(
            &[value],
            policy(),
            CodexCatalogMetadataSourceV1::UserConfigured,
            None,
        ),
        Err(CodexCatalogError::CapabilityUnproven)
    );
}

#[test]
fn codex_catalog_rejects_missing_instruction_roles_even_when_tools_and_stream_are_exact() {
    let mut value = plan();
    let candidate = &mut std::sync::Arc::make_mut(&mut value.body)
        .materialized
        .attempt_owned
        .groups[0]
        .candidates[0];
    let binding_id = candidate.binding_id.clone();
    for profile in &mut candidate.protocol_profiles {
        if profile.ingress_protocol == UpstreamProtocol::Responses {
            profile.capability.request.mid_conversation_instructions =
                GatewayFidelityV1::Unsupported;
        }
    }
    let value = reseal(value);
    assert_eq!(
        codex_plan_capability_preview(&value),
        CodexClientCapabilityPreviewV1::Unavailable {
            issues: vec![CodexCapabilityIssueV1 {
                kind: CodexCapabilityIssueKindV1::InstructionRoles,
                binding_id: Some(binding_id),
            }],
        }
    );
    assert_eq!(
        original().append_plans(
            &[value],
            policy(),
            CodexCatalogMetadataSourceV1::UserConfigured,
            None,
        ),
        Err(CodexCatalogError::CapabilityUnproven)
    );
}

#[test]
fn messages_candidate_cannot_enter_codex_catalog_beside_a_native_responses_candidate() {
    let mut value = plan();
    let candidate = &mut std::sync::Arc::make_mut(&mut value.body)
        .materialized
        .attempt_owned
        .groups[0]
        .candidates[0];
    let binding_id = candidate.binding_id.clone();
    candidate.endpoint = candidate.endpoint.replace("/v1/responses", "/v1/messages");
    candidate.operational_target = serde_json::from_value(json!({
        "kind": "registered_https", "uri": candidate.endpoint
    }))
    .unwrap();
    candidate.operational_target_digest =
        CanonicalDigest::of(&candidate.operational_target).unwrap();
    candidate.upstream_protocol = UpstreamProtocol::Messages;
    for profile in &mut candidate.protocol_profiles {
        profile.capability.upstream_protocol = UpstreamProtocol::Messages;
        profile.capability.request.mid_conversation_instructions = GatewayFidelityV1::Unsupported;
        profile.connector.upstream_protocol = UpstreamProtocol::Messages;
        profile.connector.request_path = "/v1/messages".into();
        for reasoning in &mut profile.capability.reasoning_profiles {
            let mut render = serde_json::to_value(&reasoning.render).unwrap();
            render["protocol"] = json!("messages");
            reasoning.render = serde_json::from_value(render).unwrap();
        }
    }
    let value = reseal(value);
    assert_eq!(
        value.body.materialized.attempt_owned.groups[0]
            .candidates
            .len(),
        2
    );
    assert_eq!(
        codex_plan_capability_preview(&value),
        CodexClientCapabilityPreviewV1::Unavailable {
            issues: vec![CodexCapabilityIssueV1 {
                kind: CodexCapabilityIssueKindV1::InstructionRoles,
                binding_id: Some(binding_id),
            }],
        }
    );
    assert_eq!(
        original().append_plans(
            &[value],
            policy(),
            CodexCatalogMetadataSourceV1::UserConfigured,
            None,
        ),
        Err(CodexCatalogError::CapabilityUnproven)
    );
}

#[test]
fn messages_only_candidate_does_not_claim_codex_responses_capability() {
    let mut value = plan();
    let candidate = &mut std::sync::Arc::make_mut(&mut value.body)
        .materialized
        .attempt_owned
        .groups[0]
        .candidates[0];
    candidate
        .protocol_profiles
        .retain(|profile| profile.ingress_protocol != UpstreamProtocol::Responses);
    let binding_id = candidate.binding_id.clone();
    let value = reseal(value);
    assert_eq!(
        codex_plan_capability_preview(&value),
        CodexClientCapabilityPreviewV1::Unavailable {
            issues: vec![CodexCapabilityIssueV1 {
                kind: CodexCapabilityIssueKindV1::ResponsesProtocol,
                binding_id: Some(binding_id),
            }],
        }
    );
    let plan_id = value.agent_plan_id().clone();
    let mut aliases = hiroute_domain::AliasRegistryV1::default();
    aliases
        .active
        .insert(plan_id.clone(), value.model_alias().clone());
    let publication = hiroute_domain::GatewayPublicationV1::seal(
        hiroute_domain::WorkspaceId::default(),
        "workspace/personal/default/gateway",
        1,
        hiroute_domain::GatewayPublicationRevision::new(1).unwrap(),
        hiroute_domain::DEFAULT_CATALOG_RENDERER_REVISION,
        aliases,
        vec![value],
        vec![],
    )
    .unwrap();
    assert_eq!(
        hiroute_domain::AgentModelGrantV2::from_plan_ids(
            hiroute_domain::AgentIngressProtocolV1::Responses,
            [plan_id].into(),
            &publication,
        )
        .unwrap_err(),
        hiroute_domain::AgentConnectionError::PlanNotRoutable
    );
}

#[test]
fn unknown_context_rejects_plan_instead_of_copying_fallback_limits() {
    let mut value = plan();
    for profile in &mut std::sync::Arc::make_mut(&mut value.body)
        .materialized
        .attempt_owned
        .groups[0]
        .candidates[0]
        .protocol_profiles
    {
        profile.capability.context.max_input_tokens = GatewayCriticalFactV1::Unknown;
    }
    let value = reseal(value);
    let binding_id = value.body.materialized.attempt_owned.groups[0].candidates[0]
        .binding_id
        .clone();
    assert_eq!(
        codex_plan_capability_preview(&value),
        CodexClientCapabilityPreviewV1::Unavailable {
            issues: vec![CodexCapabilityIssueV1 {
                kind: CodexCapabilityIssueKindV1::ContextInput,
                binding_id: Some(binding_id),
            }],
        }
    );
    assert_eq!(
        original().append_plans(
            &[value],
            policy(),
            CodexCatalogMetadataSourceV1::UserConfigured,
            None,
        ),
        Err(CodexCatalogError::CapabilityUnproven)
    );
}

#[test]
fn context_and_modalities_use_all_candidates_common_capabilities() {
    let mut value = plan();
    let limiting_binding = value.body.materialized.attempt_owned.groups[0].candidates[1]
        .binding_id
        .clone();
    for profile in &mut std::sync::Arc::make_mut(&mut value.body)
        .materialized
        .attempt_owned
        .groups[0]
        .candidates[1]
        .protocol_profiles
    {
        profile.capability.context.max_input_tokens = GatewayCriticalFactV1::Exact(64000);
        profile.capability.request.image_url = GatewayFidelityV1::Unsupported;
        profile.capability.request.image_base64 = GatewayFidelityV1::Unsupported;
    }
    let value = reseal(value);
    let preview = codex_plan_capability_preview(&value);
    let CodexClientCapabilityPreviewV1::Available {
        context_window,
        input_modalities,
        limitations,
        fixed_limits,
        ..
    } = preview
    else {
        panic!("complete candidate metadata must produce a preview");
    };
    assert_eq!(context_window, 64000);
    assert_eq!(input_modalities, vec![CodexInputModalityV1::Text]);
    assert_eq!(limitations.len(), 2);
    assert!(
        limitations
            .iter()
            .all(|limit| limit.binding_ids == [limiting_binding.clone()])
    );
    assert_eq!(
        limitations
            .iter()
            .map(|limit| limit.kind)
            .collect::<Vec<_>>(),
        vec![
            CodexCandidateCapabilityLimitKindV1::ContextWindow,
            CodexCandidateCapabilityLimitKindV1::ImageInput,
        ]
    );
    assert_eq!(
        fixed_limits,
        vec![CodexFixedCapabilityLimitV1::ParallelToolCallsDisabled]
    );
    let entry = plan_entry(&value, 99).unwrap();
    assert_eq!(entry["context_window"], 64000);
    assert_eq!(entry["input_modalities"], json!(["text"]));
}

#[test]
fn alias_collision_does_not_replace_an_original_entry() {
    let value = plan();
    let mut raw = original().original().clone();
    raw["models"][0]["slug"] = json!(value.model_alias().as_str());
    let source = CodexCatalogSelection::parse(raw.clone()).unwrap();
    assert_eq!(
        source.append_plans(
            &[value],
            policy(),
            CodexCatalogMetadataSourceV1::UserConfigured,
            None,
        ),
        Err(CodexCatalogError::InvalidCatalog)
    );
    assert_eq!(source.original(), &raw);
}

#[test]
fn plan_append_rejects_implicit_default_change_for_all_hidden_catalog() {
    let mut raw = original().original().clone();
    for entry in raw["models"].as_array_mut().unwrap() {
        entry["visibility"] = json!("hide");
    }
    let source = CodexCatalogSelection::parse(raw).unwrap();
    assert_eq!(
        source.append_plans(
            &[plan()],
            policy(),
            CodexCatalogMetadataSourceV1::UserConfigured,
            None,
        ),
        Err(CodexCatalogError::DefaultChanged)
    );
}

#[test]
fn generic_prompt_and_test_catalog_are_exact_pinned_assets() {
    assert_eq!(
        CanonicalDigest::of_bytes(generic_prompt().as_bytes()).as_str(),
        "sha256:ac8ae107a0d72fe3476b430afb161ea4e67da2e446d778aefc44828160559807"
    );
    assert_eq!(
        CanonicalDigest::of_bytes(include_bytes!("codex_bundled_catalog.json")).as_str(),
        "sha256:384ff2e0ca67f65d2866e422e2ec7dfa5ed9e3fec7a84fe14005247a7087a302"
    );
    assert!(include_str!("codex_generic_prompt.license").contains("Apache License"));
}

#[test]
fn context_window_default_custom_and_worker_catalog_agree() {
    let mut large = plan();
    for candidate in std::sync::Arc::make_mut(&mut large.body)
        .materialized
        .attempt_owned
        .groups
        .iter_mut()
        .flat_map(|g| &mut g.candidates)
    {
        for profile in &mut candidate.protocol_profiles {
            let context = &mut profile.capability.context;
            context.max_input_tokens = GatewayCriticalFactV1::Exact(1_050_000);
            context.max_output_tokens = GatewayCriticalFactV1::Exact(128_000);
            context.max_total_tokens = GatewayCriticalFactV1::Exact(Some(1_178_000));
        }
    }
    let large = reseal(large);
    assert_eq!(
        large
            .body
            .materialized
            .context_window_upper_bound()
            .unwrap(),
        1_050_000
    );
    for (setting, expected) in [
        (None, 272_000),
        (Some(128_000), 128_000),
        (Some(500_000), 500_000),
        (Some(1_050_000), 1_050_000),
    ] {
        let mut selected = large.clone();
        std::sync::Arc::make_mut(&mut selected.body)
            .materialized
            .attempt_owned
            .limits
            .context_window_tokens = setting;
        let selected = reseal(selected);
        let entry = plan_entry(&selected, 0).unwrap();
        assert_eq!(entry["context_window"], expected);
        assert_eq!(entry["max_context_window"], expected);
        let worker: Value =
            serde_json::from_slice(&codex_private_worker_catalog(&selected).unwrap()).unwrap();
        assert_eq!(worker["models"][0], entry);
        assert!(
            matches!(codex_plan_capability_preview(&selected), CodexClientCapabilityPreviewV1::Available { context_window, .. } if context_window == expected)
        );
    }
}

#[test]
fn context_window_overrides_block_plan_catalog_without_editing_user_files() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.toml");
    let mut scope = CodexConfigurationScope::user_file(config.clone());
    let plan = plan();
    for key in ["model_context_window", "model_auto_compact_token_limit"] {
        let content = format!("{key} = 900000\n");
        std::fs::write(&config, &content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(matches!(
            sample_codex_hiroute_only_catalog_plan(
                &scope,
                std::slice::from_ref(&plan),
                plan.model_alias().as_str()
            ),
            Err(CodexCatalogError::ContextOverride)
        ));
        assert_eq!(std::fs::read_to_string(&config).unwrap(), content);
        std::fs::write(&config, "").unwrap();
        scope.cli_overrides.push(format!("{key}=900000"));
        assert!(matches!(
            sample_codex_hiroute_only_catalog_plan(
                &scope,
                std::slice::from_ref(&plan),
                plan.model_alias().as_str()
            ),
            Err(CodexCatalogError::ContextOverride)
        ));
        scope.cli_overrides.clear();
    }
    assert!(
        sample_codex_hiroute_only_catalog_plan(
            &scope,
            std::slice::from_ref(&plan),
            plan.model_alias().as_str()
        )
        .is_ok()
    );
}

#[test]
fn claude_capability_preview_rejects_missing_tools_and_streaming_in_any_candidate() {
    use crate::agents::claude_plan_capability_preview;
    use hiroute_application_api::ClaudeClientCapabilityPreviewV1 as Preview;
    assert!(matches!(
        claude_plan_capability_preview(&plan()),
        Preview::Available { .. }
    ));
    for missing_tools in [true, false] {
        let mut value = plan();
        let candidate = &mut std::sync::Arc::make_mut(&mut value.body)
            .materialized
            .attempt_owned
            .groups[0]
            .candidates[1];
        for profile in &mut candidate.protocol_profiles {
            if profile.ingress_protocol == UpstreamProtocol::Messages {
                if missing_tools {
                    profile.capability.request.function_tools = GatewayFidelityV1::Unsupported;
                } else {
                    profile.capability.native_streaming = GatewayCriticalFactV1::Exact(false);
                }
            }
        }
        assert!(
            matches!(claude_plan_capability_preview(&reseal(value)), Preview::Unavailable { reason } if reason == "request_capabilities")
        );
    }
    let mut value = plan();
    std::sync::Arc::make_mut(&mut value.body)
        .materialized
        .attempt_owned
        .limits
        .context_window_tokens = Some(99_999);
    assert!(
        matches!(claude_plan_capability_preview(&reseal(value)), Preview::Unavailable { reason } if reason == "context_window_below_minimum")
    );
}
