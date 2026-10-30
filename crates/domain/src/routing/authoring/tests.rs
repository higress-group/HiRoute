use super::*;

fn editor() -> PlanEditorStateV2 {
    serde_json::from_value(serde_json::json!({
        "schema": PLAN_EDITOR_SCHEMA_V2, "display_name": "代码整理", "purpose": "整理代码",
        "mode": "fixed_model", "candidates": [{"binding_id":"binding/a"}],
        "smart": {"economy":[],"primary":[],"judgment":crate::JudgmentSettingsV1::default(),"reselect_on_user_message":false,"classifier":{"kind":"local_rules"},"complex_keywords":[]},
        "free": {"candidates":[],"primary":[],"primary_fallback":false},
        "delegation_enabled": false,
        "requirements":{}, "limits":{"maximum_attempts":6,"request_timeout_ms":60000,"attempt_timeout_ms":30000}
    })).unwrap()
}

#[test]
fn incomplete_parked_modes_survive_and_only_selected_mode_is_published() {
    let mut draft = editor();
    draft.smart.economy = vec![CandidateSelectionV1 {
        binding_id: "".into(),
        reasoning: None,
    }];
    draft.validate_draft().unwrap();
    assert!(draft.effective().is_ok());
    draft.mode = PlanEditorMode::SmartSaving;
    assert!(draft.effective().is_err());
    draft.mode = PlanEditorMode::FixedModel;
    assert_eq!(draft.smart.economy[0].binding_id, "");
    assert!(draft.effective().is_ok());
}

#[test]
fn unfinished_judgment_can_be_saved_but_cannot_be_published() {
    let mut draft = editor();
    draft.mode = PlanEditorMode::SmartSaving;
    draft.smart.economy = draft.candidates.clone();
    draft.smart.primary = draft.candidates.clone();
    draft.smart.judgment.degree.simple.clear();
    draft.smart.judgment.competence.criteria[1].clear();
    draft.validate_draft().unwrap();
    assert!(draft.effective().is_err());
    draft.smart.judgment = Default::default();
    assert!(draft.effective().is_ok());
    draft.smart.judgment.degree.simple = "界".repeat(20_000);
    draft.validate_draft().unwrap();
    let authored = draft.effective().unwrap();
    let AgentPlanStrategyV2::SmartSaving { judgment, .. } = authored.strategy else {
        panic!("smart saving");
    };
    assert_eq!(judgment.degree.simple, draft.smart.judgment.degree.simple);
    draft.smart.judgment.degree.simple = "bad\u{0000}prompt".into();
    assert!(draft.validate_draft().is_err());
}

#[test]
fn fixed_model_keeps_an_explicit_order_and_delegation_requires_a_worker() {
    let mut draft = editor();
    draft.candidates.push(CandidateSelectionV1 {
        binding_id: "binding/b".into(),
        reasoning: None,
    });
    let effective = draft.effective().unwrap();
    assert!(matches!(
        effective.strategy,
        AgentPlanStrategyV2::Custom { candidates }
            if candidates.iter().map(|candidate| candidate.binding_id.as_str()).collect::<Vec<_>>()
                == ["binding/a", "binding/b"]
    ));
    draft.delegation_enabled = true;
    assert!(draft.effective().is_err());
    draft.work = Some(WorkerPlanV1 {
        harness: WorkerHarnessV1::CodexCli,
        protocol: crate::AgentIngressProtocolV1::Responses,
    });
    assert!(draft.effective().is_ok());
}

#[test]
fn smart_and_free_fallback_are_explicit_and_disabled_groups_are_parked() {
    let mut draft = editor();
    draft.mode = PlanEditorMode::FreeFirst;
    draft.free.candidates = draft.candidates.clone();
    draft.free.primary = vec![CandidateSelectionV1 {
        binding_id: "".into(),
        reasoning: None,
    }];
    let effective = draft.effective().unwrap();
    assert!(
        matches!(effective.strategy, AgentPlanStrategyV2::FreeFirst { primary, .. } if primary.is_empty())
    );
    draft.free.primary_fallback = true;
    assert!(draft.effective().is_err());
}

