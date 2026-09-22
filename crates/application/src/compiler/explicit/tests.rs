use super::*;
use crate::compiler::test_fixtures::{compilation_facts, custom_desired};
use hiroute_domain::delegation::WorkerHarnessV1;

fn desired() -> AgentPlanAuthoringV2 {
    let legacy = custom_desired();
    let AgentPlanStrategyV1::Custom { candidates } = legacy.strategy else {
        unreachable!()
    };
    AgentPlanAuthoringV2 {
        schema: PLAN_AUTHORING_SCHEMA_V2.into(),
        display_name: legacy.display_name,
        purpose: legacy.purpose,
        mode: PlanEditorMode::FixedModel,
        requirements: legacy.requirements,
        limits: legacy.limits,
        strategy: AgentPlanStrategyV2::Custom { candidates },
        delegation_enabled: false,
        work: None,
    }
}
fn compile(
    desired: &AgentPlanAuthoringV2,
    facts: &AgentPlanCompilationFactsV1,
) -> Result<CompiledAgentPlanV1, AgentPlanCompilerError> {
    compile_agent_plan_v2(
        AgentPlanIdentityV1 {
            agent_plan_id: AgentPlanId::parse("plan/explicit").unwrap(),
            model_alias: ModelAlias::parse_custom("hiroute-explicit").unwrap(),
            display_name: desired.display_name.clone(),
            purpose: desired.purpose.clone(),
        },
        1,
        desired,
        facts,
    )
}
fn selection(id: &str) -> CandidateSelectionV1 {
    CandidateSelectionV1 {
        binding_id: format!("binding/{id}"),
        reasoning: Some(ReasoningSelectionV1::Profile {
            profile: "low".into(),
        }),
    }
}

#[test]
fn explicit_order_survives_missing_ratings_and_price_changes() {
    let desired = desired();
    let mut facts = compilation_facts();
    let before = compile(&desired, &facts).unwrap();
    for candidate in &mut facts.candidates {
        candidate.rating = None;
        candidate.ordering_price = None;
    }
    facts.refs.ratings_slice_digest = CanonicalDigest::of_bytes(b"new ratings");
    facts.refs.model_data_digest = CanonicalDigest::of_bytes(b"new catalog");
    facts.refs.free_offers_slice_digest = CanonicalDigest::of_bytes(b"new offers");
    let after = compile(&desired, &facts).unwrap();
    assert_eq!(
        before.body.materialized_route_digest,
        after.body.materialized_route_digest
    );
    let group = &after.body.materialized.attempt_owned.groups[0];
    assert!(group.pinned_ratings.is_empty());
    assert_eq!(
        group
            .candidates
            .iter()
            .map(|c| c.binding_id.as_str())
            .collect::<Vec<_>>(),
        vec!["binding/primary-b", "binding/primary-a"]
    );
    let mut reordered = desired;
    if let AgentPlanStrategyV2::Custom { candidates } = &mut reordered.strategy {
        candidates.reverse();
    }
    assert_ne!(
        after.body.materialized_route_digest,
        compile(&reordered, &facts)
            .unwrap()
            .body
            .materialized_route_digest
    );
}

#[test]
fn smart_fallback_off_keeps_primary_exclusively_for_complex_requests() {
    let mut desired = desired();
    desired.mode = PlanEditorMode::SmartSaving;
    desired.strategy = AgentPlanStrategyV2::SmartSaving {
        economy: vec![selection("economy-b"), selection("economy-a")],
        primary: vec![selection("primary-b"), selection("primary-a")],
        primary_fallback: false,
        classifier: ComplexityClassifierModeV1::LocalRules,
        complex_keywords: vec!["complex".into()],
    };
    let compiled = compile(&desired, &compilation_facts()).unwrap();
    let RequestOwnedRouteV1::Classified {
        simple_groups,
        complex_groups,
        ..
    } = &compiled.body.materialized.request_owned
    else {
        panic!("classified")
    };
    assert_eq!(simple_groups, &[MaterializedGroupId::Economy]);
    assert_eq!(complex_groups, &[MaterializedGroupId::Primary]);
    PlanVersionV1::new(WorkspaceId::default(), desired, compiled).unwrap();
}

