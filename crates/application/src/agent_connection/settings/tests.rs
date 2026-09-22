use super::*;
use hiroute_domain::{
    AgentCapability, AgentCollaborationSelectionV2, AgentCollaborationTriggerModeV2,
    CapabilityEvidence, CapabilityState,
};
use serde_json::json;
fn facts() -> AgentSettingsFacts {
    let dependency_digest = CanonicalDigest::of_bytes(b"context/all-consumed-dependencies/v1");
    let capabilities = AgentCapabilitySet::new(
        [
            AgentCapability::AtomicManagedReplace,
            AgentCapability::SkillLoading,
            AgentCapability::TrustedCliExecution,
        ]
        .into_iter()
        .map(|capability| CapabilityEvidence {
            capability,
            state: CapabilityState::Proven,
            adapter_contract: "isolated-test/1".into(),
            observed_at_unix_ms: 1,
            dependency_digest: dependency_digest.clone(),
            reason: None,
        }),
    )
    .unwrap();
    AgentSettingsFacts {
        context_id: "agent-context/test".into(),
        dependency_digest,
        capabilities,
        ingress: hiroute_domain::AgentIngressProtocolV1::Messages,
        available_surfaces: [
            hiroute_domain::AgentModelSurfaceV2::CodexCli,
            hiroute_domain::AgentModelSurfaceV2::CodexDesktop,
            hiroute_domain::AgentModelSurfaceV2::ClaudeCli,
        ]
        .into(),
        model_publication: None,
        model_catalog: None,
        login_item_required: false,
        login_item_removal_required: false,
        fixed_candidate_facts: Vec::new(),
        preserved_codex_models: Vec::new(),
        preserved_codex_bindings: BTreeMap::new(),
        required_native_model_ids: None,
        unproven_native_model_ids: Vec::new(),
        require_native_model_routes: false,
        native_default_must_be_original: false,
        native_default_model: None,
        restore_native_model_ids: None,
        restored_native_model: None,
        native_claude_presets: None,
        collaboration_file_conflict: false,
        restore_points: BTreeMap::new(),
    }
}
#[test]
fn claude_default_coverage_resolves_all_three_presets_without_changing_selection() {
    use hiroute_domain::{
        AgentClaudePresetMappingsV2, AgentClaudePresetSelectionV2, AgentClaudePresetValuesV2,
        AgentModelSurfaceV2, AgentPlanId, ModelAlias,
    };
    let plan_id = AgentPlanId::parse("plan/shared").unwrap();
    let grant = AgentModelGrantV2::seal(
        AgentIngressProtocolV1::Messages,
        BTreeMap::from([(
            "hiroute-shared".into(),
            AgentModelRouteV2::Plan {
                plan_id: plan_id.clone(),
                alias: ModelAlias::parse_custom("hiroute-shared").unwrap(),
                revision: 1,
                semantic_digest: CanonicalDigest::of_bytes(b"shared-plan"),
            },
        )]),
    )
    .unwrap();
    let mapped = AgentClaudePresetSelectionV2::Plan { plan_id };
    let mut selection = AgentModelSelectionV2::ClaudeLauncher {
        surfaces: [AgentModelSurfaceV2::ClaudeCli].into(),
        fixed_models: Vec::new(),
        preset_mappings: AgentClaudePresetMappingsV2 {
            opus: mapped.clone(),
            sonnet: mapped.clone(),
            haiku: mapped,
        },
    };
    let mut current = facts();
    current.native_claude_presets = Some(AgentClaudePresetValuesV2 {
        opus: Some("native-opus".into()),
        sonnet: None,
        haiku: Some("native-haiku".into()),
    });
    for default in ["opus", "sonnet", "haiku", "hiroute-shared", "default"] {
        current.native_default_model = Some(default.into());
        assert_eq!(validate_model_default(&selection, &current, &grant), Ok(()));
        assert_eq!(current.native_default_model.as_deref(), Some(default));
    }
    current.native_default_model = None;
    assert_eq!(validate_model_default(&selection, &current, &grant), Ok(()));
    for default in [Some("uncovered-native"), Some("opus[1m]")] {
        current.native_default_model = default.map(str::to_owned);
        assert_eq!(
            validate_model_default(&selection, &current, &grant),
            Err(SettingsPlanningError::InvalidSelection)
        );
    }
    if let AgentModelSelectionV2::ClaudeLauncher {
        preset_mappings, ..
    } = &mut selection
    {
        preset_mappings.opus = AgentClaudePresetSelectionV2::PreserveNative;
        preset_mappings.sonnet = AgentClaudePresetSelectionV2::PreserveNative;
    }
    for default in ["opus", "sonnet"] {
        current.native_default_model = Some(default.into());
        assert_eq!(
            validate_model_default(&selection, &current, &grant),
            Err(SettingsPlanningError::InvalidSelection)
        );
    }
    current.native_default_model = Some("haiku".into());
    assert_eq!(validate_model_default(&selection, &current, &grant), Ok(()));
    current.native_claude_presets = None;
    assert_eq!(
        validate_model_default(&selection, &current, &grant),
        Err(SettingsPlanningError::InvalidSelection)
    );
}

