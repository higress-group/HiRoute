use super::*;
use crate::compiler::test_fixtures::compilation_facts;

fn change() -> PlanContentChangeV2 {
    PlanContentChangeV2 {

        schema: PLAN_CONTENT_CHANGE_SCHEMA_V2.into(), target: PlanContentTargetV2::Create { creation_key: "editor-one".into() },
        editor: serde_json::from_value(serde_json::json!({
            "schema": PLAN_EDITOR_SCHEMA_V2, "display_name": "日常编码", "purpose": "辅助日常编码",
            "mode":"fixed_model", "candidates":[{"binding_id":"binding/free-b"}],
            "smart":{"economy":[],"primary":[],"judgment":hiroute_domain::JudgmentSettingsV1::default(),"reselect_on_user_message":false,"classifier":{"kind":"local_rules"},"complex_keywords":[]},
            "free":{"candidates":[],"primary":[],"primary_fallback":false},
            "delegation_enabled": false,
            "requirements":{},"limits":{"maximum_attempts":6,"request_timeout_ms":60000,"attempt_timeout_ms":30000}
        })).unwrap(), consumed_draft: None,
    }
}
fn state() -> PlanAuthoringSnapshotV2 {
    PlanAuthoringSnapshotV2 {
        workspace: WorkspaceId::default(),
        aliases: AliasRegistryV1::default(),
        active_publication: None,
        plan_heads: vec![],
        current_head: None,
        legacy_source: None,
        draft: None,
        facts: compilation_facts(),
        decision_service: None,
        expected_revisions: RevisionSetV1 {
            target: 1,
            dependencies: Default::default(),
        },
    }
}

#[test]
fn publication_requires_the_exact_saved_decision_revision_for_both_modes() {
    let service = DecisionServiceV1 {
        id: "decision/authoring".into(),
        revision: 1,
        name: "Saved extension".into(),
        connection: DecisionConnectionV1::Custom {
            endpoint: "https://extension.example/v1/decisions".into(),
            timeout_ms: 3000,
            auth_header: None,
        },
    };
    for mode in [PlanEditorMode::SmartSaving, PlanEditorMode::CustomBranches] {
        let mut change = change();
        let mut state = state();
        let classifier = ComplexityClassifierModeV1::DecisionService {
            service: Box::new(service.clone()),
        };
        change.editor.mode = mode;
        change.editor.smart.economy = change.editor.candidates.clone();
        change.editor.smart.primary = change.editor.candidates.clone();
        change.editor.smart.classifier = classifier.clone();
        change.editor.branch_routing = Some(BranchRoutingEditorV1 {
            classifier,
            branches: ["writing", "review"]
                .map(|id| RouteBranchV1 {
                    id: id.into(),
                    name: id.into(),
                    condition: format!("Handle {id}"),
                    candidates: change.editor.candidates.clone(),
                    primary_candidates: vec![],
                    judgment: None,
                })
                .into(),
            default_branch_id: "review".into(),
            judgment: Default::default(),
            reselect_on_user_message: false,
        });
        assert_eq!(
            preview_plan_content(&change, &state),
            Err(PlanPreviewError::InvalidDecisionService)
        );
        state.decision_service = Some(service.clone());
        let preview = preview_plan_content(&change, &state).unwrap();
        assert_eq!(
            preview.plan_version.configuration.decision_service(),
            Some(&service)
        );
        // Matching identity is insufficient: every frozen field must match its saved revision.
        state.decision_service.as_mut().unwrap().name = "Different saved content".into();
        assert_eq!(
            preview_plan_content(&change, &state),
            Err(PlanPreviewError::InvalidDecisionService)
        );
        state.decision_service = Some(service.clone());
        state.decision_service.as_mut().unwrap().revision = 2;
        assert_eq!(
            preview_plan_content(&change, &state),
            Err(PlanPreviewError::InvalidDecisionService)
        );
    }
}
#[test]
fn preview_is_read_only_and_seals_proposed_alias_and_registry() {
    let change = change();
    let mut state = state();
    let before = state.aliases.clone();
    let first = preview_plan_content(&change, &state).unwrap();
    assert_eq!(
        first.plan_head.model_alias.as_str(),
        "hiroute-richangbianma"
    );
    assert_eq!(before, state.aliases);
    assert_eq!(first, preview_plan_content(&change, &state).unwrap());
    state
        .aliases
        .allocate_custom(
            AgentPlanId::parse("plan/competitor").unwrap(),
            first.plan_head.model_alias.clone(),
        )
        .unwrap();
    let second = preview_plan_content(&change, &state).unwrap();
    assert_eq!(
        second.plan_head.model_alias.as_str(),
        "hiroute-richangbianma-2"
    );
    // Ordinary Publish confirms the exact edit and database revisions. Allocating another
    // alias through a real writer advances the target revision; model the same boundary here.
    assert_eq!(first.change_digest, second.change_digest);
    assert_ne!(first.alias_registry_digest, second.alias_registry_digest);
    state.expected_revisions.target += 1;
    let after_commit = preview_plan_content(&change, &state).unwrap();
    assert_ne!(first.change_digest, after_commit.change_digest);
    assert_eq!(second.plan_head, after_commit.plan_head);
}
#[test]
fn manual_alias_survives_name_changes_and_published_alias_cannot_change() {
    let mut change = change();
    let mut state = state();
    change.editor.custom_alias = Some("my-coding".into());
    let first = preview_plan_content(&change, &state).unwrap();
    change.editor.display_name = "资料整理".into();
    assert_eq!(
        preview_plan_content(&change, &state)
            .unwrap()
            .plan_head
            .model_alias
            .as_str(),
        "my-coding"
    );
    state
        .aliases
        .allocate_custom(
            first.plan_head.reference.plan_id.clone(),
            first.plan_head.model_alias.clone(),
        )
        .unwrap();
    state.current_head = Some(first.plan_head.clone());
    change.target = PlanContentTargetV2::Update {
        plan_id: first.plan_head.reference.plan_id,
        expected_head_revision: 1,
    };
    let updated = preview_plan_content(&change, &state).unwrap();
    assert_eq!(updated.plan_head.model_alias.as_str(), "my-coding");
    assert_eq!(updated.plan_head.reference.content_revision, 2);
    change.editor.custom_alias = Some("different".into());
    assert_eq!(
        preview_plan_content(&change, &state),
        Err(PlanPreviewError::AliasImmutable)
    );
}

