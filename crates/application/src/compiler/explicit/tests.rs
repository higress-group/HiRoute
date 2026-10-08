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
fn branches_publish_distinct_reasoning_and_freeze_whole_judgment_overrides() {
    let mut desired = desired();
    let low = selection("primary-a");
    let mut high = low.clone();
    high.reasoning = Some(ReasoningSelectionV1::Profile {
        profile: "high".into(),
    });
    let branch = |id: &str, candidate| RouteBranchV1 {
        id: id.into(),
        name: id.into(),
        condition: format!("Choose {id} tasks"),
        candidates: vec![candidate],
        primary_candidates: vec![],
        judgment: None,
    };
    let mut code = branch("code", low);
    code.primary_candidates = vec![high.clone()];
    code.judgment = Some(JudgmentSettingsV1 {
        competence: CompetencePolicyV1 {
            floor_millis: 700,
            ..Default::default()
        },
        ..Default::default()
    });
    desired.mode = PlanEditorMode::CustomBranches;
    desired.strategy = AgentPlanStrategyV2::Branches {
        routing: BranchRoutingV1 {
            classifier: ComplexityClassifierModeV1::DecisionService {
                service: Box::new(hiroute_domain::DecisionServiceV1 {
                    id: "decision-fixture".into(),
                    revision: 1,
                    name: "Fixture extension".into(),
                    connection: hiroute_domain::DecisionConnectionV1::Custom {
                        endpoint: "https://decision.example/choose".into(),
                        timeout_ms: 3000,
                        auth_header: None,
                    },
                }),
            },
            branches: vec![code, branch("docs", high)],
            default_branch_id: "docs".into(),
            judgment: JudgmentSettingsV1::default(),
            reselect_on_user_message: false,
        },
    };
    let plan = compile(&desired, &compilation_facts()).unwrap();
    let mut reserved = desired.clone();
    let AgentPlanStrategyV2::Branches { routing } = &mut reserved.strategy else {
        unreachable!()
    };
    routing.branches[0].id = SMART_SAVING_SCOPE_ID.into();
    assert!(!routing.validate(true));
    assert!(compile(&reserved, &compilation_facts()).is_err());
    let mut materialized = plan.body.materialized.clone();
    let RequestOwnedRouteV1::Branches { branches, .. } = &mut materialized.request_owned else {
        unreachable!()
    };
    branches[0].id = SMART_SAVING_SCOPE_ID.into();
    assert_eq!(
        materialized.validate(),
        Err(CompiledPlanError::InvalidGroups)
    );
    PlanVersionV1::new(WorkspaceId::default(), desired, plan.clone()).unwrap();
    let mut registry = AliasRegistryV1::default();
    registry
        .active
        .insert(plan.agent_plan_id().clone(), plan.model_alias().clone());
    let access = AgentModelGrantV2::seal(
        AgentIngressProtocolV1::Responses,
        BTreeMap::from([(
            plan.model_alias().as_str().into(),
            AgentModelRouteV2::Plan {
                plan_id: plan.agent_plan_id().clone(),
                alias: plan.model_alias().clone(),
                revision: plan.body.agent_plan_revision,
                semantic_digest: plan.body.materialized_route_digest.clone(),
            },
        )]),
    )
    .unwrap();
    let publication = crate::compiler::compile_publication(
        WorkspaceId::default(),
        "workspace/personal/default/gateway",
        1,
        GatewayPublicationRevision::new(1).unwrap(),
        DEFAULT_CATALOG_RENDERER_REVISION,
        registry,
        vec![plan],
        vec![
            GatewayAccessGrantV1::new(
                "grant/branches",
                1,
                CanonicalDigest::of_bytes(b"branch-fixture-token"),
                AgentIngressProtocolV1::Responses,
                access,
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let snapshot = publication.gateway_snapshot().unwrap();
    let alias = &snapshot.aliases[0];
    assert_eq!(
        alias.candidates.len(),
        2,
        "only exact execution identities may deduplicate"
    );
    assert_ne!(
        alias.candidates[0].protocol_profile_digest,
        alias.candidates[1].protocol_profile_digest
    );
    let routing = alias.routing.as_ref().unwrap();
    let groups = &routing.groups;
    assert_ne!(groups[0].candidate_local_ids, groups[1].candidate_local_ids);
    assert_eq!(groups[1].candidate_local_ids, groups[2].candidate_local_ids);
    let RequestOwnedRouteV1::Branches { branches, .. } = &routing.request_owned else {
        panic!("branches")
    };
    assert_eq!(branches[0].judgment.competence.floor_millis, 700);
    assert_eq!(branches[1].judgment.competence.floor_millis, 500);
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
fn smart_regular_failover_always_includes_primary_group() {
    let mut desired = desired();
    desired.mode = PlanEditorMode::SmartSaving;
    desired.strategy = AgentPlanStrategyV2::SmartSaving {
        economy: vec![selection("economy-b"), selection("economy-a")],
        primary: vec![selection("primary-b"), selection("primary-a")],
        judgment: JudgmentSettingsV1::default(),
        reselect_on_user_message: false,
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
    assert_eq!(
        simple_groups,
        &[MaterializedGroupId::Economy, MaterializedGroupId::Primary]
    );
    assert_eq!(complex_groups, &[MaterializedGroupId::Primary]);
    PlanVersionV1::new(WorkspaceId::default(), desired, compiled).unwrap();
}

#[test]
fn saved_classifier_is_preserved_in_the_materialized_route() {
    let mut desired = desired();
    desired.mode = PlanEditorMode::SmartSaving;
    desired.strategy = AgentPlanStrategyV2::SmartSaving {
        economy: vec![selection("economy-a")],
        primary: vec![selection("primary-a")],
        judgment: JudgmentSettingsV1::default(),
        reselect_on_user_message: false,
        classifier: ComplexityClassifierModeV1::DecisionService {
            service: Box::new(hiroute_domain::DecisionServiceV1 {
                id: "decision-fixture".into(),
                revision: 1,
                name: "Fixture extension".into(),
                connection: hiroute_domain::DecisionConnectionV1::Custom {
                    endpoint: "https://classifier.example/v1/branch".into(),
                    timeout_ms: hiroute_domain::DEFAULT_REST_CLASSIFIER_TIMEOUT_MS,
                    auth_header: Some(ClassifierAuthHeaderV1 {
                        name: "Authorization".into(),
                        value_secret_ref: "classifier/main".into(),
                    }),
                },
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
        ComplexityClassifierModeV1::DecisionService { service }
            if service.connection.transport().0 == "https://classifier.example/v1/branch"
    ));
}

#[test]
fn follow_up_reselection_is_published_and_changes_route_digest() {
    let mut desired = desired();
    desired.mode = PlanEditorMode::SmartSaving;
    desired.strategy = AgentPlanStrategyV2::SmartSaving {
        economy: vec![selection("economy-a")],
        primary: vec![selection("primary-a")],
        judgment: JudgmentSettingsV1::default(),
        reselect_on_user_message: false,
        classifier: ComplexityClassifierModeV1::LocalRules,
        complex_keywords: Vec::new(),
    };
    let before = compile(&desired, &compilation_facts()).unwrap();
    let AgentPlanStrategyV2::SmartSaving {
        reselect_on_user_message,
        ..
    } = &mut desired.strategy
    else {
        unreachable!()
    };
    *reselect_on_user_message = true;
    let after = compile(&desired, &compilation_facts()).unwrap();
    assert_ne!(
        before.body.materialized_route_digest,
        after.body.materialized_route_digest
    );
    assert!(matches!(
        after.body.materialized.request_owned,
        RequestOwnedRouteV1::Classified {
            reselect_on_user_message: true,
            ..
        }
    ));
    PlanVersionV1::new(WorkspaceId::default(), desired, after).unwrap();
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

#[test]
fn context_window_is_bounded_persisted_and_changes_the_route_digest() {
    let mut desired = desired();
    let facts = compilation_facts();
    let default = compile(&desired, &facts).unwrap();
    let upper = default
        .body
        .materialized
        .context_window_upper_bound()
        .unwrap();
    assert_eq!(
        default.body.materialized.context_window_tokens().unwrap(),
        upper.min(272_000)
    );
    desired.limits.context_window_tokens = Some(64_000);
    let custom = compile(&desired, &facts).unwrap();
    assert_eq!(
        custom.body.materialized.context_window_tokens().unwrap(),
        64_000
    );
    assert_ne!(
        default.body.materialized_route_digest,
        custom.body.materialized_route_digest
    );
    let version = PlanVersionV1::new(WorkspaceId::default(), desired.clone(), custom).unwrap();
    let saved: PlanVersionV1 =
        serde_json::from_value(serde_json::to_value(&version).unwrap()).unwrap();
    assert_eq!(saved, version);
    for invalid in [0, upper + 1, u64::MAX] {
        desired.limits.context_window_tokens = Some(invalid);
        assert!(compile(&desired, &facts).is_err());
    }
    desired.limits.context_window_tokens = Some(upper);
    assert!(compile(&desired, &facts).is_ok());
    let mut changed = facts;
    for profile in &mut changed.candidates[0].protocol_profiles {
        profile.capability.context.max_input_tokens = GatewayCriticalFactV1::Exact(32_000);
    }
    // Changing an active candidate's bound must not silently lower the saved custom value.
    if let AgentPlanStrategyV2::Custom { candidates } = &mut desired.strategy {
        candidates[0].binding_id = changed.candidates[0].binding.binding_id.clone();
    }
    assert!(compile(&desired, &changed).is_err());
}