#[test]
fn agent_settings_skill_only_needs_no_model_default_publication_or_upstream() {
    let spec: AgentSettingsSpecV2 = serde_json::from_value(json!({
        "schema_version": {"major": 2, "minor": 0}, "context_id": "agent-context/test",
        "collaboration": {"intent": "configure", "settings": {"trigger_mode": "explicit"}}
    }))
    .unwrap();
    assert_eq!(spec.model, AgentFacetIntent::Keep);
    let preview = preview_agent_settings(spec, &facts()).unwrap();
    assert!(preview.blockers.is_empty());
    assert!(preview.model_grant.is_none());
    assert_eq!(
        preview.collaboration_trigger_mode,
        Some(AgentCollaborationTriggerModeV2::Explicit)
    );
    assert_eq!(
        preview.changed_facets,
        BTreeSet::from([AgentSettingsFacet::Collaboration])
    );
}
#[test]
fn legacy_collaboration_fields_are_rejected_and_capability_drift_still_blocks() {
    assert!(
        serde_json::from_value::<AgentSettingsSpecV2>(json!({
            "schema_version": {"major": 2, "minor": 0},
            "context_id": "agent-context/test",
            "collaboration": {"intent": "configure", "settings": {"allowed_plan_ids": []}}
        }))
        .is_err()
    );
    let spec = AgentSettingsSpecV2 {
        schema_version: AGENT_SETTINGS_SCHEMA_V2,
        context_id: "agent-context/test".into(),
        model: AgentFacetIntent::Keep,
        collaboration: AgentFacetIntent::Configure {
            settings: AgentCollaborationSelectionV2 {
                trigger_mode: AgentCollaborationTriggerModeV2::DelegateByDefault,
            },
        },
        restore_native_model: None,
        protected_native_model_ids: Vec::new(),
        access_token: hiroute_domain::AgentAccessTokenIntentV1::Keep,
    };
    assert!(
        preview_agent_settings(spec.clone(), &facts())
            .unwrap()
            .blockers
            .is_empty()
    );
    let mut changed = facts();
    changed.dependency_digest = CanonicalDigest::of_bytes(b"new-context-revision");
    let preview = preview_agent_settings(spec, &changed).unwrap();
    assert!(
        preview
            .blockers
            .iter()
            .all(|block| block.facet == AgentSettingsFacet::Collaboration)
    );
    assert!(
        preview
            .blockers
            .iter()
            .any(|block| block.reason == SettingsBlockReason::CapabilityUnavailable)
    );
}
#[test]
fn agent_settings_restore_reference_cannot_cross_context_or_facet() {
    let spec = AgentSettingsSpecV2 {
        schema_version: AGENT_SETTINGS_SCHEMA_V2,
        context_id: "agent-context/test".into(),
        model: AgentFacetIntent::Restore {
            restore_point_ref: "restore/from-other-context".into(),
        },
        collaboration: AgentFacetIntent::Keep,
        restore_native_model: None,
        protected_native_model_ids: Vec::new(),
        access_token: hiroute_domain::AgentAccessTokenIntentV1::Keep,
    };
    let preview = preview_agent_settings(spec, &facts()).unwrap();
    assert!(
        preview
            .blockers
            .iter()
            .any(|block| block.reason == SettingsBlockReason::RestorePointUnavailable)
    );
    assert!(
        !preview
            .changed_facets
            .contains(&AgentSettingsFacet::Collaboration)
    );
}

#[test]
fn codex_restore_requires_an_original_catalog_model_for_a_stale_alias() {
    let mut facts = facts();
    facts.ingress = AgentIngressProtocolV1::Responses;
    facts
        .restore_points
        .insert("restore/owned".into(), AgentSettingsFacet::Model);
    facts.restore_native_model_ids = Some(vec!["gpt-5.6-sol".into(), "gpt-5.6-luna".into()]);
    facts.restored_native_model = Some("hiroute-fanyi".into());
    let mut spec: AgentSettingsSpecV2 = serde_json::from_value(json!({
        "schema_version": {"major": 2, "minor": 0},
        "context_id": "agent-context/test",
        "model": {"intent": "restore", "restore_point_ref": "restore/owned"}
    }))
    .unwrap();
    let blocked = preview_agent_settings(spec.clone(), &facts).unwrap();
    assert!(blocked.blockers.iter().any(|block| {
        block.reason == SettingsBlockReason::RestoreNativeModelInvalid
            && block.model_ids == ["hiroute-fanyi"]
    }));
    spec.restore_native_model = Some("gpt-5.6-sol".into());
    let repaired = preview_agent_settings(spec.clone(), &facts).unwrap();
    assert!(
        !repaired
            .blockers
            .iter()
            .any(|block| block.reason == SettingsBlockReason::RestoreNativeModelInvalid)
    );
    spec.restore_native_model = Some("other-account-model".into());
    let wrong = preview_agent_settings(spec, &facts).unwrap();
    assert!(
        wrong
            .blockers
            .iter()
            .any(|block| block.reason == SettingsBlockReason::RestoreNativeModelInvalid)
    );
}

