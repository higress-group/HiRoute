use hiroute_application_api::{
    ComputeCandidateFactStateV2, ComputeCandidateInputStateV2, ComputeCandidateProducerV2,
    ComputeCandidateRefV2, ComputeCandidateTargetV2, ComputeCheckCorrelationV2,
    ComputeConnectionAccessKindV1, ComputeConnectionIdentityV1, ComputeModelAvailabilityReasonV1,
    ComputeModelAvailabilityV1, ComputeModelMembershipV2, ComputePriceContextV1,
};
use hiroute_domain::{
    BillingClass, CanonicalDigest, CredentialRefV1, GatewayAuthenticationSemanticsV1,
    NativeReasoningCapabilityV1, PriceValuationKindV1, RuntimeClockSampleV1,
    RuntimeStateIdentityV1, RuntimeStateV1, UpstreamProtocol,
};

use super::*;

mod codex_checked_account;
mod connector_owned_availability;

fn unknown<T>() -> ComputeCandidateFactValueV2<T> {
    ComputeCandidateFactValueV2 {
        value: None,
        basis: ComputeCandidateFactBasisV2::Unknown,
    }
}

fn target() -> ComputeCandidateTargetV2 {
    ComputeCandidateTargetV2 {
        scheme: "https".to_owned(),
        authority: "api.example.test".to_owned(),
        port: 443,
        request_path: "/v1/responses".to_owned(),
        upstream_protocol: UpstreamProtocol::Responses,
        protocol_profile_id: "profile/responses".to_owned(),
        protocol_profile_revision: 1,
    }
}

fn model(
    suffix: &str,
    native_reasoning: ComputeCandidateFactValueV2<NativeReasoningCapabilityV1>,
) -> ComputeCandidateModelFactsV2 {
    ComputeCandidateModelFactsV2 {
        model_ref: format!("model/{suffix}"),
        upstream_model_id: format!("model-{suffix}"),
        display_name: format!("Model {suffix}"),
        catalog_configuration_id: None,
        membership: ComputeModelMembershipV2::UserDeclared,
        capabilities: ComputeCandidateCapabilityFactsV2 {
            tool: unknown(),
            vision: unknown(),
            streaming: unknown(),
            context_tokens: unknown(),
            max_output_tokens: unknown(),
            native_reasoning,
        },
        capability_evidence_digest: CanonicalDigest::of_bytes(
            format!("capability/{suffix}").as_bytes(),
        ),
        selectable: true,
        reason: None,
    }
}

fn native_facts(
    authentication: GatewayAuthenticationSemanticsV1,
    credential_binding: ComputeCredentialBindingV2,
    models: Vec<ComputeCandidateModelFactsV2>,
) -> ComputeCandidateFactsV2 {
    ComputeCandidateFactsV2 {
        candidate: ComputeCandidateRefV2 {
            candidate_ref: "candidate/native".to_owned(),
            candidate_revision: 42,
        },
        correlation: ComputeCheckCorrelationV2 {
            candidate_ref: "candidate/native".to_owned(),
            edit_revision: 7,
            check_id: "check/7".to_owned(),
            input_digest: CanonicalDigest::of_bytes(b"input"),
        },
        producer: ComputeCandidateProducerV2::Native,
        lineage_ref: "lineage/native".to_owned(),
        trusted_lineage_digest: None,
        display_name: "Native API".to_owned(),
        existing_source_id: None,
        evidence_digest: CanonicalDigest::of_bytes(b"evidence"),
        provenance: ComputeCandidateProvenanceV2::UserConfigured {
            configuration_revision: 3,
            evidence_digest: CanonicalDigest::of_bytes(b"configuration"),
        },
        target: Some(target()),
        authentication: Some(authentication),
        models,
        native_recheck: None,
        discovery_guard: None,
        credential_binding,
        validation: None,
    }
}

fn operation(id: &str) -> OperationReferenceV1 {
    OperationReferenceV1 {
        operation_id: id.to_owned(),
        state: "succeeded".to_owned(),
        sequence: 2,
        cancellable: false,
    }
}

fn validation(revision: u64) -> ComputeValidationRefV2 {
    ComputeValidationRefV2 {
        approval_operation: operation("operation/check-a"),
        validation_ref: "validation/cpa".to_owned(),
        validation_revision: revision,
    }
}

fn cpa_verified_facts() -> ComputeCandidateFactsV2 {
    let validation = validation(1);
    ComputeCandidateFactsV2 {
        candidate: ComputeCandidateRefV2 {
            candidate_ref: "candidate/cpa".to_owned(),
            candidate_revision: 11,
        },
        correlation: ComputeCheckCorrelationV2 {
            candidate_ref: "candidate/cpa".to_owned(),
            edit_revision: 2,
            check_id: "check/cpa/2".to_owned(),
            input_digest: CanonicalDigest::of_bytes(b"cpa-input"),
        },
        producer: ComputeCandidateProducerV2::Cpa,
        lineage_ref: "lineage/cpa".to_owned(),
        trusted_lineage_digest: None,
        display_name: "CPA subscription".to_owned(),
        existing_source_id: None,
        evidence_digest: CanonicalDigest::of_bytes(b"cpa-evidence"),
        provenance: ComputeCandidateProvenanceV2::ConnectorOwned {
            connector_id: "connector/cpa".to_owned(),
            account_ref: "account/current".to_owned(),
        },
        target: Some(target()),
        authentication: Some(GatewayAuthenticationSemanticsV1::Bearer),
        models: vec![model("cpa", unknown())],
        native_recheck: None,
        discovery_guard: None,
        credential_binding: ComputeCredentialBindingV2::CpaOwned {
            account_ref: "account/current".to_owned(),
            validation: validation.clone(),
        },
        validation: Some(validation),
    }
}

fn connector_verified_fact<T>(value: T) -> ComputeCandidateFactValueV2<T> {
    ComputeCandidateFactValueV2 {
        value: Some(value),
        basis: ComputeCandidateFactBasisV2::ConnectorVerified,
    }
}