#[test]
fn saved_classifier_survives_editor_to_authoring_projection() {
    let mut draft = editor();
    draft.mode = PlanEditorMode::SmartSaving;
    draft.smart.reselect_on_user_message = true;
    draft.smart.economy = draft.candidates.clone();
    draft.smart.primary = draft.candidates.clone();
    draft.smart.classifier = ComplexityClassifierModeV1::DecisionService {
        service: Box::new(crate::DecisionServiceV1 {
            id: "decision-fixture".into(),
            revision: 1,
            name: "Fixture extension".into(),
            connection: crate::DecisionConnectionV1::Custom {
                endpoint: "https://classifier.example/v1/branch".into(),
                timeout_ms: crate::DEFAULT_REST_CLASSIFIER_TIMEOUT_MS,
                auth_header: Some(crate::ClassifierAuthHeaderV1 {
                    name: "Authorization".into(),
                    value_secret_ref: "classifier/main".into(),
                }),
            },
        }),
    };
    let effective = draft.effective().unwrap();
    assert!(
        effective
            .editor(None)
            .unwrap()
            .smart
            .reselect_on_user_message
    );
    assert!(matches!(
        effective.strategy,
        AgentPlanStrategyV2::SmartSaving {
            classifier: ComplexityClassifierModeV1::DecisionService { service },
            reselect_on_user_message: true,
            ..
        } if service.connection.transport().0 == "https://classifier.example/v1/branch"
    ));
}

#[test]
fn draft_bounds_do_not_require_a_publishable_name_purpose_or_budget() {
    let mut draft = editor();
    draft.display_name.clear();
    draft.purpose.clear();
    draft.candidates[0].reasoning = None;
    draft.validate_draft().unwrap();
    assert!(draft.effective().is_err());
    draft.candidates = vec![draft.candidates[0].clone(); 129];
    assert!(draft.validate_draft().is_err());
}

#[test]
fn context_window_round_trips_without_changing_omitted_legacy_limits() {
    let mut value = editor();
    let old = serde_json::to_value(&value).unwrap();
    assert!(old["limits"].get("context_window_tokens").is_none());
    value.limits.context_window_tokens = Some(500_000);
    let saved: PlanEditorStateV2 =
        serde_json::from_value(serde_json::to_value(&value).unwrap()).unwrap();
    assert_eq!(
        saved.effective().unwrap().limits.context_window_tokens,
        Some(500_000)
    );
    value.limits.context_window_tokens = None;
    assert_eq!(serde_json::to_value(&value).unwrap(), old);
    for invalid in [serde_json::json!(-1), serde_json::json!(1.5)] {
        let mut wire = old.clone();
        wire["limits"]["context_window_tokens"] = invalid;
        assert!(serde_json::from_value::<PlanEditorStateV2>(wire).is_err());
    }
}

#[test]
fn retired_follow_up_field_can_be_omitted_without_rewriting_legacy_values() {
    let released: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/released-plan-editor.v2.json")).unwrap();
    let mut draft: PlanEditorStateV2 = serde_json::from_value(released["editor"].clone()).unwrap();
    draft.validate_draft().unwrap();
    draft.mode = PlanEditorMode::SmartSaving;
    draft.smart.economy = draft.candidates.clone();
    draft.smart.primary = draft.candidates.clone();
    for legacy in [false, true] {
        draft.smart.reselect_on_user_message = legacy;
        let bytes = serde_json::to_vec(&draft).unwrap();
        let reopened: PlanEditorStateV2 = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(serde_json::to_vec(&reopened).unwrap(), bytes);
        let authored = reopened.effective().unwrap();
        assert_eq!(
            authored
                .editor(None)
                .unwrap()
                .smart
                .reselect_on_user_message,
            legacy
        );
    }
    let mut omitted = serde_json::to_value(&draft).unwrap();
    omitted["smart"]
        .as_object_mut()
        .unwrap()
        .remove("reselect_on_user_message");
    let current: PlanEditorStateV2 = serde_json::from_value(omitted.clone()).unwrap();
    assert!(!current.smart.reselect_on_user_message);
    let mut authoring = serde_json::to_value(current.effective().unwrap()).unwrap();
    authoring["strategy"]
        .as_object_mut()
        .unwrap()
        .remove("reselect_on_user_message");
    let authoring: AgentPlanAuthoringV2 = serde_json::from_value(authoring).unwrap();
    authoring.validate().unwrap();
    assert!(
        !authoring
            .editor(None)
            .unwrap()
            .smart
            .reselect_on_user_message
    );
    omitted["smart"]["reselect_on_user_message"] = serde_json::json!("true");
    assert!(serde_json::from_value::<PlanEditorStateV2>(omitted).is_err());
}

