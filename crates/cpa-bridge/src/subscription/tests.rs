use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use hiroute_application::compute_management::{
    ComputeApprovedSubscriptionCheckV2, ComputeSubscriptionMaterializationPort,
    ComputeSubscriptionResourceOwnerV2, ComputeSubscriptionResourceReceiptV2,
    ProtectedInputSourceDescriptorV1,
};
use hiroute_application_api::{
    ComputeCandidateRefV2, ComputeSavedSourceExpectationV2, OperationReferenceV1,
};
use hiroute_domain::{
    BillingClass, COMPUTE_STATE_SCHEMA_V1, CanonicalDigest, ComputeSourceV1, CredentialRefV1,
    EffectiveInventoryModelV1, InventoryDisposition, MaterializationState,
    NativeReasoningCapabilityV1, SourceIdentityV1, SourceOrigin,
};

use crate::config::{ensure_private_dir, private_atomic_write};

use super::*;

struct FakeRuntime {
    evidence: parking_lot::Mutex<BorrowedSubscriptionEvidence>,
    materialize_calls: AtomicUsize,
    sources: Vec<CpaRegisteredSourceV1>,
}

impl CpaSubscriptionRuntimePort for FakeRuntime {
    fn inspect(&self) -> Result<BorrowedSubscriptionEvidence, CpaLifecycleError> {
        Ok(self.evidence.lock().clone())
    }

    fn materialize(
        &self,
        expected: &BorrowedSubscriptionEvidence,
    ) -> Result<Vec<CpaRegisteredSourceV1>, CpaLifecycleError> {
        assert_eq!(expected, &*self.evidence.lock());
        self.materialize_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.sources.clone())
    }

    fn catalog_model_facts(
        &self,
        _source: &CpaRegisteredSourceV1,
        model: &EffectiveInventoryModelV1,
    ) -> Result<Option<CpaCatalogModelFacts>, CpaLifecycleError> {
        Ok(
            (model.disposition == InventoryDisposition::CatalogMatched).then(|| {
                CpaCatalogModelFacts {
                    display_name: "Known model".to_owned(),
                    tool: true,
                    vision: false,
                    streaming: true,
                    context_tokens: 128_000,
                    max_output_tokens: 16_384,
                    native_reasoning: Some(NativeReasoningCapabilityV1::Discrete {
                        parameter: "reasoning_effort".to_owned(),
                        profiles: vec!["low".to_owned(), "high".to_owned()],
                        default_profile: None,
                    }),
                    capability_evidence_digest: CanonicalDigest::of_bytes(b"catalog-capability"),
                }
            }),
        )
    }

    fn runtime_fallback_allowed(&self, upstream_model_id: &str) -> bool {
        upstream_model_id != "gpt-image-2"
    }
}

fn operation(id: &str) -> OperationReferenceV1 {
    OperationReferenceV1 {
        operation_id: id.to_owned(),
        state: "accepted".to_owned(),
        sequence: 1,
        cancellable: true,
    }
}

fn candidate() -> ComputeCandidateRefV2 {
    ComputeCandidateRefV2 {
        candidate_ref: "candidate/cpa/codex".to_owned(),
        candidate_revision: 4,
    }
}

fn descriptor(source_ref: &str) -> ProtectedInputSourceDescriptorV1 {
    ProtectedInputSourceDescriptorV1::DiscoveredConfig {
        scanner_id: "codex".to_owned(),
        scanner_version: "1".to_owned(),
        source_ref: source_ref.to_owned(),
        field_selector: "tokens".to_owned(),
        observed_revision: 1,
    }
}

fn source_evidence(
    account: &str,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    BorrowedSubscriptionEvidence,
) {
    let temp = tempfile::tempdir().unwrap();
    ensure_private_dir(temp.path()).unwrap();
    let path = temp.path().join("auth.json");
    let bytes = serde_json::to_vec(&serde_json::json!({
        "OPENAI_API_KEY": null,
        "auth_mode": "chatgpt",
        "last_refresh": "fixture",
        "tokens": {
            "access_token": "fixture-access",
            "id_token": "fixture-id",
            "refresh_token": "must-not-copy",
            "account_id": account
        }
    }))
    .unwrap();
    private_atomic_write(&path, &bytes).unwrap();
    let evidence = crate::BorrowedCodexAuthSpec::new(&path).inspect().unwrap();
    (temp, path, evidence.into())
}

