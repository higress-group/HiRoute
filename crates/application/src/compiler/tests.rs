use std::collections::BTreeSet;

use hiroute_domain::{
    CanonicalDigest, CompiledPlanError, ComplexityBranch, ComplexityInputV1,
    ExactNativeReasoningV1, FreeFallbackPolicy, FreePoolMode, GatewayAuthenticationSemanticsV1,
    GatewayCriticalFactV1, GatewayOperationalTargetV1, MaterializedCostPolicyV1,
    MaterializedGroupId, MaterializedOrderingV1, RequestOwnedRouteV1, UpstreamProtocol,
};

use super::test_fixtures::*;
use super::*;

#[test]
fn compiler_smart_saving_materializes_two_deterministic_branches() {
    let desired = smart_desired();
    let facts = compilation_facts();
    assert!(
        facts
            .candidates
            .iter()
            .find(|candidate| candidate.binding.binding_id == "binding/economy-c")
            .unwrap()
            .ordering_price
            .is_none()
    );
    let result = materialize_agent_plan(&desired, &facts).unwrap();
    let groups = &result.materialized.attempt_owned.groups;
    assert_eq!(groups[0].group_id, MaterializedGroupId::Economy);
    assert!(matches!(
        &groups[0].ordering_evidence,
        MaterializedOrderingV1::CheapestWithRatingGuard {
            quality_anchor_binding_id,
            maximum_score_gap_tenths: 2,
        } if quality_anchor_binding_id == "binding/economy-a"
    ));
    assert_eq!(
        binding_order(&groups[0]),
        vec!["binding/economy-a", "binding/economy-c"]
    );
    assert_eq!(groups[1].group_id, MaterializedGroupId::Primary);
    assert_eq!(
        binding_order(&groups[1]),
        vec!["binding/primary-a", "binding/primary-b"]
    );
    assert!(matches!(
        groups[0].candidates[0].exact_reasoning,
        ExactNativeReasoningV1::Profile { ref profile, .. } if profile == "low"
    ));
    assert!(matches!(
        groups[1].candidates[0].exact_reasoning,
        ExactNativeReasoningV1::Profile { ref profile, .. } if profile == "high"
    ));
    let RequestOwnedRouteV1::Classified {
        classifier,
        reselect_on_user_message,
        simple_groups,
        complex_groups,
    } = &result.materialized.request_owned
    else {
        panic!("smart-saving request phase");
    };
    assert!(!reselect_on_user_message);
    assert_eq!(
        simple_groups,
        &[MaterializedGroupId::Economy, MaterializedGroupId::Primary]
    );
    assert_eq!(complex_groups, &[MaterializedGroupId::Primary]);
    let decision = classifier
        .classify(&ComplexityInputV1 {
            human_text: Some("please investigate 性能回归".into()),
            ..ComplexityInputV1::default()
        })
        .unwrap();
    assert_eq!(decision.branch, ComplexityBranch::Complex);
}

#[test]
fn compiler_economy_all_unknown_costs_fall_back_to_pinned_quality_order() {
    let desired = smart_desired();
    let mut facts = compilation_facts();
    for candidate in &mut facts.candidates {
        if candidate.binding.binding_id.starts_with("binding/economy-") {
            candidate.ordering_price = None;
        }
    }
    let result = materialize_agent_plan(&desired, &facts).unwrap();
    assert_eq!(
        binding_order(&result.materialized.attempt_owned.groups[0]),
        vec!["binding/economy-c", "binding/economy-a"]
    );
}

#[test]
fn compiler_accepts_release_seed_rating_with_zero_observation_count() {
    let desired = smart_desired();
    let mut facts = compilation_facts();
    facts
        .candidates
        .iter_mut()
        .find(|candidate| candidate.binding.binding_id == "binding/economy-c")
        .unwrap()
        .rating
        .as_mut()
        .unwrap()
        .rating_count = 0;
    let result = materialize_agent_plan(&desired, &facts).unwrap();
    assert_eq!(
        binding_order(&result.materialized.attempt_owned.groups[0]),
        vec!["binding/economy-a", "binding/economy-c"]
    );
}