fn connector_verified_model(
    model_ref: &str,
    upstream_model_id: &str,
) -> ComputeCandidateModelFactsV2 {
    ComputeCandidateModelFactsV2 {
        model_ref: model_ref.into(),
        upstream_model_id: upstream_model_id.into(),
        display_name: format!("Verified {upstream_model_id}"),
        catalog_configuration_id: Some(format!("catalog/{upstream_model_id}")),
        membership: ComputeModelMembershipV2::Catalog,
        capabilities: ComputeCandidateCapabilityFactsV2 {
            tool: connector_verified_fact(true),
            vision: connector_verified_fact(false),
            streaming: connector_verified_fact(true),
            context_tokens: connector_verified_fact(32_768),
            max_output_tokens: connector_verified_fact(4_096),
            native_reasoning: connector_verified_fact(NativeReasoningCapabilityV1::Fixed {
                profile: "provider-default".into(),
            }),
        },
        capability_evidence_digest: CanonicalDigest::of_bytes(upstream_model_id.as_bytes()),
        selectable: true,
        reason: None,
    }
}

#[test]
fn public_projection_omits_protected_slots_and_source_locators() {
    let mut facts = native_facts(
        GatewayAuthenticationSemanticsV1::ApiKeyHeader {
            header: "x-api-key".to_owned(),
        },
        ComputeCredentialBindingV2::NativeProtected {
            descriptor: ProtectedInputSourceDescriptorV1::DiscoveredConfig {
                scanner_id: "scanner/native".to_owned(),
                scanner_version: "1".to_owned(),
                source_ref: "protected-source-canary".to_owned(),
                field_selector: "protected-field-canary".to_owned(),
                observed_revision: 2,
            },
            input_slot: "protected-slot-canary".to_owned(),
        },
        vec![model("one", unknown())],
    );
    facts.native_recheck = Some(hiroute_domain::ComputeNativeRecheckDescriptorV2 {
        display_template_id: None,
        inventory_path: Some("/private-recheck-canary/models".into()),
        protocol_header_semantics: hiroute_domain::GatewayHeaderSemanticsV1 {
            content_type: "application/json".into(),
            required_headers: vec![("anthropic-version".into(), "recheck-canary".into())],
            forbidden_forward_headers: vec!["authorization".into()],
        },
    });
    facts.discovery_guard = Some(ComputeDiscoveryEvidenceGuardV1 {
        evidence_digest: facts.evidence_digest.clone(),
    });

    assert!(facts.validate_shape().is_ok());
    let view = facts
        .public_view(ComputeCandidateInputStateV2::Provided, Vec::new())
        .unwrap();
    let encoded = serde_json::to_string(&view).unwrap();
    for canary in [
        "protected-source-canary",
        "protected-field-canary",
        "protected-slot-canary",
        "private-recheck-canary",
        "recheck-canary",
    ] {
        assert!(!encoded.contains(canary), "public view leaked {canary}");
    }
    assert_eq!(view.candidate.candidate_revision, 42);
    assert_eq!(view.correlation.edit_revision, 7);
    assert_eq!(view.fact_state, ComputeCandidateFactStateV2::Complete);

    let mut mismatched = facts;
    mismatched.authentication = Some(GatewayAuthenticationSemanticsV1::None);
    assert!(mismatched.validate_shape().is_err());
}

#[test]
fn saved_recheck_lineage_is_internal_and_requires_an_existing_source() {
    let trusted = CanonicalDigest::of_bytes(b"exact-saved-source-lineage");
    let mut facts = native_facts(
        GatewayAuthenticationSemanticsV1::None,
        ComputeCredentialBindingV2::None,
        vec![model("one", unknown())],
    );
    facts.existing_source_id = Some("source/existing".into());
    facts.trusted_lineage_digest = Some(trusted.clone());
    assert!(facts.validate_shape().is_ok());
    assert_eq!(
        super::mutation_support::candidate_lineage_digest(&facts).unwrap(),
        trusted
    );
    let public = serde_json::to_string(
        &facts
            .public_view(ComputeCandidateInputStateV2::NotRequired, Vec::new())
            .unwrap(),
    )
    .unwrap();
    assert!(!public.contains("trusted_lineage"));
    assert!(!public.contains("exact-saved-source-lineage"));

    let mut unbound = facts.clone();
    unbound.existing_source_id = None;
    assert!(unbound.validate_shape().is_err());

    let mut cpa = cpa_verified_facts();
    cpa.existing_source_id = Some("source/subscription".into());
    cpa.trusted_lineage_digest = Some(trusted.clone());
    assert!(cpa.validate_shape().is_ok());
    assert_eq!(
        super::mutation_support::candidate_lineage_digest(&cpa).unwrap(),
        trusted
    );
    cpa.existing_source_id = None;
    assert!(cpa.validate_shape().is_err());
}

#[test]
fn native_reasoning_preserves_discrete_budget_and_unknown_facts() {
    let discrete = NativeReasoningCapabilityV1::Discrete {
        parameter: "reasoning_effort".to_owned(),
        profiles: vec!["low".to_owned(), "medium".to_owned(), "high".to_owned()],
    };
    let budget = NativeReasoningCapabilityV1::Budget {
        parameter: "thinking.budget_tokens".to_owned(),
        minimum_tokens: 1_024,
        maximum_tokens: 8_192,
        step_tokens: 1_024,
    };
    let facts = native_facts(
        GatewayAuthenticationSemanticsV1::None,
        ComputeCredentialBindingV2::None,
        vec![
            model(
                "discrete",
                ComputeCandidateFactValueV2 {
                    value: Some(discrete.clone()),
                    basis: ComputeCandidateFactBasisV2::UserDeclared,
                },
            ),
            model(
                "budget",
                ComputeCandidateFactValueV2 {
                    value: Some(budget.clone()),
                    basis: ComputeCandidateFactBasisV2::UserDeclared,
                },
            ),
            model("unknown", unknown()),
        ],
    );

    assert!(facts.validate_shape().is_ok());
    assert_eq!(
        facts.models[0].capabilities.native_reasoning.value,
        Some(discrete)
    );
    assert_eq!(
        facts.models[1].capabilities.native_reasoning.value,
        Some(budget)
    );
    assert_eq!(
        facts.models[2].capabilities.native_reasoning,
        ComputeCandidateFactValueV2 {
            value: None,
            basis: ComputeCandidateFactBasisV2::Unknown,
        }
    );

    let mut guessed_unknown = facts;
    guessed_unknown.models[2]
        .capabilities
        .native_reasoning
        .value = Some(NativeReasoningCapabilityV1::Fixed {
        profile: "provider-default".to_owned(),
    });
    assert!(guessed_unknown.validate_shape().is_err());
}