#[test]
fn agent_settings_confirmation_rebuilds_preview_and_rejects_drift_or_changed_facets() {
    use hiroute_application_api::AgentSettingsApplyV2;
    let spec: AgentSettingsSpecV2 = serde_json::from_value(json!({
        "schema_version": {"major":2,"minor":0}, "context_id":"agent-context/test",
        "collaboration":{"intent":"configure","settings":{"trigger_mode":"explicit"}}
    }))
    .unwrap();
    let fresh = facts();
    let preview = preview_agent_settings(spec.clone(), &fresh).unwrap();
    let request = AgentSettingsApplyV2 {
        expected_revisions: hiroute_domain::RevisionSetV1 {
            target: 0,
            dependencies: Default::default(),
        },
        spec,
        accept_digest: preview.accept_digest,
        dependency_digest: preview.dependency_digest,
        idempotency_key: "settings-once".into(),
        login_item: None,
    };
    let confirmed = confirm_agent_settings(request.clone(), &fresh).unwrap();
    assert_eq!(confirmed.preview().spec.model, AgentFacetIntent::Keep);
    assert_eq!(confirmed.idempotency_key(), "settings-once");
    let mut changed = request.clone();
    changed.spec.model = AgentFacetIntent::Restore {
        restore_point_ref: "restore/not-confirmed".into(),
    };
    assert!(matches!(
        confirm_agent_settings(changed, &fresh),
        Err(SettingsConfirmationError::Stale)
    ));
    let mut drift = facts();
    drift.dependency_digest = CanonicalDigest::of_bytes(b"replaced-native-file");
    assert!(matches!(
        confirm_agent_settings(request.clone(), &drift),
        Err(SettingsConfirmationError::Stale)
    ));
    let mut unavailable = facts();
    unavailable.capabilities = AgentCapabilitySet::new([]).unwrap();
    let blocked = preview_agent_settings(request.spec.clone(), &unavailable).unwrap();
    assert_eq!(blocked.blockers[0].capabilities.len(), 3);
    let blocked_request = AgentSettingsApplyV2 {
        expected_revisions: hiroute_domain::RevisionSetV1 {
            target: 0,
            dependencies: Default::default(),
        },
        accept_digest: blocked.accept_digest,
        ..request
    };
    assert!(matches!(
        confirm_agent_settings(blocked_request, &unavailable),
        Err(SettingsConfirmationError::Blocked)
    ));
}

#[test]
fn confirmed_skill_only_settings_seal_exact_effects_and_confirmation_dependencies() {
    use hiroute_application_api::AgentSettingsApplyV2;
    use hiroute_domain::{
        AgentConnectionEffectRoleV1, AgentConnectionTransactionSubjectV1, ExternalEffectIntentV1,
    };
    let fresh = facts();
    let spec: AgentSettingsSpecV2 = serde_json::from_value(json!({
        "schema_version":{"major":2,"minor":0}, "context_id":"agent-context/test",
        "collaboration":{"intent":"configure","settings":{"trigger_mode":"explicit"}}
    }))
    .unwrap();
    let preview = preview_agent_settings(spec.clone(), &fresh).unwrap();
    let confirmed = confirm_agent_settings(
        AgentSettingsApplyV2 {
            expected_revisions: hiroute_domain::RevisionSetV1 {
                target: 0,
                dependencies: Default::default(),
            },
            spec,
            accept_digest: preview.accept_digest.clone(),
            dependency_digest: preview.dependency_digest.clone(),
            idempotency_key: "settings-sealed".into(),
            login_item: None,
        },
        &fresh,
    )
    .unwrap();
    let subject = || {
        AgentConnectionTransactionSubjectV1::from_registered_profile(
            "agent.codex",
            "default",
            "codex.profile.v1",
        )
        .unwrap()
    };
    let plan = confirmed
        .seal_effects(subject(), &json!({"revision":1}), true, vec![], |control| {
            Ok(vec![ExternalEffectIntentV1::from_agent_connection_planner(
                control,
                AgentConnectionEffectRoleV1::RoutingSkill,
                None,
                &json!({"content_digest":CanonicalDigest::of_bytes(b"shared-template"), "context_id":"agent-context/test", "action":"install"}),
                0o644,
            )?])
        })
        .unwrap();
    assert_eq!(plan.spec().command_id, "agents.settings.apply");
    assert!(plan.agent_access_grants().is_empty());
    assert_eq!(
        plan.control()["payload"]["state"]["accept_digest"],
        json!(preview.accept_digest)
    );
    assert_eq!(
        plan.control()["payload"]["state"]["dependency_digest"],
        json!(fresh.dependency_digest)
    );
    assert!(
        confirmed
            .seal_effects(subject(), &json!({"revision":1}), true, vec![], |_| Ok(
                vec![]
            ))
            .is_err()
    );
    assert!(
        confirmed
            .seal_effects(
                subject(),
                &json!({"api_key":"must-not-be-journaled"}),
                false,
                vec![],
                |_| Ok(vec![])
            )
            .is_err()
    );
}