#[test]
fn compiler_free_first_closes_strict_free_and_primary_fallback_separately() {
    let facts = compilation_facts();
    let strict =
        materialize_agent_plan(&free_desired(FreeFallbackPolicy::FreeOnly), &facts).unwrap();
    assert_eq!(strict.materialized.attempt_owned.groups.len(), 1);
    assert_eq!(
        binding_order(&strict.materialized.attempt_owned.groups[0]),
        vec!["binding/free-a", "binding/free-b"]
    );
    assert!(strict.excluded_candidates.iter().any(|value| {
        value.binding_id == "binding/free-toggle"
            && value.reason == CandidateExclusionReason::ReasoningSelectionRequired
    }));
    assert!(strict.excluded_candidates.iter().any(|value| {
        value.binding_id == "binding/free-budget"
            && value.reason == CandidateExclusionReason::ReasoningBudgetRequired
    }));
    let RequestOwnedRouteV1::Ordered {
        cost_policy,
        ordered_groups,
    } = &strict.materialized.request_owned
    else {
        panic!("free-first request phase");
    };
    assert_eq!(cost_policy, &MaterializedCostPolicyV1::StrictFree);
    assert_eq!(ordered_groups, &[MaterializedGroupId::Free]);

    let fallback =
        materialize_agent_plan(&free_desired(FreeFallbackPolicy::PrimaryFallback), &facts).unwrap();
    assert_eq!(fallback.materialized.attempt_owned.groups.len(), 2);
    assert_eq!(
        binding_order(&fallback.materialized.attempt_owned.groups[1]),
        vec!["binding/primary-a", "binding/primary-b"]
    );
}

#[test]
fn compiler_free_first_manual_preserves_order_and_empty_pool_fails_closed() {
    let facts = compilation_facts();
    let mut desired = free_desired(FreeFallbackPolicy::FreeOnly);
    {
        let hiroute_domain::AgentPlanStrategyV1::FreeFirst { free_pool, .. } =
            &mut desired.strategy
        else {
            unreachable!()
        };
        free_pool.mode = FreePoolMode::Manual;
        free_pool.candidates = vec![
            hiroute_domain::CandidateSelectionV1 {
                binding_id: "binding/free-b".into(),
                reasoning: None,
            },
            hiroute_domain::CandidateSelectionV1 {
                binding_id: "binding/free-a".into(),
                reasoning: Some(hiroute_domain::ReasoningSelectionV1::Profile {
                    profile: "low".into(),
                }),
            },
        ];
    }
    let manual = materialize_agent_plan(&desired, &facts).unwrap();
    assert_eq!(
        binding_order(&manual.materialized.attempt_owned.groups[0]),
        vec!["binding/free-b", "binding/free-a"]
    );

    let hiroute_domain::AgentPlanStrategyV1::FreeFirst { free_pool, .. } = &mut desired.strategy
    else {
        unreachable!()
    };
    free_pool.candidates.clear();
    assert_eq!(
        materialize_agent_plan(&desired, &facts).unwrap_err(),
        AgentPlanCompilerError::NoFreeCandidates
    );
}

#[test]
fn compiler_free_first_automatic_accepts_exact_toggle_and_budget_choices() {
    let facts = compilation_facts();
    let mut desired = free_desired(FreeFallbackPolicy::FreeOnly);
    let hiroute_domain::AgentPlanStrategyV1::FreeFirst { free_pool, .. } = &mut desired.strategy
    else {
        unreachable!()
    };
    free_pool.automatic_reasoning.insert(
        "binding/free-toggle".into(),
        hiroute_domain::ReasoningSelectionV1::Toggle { enabled: true },
    );
    free_pool.automatic_reasoning.insert(
        "binding/free-budget".into(),
        hiroute_domain::ReasoningSelectionV1::Budget { tokens: 1_024 },
    );
    let result = materialize_agent_plan(&desired, &facts).unwrap();
    assert_eq!(
        binding_order(&result.materialized.attempt_owned.groups[0]),
        vec![
            "binding/free-a",
            "binding/free-toggle",
            "binding/free-budget",
            "binding/free-b",
        ]
    );
    assert!(result.excluded_candidates.iter().all(|candidate| {
        candidate.binding_id != "binding/free-toggle"
            && candidate.binding_id != "binding/free-budget"
    }));
}

#[test]
fn compiler_custom_preserves_only_the_explicit_candidate_order() {
    let result = materialize_agent_plan(&custom_desired(), &compilation_facts()).unwrap();
    assert_eq!(
        binding_order(&result.materialized.attempt_owned.groups[0]),
        vec!["binding/primary-b", "binding/primary-a"]
    );
}