#[test]
fn native_candidate_without_key_is_visible_but_not_complete_for_save() {
    let facts = native_facts(
        GatewayAuthenticationSemanticsV1::Bearer,
        ComputeCredentialBindingV2::NativePendingInput,
        vec![model("one", unknown())],
    );

    assert!(facts.validate_shape().is_ok());
    let view = facts
        .public_view(ComputeCandidateInputStateV2::Missing, Vec::new())
        .unwrap();
    assert_eq!(view.input_state, ComputeCandidateInputStateV2::Missing);
    assert_eq!(
        view.fact_state,
        ComputeCandidateFactStateV2::PendingCredential
    );
    assert!(!view.fact_state.is_complete());
    assert!(
        facts
            .public_view(ComputeCandidateInputStateV2::Provided, Vec::new())
            .is_err()
    );
}

#[test]
fn cpa_candidate_without_models_or_lease_can_be_registered_for_approval() {
    let facts = ComputeCandidateFactsV2 {
        candidate: ComputeCandidateRefV2 {
            candidate_ref: "candidate/cpa".to_owned(),
            candidate_revision: 10,
        },
        correlation: ComputeCheckCorrelationV2 {
            candidate_ref: "candidate/cpa".to_owned(),
            edit_revision: 1,
            check_id: "check/cpa/1".to_owned(),
            input_digest: CanonicalDigest::of_bytes(b"cpa-protected-context"),
        },
        producer: ComputeCandidateProducerV2::Cpa,
        lineage_ref: "lineage/cpa".to_owned(),
        trusted_lineage_digest: None,
        display_name: "CPA subscription".to_owned(),
        existing_source_id: None,
        evidence_digest: CanonicalDigest::of_bytes(b"cpa-pending-evidence"),
        provenance: ComputeCandidateProvenanceV2::ConnectorOwnedPendingApproval {
            connector_id: "connector/cpa".to_owned(),
        },
        target: None,
        authentication: None,
        models: Vec::new(),
        native_recheck: None,
        discovery_guard: None,
        credential_binding: ComputeCredentialBindingV2::CpaPendingApproval {
            protected_source: ProtectedInputSourceDescriptorV1::DiscoveredConfig {
                scanner_id: "scanner/cpa".to_owned(),
                scanner_version: "1".to_owned(),
                source_ref: "cpa-context-canary".to_owned(),
                field_selector: "native-context".to_owned(),
                observed_revision: 1,
            },
        },
        validation: None,
    };

    assert!(facts.validate_shape().is_ok());
    let view = facts
        .public_view(ComputeCandidateInputStateV2::NotRequired, Vec::new())
        .unwrap();
    assert!(view.models.is_empty());
    assert!(view.validation.is_none());
    assert_eq!(
        view.fact_state,
        ComputeCandidateFactStateV2::PendingApproval
    );
    assert!(!view.fact_state.is_complete());
    assert!(
        !serde_json::to_string(&view)
            .unwrap()
            .contains("cpa-context-canary")
    );
}

#[test]
fn cpa_verified_facts_require_matching_stable_account_and_validation() {
    let facts = cpa_verified_facts();
    assert!(facts.validate_shape().is_ok());
    let view = facts
        .public_view(ComputeCandidateInputStateV2::NotRequired, Vec::new())
        .unwrap();
    assert_eq!(view.fact_state, ComputeCandidateFactStateV2::Complete);
    assert_eq!(view.validation, Some(validation(1)));

    let mut mismatched_account = facts.clone();
    if let ComputeCandidateProvenanceV2::ConnectorOwned { account_ref, .. } =
        &mut mismatched_account.provenance
    {
        *account_ref = "account/stale".to_owned();
    }
    assert!(mismatched_account.validate_shape().is_err());

    let mut mismatched_validation = facts.clone();
    mismatched_validation.validation = Some(validation(2));
    assert!(mismatched_validation.validate_shape().is_err());

    let mut native_with_cpa_binding = facts.clone();
    native_with_cpa_binding.producer = ComputeCandidateProducerV2::Native;
    assert!(native_with_cpa_binding.validate_shape().is_err());

    let mut cpa_with_native_binding = facts;
    cpa_with_native_binding.credential_binding = ComputeCredentialBindingV2::NativeSaved {
        credential_id: "credential/native".to_owned(),
        expected_generation: 1,
    };
    assert!(cpa_with_native_binding.validate_shape().is_err());
}