#[test]
fn current_unversioned_publication_source_can_be_adopted_on_first_update() {
    let mut change = change();
    let mut state = state();
    let existing = preview_plan_content(&change, &state).unwrap();
    state
        .aliases
        .allocate_custom(
            existing.plan_head.reference.plan_id.clone(),
            existing.plan_head.model_alias.clone(),
        )
        .unwrap();
    state.current_head = Some(existing.plan_head.clone());
    state.plan_heads = vec![existing.plan_head.clone()];
    state.legacy_source = Some(existing.plan_version.clone());
    change.target = PlanContentTargetV2::Update {
        plan_id: existing.plan_head.reference.plan_id.clone(),
        expected_head_revision: existing.plan_head.head_revision,
    };

    let updated = preview_plan_content(&change, &state).unwrap();
    assert_eq!(updated.legacy_source, Some(existing.plan_version));
    assert_eq!(updated.plan_head.reference.content_revision, 2);
    assert_eq!(updated.plan_head.head_revision, 2);
}

#[test]
fn consumed_draft_binds_identity_and_revision_but_publishes_the_submitted_editor() {
    let mut change = change();
    let mut state = state();
    change.consumed_draft = Some(PlanDraftRefV1 {
        draft_id: "draft/one".into(),
        revision: 1,
    });
    assert_eq!(
        preview_plan_content(&change, &state),
        Err(PlanPreviewError::Stale)
    );
    state.draft = Some(PlanDraftV1 {
        schema: PLAN_DRAFT_SCHEMA_V1.into(),
        workspace_id: state.workspace.clone(),
        draft_id: "draft/one".into(),
        revision: 1,
        plan_id: None,
        base_head_revision: None,
        editor: change.editor.clone(),
    });
    let before = preview_plan_content(&change, &state).unwrap();
    change.editor.purpose = "Edited after saving the draft".into();
    let edited = preview_plan_content(&change, &state).unwrap();
    assert_eq!(
        edited.plan_version.configuration.purpose.as_str(),
        change.editor.purpose
    );
    assert_ne!(before.change_digest, edited.change_digest);
    assert_eq!(edited.consumed_draft, change.consumed_draft);
    let original = state.draft.as_ref().unwrap().clone();
    for mismatch in ["identity", "plan"] {
        let draft = state.draft.as_mut().unwrap();
        *draft = original.clone();
        match mismatch {
            "identity" => draft.draft_id = "draft/other".into(),
            "plan" => {
                draft.plan_id = Some(AgentPlanId::parse("plan/other").unwrap());
                draft.base_head_revision = Some(1);
            }
            _ => unreachable!(),
        }
        assert_eq!(
            preview_plan_content(&change, &state),
            Err(PlanPreviewError::Stale)
        );
    }
    state.draft = Some(original);
    state.draft.as_mut().unwrap().revision = 2;
    assert_eq!(
        preview_plan_content(&change, &state),
        Err(PlanPreviewError::Stale)
    );

    state
        .aliases
        .allocate_custom(
            before.plan_head.reference.plan_id.clone(),
            before.plan_head.model_alias.clone(),
        )
        .unwrap();
    state.current_head = Some(before.plan_head.clone());
    change.target = PlanContentTargetV2::Update {
        plan_id: before.plan_head.reference.plan_id.clone(),
        expected_head_revision: before.plan_head.head_revision,
    };
    let draft = state.draft.as_mut().unwrap();
    draft.revision = 1;
    draft.plan_id = Some(before.plan_head.reference.plan_id.clone());
    draft.base_head_revision = Some(before.plan_head.head_revision);
    preview_plan_content(&change, &state).unwrap();
    state.draft.as_mut().unwrap().base_head_revision = Some(before.plan_head.head_revision + 1);
    assert_eq!(
        preview_plan_content(&change, &state),
        Err(PlanPreviewError::Stale)
    );
}