#[test]
fn compiler_preserves_exact_ordered_credential_refs_and_rejects_duplicates() {
    let mut facts = compilation_facts();
    let selected = facts
        .candidates
        .iter_mut()
        .find(|candidate| candidate.binding.binding_id == "binding/primary-b")
        .unwrap();
    selected.credential_refs = vec!["credential/key-a".into(), "credential/key-b".into()];
    let compiled = materialize_agent_plan(&custom_desired(), &facts).unwrap();
    let candidate = &compiled.materialized.attempt_owned.groups[0].candidates[0];
    assert_eq!(
        candidate.credential_refs,
        ["credential/key-a", "credential/key-b"]
    );

    let mut duplicate = facts;
    duplicate
        .candidates
        .iter_mut()
        .find(|candidate| candidate.binding.binding_id == "binding/primary-b")
        .unwrap()
        .credential_refs = vec!["credential/key-a".into(), "credential/key-a".into()];
    assert!(matches!(
        materialize_agent_plan(&custom_desired(), &duplicate),
        Err(AgentPlanCompilerError::Facts(
            CompilerFactError::CrossReference
        ))
    ));
}

#[test]
fn compiler_unknown_fact_revision_and_unqualified_capability_fail_closed() {
    let desired = smart_desired();
    let mut facts = compilation_facts();
    facts.compiler_revision = "agent-plan-compiler/v2".into();
    assert!(matches!(
        materialize_agent_plan(&desired, &facts).unwrap_err(),
        AgentPlanCompilerError::Facts(CompilerFactError::UnsupportedCompilerRevision)
    ));

    let mut facts = compilation_facts();
    facts
        .candidates
        .iter_mut()
        .find(|value| value.binding.binding_id == "binding/primary-a")
        .unwrap()
        .model
        .capabilities
        .tool = false;
    assert_eq!(
        materialize_agent_plan(&desired, &facts).unwrap_err(),
        AgentPlanCompilerError::CapabilityUnqualified("binding/primary-a".into())
    );

    let mut facts = compilation_facts();
    facts.candidates[0].protocol_endpoint.adapter_ref = "adapter.untrusted".into();
    assert!(matches!(
        materialize_agent_plan(&desired, &facts).unwrap_err(),
        AgentPlanCompilerError::Facts(CompilerFactError::CrossReference)
    ));

    let mut facts = compilation_facts();
    facts.candidates[0].credential_refs.clear();
    assert!(matches!(
        materialize_agent_plan(&desired, &facts).unwrap_err(),
        AgentPlanCompilerError::Facts(CompilerFactError::CrossReference)
    ));

    let mut facts = compilation_facts();
    facts.candidates[0].protocol_profiles[0]
        .capability
        .native_model = "drifted-native-model".into();
    assert_eq!(
        materialize_agent_plan(&desired, &facts).unwrap_err(),
        AgentPlanCompilerError::InvalidCompiledPlan
    );

    let mut facts = compilation_facts();
    facts.candidates[0].protocol_profiles.clear();
    assert!(matches!(
        materialize_agent_plan(&desired, &facts).unwrap_err(),
        AgentPlanCompilerError::Facts(CompilerFactError::CrossReference)
    ));
}

#[test]
fn compiler_compiled_revision_is_immutable_and_unknown_compiler_revision_is_rejected() {
    let mut publication = compiled_publication(11);
    let plan = &mut publication.plans[0];
    std::sync::Arc::make_mut(&mut plan.body).agent_plan_revision += 1;
    assert_eq!(
        plan.validate().unwrap_err(),
        CompiledPlanError::DigestMismatch
    );

    let mut publication = compiled_publication(11);
    let plan = &mut publication.plans[0];
    assert_eq!(
        plan.body.compiler_revision,
        hiroute_domain::AGENT_PLAN_COMPILER_REVISION_V2
    );
    std::sync::Arc::make_mut(&mut plan.body).compiler_revision =
        "unsupported-compiler-revision".into();
    assert_eq!(
        plan.validate().unwrap_err(),
        CompiledPlanError::UnsupportedCompilerRevision
    );
}