#[test]
fn codex_plan_selections_preview_only_with_catalog_facts() {
    let publication = crate::compiler::test_fixtures::compiled_publication(1);
    let plan = publication
        .published_agent_plans()
        .unwrap()
        .into_iter()
        .find(|plan| plan.active)
        .unwrap();
    let plan_id = serde_json::to_value(&plan.agent_plan_id).unwrap();
    let spec = |settings: serde_json::Value| -> AgentSettingsSpecV2 {
        serde_json::from_value(json!({
            "schema_version": {"major": 2, "minor": 0}, "context_id": "agent-context/test",
            "model": {"intent": "configure", "settings": settings}
        }))
        .unwrap()
    };
    let plan_selection = json!({
        "mode": "codex_default", "native_model_mode": "hiroute_only", "fixed_models": [],
        "allowed_plan_ids": [plan_id.clone()],
        "default_selection": {"kind": "plan", "plan_id": plan_id}
    });
    let codex_facts = |model_catalog: Option<SettingsModelCatalogFacts>| {
        let dependency_digest = CanonicalDigest::of_bytes(b"context/catalog-gate/v1");
        let capabilities = AgentCapabilitySet::new(
            [
                AgentCapability::EffectiveConfiguration,
                AgentCapability::AtomicManagedReplace,
                AgentCapability::IngressAuthentication,
            ]
            .into_iter()
            .map(|capability| CapabilityEvidence {
                capability,
                state: CapabilityState::Proven,
                adapter_contract: "isolated-test/1".into(),
                observed_at_unix_ms: 1,
                dependency_digest: dependency_digest.clone(),
                reason: None,
            }),
        )
        .unwrap();
        AgentSettingsFacts {
            context_id: "agent-context/test".into(),
            dependency_digest,
            capabilities,
            ingress: AgentIngressProtocolV1::Responses,
            available_surfaces: [
                hiroute_domain::AgentModelSurfaceV2::CodexCli,
                hiroute_domain::AgentModelSurfaceV2::CodexDesktop,
            ]
            .into(),
            model_publication: Some(publication.clone()),
            model_catalog,
            login_item_required: false,
            login_item_removal_required: false,
            fixed_candidate_facts: Vec::new(),
            preserved_codex_models: Vec::new(),
            preserved_codex_bindings: BTreeMap::new(),
            required_native_model_ids: None,
            unproven_native_model_ids: Vec::new(),
            require_native_model_routes: false,
            native_default_must_be_original: false,
            native_default_model: None,
            restore_native_model_ids: None,
            restored_native_model: None,
            native_claude_presets: None,
            collaboration_file_conflict: false,
            restore_points: BTreeMap::new(),
        }
    };
    let catalog_block = |reason: hiroute_domain::CapabilityBlockReason| AgentSettingsBlock {
        facet: AgentSettingsFacet::Model,
        reason: SettingsBlockReason::CapabilityUnavailable,
        capabilities: vec![hiroute_domain::CapabilityBlock {
            capability: AgentCapability::ModelCatalog,
            reason,
        }],
        model_ids: Vec::new(),
    };

    // An unavailable full-catalog source leaves the catalog underivable; the selection is
    // blocked, not sealed.
    let unproven =
        preview_agent_settings(spec(plan_selection.clone()), &codex_facts(None)).unwrap();
    assert!(unproven.model_grant.is_some());
    assert_eq!(
        unproven.blockers,
        vec![catalog_block(
            hiroute_domain::CapabilityBlockReason::Unknown
        )]
    );

    // Structural catalog support whose source still cannot be derived reports unavailable, not
    // unknown.
    let mut unavailable_facts = codex_facts(None);
    unavailable_facts.capabilities = AgentCapabilitySet::new(
        [
            AgentCapability::EffectiveConfiguration,
            AgentCapability::AtomicManagedReplace,
            AgentCapability::IngressAuthentication,
            AgentCapability::ModelCatalog,
        ]
        .into_iter()
        .map(|capability| CapabilityEvidence {
            capability,
            state: CapabilityState::Proven,
            adapter_contract: "isolated-test/1".into(),
            observed_at_unix_ms: 1,
            dependency_digest: unavailable_facts.dependency_digest.clone(),
            reason: None,
        }),
    )
    .unwrap();
    let unavailable =
        preview_agent_settings(spec(plan_selection.clone()), &unavailable_facts).unwrap();
    assert_eq!(
        unavailable.blockers,
        vec![catalog_block(
            hiroute_domain::CapabilityBlockReason::Unavailable
        )]
    );

    let mut no_detected_surface = codex_facts(Some(SettingsModelCatalogFacts {
        source_revision: "be6e8eac029b183056b7e4402879f15d2c85f61b".into(),
        content_digest: CanonicalDigest::of_bytes(b"merged-catalog"),
        before_fingerprint: None,
        producer_kind: CodexCatalogProducerKindV1::TargetCache,
        producer_path: "/target/.codex/models_cache.json".into(),
        producer_content_digest: CanonicalDigest::of_bytes(b"native-catalog"),
        producer_context_digest: CanonicalDigest::of_bytes(b"codex-context"),
        producer_dependency_digest: CanonicalDigest::of_bytes(b"catalog-dependencies"),
    }));
    no_detected_surface.available_surfaces.clear();
    let without_client =
        preview_agent_settings(spec(plan_selection.clone()), &no_detected_surface).unwrap();
    assert!(without_client.blockers.is_empty());
    assert!(without_client.model_grant.is_some());

    // Catalog facts make the same plan selection preview cleanly.
    let mut proven_facts = codex_facts(Some(SettingsModelCatalogFacts {
        source_revision: "be6e8eac029b183056b7e4402879f15d2c85f61b".into(),
        content_digest: CanonicalDigest::of_bytes(b"merged-catalog"),
        before_fingerprint: None,
        producer_kind: CodexCatalogProducerKindV1::TargetCache,
        producer_path: "/target/.codex/models_cache.json".into(),
        producer_content_digest: CanonicalDigest::of_bytes(b"native-catalog"),
        producer_context_digest: CanonicalDigest::of_bytes(b"codex-context"),
        producer_dependency_digest: CanonicalDigest::of_bytes(b"catalog-dependencies"),
    }));
    let mut preserve_plan_alias = plan_selection.clone();
    preserve_plan_alias["native_model_mode"] = json!("preserve_available");
    preserve_plan_alias["default_selection"] = json!({"kind": "preserve_native"});
    proven_facts.native_default_model = Some(plan.model_alias.as_str().into());
    let matched_alias =
        preview_agent_settings(spec(preserve_plan_alias.clone()), &proven_facts).unwrap();
    assert!(matched_alias.blockers.is_empty());
    assert!(
        matched_alias
            .model_grant
            .unwrap()
            .permits_name(plan.model_alias.as_str()),
        "an explicitly allowed plan covers the same current Codex default alias"
    );
    proven_facts.native_default_model = Some("hiroute-unpublished".into());
    let unmatched_alias = preview_agent_settings(spec(preserve_plan_alias), &proven_facts).unwrap();
    assert_eq!(
        unmatched_alias.blockers[0].reason,
        SettingsBlockReason::ModelPlanUnavailable,
        "an unrelated default name still cannot be published"
    );
    proven_facts.native_default_model = None;
    let candidates = crate::compiler::test_fixtures::compilation_facts().candidates;
    let candidate = candidates
        .iter()
        .find(|candidate| candidate.binding.binding_id == "binding/primary-a")
        .unwrap()
        .clone();
    let replacement_candidate = candidates
        .iter()
        .find(|candidate| candidate.binding.binding_id == "binding/primary-b")
        .unwrap()
        .clone();
    let preserved = hiroute_domain::AgentFixedModelSelectionV2 {
        client_model_id: candidate.native_transport_model.clone(),
        candidate: hiroute_domain::CandidateSelectionV1 {
            binding_id: candidate.binding.binding_id.clone(),
            reasoning: Some(hiroute_domain::ReasoningSelectionV1::Profile {
                profile: "medium".into(),
            }),
        },
    };
    proven_facts.fixed_candidate_facts = vec![candidate, replacement_candidate.clone()];
    proven_facts.preserved_codex_models = vec![preserved.clone()];
    proven_facts.native_default_model = Some(preserved.client_model_id.clone());
    proven_facts.required_native_model_ids = Some(vec![
        preserved.client_model_id.clone(),
        "native-unselected".into(),
    ]);
    proven_facts.require_native_model_routes = true;
    proven_facts.native_default_must_be_original = true;
    let mut preserve_selection = plan_selection.clone();
    preserve_selection["native_model_mode"] = json!("preserve_available");
    let incomplete =
        preview_agent_settings(spec(preserve_selection.clone()), &proven_facts).unwrap();
    assert!(incomplete.blockers.iter().any(|block| {
        block.reason == SettingsBlockReason::NativeModelCoverageUnavailable
            && block.model_ids == ["native-unselected"]
    }));
    proven_facts.required_native_model_ids = Some(vec![preserved.client_model_id.clone()]);
    proven_facts.native_default_model = Some(plan.model_alias.as_str().into());
    let stale_default =
        preview_agent_settings(spec(preserve_selection.clone()), &proven_facts).unwrap();
    assert!(stale_default.blockers.iter().any(|block| {
        block.reason == SettingsBlockReason::NativeDefaultInvalid
            && block.model_ids == [plan.model_alias.as_str()]
    }));
    proven_facts.native_default_model = Some(preserved.client_model_id.clone());
    let mut replacing = preserve_selection.clone();
    let mut replaced = preserved.clone();
    replaced.candidate.binding_id = replacement_candidate.binding.binding_id.clone();
    replacing["fixed_models"] = serde_json::to_value([replaced.clone()]).unwrap();
    let replacement = preview_agent_settings(spec(replacing), &proven_facts).unwrap();
    assert!(replacement.blockers.iter().any(|block| {
        block.reason == SettingsBlockReason::NativeModelCoverageUnavailable
            && block.model_ids == [preserved.client_model_id.as_str()]
    }));
    let AgentFacetIntent::Configure {
        settings: AgentModelSelectionV2::CodexDefault { fixed_models, .. },
    } = &replacement.spec.model
    else {
        panic!("expected Codex model configuration")
    };
    assert_eq!(fixed_models, std::slice::from_ref(&replaced));

    proven_facts.unproven_native_model_ids = vec!["cache-name-without-account-proof".into()];
    let proven = preview_agent_settings(spec(preserve_selection.clone()), &proven_facts).unwrap();
    assert!(proven.blockers.is_empty());
    assert_eq!(
        proven.unproven_native_model_ids,
        ["cache-name-without-account-proof"]
    );
    assert!(proven.model_grant.is_some());
    let AgentFacetIntent::Configure {
        settings: AgentModelSelectionV2::CodexDefault { fixed_models, .. },
    } = &proven.spec.model
    else {
        panic!("expected Codex model configuration")
    };
    assert_eq!(fixed_models, std::slice::from_ref(&preserved));

    // A later full-state edit keeps the sealed native route even when the ephemeral
    // connection-only candidate has vanished from current discovery.
    let prior = proven.model_grant.as_ref().unwrap();
    let hiroute_domain::AgentModelRouteV2::Fixed { binding, .. } =
        &prior.routes[&preserved.client_model_id]
    else {
        panic!("fixed route")
    };
    proven_facts
        .preserved_codex_bindings
        .insert(preserved.client_model_id.clone(), binding.as_ref().clone());
    proven_facts.fixed_candidate_facts.clear();
    let later = preview_agent_settings(spec(preserve_selection.clone()), &proven_facts).unwrap();
    assert!(later.blockers.is_empty());
    assert_eq!(
        later.spec.protected_native_model_ids.as_slice(),
        std::slice::from_ref(&preserved.client_model_id)
    );
    assert_eq!(
        later.model_grant.unwrap().routes[&preserved.client_model_id],
        prior.routes[&preserved.client_model_id]
    );
    let mut changed_binding = preserve_selection;
    changed_binding["fixed_models"] = serde_json::to_value([replaced]).unwrap();
    proven_facts.fixed_candidate_facts = vec![replacement_candidate];
    let rejected = preview_agent_settings(spec(changed_binding), &proven_facts).unwrap();
    assert!(!rejected.blockers.is_empty());
    assert!(
        rejected
            .blockers
            .iter()
            .any(|block| { block.reason == SettingsBlockReason::ModelPlanUnavailable })
    );

    // Plan-only settings without protected native bindings remain a full-state edit.
    proven_facts.preserved_codex_models.clear();
    proven_facts.preserved_codex_bindings.clear();
    proven_facts.required_native_model_ids = None;
    proven_facts.require_native_model_routes = false;
    let removed = preview_agent_settings(spec(plan_selection), &proven_facts).unwrap();
    let AgentFacetIntent::Configure {
        settings: AgentModelSelectionV2::CodexDefault { fixed_models, .. },
    } = &removed.spec.model
    else {
        panic!("expected Codex model configuration")
    };
    assert!(fixed_models.is_empty());
}