#[test]
fn complete_content_operation_binds_the_version_head_and_publication() {
    assert_complete_content_operation(false, None, false);
}

#[test]
fn retained_native_operation_uses_frozen_semantics_and_still_rejects_tampering() {
    assert_complete_content_operation(true, None, false);
}

#[test]
fn retained_v1_native_operation_reconstructs_its_sealed_publication() {
    assert_complete_content_operation(false, None, true);
}

pub(crate) fn retained_v1_operation_plan() -> TransactionPlanV1 {
    assert_complete_content_operation(false, None, true)
}

#[test]
fn current_custom_operations_preserve_parked_source_and_reject_sealed_input_tampering() {
    for mode in [PlanEditorMode::SmartSaving, PlanEditorMode::CustomBranches] {
        assert_complete_content_operation(false, Some(mode), false);
    }
}

fn assert_complete_content_operation(
    legacy_native: bool,
    custom_mode: Option<PlanEditorMode>,
    sealed_v1: bool,
) -> TransactionPlanV1 {
    let mut change = change();
    let mut state = state();
    if legacy_native || sealed_v1 {
        let service = DecisionServiceV1 {
            id: "decision/retained-operation".into(),
            revision: 1,
            name: "Retained native".into(),
            connection: DecisionConnectionV1::SystemOne {
                provider: "compatible".into(),
                model: "synthetic".into(),
                endpoint: "https://decision.example/systemone".into(),
                timeout_ms: 3000,
                auth_header: ClassifierAuthHeaderV1 {
                    name: "Authorization".into(),
                    value_secret_ref: "fixture".into(),
                },
            },
        };
        change.editor.mode = PlanEditorMode::SmartSaving;
        change.editor.smart.economy = change.editor.candidates.clone();
        change.editor.smart.primary = change.editor.candidates.clone();
        change.editor.smart.classifier = ComplexityClassifierModeV1::DecisionService {
            service: Box::new(service.clone()),
        };
        state.decision_service = Some(service);
    }
    if let Some(mode) = custom_mode {
        let service = DecisionServiceV1 {
            id: "decision/current-operation".into(),
            revision: 1,
            name: "Current extension".into(),
            connection: DecisionConnectionV1::Custom {
                endpoint: "https://decision.example/choose".into(),
                timeout_ms: 3000,
                auth_header: None,
            },
        };
        let classifier = ComplexityClassifierModeV1::DecisionService {
            service: Box::new(service.clone()),
        };
        change.editor.mode = mode;
        change.editor.smart.classifier = classifier.clone();
        change.editor.smart.economy = change.editor.candidates.clone();
        change.editor.smart.primary = change.editor.candidates.clone();
        change.editor.smart.judgment.degree.instructions = "   ".into();
        change.editor.smart.judgment.competence.instructions = "  parked assessment  ".into();
        change.editor.smart.judgment.competence.floor_millis = 730;
        change.editor.branch_routing = Some(BranchRoutingEditorV1 {
            classifier,
            branches: ["code", "docs"]
                .map(|id| RouteBranchV1 {
                    id: id.into(),
                    name: id.into(),
                    condition: format!("Handle {id}"),
                    candidates: change.editor.candidates.clone(),
                    primary_candidates: if id == "code" {
                        change.editor.candidates.clone()
                    } else {
                        vec![]
                    },
                    judgment: Some(change.editor.smart.judgment.clone()),
                })
                .into(),
            default_branch_id: "docs".into(),
            judgment: change.editor.smart.judgment.clone(),
            reselect_on_user_message: false,
        });
        state.decision_service = Some(service);
    }
    let mut preview = preview_plan_content(&change, &state).unwrap();
    if custom_mode.is_some() {
        assert_eq!(
            preview.plan_version.configuration,
            change.editor.effective().unwrap()
        );
        let mut invalid = preview.plan_version.compiled.body.materialized.clone();
        match &mut invalid.request_owned {
            RequestOwnedRouteV1::Classified { judgment, .. } => {
                assert!(judgment.is_none());
                *judgment = Some(JudgmentSettingsV1::default());
            }
            RequestOwnedRouteV1::Branches { branches, .. } => {
                assert!(branches.iter().all(|b| b.judgment.is_none()));
                branches[0].judgment = Some(JudgmentSettingsV1::default());
            }
            _ => unreachable!(),
        }
        assert_eq!(invalid.validate(), Err(CompiledPlanError::InvalidGroups));
    }
    if legacy_native {
        // Construct the supported sealed legacy shape. The real released-producer
        // acceptance additionally authenticates actual v0.3.1 Operations on reopen.
        let mut configuration = preview.plan_version.configuration.clone();
        let AgentPlanStrategyV2::SmartSaving {
            native_protocol,
            judgment,
            ..
        } = &mut configuration.strategy
        else {
            unreachable!()
        };
        *native_protocol = None;
        *judgment = change.editor.smart.judgment.clone();
        let compiled = crate::compiler::compile_agent_plan_v2(
            preview.plan_version.compiled.body.identity.clone(),
            preview.plan_version.reference.content_revision,
            &configuration,
            &state.facts,
        )
        .unwrap();
        preview.plan_version =
            PlanVersionV1::new(state.workspace.clone(), configuration, compiled).unwrap();
        preview.plan_head.reference = preview.plan_version.reference.clone();
        assert!(
            change
                .editor
                .effective()
                .unwrap()
                .native_protocol()
                .is_some()
        );
        assert!(
            preview
                .plan_version
                .configuration
                .native_protocol()
                .is_none()
        );
    }
    if sealed_v1 {
        let baseline: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../domain/src/routing/fixtures/baseline-c-default-publication.json"
        ))
        .unwrap();
        let mut configuration = preview.plan_version.configuration.clone();
        let AgentPlanStrategyV2::SmartSaving {
            native_protocol,
            judgment,
            ..
        } = &mut configuration.strategy
        else {
            unreachable!()
        };
        *native_protocol = Some(NativeProtocolSelectionV1 {
            schema: SYSTEM_ONE_C_V1.into(),
            prior_context_max_turns: 8,
        });
        *judgment = serde_json::from_value(baseline["judgment"].clone()).unwrap();
        let compiled = crate::compiler::compile_agent_plan_v2(
            preview.plan_version.compiled.body.identity.clone(),
            preview.plan_version.reference.content_revision,
            &configuration,
            &state.facts,
        )
        .unwrap();
        let RequestOwnedRouteV1::Classified {
            native_protocol: Some(frozen),
            ..
        } = &compiled.body.materialized.request_owned
        else {
            panic!("sealed v1 protocol missing")
        };
        assert_eq!(serde_json::to_value(frozen).unwrap(), baseline["frozen"]);
        preview.plan_version =
            PlanVersionV1::new(state.workspace.clone(), configuration, compiled).unwrap();
        preview.plan_head.reference = preview.plan_version.reference.clone();
    }
    let mut aliases = state.aliases.clone();
    aliases
        .allocate_custom(
            preview.plan_head.reference.plan_id.clone(),
            preview.plan_head.model_alias.clone(),
        )
        .unwrap();
    let revision = GatewayPublicationRevision::new(1).unwrap();
    let publication = GatewayPublicationV1::new(
        state.workspace.clone(),
        revision,
        AliasRegistryV1::default(),
        vec![],
    )
    .unwrap()
    .next_with_plan_content(
        revision,
        aliases,
        preview.plan_version.compiled.clone(),
        vec![preview.plan_head.clone()],
    )
    .unwrap();
    let record =
        PublicationRecordV1::from_publication(state.workspace.clone(), &publication).unwrap();
    let spec = ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: "routing.apply".into(),
        resource_id: Some(format!(
            "agent-plan/{}",
            preview.plan_head.reference.plan_id.as_str()
        )),
        desired_state: serde_json::to_value(&change).unwrap(),
    };
    let plan = TransactionPlanV1::from_plan_content_planner(
        spec,
        preview.plan_version.clone(),
        preview.plan_head.clone(),
        None,
        None,
        record.clone(),
        None,
    )
    .unwrap();
    let content = plan.plan_content_control().unwrap().unwrap();
    assert_eq!(content.plan_version, preview.plan_version);
    assert_eq!(
        routing_publication_record(&plan.external()[0]).unwrap(),
        Some(record)
    );
    let roundtrip = TransactionPlanV1::from_registered_typed_planner(
        plan.spec().clone(),
        plan.control().clone(),
        None,
        vec![],
        vec![],
        plan.external().to_vec(),
    )
    .unwrap();
    assert_eq!(roundtrip.plan_content_control().unwrap().unwrap(), content);
    if sealed_v1 {
        let stored = StoredOperationInputV1::freeze(&plan).unwrap();
        let bytes = serde_json::to_vec(&stored).unwrap();
        let reopened: StoredOperationInputV1 = serde_json::from_slice(&bytes).unwrap();
        let rebuilt = reopened.build().unwrap();
        assert_eq!(rebuilt, plan);
        assert_eq!(serde_json::to_vec(&reopened).unwrap(), bytes);
        assert_eq!(
            rebuilt
                .plan_content_control()
                .unwrap()
                .unwrap()
                .plan_version
                .configuration
                .native_protocol()
                .unwrap()
                .schema,
            SYSTEM_ONE_C_V1
        );
    }
    // Alter both envelope copies and the change digest consistently: rejection must
    // still come from the editor-to-sealed-configuration check.
    let mut tampered = plan.spec().clone();
    tampered.desired_state["editor"]["purpose"] = serde_json::json!("Changed after sealing");
    let mut control = plan.control().clone();
    control["change_spec_digest"] =
        serde_json::to_value(CanonicalDigest::of(&tampered).unwrap()).unwrap();
    let mut effect = serde_json::to_value(&plan.external()[0]).unwrap();
    effect["desired"]["change_spec"] = serde_json::to_value(&tampered).unwrap();
    effect["desired"]["control"] = control;
    assert!(
        ExternalEffectIntentV1::from_registered_adapter(
            effect["effect_id"].as_str().unwrap(),
            serde_json::from_value(effect["kind"].clone()).unwrap(),
            effect["target"].as_str().unwrap(),
            serde_json::from_value(effect["before_fingerprint"].clone()).unwrap(),
            effect["desired"].clone(),
            effect["desired_mode"].as_u64().unwrap() as u32,
            effect["sensitive"].as_bool().unwrap(),
        )
        .is_err()
    );
    let mut changed = plan.control().clone();
    changed["plan_head"]["head_revision"] = serde_json::json!(99);
    assert!(
        TransactionPlanV1::from_registered_typed_planner(
            plan.spec().clone(),
            changed,
            None,
            vec![],
            vec![],
            plan.external().to_vec(),
        )
        .is_err()
    );
    plan
}