fn registered(evidence: &BorrowedSubscriptionEvidence) -> CpaRegisteredSourceV1 {
    let evidence_ref = CanonicalDigest::of_bytes(b"registered");
    let identity = SourceIdentityV1 {
        identity_revision: 1,
        provider_platform_id: "openai".to_owned(),
        service_offering_id: "chatgpt".to_owned(),
        entitlement_id: "subscription".to_owned(),
        usage_scope: "personal".to_owned(),
        endpoint_profile_id: "endpoint.cpa.codex".to_owned(),
        endpoint_profile_revision: 1,
        region_id: "global".to_owned(),
        account_subject_ref: evidence.account_ref(),
        evidence_refs: vec![evidence_ref],
    };
    let identity_digest = identity.digest().unwrap();
    let source_id = evidence
        .account_ref()
        .strip_prefix("account/")
        .unwrap()
        .to_owned();
    CpaRegisteredSourceV1 {
        source: ComputeSourceV1 {
            schema: COMPUTE_STATE_SCHEMA_V1.to_owned(),
            source_id: source_id.clone(),
            revision: 1,
            connection_option_id: "codex.subscription.v1".to_owned(),
            connector_id: "connector.cpa.codex".to_owned(),
            connector_revision: 1,
            origin: SourceOrigin::Cpa,
            identity,
            identity_digest,
            billing_class: BillingClass::Subscription,
            state: MaterializationState::Ready,
        },
        credential_ref: CredentialRefV1::new(
            "credential/cpa/codex",
            format!("source/{source_id}"),
            "connector/connector.cpa.codex",
            "provider-auth",
            ["connection-option/codex.subscription.v1".to_owned()],
            3,
        )
        .unwrap(),
        inventory: vec![
            EffectiveInventoryModelV1 {
                upstream_model_id: "known-model".to_owned(),
                disposition: InventoryDisposition::CatalogMatched,
                model_configuration_id: Some("model.known".to_owned()),
                metadata: BTreeMap::new(),
            },
            EffectiveInventoryModelV1 {
                upstream_model_id: "new-model".to_owned(),
                disposition: InventoryDisposition::InventoryOnly,
                model_configuration_id: None,
                metadata: BTreeMap::new(),
            },
        ],
    }
}

fn context(evidence: BorrowedSubscriptionEvidence) -> CpaSubscriptionEffectContext {
    CpaSubscriptionEffectContext::new(
        operation("operation-a"),
        candidate(),
        descriptor("source/codex"),
        evidence,
        None,
        "connector.cpa.codex",
        ComputeSubscriptionResourceReceiptV2::new("effect/operation-a/cpa", 1).unwrap(),
    )
    .unwrap()
}

fn approved(context: &CpaSubscriptionEffectContext) -> ComputeApprovedSubscriptionCheckV2 {
    ComputeApprovedSubscriptionCheckV2 {
        approval_operation: context.approval_operation.clone(),
        candidate: context.candidate.clone(),
        expected_evidence_digest: context.evidence.evidence_digest().clone(),
        existing_source: context.existing_source.clone(),
        protected_source: context.protected_source.clone(),
    }
}