#[test]
fn subscription_refresh_retains_exact_membership_and_excludes_lost_models_from_execution() {
    let mut current = complete_management_source();
    current.source_id = "source/subscription".into();
    current.revision = 7;
    current.lineage_digest = CanonicalDigest::of_bytes(b"stable-subscription-lineage");
    current.provenance = hiroute_domain::ComputeManagementProvenanceV2::ConnectorOwned {
        connector_id: "connector/cpa".into(),
        account_ref: "account/current".into(),
    };
    current.authentication = GatewayAuthenticationSemanticsV1::Bearer;
    current.validation = Some(hiroute_domain::ComputeManagementValidationV2 {
        approval_operation_id: "operation/check-a".into(),
        validation_ref: "validation/cpa".into(),
        validation_revision: 1,
    });
    current.last_candidate_ref = "candidate/cpa".into();
    current.last_candidate_revision = 11;
    current.models[0].model_ref = "model/kept".into();
    current.models[0].binding_id = "binding/kept".into();
    current.models[0].catalog_configuration_id = Some("catalog/kept".into());
    current.models[0].membership = hiroute_domain::ComputeManagementMembershipV2::Catalog;
    let mut removed = current.models[0].clone();
    removed.model_ref = "model/removed".into();
    removed.binding_id = "binding/removed".into();
    removed.upstream_model_id = "removed-upstream".into();
    removed.display_name = "Removed model".into();
    removed.catalog_configuration_id = Some("catalog/removed".into());
    current.models.push(removed.clone());
    current.validate().unwrap();

    let mut refreshed = cpa_verified_facts();
    refreshed.models = vec![
        connector_verified_model("model/kept", "kept-upstream-v2"),
        connector_verified_model("model/new", "new-upstream"),
    ];
    let selected = vec!["model/kept".to_owned(), "model/removed".to_owned()];
    let retained = super::mutation_support::retain_subscription_models(
        &current.source_id,
        &current,
        &refreshed,
        &selected,
    )
    .unwrap();

    assert_eq!(retained.len(), 2);
    assert_eq!(retained[0].model_ref, "model/kept");
    assert_eq!(retained[0].binding_id, "binding/kept");
    assert_eq!(retained[0].upstream_model_id, "kept-upstream-v2");
    assert!(retained[0].execution_eligible);
    assert_eq!(retained[0].revision, current.models[0].revision + 1);
    assert_eq!(retained[1].model_ref, "model/removed");
    assert_eq!(retained[1].binding_id, removed.binding_id);
    assert!(!retained[1].execution_eligible);
    assert_eq!(retained[1].revision, removed.revision + 1);
    assert!(retained.iter().all(|model| model.model_ref != "model/new"));

    let mut updated = current.clone();
    updated.revision += 1;
    updated.models = retained;
    let compiled = compile_compute_management_source(&updated).unwrap();
    assert_eq!(compiled.len(), 1);
    assert_eq!(compiled[0].model_ref, "model/kept");

    assert!(
        super::mutation_support::retain_subscription_models(
            &current.source_id,
            &current,
            &refreshed,
            &["model/kept".into()],
        )
        .is_err()
    );
}

#[test]
fn resource_receipts_reject_missing_identity_or_revision() {
    assert!(ComputeSubscriptionResourceReceiptV2::new("", 1).is_err());
    assert!(ComputeSubscriptionResourceReceiptV2::new("receipt/cpa", 0).is_err());
    assert_eq!(
        ComputeSubscriptionResourceReceiptV2::new("receipt/cpa", 2)
            .unwrap()
            .revision(),
        2
    );
}

#[test]
fn trusted_registry_keeps_only_the_current_immutable_revision() {
    let registry = TrustedComputeCandidateRegistry::new();
    let facts = native_facts(
        GatewayAuthenticationSemanticsV1::None,
        ComputeCredentialBindingV2::None,
        vec![model("one", unknown())],
    );
    let view = registry.register_compute_candidate(facts.clone()).unwrap();
    assert_eq!(view.candidate, facts.candidate);
    assert_eq!(
        registry.register_compute_candidate(facts.clone()).unwrap(),
        view
    );
    assert!(
        registry
            .resolve_compute_candidate(&facts.candidate)
            .is_ok_and(|resolved| resolved == facts)
    );

    let mut changed_same_revision = facts.clone();
    changed_same_revision.display_name = "mutated".into();
    let immutable = registry
        .register_compute_candidate(changed_same_revision)
        .unwrap_err();
    assert_eq!(immutable.code, PortErrorCode::Conflict);

    let old_candidate = facts.candidate.clone();
    let mut newer = facts.clone();
    newer.candidate.candidate_revision += 1;
    newer.correlation.edit_revision += 1;
    newer.correlation.check_id = "check/8".into();
    newer.correlation.input_digest = CanonicalDigest::of_bytes(b"new-input");
    newer.display_name = "Native API refreshed".into();
    newer.evidence_digest = CanonicalDigest::of_bytes(b"new-evidence");
    registry.register_compute_candidate(newer.clone()).unwrap();

    for rejected in [
        registry.get_compute_candidate(&old_candidate).map(|_| ()),
        registry
            .resolve_compute_candidate(&old_candidate)
            .map(|_| ()),
    ] {
        let error = rejected.unwrap_err();
        assert_eq!(error.code, PortErrorCode::Conflict);
        assert_eq!(error.context, "compute.candidate.registry.revision_changed");
    }
    assert!(
        registry
            .resolve_compute_candidate(&newer.candidate)
            .is_ok_and(|resolved| resolved == newer)
    );

    let mut rollback = facts;
    rollback.candidate.candidate_revision -= 1;
    let rollback = registry.register_compute_candidate(rollback).unwrap_err();
    assert_eq!(rollback.code, PortErrorCode::Conflict);
    assert_eq!(rollback.context, "compute.candidate.registry.rollback");
}