#[test]
fn codex_model_file_journal_binds_the_catalog_to_plan_carrying_grants() {
    use crate::agent_connection::{
        CodexModelFileAction, settings_codex_catalog_intent,
        settings_codex_model_file_for_operation, settings_codex_model_file_intent,
    };
    use hiroute_domain::{
        AgentAccessGrantMutationV1, AgentAccessGrantScopeV1, AgentConnectionControlIntentV1,
        AgentConnectionTransactionSubjectV1, AgentModelRouteV2, AgentPlanId, CHANGE_SPEC_SCHEMA_V1,
        ChangeSpecV1, IdempotencyScopeV1, ModelAlias, OperationId, OperationV1, RevisionSetV1,
        TransactionPlanV1, WorkspaceId,
    };
    const CONTEXT: &str = "agent-context/catalog-journal";
    let plan_id = AgentPlanId::parse("plan/journal").unwrap();
    let grant = AgentModelGrantV2::seal(
        AgentIngressProtocolV1::Responses,
        BTreeMap::from([(
            "hiroute-journal".to_owned(),
            AgentModelRouteV2::Plan {
                plan_id: plan_id.clone(),
                alias: ModelAlias::parse_custom("hiroute-journal").unwrap(),
                revision: 1,
                semantic_digest: CanonicalDigest::of_bytes(b"plan-journal"),
            },
        )]),
    )
    .unwrap();
    let catalog_digest = CanonicalDigest::of_bytes(b"merged-catalog");
    let change = |model_catalog: Option<CanonicalDigest>| CodexModelFileAction::Configure {
        previous_operation: None,
        provider_id: "hiroute".to_owned(),
        endpoint: "http://127.0.0.1:8787/v1".to_owned(),
        model: None,
        model_catalog,
    };
    let operation = |grant: &AgentModelGrantV2,
                     plan_ids: serde_json::Value,
                     change: CodexModelFileAction,
                     catalog: Option<CanonicalDigest>|
     -> OperationV1 {
        let change_spec = ChangeSpecV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            command_id: "agents.settings.apply".to_owned(),
            resource_id: Some(CONTEXT.to_owned()),
            desired_state: json!({
                "schema_version": {"major": 2, "minor": 0}, "context_id": CONTEXT,
                "model": {"intent": "configure", "settings": {
                    "mode": "codex_default",
                    "native_model_mode": "preserve_available",
                    "fixed_models": [], "allowed_plan_ids": plan_ids,
                    "default_selection": {"kind": "preserve_native"}
                }}
            }),
        };
        let accept = CanonicalDigest::of_bytes(b"catalog-journal-accept");
        let control = AgentConnectionControlIntentV1::from_settings_planner(
            AgentConnectionTransactionSubjectV1::from_registered_profile(
                "agent_codex_default",
                "codex-responses-v1",
                "builtin/codex-responses/v1",
            )
            .unwrap(),
            &change_spec,
            false,
            &json!({"accept_digest": accept, "model_grant": grant}),
        )
        .unwrap();
        let mutation = AgentAccessGrantMutationV1::ensure(
            WorkspaceId::DEFAULT,
            AgentAccessGrantScopeV1::new(format!("agent-connection/{CONTEXT}"), grant.clone())
                .unwrap(),
            0,
        )
        .unwrap();
        let model_file = settings_codex_model_file_intent(
            &control,
            CONTEXT,
            CanonicalDigest::of_bytes(b"model-file"),
            None,
            change,
        )
        .unwrap();
        let mut external = catalog
            .map(|digest| {
                settings_codex_catalog_intent(
                    &control,
                    CONTEXT,
                    &SettingsModelCatalogFacts {
                        source_revision: "be6e8eac029b183056b7e4402879f15d2c85f61b".into(),
                        content_digest: digest,
                        before_fingerprint: None,
                        producer_kind: CodexCatalogProducerKindV1::TargetCache,
                        producer_path: "/target/.codex/models_cache.json".into(),
                        producer_content_digest: CanonicalDigest::of_bytes(b"native-catalog"),
                        producer_context_digest: CanonicalDigest::of_bytes(b"codex-context"),
                        producer_dependency_digest: CanonicalDigest::of_bytes(
                            b"catalog-dependencies",
                        ),
                    },
                )
                .unwrap()
            })
            .into_iter()
            .collect::<Vec<_>>();
        external.push(model_file);
        external.push(
            hiroute_domain::settings_model_publication_intent(
                &control,
                CONTEXT,
                CanonicalDigest::of_bytes(b"publication"),
                None,
            )
            .unwrap(),
        );
        let plan = TransactionPlanV1::from_agent_connection_planner_with_agent_access_grants(
            change_spec,
            control,
            vec![mutation],
            external,
        )
        .unwrap();
        let workspace = WorkspaceId::default();
        let idempotency =
            IdempotencyScopeV1::new("interactive-user", "agents.settings.apply", "journal")
                .unwrap();
        let request = CanonicalDigest::of_bytes(b"catalog-journal-request");
        OperationV1::new(
            OperationId::derive(&workspace, &idempotency, &request),
            workspace.clone(),
            idempotency,
            request,
            accept,
            RevisionSetV1 {
                target: 0,
                dependencies: BTreeMap::new(),
            },
            plan,
        )
        .unwrap()
    };
    let model_intent = |operation: &OperationV1| {
        operation
            .plan
            .external()
            .iter()
            .find(|intent| intent.effect_id() == "agent-connection-managed-configuration")
            .unwrap()
            .clone()
    };
    let plan_ids = json!(["plan/journal"]);

    // A plan-carrying grant references its own registered catalog artifact.
    let bound = operation(
        &grant,
        plan_ids.clone(),
        change(Some(catalog_digest.clone())),
        Some(catalog_digest.clone()),
    );
    let payload = settings_codex_model_file_for_operation(&bound, &model_intent(&bound)).unwrap();
    assert!(matches!(
        &payload.change,
        CodexModelFileAction::Configure { model_catalog: Some(digest), .. }
            if digest == &catalog_digest
    ));

    // A plan-carrying grant without any catalog reference is rejected.
    let missing = operation(&grant, plan_ids.clone(), change(None), None);
    assert!(settings_codex_model_file_for_operation(&missing, &model_intent(&missing)).is_err());

    // A reference to a catalog artifact the operation never registered is rejected.
    let dangling = operation(&grant, plan_ids, change(Some(catalog_digest.clone())), None);
    assert!(settings_codex_model_file_for_operation(&dangling, &model_intent(&dangling)).is_err());
}

