use super::*;

fn candidate(id: &str, reasoning: NativeReasoningCapabilityV1) -> PlanCandidateOptionV1 {
    PlanCandidateOptionV1 {
        binding_id: id.into(),
        model_configuration_id: format!("model/{id}"),
        display_name: id.into(),
        reasoning,
        billing_class: BillingClass::Paid,
        routable: true,
        ingress_protocols: vec![UpstreamProtocol::Responses],
        native_ingress_protocols: vec![UpstreamProtocol::Responses],
    }
}
fn selection(id: &str) -> CandidateSelectionV1 {
    CandidateSelectionV1 {
        binding_id: id.into(),
        reasoning: None,
    }
}
fn named() -> PlanAuthoringRequestV1 {
    PlanAuthoringRequestV1 {
        display_name: Some("Daily work".into()),
        purpose: Some("Implement requested changes".into()),
        ..Default::default()
    }
}
fn saved(editor: PlanEditorStateV2) -> AgentPlanStatusV2 {
    let plan_id = AgentPlanId::parse("plan/saved").unwrap();
    let alias = ModelAlias::parse("daily-work").unwrap();
    let digest = CanonicalDigest::of(&editor).unwrap();
    AgentPlanStatusV2 {
        protocol_advice: Default::default(),
        agent_plan_id: plan_id.clone(),
        desired: editor.effective().unwrap(),
        head: PlanHeadV1 {
            reference: PlanExecutionRef {
                workspace_id: WorkspaceId::default(),
                plan_id,
                content_revision: 3,
                content_digest: digest.clone(),
            },
            head_revision: 5,
            model_alias: alias.clone(),
            status: PlanLifecycleV1::Enabled,
        },
        editable_route_digest: digest.clone(),
        agent_plan_revision: 3,
        model_alias: alias,
        materialized_route_digest: digest.clone(),
        publication: PublicationIdentityViewV1 {
            revision: 7,
            digest,
        },
        execution: ClientGatewayStateV1::Ready,
        revisions: RevisionSetV1 {
            target: 7,
            dependencies: Default::default(),
        },
    }
}

#[test]
fn direct_smart_authoring_requires_a_decision_and_preserves_native_role_defaults() {
    let candidates = [candidate(
        "binding/a",
        NativeReasoningCapabilityV1::Discrete {
            parameter: "reasoning_effort".into(),
            profiles: vec!["off".into(), "low".into(), "high".into()],
            default_profile: Some("high".into()),
        },
    )];
    let mut input = PlanAuthoringRequestV1 {
        mode: Some(PlanEditorMode::SmartSaving),
        economy: Some(vec![selection("binding/a")]),
        primary: Some(vec![selection("binding/a")]),
        ..named()
    };
    let unresolved = prepare_authoring(&input, None, &candidates).unwrap();
    assert_eq!(
        unresolved.required_choices,
        vec![PlanAuthoringChoiceV1 {
            field: "smart.classifier".into(),
            reason: PlanAuthoringChoiceReasonV1::Required
        }]
    );
    assert!(unresolved.editor.effective().is_err());
    input.classifier = Some(ComplexityClassifierModeV1::LocalRules);
    let prepared = prepare_authoring(&input, None, &candidates).unwrap();
    assert!(prepared.required_choices.is_empty());
    assert!(prepared.editor.effective().is_ok());
    assert_eq!(
        prepared.editor.smart.economy[0].reasoning,
        Some(ReasoningSelectionV1::Profile {
            profile: "low".into()
        })
    );
    assert_eq!(
        prepared.editor.smart.primary[0].reasoning,
        Some(ReasoningSelectionV1::Profile {
            profile: "high".into()
        })
    );
    assert_eq!(prepared.editor.limits.maximum_attempts, 6);
    assert_eq!(prepared.editor.limits.context_window_tokens, None);
    assert_eq!(prepared.editor.limits.request_timeout_ms, 3_600_000);
    assert!(prepared.editor.smart.reselect_on_user_message);
}

#[test]
fn editing_one_group_preserves_custom_policy_and_existing_reasoning() {
    let candidates = [
        candidate(
            "binding/a",
            NativeReasoningCapabilityV1::Toggle {
                parameter: "thinking".into(),
            },
        ),
        candidate(
            "binding/b",
            NativeReasoningCapabilityV1::Fixed {
                profile: "provider-default".into(),
            },
        ),
    ];
    let input = PlanAuthoringRequestV1 {
        mode: Some(PlanEditorMode::SmartSaving),
        economy: Some(vec![selection("binding/a")]),
        primary: Some(vec![selection("binding/a")]),
        classifier: Some(ComplexityClassifierModeV1::LocalRules),
        ..named()
    };
    let mut original = prepare_authoring(&input, None, &candidates).unwrap().editor;
    original.limits.maximum_attempts = 2;
    original.limits.context_window_tokens = Some(32_000);
    original.smart.reselect_on_user_message = false;
    original.smart.judgment.degree.simple_threshold_millis = 650;
    original.smart.complex_keywords = vec!["architecture".into()];
    original.smart.primary[0].reasoning = Some(ReasoningSelectionV1::Toggle { enabled: false });
    let current = saved(original.clone());
    let update = PlanAuthoringRequestV1 {
        plan_id: Some(current.agent_plan_id.clone()),
        economy: Some(vec![selection("binding/b")]),
        primary: Some(vec![selection("binding/a")]),
        ..Default::default()
    };
    let result = prepare_authoring(&update, Some(&current), &candidates).unwrap();
    assert!(result.required_choices.is_empty());
    assert_eq!(result.expected_head_revision, Some(5));
    let mut expected = original;
    expected.custom_alias = Some("daily-work".into());
    expected.smart.economy = vec![selection("binding/b")];
    assert_eq!(result.editor, expected);
}

#[test]
fn unavailable_models_and_reasoning_budgets_stay_explicit_choices() {
    let candidates = [candidate(
        "binding/budget",
        NativeReasoningCapabilityV1::Budget {
            parameter: "budget_tokens".into(),
            minimum_tokens: 1024,
            maximum_tokens: 4096,
            step_tokens: 1024,
        },
    )];
    let input = PlanAuthoringRequestV1 {
        candidates: Some(vec![
            selection("binding/missing"),
            selection("binding/budget"),
        ]),
        ..named()
    };
    let result = prepare_authoring(&input, None, &candidates).unwrap();
    assert_eq!(
        result
            .required_choices
            .iter()
            .map(|v| v.reason)
            .collect::<Vec<_>>(),
        vec![
            PlanAuthoringChoiceReasonV1::CandidateUnavailable,
            PlanAuthoringChoiceReasonV1::ReasoningRequired
        ]
    );
    assert!(
        result
            .editor
            .candidates
            .iter()
            .all(|v| v.reasoning.is_none())
    );
    assert!(result.candidate_defaults[0].primary.is_none());
}

#[test]
fn old_options_request_omits_the_new_projection() {
    let request: PlanEditorOptionsRequestV1 = serde_json::from_str("{}").unwrap();
    assert!(request.authoring.is_none());
    assert!(request.preview_agent.is_none());
    let serialized = serde_json::to_value(request).unwrap();
    assert!(serialized.get("authoring").is_none());
    assert!(serialized.get("preview_agent").is_none());
    assert!(
        serde_json::from_str::<PlanEditorOptionsRequestV1>(
            r#"{"authoring":{"maximum_attempts":1}}"#
        )
        .is_err()
    );
}