#[test]
fn forged_protected_selector_is_rejected_before_runtime_work() {
    let (_temp, _path, evidence) = source_evidence("account-a");
    let context = context(evidence.clone());
    let runtime = Arc::new(FakeRuntime {
        evidence: parking_lot::Mutex::new(evidence.clone()),
        materialize_calls: AtomicUsize::new(0),
        sources: vec![registered(&evidence)],
    });
    let materializer = CpaSubscriptionMaterializer::with_runtime(runtime.clone(), context.clone());
    let mut input = approved(&context);
    input.protected_source = descriptor("source/forged");

    let error = materializer.materialize_selected(input).err().unwrap();

    assert_eq!(error.code, PortErrorCode::PermissionDenied);
    assert_eq!(runtime.materialize_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn changed_account_is_rejected_before_access_only_materialization() {
    let (_first_temp, _first_path, first) = source_evidence("account-a");
    let (_second_temp, _second_path, second) = source_evidence("account-b");
    let context = context(first.clone());
    let runtime = Arc::new(FakeRuntime {
        evidence: parking_lot::Mutex::new(second),
        materialize_calls: AtomicUsize::new(0),
        sources: vec![registered(&first)],
    });
    let materializer = CpaSubscriptionMaterializer::with_runtime(runtime.clone(), context.clone());

    let error = materializer
        .materialize_selected(approved(&context))
        .err()
        .unwrap();

    assert_eq!(error.code, PortErrorCode::Conflict);
    assert_eq!(runtime.materialize_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn verified_result_preserves_known_and_inventory_only_models() {
    let (_temp, _path, evidence) = source_evidence("account-a");
    let context = context(evidence.clone());
    let runtime = Arc::new(FakeRuntime {
        evidence: parking_lot::Mutex::new(evidence.clone()),
        materialize_calls: AtomicUsize::new(0),
        sources: vec![registered(&evidence)],
    });
    let materializer = CpaSubscriptionMaterializer::with_runtime(runtime.clone(), context.clone());

    let result = materializer
        .materialize_selected(approved(&context))
        .unwrap();

    assert_eq!(runtime.materialize_calls.load(Ordering::SeqCst), 1);
    assert_eq!(result.account_ref, evidence.account_ref());
    assert_eq!(
        result.verified_evidence_digest,
        evidence.binding_evidence_digest().clone()
    );
    assert_ne!(
        result.verified_evidence_digest,
        evidence.evidence_digest().clone()
    );
    assert_eq!(result.inventory.len(), 2);
    assert!(result.inventory[0].selectable);
    assert_eq!(result.inventory[0].display_name, "Known model");
    assert_eq!(result.inventory[0].capabilities.tool.value, Some(true));
    assert_eq!(
        result.inventory[0].capabilities.native_reasoning.value,
        Some(NativeReasoningCapabilityV1::Discrete {
            parameter: "reasoning_effort".to_owned(),
            profiles: vec!["low".to_owned(), "high".to_owned()],
            default_profile: None,
        })
    );
    assert!(result.inventory[1].selectable);
    assert_eq!(result.inventory[1].reason, None);
    assert_eq!(
        result.inventory[1].capabilities.tool.basis,
        ComputeCandidateFactBasisV2::RuntimeFallback
    );
    assert!(matches!(
        result.resource_owner,
        ComputeSubscriptionResourceOwnerV2::ApprovalOperation { .. }
    ));
}

#[test]
fn all_inventory_only_models_use_the_marked_conservative_fallback() {
    let (_temp, _path, evidence) = source_evidence("account-a");
    let context = context(evidence.clone());
    let mut source = registered(&evidence);
    source.inventory = vec![EffectiveInventoryModelV1 {
        upstream_model_id: "brand-new-model".to_owned(),
        disposition: InventoryDisposition::InventoryOnly,
        model_configuration_id: None,
        metadata: BTreeMap::new(),
    }];
    let runtime = Arc::new(FakeRuntime {
        evidence: parking_lot::Mutex::new(evidence),
        materialize_calls: AtomicUsize::new(0),
        sources: vec![source],
    });
    let materializer = CpaSubscriptionMaterializer::with_runtime(runtime, context.clone());

    let result = materializer
        .materialize_selected(approved(&context))
        .unwrap();

    assert_eq!(result.inventory.len(), 1);
    assert!(result.inventory[0].selectable);
    assert_eq!(result.inventory[0].reason, None);
    assert_eq!(result.inventory[0].capabilities.tool.value, Some(true));
    assert_eq!(
        result.inventory[0].capabilities.native_reasoning.value,
        Some(NativeReasoningCapabilityV1::Fixed {
            profile: "non-thinking".into()
        })
    );
}

#[test]
fn known_non_text_inventory_model_remains_unselectable() {
    let (_temp, _path, evidence) = source_evidence("account-a");
    let context = context(evidence.clone());
    let mut source = registered(&evidence);
    source.inventory = vec![EffectiveInventoryModelV1 {
        upstream_model_id: "gpt-image-2".to_owned(),
        disposition: InventoryDisposition::InventoryOnly,
        model_configuration_id: None,
        metadata: BTreeMap::new(),
    }];
    let runtime = Arc::new(FakeRuntime {
        evidence: parking_lot::Mutex::new(evidence),
        materialize_calls: AtomicUsize::new(0),
        sources: vec![source],
    });
    let materializer = CpaSubscriptionMaterializer::with_runtime(runtime, context.clone());

    let result = materializer
        .materialize_selected(approved(&context))
        .unwrap();

    assert_eq!(result.inventory.len(), 1);
    assert!(!result.inventory[0].selectable);
    assert_eq!(
        result.inventory[0].reason.as_deref(),
        Some("inventory_only")
    );
    assert_eq!(
        result.inventory[0].capabilities.tool.basis,
        ComputeCandidateFactBasisV2::Unknown
    );
}

#[test]
fn saved_source_recheck_keeps_existing_resource_ownership() {
    let (_temp, _path, evidence) = source_evidence("account-a");
    let mut context = context(evidence.clone());
    context.existing_source = Some(ComputeSavedSourceExpectationV2 {
        source_id: "cpa/saved".to_owned(),
        expected_revision: 7,
    });
    let runtime = Arc::new(FakeRuntime {
        evidence: parking_lot::Mutex::new(evidence.clone()),
        materialize_calls: AtomicUsize::new(0),
        sources: vec![registered(&evidence)],
    });
    let materializer = CpaSubscriptionMaterializer::with_runtime(runtime, context.clone());

    let result = materializer
        .materialize_selected(approved(&context))
        .unwrap();

    assert!(matches!(
        result.resource_owner,
        ComputeSubscriptionResourceOwnerV2::SavedSource(_)
    ));
    assert_eq!(
        decide_subscription_release(
            &result.resource_owner,
            &CpaSubscriptionSaveHandoff::NotSubmitted,
        ),
        CpaSubscriptionReleaseDecision::PreserveExistingSource
    );
}

#[test]
fn pending_or_saved_operation_b_is_never_released_as_operation_a() {
    let owner = ComputeSubscriptionResourceOwnerV2::ApprovalOperation {
        operation: operation("operation-a"),
    };
    let operation_b = operation("operation-b");

    assert_eq!(
        decide_subscription_release(
            &owner,
            &CpaSubscriptionSaveHandoff::Pending(operation_b.clone()),
        ),
        CpaSubscriptionReleaseDecision::ObserveSave(operation_b.clone())
    );
    assert_eq!(
        decide_subscription_release(
            &owner,
            &CpaSubscriptionSaveHandoff::Saved(operation_b.clone()),
        ),
        CpaSubscriptionReleaseDecision::RetainSaved(operation_b)
    );
}

#[test]
fn artifact_auth_and_runtime_failures_have_distinct_subscription_states() {
    assert_eq!(
        cpa_subscription_availability(Err(&CpaLifecycleError::UnsupportedArtifactVersion)),
        CpaSubscriptionAvailability::ArtifactUnavailable
    );
    assert_eq!(
        cpa_subscription_availability(Err(&CpaLifecycleError::BorrowedCodexAuthMissing)),
        CpaSubscriptionAvailability::NeedsAuthentication
    );
    assert_eq!(
        cpa_subscription_availability(Err(&CpaLifecycleError::BorrowedCodexAuthUnavailable)),
        CpaSubscriptionAvailability::NeedsAuthentication
    );
    for error in [
        CpaLifecycleError::BorrowedClaudeAuthMissing,
        CpaLifecycleError::BorrowedClaudeAuthUnavailable,
        CpaLifecycleError::InvalidBorrowedClaudeAuth,
        CpaLifecycleError::BorrowedClaudeAuthSourceChanged,
        CpaLifecycleError::BorrowedCodexAuthSourceChanged,
    ] {
        assert_eq!(
            cpa_subscription_availability(Err(&error)),
            CpaSubscriptionAvailability::NeedsAuthentication
        );
    }
    assert_eq!(
        cpa_subscription_availability(Err(&CpaLifecycleError::ControlUnavailable)),
        CpaSubscriptionAvailability::RuntimeUnavailable
    );
}

#[test]
fn a_provider_cannot_materialize_another_providers_native_evidence() {
    let (_temp, _path, evidence) = source_evidence("account-a");
    assert!(
        CpaSubscriptionEffectContext::new(
            operation("operation-a"),
            candidate(),
            descriptor("source/codex"),
            evidence,
            None,
            "connector.cpa.claude",
            ComputeSubscriptionResourceReceiptV2::new("effect/operation-a/cpa", 1).unwrap(),
        )
        .is_err()
    );
}