#[test]
fn ordinary_publication_confirmation_binds_edit_target_versions_and_draft_intent() {
    let original = change();
    let revisions = state().expected_revisions;
    let digest = plan_content_confirmation_digest(&original, &revisions).unwrap();
    let mut edited = original.clone();
    edited.editor.purpose = "another purpose".into();
    assert_ne!(
        digest,
        plan_content_confirmation_digest(&edited, &revisions).unwrap()
    );
    edited = original.clone();
    edited.target = PlanContentTargetV2::Create {
        creation_key: "another-plan".into(),
    };
    assert_ne!(
        digest,
        plan_content_confirmation_digest(&edited, &revisions).unwrap()
    );
    let mut new_revisions = revisions.clone();
    new_revisions.target += 1;
    assert_ne!(
        digest,
        plan_content_confirmation_digest(&original, &new_revisions).unwrap()
    );
    // Every wire field is part of the confirmation, including optional draft ownership.
    let mut json = serde_json::to_value(&original).unwrap();
    json["consumed_draft"] = serde_json::json!({"draft_id":"draft/confirmation", "revision":3});
    let with_draft: PlanContentChangeV2 = serde_json::from_value(json).unwrap();
    assert_ne!(
        digest,
        plan_content_confirmation_digest(&with_draft, &revisions).unwrap()
    );
}