#[test]
fn smart_rest_classifier_is_preserved_in_the_materialized_route() {
    let mut desired = desired();
    desired.mode = PlanEditorMode::SmartSaving;
    desired.strategy = AgentPlanStrategyV2::SmartSaving {
        economy: vec![selection("economy-a")],
        primary: vec![selection("primary-a")],
        primary_fallback: false,
        classifier: ComplexityClassifierModeV1::Rest {
            endpoint: "https://classifier.example/v1/branch".into(),
            timeout_ms: hiroute_domain::DEFAULT_REST_CLASSIFIER_TIMEOUT_MS,
            auth_header: Some(ClassifierAuthHeaderV1 {
                name: "Authorization".into(),
                value_secret_ref: "classifier/main".into(),
            }),
        },
        complex_keywords: Vec::new(),
    };
    let compiled = compile(&desired, &compilation_facts()).unwrap();
    let RequestOwnedRouteV1::Classified { classifier, .. } =
        &compiled.body.materialized.request_owned
    else {
        panic!("classified")
    };
    assert!(matches!(
        &classifier.mode,
        ComplexityClassifierModeV1::Rest { endpoint, .. }
            if endpoint == "https://classifier.example/v1/branch"
    ));
}

#[test]
fn free_membership_is_explicit_and_paid_candidate_is_rejected() {
    let mut desired = desired();
    desired.mode = PlanEditorMode::FreeFirst;
    desired.strategy = AgentPlanStrategyV2::FreeFirst {
        candidates: vec![CandidateSelectionV1 {
            binding_id: "binding/free-b".into(),
            reasoning: None,
        }],
        primary_fallback: false,
        primary: vec![],
    };
    let compiled = compile(&desired, &compilation_facts()).unwrap();
    assert_eq!(compiled.body.materialized.attempt_owned.groups.len(), 1);
    assert_eq!(
        compiled.body.materialized.attempt_owned.groups[0]
            .candidates
            .len(),
        1
    );
    if let AgentPlanStrategyV2::FreeFirst { candidates, .. } = &mut desired.strategy {
        *candidates = vec![selection("primary-a")];
    }
    assert_eq!(
        compile(&desired, &compilation_facts()),
        Err(AgentPlanCompilerError::NonFreeCandidate)
    );
}

#[test]
fn native_choice_is_required_and_protocol_is_checked() {
    let mut desired = desired();
    if let AgentPlanStrategyV2::Custom { candidates } = &mut desired.strategy {
        candidates[0].reasoning = None;
    }
    assert!(matches!(
        compile(&desired, &compilation_facts()),
        Err(AgentPlanCompilerError::Reasoning(
            ReasoningContractError::SelectionRequired
        ))
    ));
    let mut desired = self::desired();
    desired.delegation_enabled = true;
    desired.work = Some(WorkerPlanV1 {
        harness: WorkerHarnessV1::ClaudeCode,
        protocol: AgentIngressProtocolV1::Messages,
    });
    let mut facts = compilation_facts();
    for candidate in &mut facts.candidates {
        candidate
            .protocol_profiles
            .retain(|p| p.ingress_protocol == UpstreamProtocol::Responses);
    }
    assert!(matches!(
        compile(&desired, &facts),
        Err(AgentPlanCompilerError::CapabilityUnqualified(_))
    ));
}