fn native_editor() -> PlanEditorStateV2 {
    use crate::*;
    let mut value = editor();
    value.mode = PlanEditorMode::SmartSaving;
    value.smart.economy = value.candidates.clone();
    value.smart.primary = value.candidates.clone();
    value.smart.classifier = ComplexityClassifierModeV1::DecisionService {
        service: Box::new(DecisionServiceV1 {
            id: "decision/defaults".into(),
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
        }),
    };
    value
}

#[test]
fn released_defaults_move_only_at_next_publication_without_editor_controls() {
    use crate::*;
    let mut editor = native_editor();
    let released: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/released-default-assessment.v1.json")).unwrap();
    editor.smart.judgment.competence.instructions = released["assessment"]["instructions"]
        .as_str()
        .unwrap()
        .into();
    editor.smart.judgment.competence.criteria = std::array::from_fn(|i| {
        released["assessment"]["criteria"][i]
            .as_str()
            .unwrap()
            .into()
    });
    editor.smart.judgment.degree.simple_threshold_millis = 913;
    editor.smart.judgment.competence.floor_millis = 612;
    let before = serde_json::to_vec(&editor).unwrap();
    let current = editor.effective().unwrap();
    let j = current.assessment_scopes()[SMART_SAVING_SCOPE_ID];
    let protocol = NativeProtocolV1::default();
    let mut expected = JudgmentSettingsV1::default();
    protocol.current_assessment(&mut expected);
    assert_eq!(
        composed_assessment(Some(&protocol), j),
        composed_assessment(Some(&protocol), &expected)
    );
    assert_eq!(j.competence.standard, Some(AssessmentStandardV1::Default));
    assert_eq!(
        (j.degree.simple_threshold_millis, j.competence.floor_millis),
        (913, 612)
    );
    assert_eq!(
        current.native_protocol(),
        Some(&NativeProtocolSelectionV1::default())
    );
    assert_eq!(serde_json::to_vec(&editor).unwrap(), before);
    let readback = current.editor(None).unwrap();
    assert_eq!(readback.effective().unwrap(), current);
    let json = serde_json::to_value(&readback).unwrap();
    for field in [
        "native_protocol",
        "historical_applicability",
        "prior_context_max_turns",
    ] {
        assert!(json["smart"].get(field).is_none(), "{field}");
        let mut invalid = json.clone();
        invalid["smart"][field] = serde_json::json!({"prior_context_max_turns":8});
        assert!(
            serde_json::from_value::<PlanEditorStateV2>(invalid).is_err(),
            "{field}"
        );
    }
    // A sealed earlier publication is not normalized on read, but its next edit is current.
    let mut legacy = current;
    if let AgentPlanStrategyV2::SmartSaving {
        native_protocol,
        judgment,
        ..
    } = &mut legacy.strategy
    {
        *native_protocol = None;
        *judgment = JudgmentSettingsV1::default();
    }
    let old = serde_json::to_vec(&legacy).unwrap();
    let reopened: AgentPlanAuthoringV2 = serde_json::from_slice(&old).unwrap();
    assert_eq!(serde_json::to_vec(&reopened).unwrap(), old);
    assert_eq!(
        native_editor()
            .effective_for_frozen_validation(&reopened)
            .unwrap(),
        reopened
    );
    let next = reopened.editor(None).unwrap().effective().unwrap();
    assert!(next.native_protocol().is_some());
    assert_eq!(serde_json::to_vec(&legacy).unwrap(), old);
}