#[test]
fn trusted_registry_rejects_new_refs_at_capacity_without_reopening_old_revisions() {
    let registry = TrustedComputeCandidateRegistry::new();
    let mut current = native_facts(
        GatewayAuthenticationSemanticsV1::None,
        ComputeCredentialBindingV2::None,
        vec![model("one", unknown())],
    );
    current.candidate.candidate_revision = 2;
    registry
        .register_compute_candidate(current.clone())
        .unwrap();

    for index in 1..super::candidates::MAX_TRUSTED_CANDIDATES {
        let mut facts = current.clone();
        facts.candidate.candidate_ref = format!("candidate/native-{index}");
        facts.correlation.candidate_ref = facts.candidate.candidate_ref.clone();
        facts.correlation.check_id = format!("check/{index}");
        facts.correlation.input_digest =
            CanonicalDigest::of_bytes(format!("input/{index}").as_bytes());
        facts.lineage_ref = format!("lineage/native-{index}");
        facts.display_name = format!("Native API {index}");
        facts.evidence_digest = CanonicalDigest::of_bytes(format!("evidence/{index}").as_bytes());
        registry.register_compute_candidate(facts).unwrap();
    }

    let mut overflow = current.clone();
    overflow.candidate.candidate_ref = "candidate/overflow".into();
    overflow.correlation.candidate_ref = overflow.candidate.candidate_ref.clone();
    let capacity = registry.register_compute_candidate(overflow).unwrap_err();
    assert_eq!(capacity.code, PortErrorCode::Unavailable);
    assert_eq!(capacity.context, "compute.candidate.registry.capacity");

    let mut refreshed = current.clone();
    refreshed.candidate.candidate_revision = 3;
    refreshed.correlation.edit_revision += 1;
    refreshed.correlation.check_id = "check/refreshed-at-capacity".into();
    refreshed.correlation.input_digest = CanonicalDigest::of_bytes(b"refreshed-at-capacity");
    refreshed.evidence_digest = CanonicalDigest::of_bytes(b"refreshed-evidence-at-capacity");
    registry
        .register_compute_candidate(refreshed.clone())
        .unwrap();

    let mut late = current;
    late.candidate.candidate_revision = 1;
    let rollback = registry.register_compute_candidate(late).unwrap_err();
    assert_eq!(rollback.code, PortErrorCode::Conflict);
    assert_eq!(rollback.context, "compute.candidate.registry.rollback");
    assert!(
        registry
            .resolve_compute_candidate(&refreshed.candidate)
            .is_ok_and(|resolved| resolved == refreshed)
    );
}

fn management_fact<T>(value: T) -> hiroute_domain::ComputeManagementFactValueV2<T> {
    hiroute_domain::ComputeManagementFactValueV2 {
        value: Some(value),
        basis: hiroute_domain::ComputeManagementFactBasisV2::UserDeclared,
    }
}

fn complete_management_source() -> hiroute_domain::ComputeManagementSourceV2 {
    hiroute_domain::ComputeManagementSourceV2 {
        schema: hiroute_domain::COMPUTE_MANAGEMENT_SOURCE_SCHEMA_V2.into(),
        source_id: "source/manual".into(),
        revision: 2,
        lineage_digest: CanonicalDigest::of_bytes(b"manual-lineage"),
        display_name: "Manual".into(),
        provenance: hiroute_domain::ComputeManagementProvenanceV2::UserConfigured {
            configuration_revision: 2,
            evidence_digest: CanonicalDigest::of_bytes(b"manual-evidence"),
        },
        target: hiroute_domain::ComputeManagementTargetV2 {
            scheme: "http".into(),
            authority: "127.0.0.1".into(),
            port: 8_000,
            request_path: "/v1/responses".into(),
            upstream_protocol: UpstreamProtocol::Responses,
            protocol_profile_id: "profile/responses".into(),
            protocol_profile_revision: 3,
        },
        authentication: GatewayAuthenticationSemanticsV1::None,
        state: hiroute_domain::MaterializationState::Ready,
        models: vec![hiroute_domain::ComputeManagedModelV2 {
            model_ref: "model/manual".into(),
            binding_id: "binding/manual".into(),
            revision: 4,
            upstream_model_id: "local-model".into(),
            display_name: "Local model".into(),
            catalog_configuration_id: None,
            membership: hiroute_domain::ComputeManagementMembershipV2::UserDeclared,
            execution_eligible: true,
            capabilities: hiroute_domain::ComputeManagedCapabilitiesV2 {
                tool: management_fact(true),
                vision: management_fact(false),
                streaming: management_fact(true),
                context_tokens: management_fact(16_384_u64),
                max_output_tokens: management_fact(2_048_u64),
                native_reasoning: management_fact(NativeReasoningCapabilityV1::Fixed {
                    profile: "provider-default".into(),
                }),
            },
            capability_evidence_digest: CanonicalDigest::of_bytes(b"model-capability"),
        }],
        native_recheck: None,
        credentials: Vec::new(),
        validation: None,
        last_candidate_ref: "candidate/manual".into(),
        last_candidate_revision: 2,
    }
}

#[test]
fn keyless_native_source_with_saved_descriptor_can_be_rechecked() {
    let mut source = complete_management_source();
    source.native_recheck = Some(hiroute_domain::ComputeNativeRecheckDescriptorV2 {
        display_template_id: None,
        inventory_path: Some("/v1/models".into()),
        protocol_header_semantics: hiroute_domain::GatewayHeaderSemanticsV1 {
            content_type: "application/json".into(),
            required_headers: Vec::new(),
            forbidden_forward_headers: vec!["authorization".into()],
        },
    });
    let actions = super::query::source_actions(&source);
    assert!(actions.contains(&hiroute_application_api::ComputeManagementActionV2::Recheck));
    assert!(!actions.contains(&hiroute_application_api::ComputeManagementActionV2::AddKey));
    source.native_recheck = None;
    assert!(
        !super::query::source_actions(&source)
            .contains(&hiroute_application_api::ComputeManagementActionV2::Recheck)
    );
}