#[test]
fn fixed_binding_reuses_exact_compiler_without_rating_or_plan() {
    let mut facts = compilation_facts();
    for fact in &mut facts.candidates {
        fact.rating = None;
        fact.ordering_price = None;
    }
    let selected = AgentFixedModelSelectionV2 {
        client_model_id: "Native/Model.V1[1m]".into(),
        candidate: selection("primary-a"),
    };
    let fixed =
        compile_fixed_model_bindings(std::slice::from_ref(&selected), &facts.candidates).unwrap();
    assert_eq!(fixed.len(), 1);
    let binding = &fixed[&selected.client_model_id];
    assert_eq!(binding.binding_id, "binding/primary-a");
    assert_eq!(binding.source_id, "source/primary-a");
    assert_eq!(binding.credential_refs, ["credential/primary-a"]);
    let base = GatewayPublicationV1::new(
        WorkspaceId::default(),
        GatewayPublicationRevision::new(1).unwrap(),
        AliasRegistryV1::default(),
        Vec::new(),
    )
    .unwrap();
    let settings = AgentModelSelectionV2::CodexDefault {
        native_model_mode: hiroute_domain::CodexNativeModelModeV2::PreserveAvailable,
        fixed_models: vec![selected],
        allowed_plan_ids: Default::default(),
        default_selection: AgentModelDefaultSelectionV2::PreserveNative,
    };
    let grant =
        AgentModelGrantV2::derive(AgentIngressProtocolV1::Responses, &settings, &base, &fixed)
            .unwrap();
    let mut tampered = grant.clone();
    if let AgentModelRouteV2::Fixed { binding, .. } = tampered.routes.values_mut().next().unwrap() {
        binding.source_identity_digest = CanonicalDigest::of_bytes(b"other account");
    }
    assert!(tampered.validate().is_err());
    let published = base
        .next_with_access_grant(
            GatewayPublicationRevision::new(2).unwrap(),
            GatewayAccessGrantV1::new(
                "grant-fixed",
                1,
                CanonicalDigest::of_bytes(b"bearer"),
                AgentIngressProtocolV1::Responses,
                grant,
            )
            .unwrap(),
        )
        .unwrap();
    assert!(published.plans.is_empty());
    assert!(published.aliases.is_empty());
    let snapshot = published.gateway_snapshot().unwrap();
    assert_eq!(snapshot.admission, GatewayAdmissionStateV1::NewCallsAllowed);
    assert_eq!(snapshot.grants.len(), 1);
    assert!(
        snapshot.grants[0]
            .routes
            .contains_key("Native/Model.V1[1m]")
    );
}

#[test]
fn fixed_selection_rejects_missing_duplicate_or_implicit_reasoning() {
    let facts = compilation_facts();
    let mut selected = AgentFixedModelSelectionV2 {
        client_model_id: "native".into(),
        candidate: selection("primary-a"),
    };
    assert!(
        compile_fixed_model_bindings(&[selected.clone(), selected.clone()], &facts.candidates)
            .is_err()
    );
    selected.candidate.reasoning = None;
    assert!(matches!(
        compile_fixed_model_bindings(&[selected.clone()], &facts.candidates),
        Err(AgentPlanCompilerError::Reasoning(
            ReasoningContractError::SelectionRequired
        ))
    ));
    selected.candidate.binding_id = "binding/missing".into();
    assert!(matches!(
        compile_fixed_model_bindings(&[selected], &facts.candidates),
        Err(AgentPlanCompilerError::UnknownBinding(_))
    ));
}

#[test]
fn fixed_name_colliding_with_an_allowed_plan_alias_is_rejected_before_publication() {
    let facts = compilation_facts();
    let publication = crate::compiler::test_fixtures::compiled_publication(1);
    let selected = AgentFixedModelSelectionV2 {
        client_model_id: "hiroute/2590c10eeae4f930".into(),
        candidate: selection("primary-a"),
    };
    let fixed =
        compile_fixed_model_bindings(std::slice::from_ref(&selected), &facts.candidates).unwrap();
    let settings = AgentModelSelectionV2::CodexDefault {
        native_model_mode: hiroute_domain::CodexNativeModelModeV2::PreserveAvailable,
        fixed_models: vec![selected],
        allowed_plan_ids: [AgentPlanId::parse("plan/custom").unwrap()].into(),
        default_selection: AgentModelDefaultSelectionV2::PreserveNative,
    };
    assert_eq!(
        AgentModelGrantV2::derive(
            AgentIngressProtocolV1::Responses,
            &settings,
            &publication,
            &fixed
        )
        .unwrap_err(),
        AgentConnectionError::InvalidGrant
    );
}
