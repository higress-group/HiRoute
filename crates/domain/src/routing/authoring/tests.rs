use super::*;

fn editor() -> PlanEditorStateV2 {
    serde_json::from_value(serde_json::json!({
        "schema": PLAN_EDITOR_SCHEMA_V2, "display_name": "代码整理", "purpose": "整理代码",
        "mode": "fixed_model", "candidates": [{"binding_id":"binding/a"}],
        "smart": {"economy":[],"primary":[],"primary_fallback":false,"classifier":{"kind":"local_rules"},"complex_keywords":[]},
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
fn smart_rest_classifier_survives_editor_to_authoring_projection() {
    let mut draft = editor();
    draft.mode = PlanEditorMode::SmartSaving;
    draft.smart.economy = draft.candidates.clone();
    draft.smart.primary = draft.candidates.clone();
    draft.smart.classifier = ComplexityClassifierModeV1::Rest {
        endpoint: "https://classifier.example/v1/branch".into(),
        timeout_ms: crate::DEFAULT_REST_CLASSIFIER_TIMEOUT_MS,
        auth_header: Some(crate::ClassifierAuthHeaderV1 {
            name: "Authorization".into(),
            value_secret_ref: "classifier/main".into(),
        }),
    };
    let effective = draft.effective().unwrap();
    assert!(matches!(
        effective.strategy,
        AgentPlanStrategyV2::SmartSaving {
            classifier: ComplexityClassifierModeV1::Rest { endpoint, .. },
            ..
        } if endpoint == "https://classifier.example/v1/branch"
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