fn keyed_management_source() -> hiroute_domain::ComputeManagementSourceV2 {
    let mut source = complete_management_source();
    source.authentication = GatewayAuthenticationSemanticsV1::Bearer;
    let destination = source.target.credential_destination().unwrap();
    source.credentials = ["a", "b"]
        .into_iter()
        .enumerate()
        .map(|(ordinal, suffix)| {
            let key_id = format!("credential/source-manual/key-{suffix}");
            hiroute_domain::ComputeManagedCredentialV2 {
                key_id: key_id.clone(),
                credential: CredentialRefV1::new(
                    key_id,
                    format!("source/{}", source.source_id),
                    "hirouted",
                    "provider-auth",
                    [destination.clone()],
                    1,
                )
                .unwrap(),
                fingerprint: CanonicalDigest::of_bytes(format!("key-{suffix}").as_bytes()),
                ordinal: u32::try_from(ordinal).unwrap(),
                enabled: true,
            }
        })
        .collect();
    source.validate().unwrap();
    source
}

#[test]
fn local_compilation_preserves_no_credential_and_user_confirmed_reasoning() {
    let source = complete_management_source();
    let facts = compile_compute_management_source(&source).unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(
        facts[0].eligibility,
        ComputeManagementEligibilityV2::UserConfirmed
    );
    assert_eq!(facts[0].binding_revision, 4);
    assert_eq!(facts[0].target.authority, "127.0.0.1");
    assert_eq!(facts[0].capabilities.vision.value, Some(false));
    assert_eq!(
        facts[0].capabilities.vision.basis,
        hiroute_domain::ComputeManagementFactBasisV2::UserDeclared
    );
    assert!(facts[0].catalog_configuration_id.is_none());
    assert_eq!(
        facts[0].credential,
        ComputeManagementCredentialCompilationV2::Native {
            ordered: vec![hiroute_domain::ComputeCredentialSelectionV2::NoCredential],
        }
    );

    let mut forged_catalog_membership = source.clone();
    forged_catalog_membership.models[0].membership =
        hiroute_domain::ComputeManagementMembershipV2::Catalog;
    assert_eq!(
        compile_compute_management_source(&forged_catalog_membership),
        Err(ComputeManagementCompilationErrorV2::EligibilityMismatch)
    );

    let mut mixed_basis = source.clone();
    mixed_basis.models[0].capabilities.vision.basis =
        hiroute_domain::ComputeManagementFactBasisV2::Observed;
    assert_eq!(
        compile_compute_management_source(&mixed_basis),
        Err(ComputeManagementCompilationErrorV2::EligibilityMismatch)
    );

    let mut incomplete = source;
    incomplete.models[0].capabilities.tool = hiroute_domain::ComputeManagementFactValueV2 {
        value: None,
        basis: hiroute_domain::ComputeManagementFactBasisV2::Unknown,
    };
    let compiled = compile_compute_management_source(&incomplete).unwrap();
    assert_eq!(compiled[0].capabilities.tool.value, None);
}

struct OneSourceRepository(hiroute_domain::ComputeManagementSourceV2);

impl hiroute_domain::ComputeManagementRepositoryPort for OneSourceRepository {
    fn compute_management_source(
        &self,
        source_id: &str,
    ) -> hiroute_domain::PortResult<Option<hiroute_domain::ComputeManagementSourceV2>> {
        Ok((self.0.source_id == source_id).then(|| self.0.clone()))
    }

    fn compute_management_source_by_lineage(
        &self,
        lineage_digest: &CanonicalDigest,
    ) -> hiroute_domain::PortResult<Option<hiroute_domain::ComputeManagementSourceV2>> {
        Ok((self.0.lineage_digest == *lineage_digest).then(|| self.0.clone()))
    }

    fn compute_management_snapshot(
        &self,
        _workspace: &hiroute_domain::WorkspaceId,
    ) -> hiroute_domain::PortResult<hiroute_domain::ComputeManagementStoredSnapshotV2> {
        Ok(hiroute_domain::ComputeManagementStoredSnapshotV2 {
            revisions: hiroute_domain::RevisionSetV1 {
                target: 9,
                dependencies: Default::default(),
            },
            sources: vec![self.0.clone()],
        })
    }
}

struct FailingRuntimeRead;

impl hiroute_domain::ComputeRuntimeStateStoreV1 for FailingRuntimeRead {
    fn runtime_state(
        &self,
        _identity: &hiroute_domain::RuntimeStateIdentityV1,
    ) -> hiroute_domain::PortResult<Option<hiroute_domain::RuntimeStateV1>> {
        Err(hiroute_domain::PortError::new(
            hiroute_domain::PortErrorCode::Unavailable,
            "test.runtime.read",
        ))
    }

    fn compare_and_set_runtime_state(
        &self,
        _expected_generation: u64,
        _state: &hiroute_domain::RuntimeStateV1,
    ) -> hiroute_domain::PortResult<()> {
        unreachable!()
    }

    fn acquire_runtime_probe(
        &self,
        _identity: &hiroute_domain::RuntimeStateIdentityV1,
        _expected_generation: u64,
        _request: &hiroute_domain::RuntimeProbeLeaseRequestV1,
    ) -> hiroute_domain::PortResult<hiroute_domain::RuntimeProbeAcquireOutcomeV1> {
        unreachable!()
    }

    fn complete_runtime_probe(
        &self,
        _lease: &hiroute_domain::RuntimeProbeLeaseV1,
        _state: &hiroute_domain::RuntimeStateV1,
    ) -> hiroute_domain::PortResult<()> {
        unreachable!()
    }
}

#[derive(Default)]
struct RuntimeFacts(std::collections::BTreeMap<String, RuntimeStateV1>);

impl RuntimeFacts {
    fn cooling(mut self, identity: RuntimeStateIdentityV1, until_ms: i64) -> Self {
        let state = RuntimeStateV1::cooling_down(
            identity,
            1,
            until_ms,
            None,
            RuntimeClockSampleV1::from_unix_millis(until_ms - 1).unwrap(),
        )
        .unwrap();
        self.0
            .insert(state.identity().canonical_key().unwrap(), state);
        self
    }
}

impl hiroute_domain::ComputeRuntimeStateStoreV1 for RuntimeFacts {
    fn runtime_state(
        &self,
        identity: &RuntimeStateIdentityV1,
    ) -> hiroute_domain::PortResult<Option<RuntimeStateV1>> {
        Ok(self.0.get(&identity.canonical_key().unwrap()).cloned())
    }