#[test]
fn retained_native_operation_validates_its_original_default_and_new_edit_uses_current_text() {
    use crate::*;
    for source in [
        include_str!("../fixtures/baseline-c-default-publication.json"),
        include_str!("../fixtures/baseline-c-v2-default-publication.json"),
    ] {
        let fixture: serde_json::Value = serde_json::from_str(source).unwrap();
        let old: JudgmentSettingsV1 = serde_json::from_value(fixture["judgment"].clone()).unwrap();
        let editor = native_editor();
        let mut frozen = editor.effective().unwrap();
        let AgentPlanStrategyV2::SmartSaving {
            judgment,
            native_protocol,
            ..
        } = &mut frozen.strategy
        else {
            unreachable!()
        };
        *native_protocol = Some(NativeProtocolSelectionV1 {
            schema: fixture["frozen"]["protocol"]["schema"]
                .as_str()
                .unwrap()
                .into(),
            prior_context_max_turns: 8,
        });
        *judgment = old.clone();
        let before = serde_json::to_vec(&frozen).unwrap();
        assert_eq!(
            editor.effective_for_frozen_validation(&frozen).unwrap(),
            frozen
        );
        assert_eq!(serde_json::to_vec(&frozen).unwrap(), before);
        let current = editor.effective().unwrap();
        assert_eq!(current.native_protocol().unwrap().schema, SYSTEM_ONE_C_V2);
        assert_ne!(
            current.assessment_scopes()[SMART_SAVING_SCOPE_ID]
                .competence
                .instructions,
            old.competence.instructions
        );
        let mut custom = editor.clone();
        custom.smart.judgment.competence.standard = Some(AssessmentStandardV1::Custom);
        custom.smart.judgment.competence.instructions = "Custom user text".into();
        assert_ne!(
            custom.effective_for_frozen_validation(&frozen).unwrap(),
            frozen
        );
    }
}

#[test]
fn retained_native_branch_operation_preserves_default_and_override_stock_questions() {
    use crate::*;
    for source in [
        include_str!("../fixtures/baseline-c-default-publication.json"),
        include_str!("../fixtures/baseline-c-v2-default-publication.json"),
    ] {
        let fixture: serde_json::Value = serde_json::from_str(source).unwrap();
        let old: JudgmentSettingsV1 = serde_json::from_value(fixture["judgment"].clone()).unwrap();
        let mut editor = native_editor();
        editor.mode = PlanEditorMode::CustomBranches;
        editor.branch_routing = Some(BranchRoutingEditorV1 {
            classifier: editor.smart.classifier.clone(),
            default_branch_id: "general".into(),
            judgment: JudgmentSettingsV1::default(),
            reselect_on_user_message: false,
            branches: [
                ("code", Some(JudgmentSettingsV1::default())),
                ("general", None),
            ]
            .into_iter()
            .map(|(id, judgment)| RouteBranchV1 {
                id: id.into(),
                name: id.into(),
                condition: format!("Handle {id}"),
                candidates: editor.candidates.clone(),
                primary_candidates: vec![],
                judgment,
            })
            .collect(),
        });
        let mut frozen = editor.effective().unwrap();
        let AgentPlanStrategyV2::Branches { routing } = &mut frozen.strategy else {
            unreachable!()
        };
        routing.native_protocol = Some(NativeProtocolSelectionV1 {
            schema: fixture["frozen"]["protocol"]["schema"]
                .as_str()
                .unwrap()
                .into(),
            prior_context_max_turns: 8,
        });
        routing.judgment = old.clone();
        routing.branches[0].judgment = Some(old);
        assert_eq!(
            editor.effective_for_frozen_validation(&frozen).unwrap(),
            frozen
        );
        let next = editor.effective().unwrap();
        assert_eq!(next.native_protocol().unwrap().schema, SYSTEM_ONE_C_V2);
        assert_ne!(
            next.assessment_scopes()["default"].competence.instructions,
            frozen.assessment_scopes()["default"]
                .competence
                .instructions
        );
        assert_ne!(
            next.assessment_scopes()["branch/code"]
                .competence
                .instructions,
            frozen.assessment_scopes()["branch/code"]
                .competence
                .instructions
        );
    }
}