#[test]
fn compiler_publication_verifier_rejects_reordered_materialized_candidates() {
    let mut publication = compiled_publication(11);
    let plan = publication
        .plans
        .iter_mut()
        .find(|plan| plan.agent_plan_id().as_str() == "plan/smart")
        .unwrap();
    let primary = std::sync::Arc::make_mut(&mut plan.body)
        .materialized
        .attempt_owned
        .groups
        .iter_mut()
        .find(|group| group.group_id == MaterializedGroupId::Primary)
        .unwrap();
    primary.candidates.swap(0, 1);
    assert_eq!(
        plan.validate().unwrap_err(),
        CompiledPlanError::MaterializedRouteDigestMismatch
    );
}

#[test]
fn compiler_retired_alias_is_tombstoned_and_cannot_be_reallocated() {
    let publication = compiled_publication(11);
    let retired_id = publication.plans[0].agent_plan_id().clone();
    let retired_alias = publication.plans[0].model_alias().clone();
    let mut registry = publication.alias_registry.clone();
    assert_eq!(registry.retire(&retired_id).unwrap(), retired_alias);
    assert!(registry.tombstones.contains(&retired_alias));
    assert!(registry.allocate(retired_id).is_err());

    let remaining = publication
        .plans
        .iter()
        .filter(|plan| plan.model_alias() != &retired_alias)
        .cloned()
        .collect::<Vec<_>>();
    let grants = publication_grants(&remaining);
    let next = compile_publication(
        hiroute_domain::WorkspaceId::default(),
        "workspace/personal/default/gateway",
        1,
        hiroute_domain::GatewayPublicationRevision::new(12).unwrap(),
        hiroute_domain::DEFAULT_CATALOG_RENDERER_REVISION,
        registry,
        remaining,
        grants,
    )
    .unwrap();
    assert!(next.alias_registry.tombstones.contains(&retired_alias));
    for alias in &next.aliases {
        assert_eq!(
            Some(alias),
            publication
                .aliases
                .iter()
                .find(|previous| previous.served_model_id == alias.served_model_id)
        );
    }
}

#[test]
fn compiler_equivalent_fact_order_produces_identical_preview_digest() {
    let desired = smart_desired();
    let mut first = compilation_facts();
    let first_digest = first.digest().unwrap();
    first.candidates.reverse();
    assert_eq!(first.digest().unwrap(), first_digest);
    let first_plan = materialize_agent_plan(&desired, &first).unwrap();
    let second_plan = materialize_agent_plan(&desired, &compilation_facts()).unwrap();
    assert_eq!(first_plan, second_plan);
}

#[test]
fn compiler_materialized_route_digest_excludes_price_provenance_when_order_is_unchanged() {
    let desired = smart_desired();
    let first = materialize_agent_plan(&desired, &compilation_facts())
        .unwrap()
        .materialized;
    let mut changed = compilation_facts();
    changed.ordering_price_version = "prices.v2".into();
    changed.ordering_price_digest = hiroute_domain::CanonicalDigest::of_bytes(b"prices-v2");
    for fact in &mut changed.candidates {
        if let Some(price) = &mut fact.ordering_price {
            price.price_rate_revision += 1;
            price.input_micros_per_million += 1_000;
            price.output_micros_per_million += 1_000;
            price.frozen_digest = hiroute_domain::CanonicalDigest::of_bytes(
                format!("{}-v2", price.price_rate_id).as_bytes(),
            );
        }
    }
    let second = materialize_agent_plan(&desired, &changed)
        .unwrap()
        .materialized;
    assert_eq!(first, second);
    assert_eq!(
        first.legacy_route_digest().unwrap(),
        second.legacy_route_digest().unwrap()
    );
}