    fn compare_and_set_runtime_state(
        &self,
        _expected_generation: u64,
        _state: &RuntimeStateV1,
    ) -> hiroute_domain::PortResult<()> {
        unreachable!()
    }

    fn acquire_runtime_probe(
        &self,
        _identity: &RuntimeStateIdentityV1,
        _expected_generation: u64,
        _request: &hiroute_domain::RuntimeProbeLeaseRequestV1,
    ) -> hiroute_domain::PortResult<hiroute_domain::RuntimeProbeAcquireOutcomeV1> {
        unreachable!()
    }

    fn complete_runtime_probe(
        &self,
        _lease: &hiroute_domain::RuntimeProbeLeaseV1,
        _state: &RuntimeStateV1,
    ) -> hiroute_domain::PortResult<()> {
        unreachable!()
    }
}

fn presentation_facts() -> ComputeManagementPresentationFactsV1 {
    ComputeManagementPresentationFactsV1 {
        evaluated_at_ms: 1_700_000_000_000,
        complete: true,
        revisions: Some(hiroute_domain::RevisionSetV1 {
            target: 9,
            dependencies: Default::default(),
        }),
        sources: vec![ComputeManagementSourcePresentationFactV1 {
            source_id: "source/manual".into(),
            identity: ComputeConnectionIdentityV1 {
                access_kind: ComputeConnectionAccessKindV1::Api,
                connection_option_id: None,
                product_label: Some("Manual".into()),
            },
        }],
        models: vec![ComputeManagementModelPresentationFactV1 {
            binding_id: "binding/manual".into(),
            billing_class: BillingClass::Paid,
            price_contexts: vec![
                ComputePriceContextV1 {
                    currency: "USD".into(),
                    valuation_kind: PriceValuationKindV1::UsageEstimate,
                },
                ComputePriceContextV1 {
                    currency: "CNY".into(),
                    valuation_kind: PriceValuationKindV1::UsageEstimate,
                },
            ],
            runtime_availability:
                super::query::ComputeManagementModelRuntimeAvailabilityFactV1::BindingAndKeys,
        }],
    }
}

fn credential_runtime_identity(
    source: &hiroute_domain::ComputeManagementSourceV2,
    index: usize,
) -> RuntimeStateIdentityV1 {
    let key = &source.credentials[index];
    RuntimeStateIdentityV1::credential(
        source.models[0].binding_id.clone(),
        key.credential.credential_id(),
        key.key_id.clone(),
        key.credential.generation(),
    )
    .unwrap()
}

#[test]
fn management_model_availability_uses_all_keys_and_closed_priority() {
    let source = keyed_management_source();
    let one_cooling = RuntimeFacts::default().cooling(credential_runtime_identity(&source, 0), 200);
    let available = query_compute_management_with_presentation(
        &OneSourceRepository(source.clone()),
        &one_cooling,
        &hiroute_domain::WorkspaceId::default(),
        &hiroute_application_api::ComputeManagementQueryV2::default(),
        Some(&presentation_facts()),
    )
    .unwrap();
    let model = &available.sources[0].models[0];
    assert_eq!(
        available.sources[0].connection_identity,
        ComputeConnectionIdentityV1 {
            access_kind: ComputeConnectionAccessKindV1::Api,
            connection_option_id: None,
            product_label: Some("Manual".into()),
        }
    );
    assert_eq!(model.presentation.billing_class, BillingClass::Paid);
    assert_eq!(
        model.presentation.availability,
        ComputeModelAvailabilityV1::Available
    );
    assert!(model.presentation.reason_code.is_none());
    assert_eq!(
        model.presentation.price_contexts,
        vec![
            ComputePriceContextV1 {
                currency: "CNY".into(),
                valuation_kind: PriceValuationKindV1::UsageEstimate,
            },
            ComputePriceContextV1 {
                currency: "USD".into(),
                valuation_kind: PriceValuationKindV1::UsageEstimate,
            },
        ]
    );
    assert_eq!(available.sources[0].ready_model_count, 1);

    let all_cooling = one_cooling.cooling(credential_runtime_identity(&source, 1), 300);
    let cooling = query_compute_management_with_presentation(
        &OneSourceRepository(source.clone()),
        &all_cooling,
        &hiroute_domain::WorkspaceId::default(),
        &hiroute_application_api::ComputeManagementQueryV2::default(),
        Some(&presentation_facts()),
    )
    .unwrap();
    assert_eq!(
        cooling.sources[0].models[0].presentation.availability,
        ComputeModelAvailabilityV1::CoolingDown
    );
    assert_eq!(
        cooling.sources[0].models[0].presentation.reason_code,
        Some(ComputeModelAvailabilityReasonV1::AllCredentialsCooling)
    );
    assert_eq!(cooling.sources[0].ready_model_count, 0);

    let mut disabled_source = source;
    disabled_source.state = hiroute_domain::MaterializationState::Disabled;
    let disabled = query_compute_management_with_presentation(
        &OneSourceRepository(disabled_source),
        &FailingRuntimeRead,
        &hiroute_domain::WorkspaceId::default(),
        &hiroute_application_api::ComputeManagementQueryV2::default(),
        Some(&presentation_facts()),
    )
    .unwrap();
    assert_eq!(
        disabled.sources[0].models[0].presentation.availability,
        ComputeModelAvailabilityV1::Disabled
    );
    assert_eq!(
        disabled.sources[0].models[0].presentation.reason_code,
        Some(ComputeModelAvailabilityReasonV1::SourceDisabled)
    );
}