#[test]
fn ordinary_preview_resolves_defaults_and_seals_concrete_contents_without_review() {
    let mut change = change();
    let mut state = state();
    let service = DecisionServiceV1 {
        id: "decision/preview".into(),
        revision: 1,
        name: "Fixture".into(),
        connection: DecisionConnectionV1::SystemOne {
            provider: "compatible".into(),
            model: "fixture".into(),
            endpoint: "https://decision.example/systemone".into(),
            timeout_ms: 3000,
            auth_header: ClassifierAuthHeaderV1 {
                name: "Authorization".into(),
                value_secret_ref: "fixture".into(),
            },
        },
    };
    change.editor.mode = PlanEditorMode::SmartSaving;
    change.editor.smart.classifier = ComplexityClassifierModeV1::DecisionService {
        service: Box::new(service.clone()),
    };
    change.editor.smart.economy = change.editor.candidates.clone();
    change.editor.smart.primary = change.editor.candidates.clone();
    state.decision_service = Some(service);
    let legacy = preview_plan_content(&change, &state).unwrap();
    let old_bytes = serde_json::to_vec(&legacy.plan_version).unwrap();
    assert_eq!(
        legacy.change_digest,
        plan_content_confirmation_digest(&change, &state.expected_revisions).unwrap()
    );
    let enabled = preview_plan_content(&change, &state).unwrap();
    enabled.plan_version.validate().unwrap();
    let encoded = serde_json::to_value(&enabled).unwrap();
    assert!(encoded.get("assessment_transition").is_none());
    assert!(
        encoded["plan_version"]
            .get("assessment_transition")
            .is_none()
    );
    assert_eq!(
        enabled.change_digest,
        plan_content_confirmation_digest(&change, &state.expected_revisions).unwrap()
    );
    let route = &enabled
        .plan_version
        .compiled
        .body
        .materialized
        .request_owned;
    let RequestOwnedRouteV1::Classified {
        native_protocol: Some(frozen),
        ..
    } = route
    else {
        panic!("native execution missing")
    };
    assert_eq!(frozen.scopes[SMART_SAVING_SCOPE_ID].standard, "default");
    let protocol = NativeProtocolV1::default();
    let mut current = JudgmentSettingsV1::default();
    protocol.current_assessment(&mut current);
    assert_eq!(
        frozen.scopes[SMART_SAVING_SCOPE_ID].assessment,
        composed_assessment(Some(&protocol), &current)
    );
    assert_eq!(frozen.protocol.fixed, NativeFixedContentV1::c_v2());
    let reopened = enabled.plan_version.configuration.editor(None).unwrap();
    change.editor = reopened;
    let off = preview_plan_content(&change, &state).unwrap();
    off.plan_version.validate().unwrap();
    assert_eq!(
        off.plan_version.configuration.assessment_scopes()[SMART_SAVING_SCOPE_ID]
            .competence
            .instructions,
        current.competence.instructions
    );
    assert_eq!(serde_json::to_vec(&legacy.plan_version).unwrap(), old_bytes);
}