#[test]
fn compiler_publication_is_a_closed_g0_authority_projection() {
    let publication = compiled_publication(11);
    assert_eq!(
        publication.schema,
        hiroute_domain::GATEWAY_PUBLICATION_SCHEMA_V3
    );
    publication.validate_current_contract().unwrap();
    assert_eq!(
        publication.authority_id,
        "workspace/personal/default/gateway"
    );
    assert_eq!(publication.authority_epoch, 1);
    assert_eq!(
        publication.catalog_renderer_revision,
        hiroute_domain::DEFAULT_CATALOG_RENDERER_REVISION
    );
    assert_eq!(publication.plans.len(), publication.aliases.len());
    assert_eq!(publication.grants.len(), 2);

    let snapshot = publication.gateway_snapshot().unwrap();
    assert_eq!(
        snapshot.schema_version,
        hiroute_domain::GATEWAY_SNAPSHOT_SCHEMA_V3
    );
    assert_eq!(
        snapshot.canonical_digest().unwrap().as_str(),
        snapshot.payload_digest
    );
    let local_ids = snapshot
        .aliases
        .iter()
        .flat_map(|alias| alias.candidates.iter().map(|candidate| candidate.local_id))
        .collect::<Vec<_>>();
    assert!(local_ids.iter().all(|local_id| *local_id > 0));
    assert_eq!(
        local_ids.iter().copied().collect::<BTreeSet<_>>().len(),
        local_ids.len()
    );
    for candidate in snapshot.aliases.iter().flat_map(|alias| &alias.candidates) {
        assert!(candidate.endpoint.starts_with("https://"));
        assert!(candidate.adapter_id.ends_with("@1"));
        assert!(!candidate.credential_refs.is_empty());
        assert!(
            candidate
                .credential_destination_ref
                .starts_with("connection-option/")
        );
        assert!(matches!(
            candidate.operational_target,
            GatewayOperationalTargetV1::RegisteredHttps { ref uri } if uri == &candidate.endpoint
        ));
        assert_eq!(
            CanonicalDigest::of(&candidate.operational_target).unwrap(),
            candidate.operational_target_digest
        );
        assert_eq!(
            CanonicalDigest::of(&candidate.protocol_profiles).unwrap(),
            candidate.protocol_profile_digest
        );
        assert!(candidate.protocol_profiles.iter().all(|profile| {
            profile.capability.native_model == candidate.native_transport_model
                && candidate.upstream_model_id == candidate.native_transport_model
                && profile.connector.request_path == "/v1/responses"
                && profile.connector.authentication
                    == GatewayCriticalFactV1::Exact(GatewayAuthenticationSemanticsV1::Bearer)
                && profile.capability.upstream_protocol == UpstreamProtocol::Responses
        }));
    }

    let smart_alias = publication
        .plans
        .iter()
        .find(|plan| plan.agent_plan_id().as_str() == "plan/smart")
        .unwrap()
        .model_alias();
    let grants_for_smart = snapshot
        .grants
        .iter()
        .filter(|grant| {
            matches!(grant.routes.get(smart_alias.as_str()),
            Some(hiroute_domain::GatewayModelRouteV2::Plan { alias, .. }) if alias == smart_alias)
        })
        .collect::<Vec<_>>();
    assert_eq!(grants_for_smart.len(), 2);
    assert_ne!(
        grants_for_smart[0].bearer_token_sha256,
        grants_for_smart[1].bearer_token_sha256
    );
    assert_ne!(grants_for_smart[0].protocol, grants_for_smart[1].protocol);
}

#[test]
fn compiler_publication_bytes_are_current_deterministic_and_secret_free() {
    let first = compiled_publication(11);
    let second = compiled_publication(11);
    let bytes = first.canonical_bytes().unwrap();
    assert_eq!(bytes, second.canonical_bytes().unwrap());
    assert_eq!(first.digest().unwrap(), second.digest().unwrap());
    let lowercase = String::from_utf8(bytes.clone()).unwrap().to_lowercase();
    assert!(lowercase.contains("\"mode\":{\"kind\":\"local_rules\"}"));
    assert!(!lowercase.contains("\"smart_saving\""));
    assert!(!lowercase.contains("\"free_first\""));
    assert!(!lowercase.contains("ordering_price_version"));
    assert!(!lowercase.contains("price_rate_id"));
    assert!(lowercase.contains("\"bearer_token_sha256\""));
    assert!(lowercase.contains("\"catalog_renderer_revision\""));
    assert!(lowercase.contains("\"credential_refs\""));
    assert!(lowercase.contains("\"endpoint\""));
    assert!(!lowercase.contains("\"authorization\":"));
    for forbidden in [
        "api_key_value",
        "api_key_secret",
        "codex-grant-verifier",
        "claude-grant-verifier",
        "endpoint_url",
        "secret",
        "rating_service",
        "price_service",
    ] {
        assert!(
            !lowercase.contains(forbidden),
            "forbidden field {forbidden}"
        );
    }

    assert_eq!(
        hiroute_domain::GatewayPublicationV1::decode(&bytes).unwrap(),
        first
    );
}

fn binding_order(group: &hiroute_domain::MaterializedModelGroupV1) -> Vec<&str> {
    group
        .candidates
        .iter()
        .map(|candidate| candidate.binding_id.as_str())
        .collect()
}