#[test]
fn management_missing_presentation_fact_keeps_model_visible_and_marks_partial() {
    let source = complete_management_source();
    let mut facts = presentation_facts();
    facts.models.clear();
    let result = query_compute_management_with_presentation(
        &OneSourceRepository(source),
        &RuntimeFacts::default(),
        &hiroute_domain::WorkspaceId::default(),
        &hiroute_application_api::ComputeManagementQueryV2::default(),
        Some(&facts),
    )
    .unwrap();
    assert_eq!(
        result.runtime_state,
        hiroute_application_api::ComputeManagementRuntimeReadStateV2::Partial
    );
    assert_eq!(result.sources[0].models.len(), 1);
    assert_eq!(
        result.sources[0].models[0].presentation.availability,
        ComputeModelAvailabilityV1::Unknown
    );
    assert_eq!(
        result.sources[0].models[0].presentation.reason_code,
        Some(ComputeModelAvailabilityReasonV1::FactsUnavailable)
    );
    assert_eq!(result.sources[0].ready_model_count, 0);
}

#[test]
fn management_query_retains_ineligible_model_as_explicitly_not_allowed() {
    let mut source = complete_management_source();
    source.models[0].execution_eligible = false;
    let result = query_compute_management_with_presentation(
        &OneSourceRepository(source),
        &RuntimeFacts::default(),
        &hiroute_domain::WorkspaceId::default(),
        &hiroute_application_api::ComputeManagementQueryV2::default(),
        Some(&presentation_facts()),
    )
    .unwrap();
    assert_eq!(result.sources[0].models.len(), 1);
    assert_eq!(
        result.sources[0].models[0].presentation.availability,
        ComputeModelAvailabilityV1::Unavailable
    );
    assert_eq!(
        result.sources[0].models[0].presentation.reason_code,
        Some(ComputeModelAvailabilityReasonV1::ModelNotAllowed)
    );
    assert_eq!(result.sources[0].ready_model_count, 0);
}

#[test]
fn management_identity_is_catalog_bound_and_connector_ownership_is_not_guessed() {
    let mut registered = complete_management_source();
    registered.provenance = hiroute_domain::ComputeManagementProvenanceV2::Registered {
        connection_option_id: "provider.payg.test.v1".into(),
        registry_version: "registry-v1".into(),
        catalog_digest: CanonicalDigest::of_bytes(b"registered-catalog"),
    };
    let mut registered_facts = presentation_facts();
    registered_facts.sources[0].identity = ComputeConnectionIdentityV1 {
        access_kind: ComputeConnectionAccessKindV1::Api,
        connection_option_id: Some("provider.payg.test.v1".into()),
        product_label: Some("Provider PAYG".into()),
    };
    let result = query_compute_management_with_presentation(
        &OneSourceRepository(registered.clone()),
        &RuntimeFacts::default(),
        &hiroute_domain::WorkspaceId::default(),
        &hiroute_application_api::ComputeManagementQueryV2::default(),
        Some(&registered_facts),
    )
    .unwrap();
    assert_eq!(
        result.sources[0].connection_identity,
        registered_facts.sources[0].identity
    );
    assert_eq!(
        result.sources[0].models[0].presentation.availability,
        ComputeModelAvailabilityV1::Available
    );

    let mut stale_facts = registered_facts.clone();
    stale_facts.revisions.as_mut().unwrap().target += 1;
    let stale = query_compute_management_with_presentation(
        &OneSourceRepository(registered),
        &RuntimeFacts::default(),
        &hiroute_domain::WorkspaceId::default(),
        &hiroute_application_api::ComputeManagementQueryV2::default(),
        Some(&stale_facts),
    )
    .unwrap();
    assert_eq!(
        stale.sources[0].connection_identity.access_kind,
        ComputeConnectionAccessKindV1::Unknown
    );
    assert_eq!(
        stale.sources[0].models[0].presentation.billing_class,
        BillingClass::Unknown
    );
    assert!(
        stale.sources[0].models[0]
            .presentation
            .price_contexts
            .is_empty()
    );
    assert_eq!(
        stale.sources[0].models[0].presentation.availability,
        ComputeModelAvailabilityV1::Unknown
    );

    let mut connector = complete_management_source();
    connector.provenance = hiroute_domain::ComputeManagementProvenanceV2::ConnectorOwned {
        connector_id: "connector.test".into(),
        account_ref: "account/test".into(),
    };
    connector.validation = Some(hiroute_domain::ComputeManagementValidationV2 {
        approval_operation_id: "operation/approval".into(),
        validation_ref: "validation/connector".into(),
        validation_revision: 1,
    });
    let mut incomplete_facts = presentation_facts();
    incomplete_facts.sources.clear();
    let result = query_compute_management_with_presentation(
        &OneSourceRepository(connector),
        &RuntimeFacts::default(),
        &hiroute_domain::WorkspaceId::default(),
        &hiroute_application_api::ComputeManagementQueryV2::default(),
        Some(&incomplete_facts),
    )
    .unwrap();
    assert_eq!(
        result.runtime_state,
        hiroute_application_api::ComputeManagementRuntimeReadStateV2::Partial
    );
    assert_eq!(
        result.sources[0].connection_identity.access_kind,
        ComputeConnectionAccessKindV1::Unknown
    );
    assert_eq!(
        result.sources[0].models[0].presentation.availability,
        ComputeModelAvailabilityV1::Unknown
    );
    assert_eq!(
        result.sources[0].models[0].presentation.reason_code,
        Some(ComputeModelAvailabilityReasonV1::FactsUnavailable)
    );
}

#[test]
fn management_query_keeps_runtime_read_failures_visible_as_unknown() {
    let source = complete_management_source();
    let result = query_compute_management(
        &OneSourceRepository(source),
        &FailingRuntimeRead,
        &hiroute_domain::WorkspaceId::default(),
        &hiroute_application_api::ComputeManagementQueryV2::default(),
    )
    .unwrap();
    assert_eq!(
        result.runtime_state,
        hiroute_application_api::ComputeManagementRuntimeReadStateV2::Partial
    );
    assert_eq!(result.sources[0].ready_model_count, 0);
}