#[test]
fn custom_text_and_criteria_remain_byte_exact_even_when_equal_to_a_default() {
    use crate::*;
    for equal_default in [false, true] {
        let mut value = native_editor();
        value.smart.judgment.competence.standard = Some(AssessmentStandardV1::Custom);
        if !equal_default {
            value.smart.judgment.competence.instructions =
                "  Preserve this line.\nSecond line.  ".into();
            value.smart.judgment.competence.criteria[1] = " criterion with spaces  ".into();
        }
        let original = value.smart.judgment.competence.clone();
        for _ in 0..3 {
            let authored = value.effective().unwrap();
            assert_eq!(
                authored.assessment_scopes()[SMART_SAVING_SCOPE_ID].competence,
                original
            );
            value = authored.editor(None).unwrap();
        }
    }
    let mut value = native_editor();
    value.smart.judgment.competence.criteria[2].push(' ');
    assert_eq!(
        value.effective().unwrap().assessment_scopes()[SMART_SAVING_SCOPE_ID]
            .competence
            .standard,
        Some(AssessmentStandardV1::Custom)
    );
    value.smart.judgment.competence.instructions =
        "Use state.visible_conversation as evidence".into();
    assert!(
        matches!(value.effective(),Err(PlanAuthoringError::NativeProtocol(NativeProtocolError::StateReference(field))) if field=="smart_saving.competence.instructions")
    );
}

#[test]
fn inherited_defaults_and_independent_custom_overrides_resolve_separately() {
    use crate::*;
    let mut value = native_editor();
    value.mode = PlanEditorMode::CustomBranches;
    let mut custom = JudgmentSettingsV1::default();
    custom.competence.criteria[0].push_str(" Custom.");
    value.branch_routing = Some(BranchRoutingEditorV1 {
        classifier: value.smart.classifier.clone(),
        default_branch_id: "code".into(),
        judgment: JudgmentSettingsV1::default(),
        reselect_on_user_message: false,
        branches: [("code", None), ("default", Some(custom.clone()))]
            .into_iter()
            .map(|(id, judgment)| RouteBranchV1 {
                id: id.into(),
                name: id.into(),
                condition: format!("Handle {id}"),
                candidates: value.candidates.clone(),
                primary_candidates: vec![],
                judgment,
            })
            .collect(),
    });
    let authored = value.effective().unwrap();
    let internal = serde_json::to_value(&authored).unwrap();
    assert_eq!(
        internal["strategy"]["routing"]["native_protocol"]["schema"],
        SYSTEM_ONE_C_V2
    );
    let editor = serde_json::to_value(authored.editor(None).unwrap()).unwrap();
    assert!(
        editor["branch_routing"]
            .get("historical_applicability")
            .is_none()
    );
    let mut invalid = editor.clone();
    invalid["branch_routing"]["historical_applicability"] =
        serde_json::json!({"prior_context_max_turns":8});
    assert!(serde_json::from_value::<PlanEditorStateV2>(invalid).is_err());
    assert!(editor["branch_routing"].get("native_protocol").is_none());
    assert_eq!(
        serde_json::from_value::<PlanEditorStateV2>(editor)
            .unwrap()
            .effective()
            .unwrap(),
        authored
    );
    let scopes = authored.assessment_scopes();
    assert_eq!(
        scopes["default"].competence.standard,
        Some(AssessmentStandardV1::Default)
    );
    assert_eq!(
        scopes["branch/default"].competence.criteria,
        custom.competence.criteria
    );
    assert_eq!(
        scopes["branch/default"].competence.standard,
        Some(AssessmentStandardV1::Custom)
    );
}