#[test]
fn login_item_intent_binds_only_active_host_declarations() {
    use crate::agent_connection::{settings_login_item_effect, settings_login_item_intent};
    use hiroute_application_api::{
        AgentLoginItemDeclarationV2 as Declaration, AgentLoginItemStatusV2 as Status,
    };
    use hiroute_domain::{
        AgentConnectionControlIntentV1, AgentConnectionTransactionSubjectV1, CHANGE_SPEC_SCHEMA_V1,
        ChangeSpecV1,
    };
    const CONTEXT: &str = "agent-context/test";
    let change_spec = ChangeSpecV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        command_id: "agents.settings.apply".to_owned(),
        resource_id: Some(CONTEXT.to_owned()),
        desired_state: json!({
            "schema_version": {"major": 2, "minor": 0}, "context_id": CONTEXT,
            "model": {"intent": "configure", "settings": {
                "mode": "codex_default",
                "native_model_mode": "hiroute_only",
                "fixed_models": [], "allowed_plan_ids": ["plan/login-item"],
                "default_selection": {"kind": "plan", "plan_id": "plan/login-item"}
            }}
        }),
    };
    let control = AgentConnectionControlIntentV1::from_settings_planner(
        AgentConnectionTransactionSubjectV1::from_registered_profile(
            "agent_codex_default",
            "codex-responses-v1",
            "builtin/codex-responses/v1",
        )
        .unwrap(),
        &change_spec,
        false,
        &json!({"accept_digest": CanonicalDigest::of_bytes(b"login-item-accept")}),
    )
    .unwrap();
    let created = Declaration {
        before: Status::NotRegistered,
        after: Status::Enabled,
        created: true,
    };
    let intent = settings_login_item_intent(&control, CONTEXT, &created).unwrap();
    assert_eq!(intent.effect_id(), "agent-connection-login-item");
    assert_eq!(intent.kind(), hiroute_domain::OwnedEffectKind::LoginItem);
    let effect = settings_login_item_effect(&intent).unwrap();
    assert_eq!(effect.effect_id, "agent-connection-login-item");
    assert_eq!(effect.kind, hiroute_domain::OwnedEffectKind::LoginItem);
    assert_ne!(effect.before_fingerprint, effect.after_fingerprint);
    assert_eq!(
        *effect.compensation,
        json!({"revert":"unregister","owner":"desktop-host"})
    );
    // A pre-existing user login item is journaled without compensation metadata.
    let present = Declaration {
        before: Status::Enabled,
        after: Status::Enabled,
        created: false,
    };
    let effect = settings_login_item_effect(
        &settings_login_item_intent(&control, CONTEXT, &present).unwrap(),
    )
    .unwrap();
    assert_eq!(*effect.compensation, json!({}));
    // The last managed connection's restore releases only the item this feature owns: the
    // removal declaration proves the host left the item not active, and a definitively failed
    // restore replays the re-registration.
    let released = Declaration {
        before: Status::Enabled,
        after: Status::NotRegistered,
        created: false,
    };
    let effect = settings_login_item_effect(
        &settings_login_item_intent(&control, CONTEXT, &released).unwrap(),
    )
    .unwrap();
    assert_ne!(effect.before_fingerprint, effect.after_fingerprint);
    assert_eq!(
        *effect.compensation,
        json!({"revert":"register","owner":"desktop-host"})
    );
    // A removal declaration that leaves the item active never seals.
    let still_active = Declaration {
        before: Status::Enabled,
        after: Status::RequiresApproval,
        created: false,
    };
    assert!(
        settings_login_item_intent(&control, CONTEXT, &still_active).is_err(),
        "a release that leaves the login item active must not seal"
    );
    // Unapproved or failed registrations never seal: they stay service_unavailable.
    for declaration in [
        Declaration {
            before: Status::NotRegistered,
            after: Status::RequiresApproval,
            created: true,
        },
        Declaration {
            before: Status::RequiresApproval,
            after: Status::RequiresApproval,
            created: false,
        },
        Declaration {
            before: Status::NotRegistered,
            after: Status::Enabled,
            created: false,
        },
    ] {
        assert!(
            settings_login_item_intent(&control, CONTEXT, &declaration).is_err(),
            "an inactive or inconsistent declaration must not seal"
        );
    }
}