#[test]
fn local_rules_and_custom_extensions_keep_their_assessment_and_omission() {
    use crate::*;
    let mut editor = native_editor();
    let mut service = match &editor.smart.classifier {
        ComplexityClassifierModeV1::DecisionService { service } => service.clone(),
        _ => unreachable!(),
    };
    service.connection = DecisionConnectionV1::Custom {
        endpoint: "https://decision.example/v1/decisions".into(),
        timeout_ms: 3000,
        auth_header: None,
    };
    for classifier in [
        ComplexityClassifierModeV1::LocalRules,
        ComplexityClassifierModeV1::DecisionService { service },
    ] {
        editor.smart.classifier = classifier;
        let current = editor.effective().unwrap();
        assert!(current.native_protocol().is_none());
        assert_eq!(
            current.assessment_scopes()[SMART_SAVING_SCOPE_ID],
            &editor.smart.judgment
        );
        assert!(
            serde_json::to_value(&current).unwrap()["strategy"]
                .get("native_protocol")
                .is_none()
        );
    }
}

#[test]
fn custom_extension_parks_unfinished_prompts_without_erasing_them() {
    use crate::*;
    let mut editor = native_editor();
    let native = editor.smart.classifier.clone();
    let ComplexityClassifierModeV1::DecisionService { service } = &mut editor.smart.classifier
    else {
        unreachable!()
    };
    service.connection = DecisionConnectionV1::Custom {
        endpoint: "https://decision.example/v1/decisions".into(),
        timeout_ms: 3000,
        auth_header: None,
    };
    editor.smart.judgment.degree.instructions = "   ".into();
    editor.smart.judgment.competence.criteria[1] = "\n\t".into();
    let parked = editor.smart.judgment.clone();
    editor.validate_draft().unwrap();
    let effective = editor.effective().unwrap();
    assert_eq!(
        effective.assessment_scopes()[SMART_SAVING_SCOPE_ID],
        &parked
    );
    editor.smart.classifier = native;
    assert!(editor.effective().is_err());
    assert_eq!(editor.smart.judgment, parked);
}

#[test]
fn extension_branch_overrides_keep_parked_whitespace_and_restore_native_validation() {
    use crate::*;
    let mut value = native_editor();
    value.mode = PlanEditorMode::CustomBranches;
    let native = value.smart.classifier.clone();
    let ComplexityClassifierModeV1::DecisionService { service } = &mut value.smart.classifier
    else {
        unreachable!()
    };
    service.connection = DecisionConnectionV1::Custom {
        endpoint: "https://decision.example/v1/decisions".into(),
        timeout_ms: 3000,
        auth_header: None,
    };
    let mut parked = JudgmentSettingsV1::default();
    parked.degree.instructions = "   ".into();
    parked.competence.criteria[0] = "\n\t".into();
    value.branch_routing = Some(BranchRoutingEditorV1 {
        classifier: value.smart.classifier.clone(),
        default_branch_id: "code".into(),
        judgment: parked.clone(),
        reselect_on_user_message: false,
        branches: ["code", "review"]
            .into_iter()
            .map(|id| RouteBranchV1 {
                id: id.into(),
                name: id.into(),
                condition: format!("Handle {id}"),
                candidates: value.candidates.clone(),
                primary_candidates: value.candidates.clone(),
                judgment: Some(parked.clone()),
            })
            .collect(),
    });
    value.effective().unwrap();
    let before = value.branch_routing.as_ref().unwrap().branches.clone();
    value.branch_routing.as_mut().unwrap().classifier = native;
    assert!(value.effective().is_err());
    assert_eq!(value.branch_routing.as_ref().unwrap().branches, before);
}
