use std::sync::atomic::{AtomicBool, Ordering};

use crate::test_tempdir as tempdir;
use hiroute_application::compute_management::{
    ComputeCandidateCapabilityFactsV2, ComputeCandidateFactBasisV2, ComputeCandidateFactValueV2,
    ComputeCandidateFactsV2, ComputeCandidateModelFactsV2, ComputeCandidatePort,
    ComputeCandidateProvenanceV2, ComputeCredentialBindingV2, ComputeDiscoveryEvidenceGuardV1,
    ComputeManagementCompilationErrorV2, ComputeManagementPlanner,
    ComputeManagementPlanningErrorV2, ComputeManagementPreparedPreviewV2,
    ProtectedInputSourceDescriptorV1, TrustedComputeCandidateRegistry,
    compile_compute_management_source, query_compute_management,
};
use hiroute_application::{
    ConnectionOptionAuthorizationPort, ProtectedInputPort, TransactionCoordinator,
    TransactionError, TransactionRuntime, VerifiedPrincipal,
};
use hiroute_application_api::{
    ComputeCandidateProducerV2, ComputeCandidateRefV2, ComputeCandidateTargetV2,
    ComputeCheckCorrelationV2, ComputeConnectionApplyRequestV1, ComputeKeyEditV2,
    ComputeManagementChangeV2, ComputeManagementIntentV2, ComputeManagementQueryV2,
    ComputeManagementSubjectV2, ComputeModelMembershipV2,
};
use hiroute_domain::{
    CanonicalDigest, CompensationOutcome, ControlRepositoryPort, CredentialPoolIdentityV1,
    CredentialPoolV1, EffectReconciliation, ExternalEffectIntentV1, ExternalEffectPort,
    GatewayAuthenticationSemanticsV1, MaterializationState, NativeReasoningCapabilityV1,
    OperationState, OperationV1, OwnedEffectV1, PortError, PortErrorCode, PortResult,
    ProtectedSecret, SecretStorePort, UpstreamProtocol, WorkspaceId,
};

use super::super::super::ControlStore;
use crate::{LocalSecretStore, LocalStorageSet, RuntimeStore};

struct ProtectedInput;

impl ProtectedInputPort for ProtectedInput {
    fn read_secret(&self, input_slot: &str) -> PortResult<ProtectedSecret> {
        let value: &[u8] = match input_slot {
            "slot/primary" => b"management-secret-canary",
            "slot/secondary" => b"secondary-secret-canary",
            "slot/replacement" => b"replacement-secret-canary",
            _ => return Err(PortError::new(PortErrorCode::NotFound, "test.input")),
        };
        ProtectedSecret::new(value.to_vec())
            .map_err(|_| PortError::new(PortErrorCode::InvalidData, "test.input"))
    }

    fn validate_discovery_evidence(
        &self,
        _input_slot: &str,
        _expected_evidence: &CanonicalDigest,
    ) -> PortResult<()> {
        Err(PortError::new(
            PortErrorCode::Unavailable,
            "test.discovery_evidence",
        ))
    }
}

struct DiscoveryProtectedInput {
    evidence_valid: AtomicBool,
    evidence: CanonicalDigest,
}

impl DiscoveryProtectedInput {
    fn new(evidence: CanonicalDigest) -> Self {
        Self {
            evidence_valid: AtomicBool::new(true),
            evidence,
        }
    }
}

impl ProtectedInputPort for DiscoveryProtectedInput {
    fn read_secret(&self, input_slot: &str) -> PortResult<ProtectedSecret> {
        if input_slot != "slot/discovered" {
            return Err(PortError::new(PortErrorCode::NotFound, "test.input"));
        }
        ProtectedSecret::new(b"discovered-secret-canary".to_vec())
            .map_err(|_| PortError::new(PortErrorCode::InvalidData, "test.input"))
    }

    fn validate_discovery_evidence(
        &self,
        input_slot: &str,
        expected_evidence: &CanonicalDigest,
    ) -> PortResult<()> {
        if input_slot == "slot/discovered"
            && expected_evidence == &self.evidence
            && self.evidence_valid.load(Ordering::Acquire)
        {
            Ok(())
        } else {
            Err(PortError::new(
                PortErrorCode::Conflict,
                "test.discovery_evidence",
            ))
        }
    }
}

struct NoExternal;

impl ExternalEffectPort for NoExternal {
    fn current_external_fingerprint(&self, _target: &str) -> PortResult<Option<CanonicalDigest>> {
        Err(unexpected("test.external.current"))
    }

    fn apply_external(
        &self,
        _operation: &OperationV1,
        _intent: &ExternalEffectIntentV1,
    ) -> PortResult<OwnedEffectV1> {
        Err(unexpected("test.external.apply"))
    }

    fn observe_external(
        &self,
        _operation: &OperationV1,
        _intent: &ExternalEffectIntentV1,
    ) -> PortResult<EffectReconciliation> {
        Err(unexpected("test.external.observe"))
    }

    fn activate_external(
        &self,
        _operation: &OperationV1,
        _effect: &OwnedEffectV1,
    ) -> PortResult<OwnedEffectV1> {
        Err(unexpected("test.external.activate"))
    }

    fn compensate_external(
        &self,
        _operation: &OperationV1,
        _effect: &OwnedEffectV1,
    ) -> PortResult<CompensationOutcome> {
        Err(unexpected("test.external.compensate"))
    }
}

fn unexpected(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::Unavailable, context)
}

impl ConnectionOptionAuthorizationPort for ControlStore {
    fn connection_option_authorization(
        &self,
        _connection_option_id: &str,
    ) -> PortResult<Option<hiroute_application::ConnectionOptionAuthorizationV1>> {
        Ok(None)
    }

    fn source_uses_connection_option(
        &self,
        _source_id: &str,
        _connection_option_id: &str,
    ) -> PortResult<bool> {
        Ok(false)
    }

    fn is_registered_compute_source(&self, _source_id: &str) -> PortResult<bool> {
        Ok(false)
    }

    fn is_registered_price_target(
        &self,
        _offer_ref: &str,
        _model_configuration_id: &str,
        _currency: &str,
        _target_rule_id: Option<&str>,
    ) -> PortResult<bool> {
        Ok(false)
    }

    fn credential_pool_identity(
        &self,
        _pool_id: &str,
        _binding_id: &str,
    ) -> PortResult<Option<CredentialPoolIdentityV1>> {
        Ok(None)
    }

    fn credential_pool(&self, _pool_id: &str) -> PortResult<Option<CredentialPoolV1>> {
        Ok(None)
    }

    fn credential_reference_count(&self, _credential_id: &str) -> PortResult<u64> {
        Ok(0)
    }

    fn compute_source_materialization(
        &self,
        _connection_option_id: &str,
        _source_id: &str,
        _expected_revision: u64,
        _explicit_materialization: bool,
    ) -> PortResult<Option<hiroute_application::RegisteredComputeSourceMaterializationV1>> {
        Ok(None)
    }
}

fn fact<T>(value: T) -> ComputeCandidateFactValueV2<T> {
    ComputeCandidateFactValueV2 {
        value: Some(value),
        basis: ComputeCandidateFactBasisV2::UserDeclared,
    }
}

fn candidate(candidate_ref: &str, input_slot: &str) -> ComputeCandidateFactsV2 {
    let candidate = ComputeCandidateRefV2 {
        candidate_ref: candidate_ref.into(),
        candidate_revision: 1,
    };
    ComputeCandidateFactsV2 {
        correlation: ComputeCheckCorrelationV2 {
            candidate_ref: candidate.candidate_ref.clone(),
            edit_revision: 1,
            check_id: format!("check/{candidate_ref}"),
            input_digest: CanonicalDigest::of_bytes(b"input-facts"),
        },
        candidate,
        producer: ComputeCandidateProducerV2::Native,
        lineage_ref: "lineage/manual-primary".into(),
        trusted_lineage_digest: None,
        display_name: "Manual endpoint".into(),
        existing_source_id: None,
        evidence_digest: CanonicalDigest::of_bytes(b"candidate-evidence"),
        provenance: ComputeCandidateProvenanceV2::UserConfigured {
            configuration_revision: 1,
            evidence_digest: CanonicalDigest::of_bytes(b"configuration-evidence"),
        },
        target: Some(ComputeCandidateTargetV2 {
            scheme: "https".into(),
            authority: "api.example.test".into(),
            port: 443,
            request_path: "/v1/responses".into(),
            upstream_protocol: UpstreamProtocol::Responses,
            protocol_profile_id: "profile/responses".into(),
            protocol_profile_revision: 1,
        }),
        authentication: Some(GatewayAuthenticationSemanticsV1::ApiKeyHeader {
            header: "x-api-key".into(),
        }),
        additional_native_endpoints: Vec::new(),
        models: ["one", "two"]
            .into_iter()
            .map(|suffix| ComputeCandidateModelFactsV2 {
                model_ref: format!("model/{suffix}"),
                upstream_model_id: format!("upstream-{suffix}"),
                display_name: format!("Model {suffix}"),
                catalog_configuration_id: None,
                membership: ComputeModelMembershipV2::UserDeclared,
                capabilities: ComputeCandidateCapabilityFactsV2 {
                    tool: fact(true),
                    vision: fact(false),
                    streaming: fact(true),
                    context_tokens: fact(32_000),
                    max_output_tokens: fact(4_096),
                    native_reasoning: fact(NativeReasoningCapabilityV1::Discrete {
                        parameter: "reasoning_effort".into(),
                        profiles: vec!["low".into(), "high".into()],
                    }),
                },
                capability_evidence_digest: CanonicalDigest::of_bytes(
                    format!("capability-{suffix}").as_bytes(),
                ),
                selectable: true,
                reason: None,
            })
            .collect(),
        native_recheck: Some(hiroute_domain::ComputeNativeRecheckDescriptorV2 {
            display_template_id: None,
            inventory_path: Some("/v1/models".into()),
            protocol_header_semantics: hiroute_domain::GatewayHeaderSemanticsV1 {
                content_type: "application/json".into(),
                required_headers: vec![("anthropic-version".into(), "2023-06-01".into())],
                forbidden_forward_headers: vec!["authorization".into(), "x-api-key".into()],
            },
        }),
        discovery_guard: None,
        credential_binding: ComputeCredentialBindingV2::NativeProtected {
            descriptor: ProtectedInputSourceDescriptorV1::ManualInput,
            input_slot: input_slot.into(),
        },
        validation: None,
    }
}

fn discovered_candidate(evidence: CanonicalDigest) -> ComputeCandidateFactsV2 {
    let mut facts = candidate("candidate/discovered", "slot/discovered");
    facts.native_recheck = None;
    facts.provenance = ComputeCandidateProvenanceV2::Registered {
        connection_option_id: "zhipu.coding-plan.cn.v1".into(),
        registry_version: "mvp-current".into(),
        catalog_digest: CanonicalDigest::of_bytes(b"catalog"),
    };
    facts.credential_binding = ComputeCredentialBindingV2::NativeProtected {
        descriptor: ProtectedInputSourceDescriptorV1::DiscoveredConfig {
            scanner_id: "builtin.agent-filesystem".into(),
            scanner_version: "1".into(),
            source_ref: "private-source-ref".into(),
            field_selector: "env.ANTHROPIC_AUTH_TOKEN".into(),
            observed_revision: 1,
        },
        input_slot: "slot/discovered".into(),
    };
    facts.discovery_guard = Some(ComputeDiscoveryEvidenceGuardV1 {
        evidence_digest: evidence.clone(),
    });
    facts.evidence_digest = evidence;
    facts
}

fn discovery_change(
    facts: &ComputeCandidateFactsV2,
    revisions: hiroute_domain::RevisionSetV1,
) -> ComputeManagementChangeV2 {
    ComputeManagementChangeV2 {
        schema: "hiroute.compute-management-change/v2".into(),
        subject: ComputeManagementSubjectV2::Candidate {
            candidate: facts.candidate.clone(),
        },
        expected_revisions: revisions,
        selected_model_refs: vec![facts.models[0].model_ref.clone()],
        intent: ComputeManagementIntentV2::SaveReady,
        key_edits: Vec::new(),
        validation: None,
    }
}

fn apply_preview(
    _stores: &LocalStorageSet,
    planner: &ComputeManagementPlanner<
        '_,
        TrustedComputeCandidateRegistry,
        ControlStore,
        LocalSecretStore,
        ProtectedInput,
    >,
    coordinator: &TransactionCoordinator<
        '_,
        ControlStore,
        LocalSecretStore,
        RuntimeStore,
        NoExternal,
        ProtectedInput,
    >,
    workspace: &WorkspaceId,
    preview: ComputeManagementPreparedPreviewV2,
    idempotency_key: &str,
) {
    let apply = ComputeConnectionApplyRequestV1 {
        spec: preview.result.spec.clone(),
        accept_digest: preview.result.accept_digest.clone(),
        expected_revisions: preview.result.expected_revisions.clone(),
        idempotency_key: idempotency_key.into(),
    };
    let prepared = planner.prepare_apply(apply).unwrap();
    let accepted = coordinator
        .accept_prepared(workspace, &VerifiedPrincipal::for_local_control(), prepared)
        .unwrap();
    let completed = coordinator.run(&accepted.operation().operation_id).unwrap();
    assert_eq!(completed.state, OperationState::Succeeded);
}

fn populate_saved_source(
    directory: &std::path::Path,
    workspace: &WorkspaceId,
) -> Option<hiroute_domain::ComputeNativeRecheckDescriptorV2> {
    let stores = LocalStorageSet::open_for_daemon_startup(directory).unwrap();
    let registry = TrustedComputeCandidateRegistry::new();
    let facts = candidate("candidate/manual-primary", "slot/primary");
    registry.register_compute_candidate(facts.clone()).unwrap();
    let input = ProtectedInput;
    let planner =
        ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
    let revisions = hiroute_domain::ComputeManagementRepositoryPort::compute_management_snapshot(
        stores.control(),
        workspace,
    )
    .unwrap()
    .revisions;
    let preview = planner
        .preview(ComputeManagementChangeV2 {
            schema: "hiroute.compute-management-change/v2".into(),
            subject: ComputeManagementSubjectV2::Candidate {
                candidate: facts.candidate.clone(),
            },
            expected_revisions: revisions.clone(),
            selected_model_refs: vec!["model/one".into(), "model/two".into()],
            intent: ComputeManagementIntentV2::SaveReady,
            key_edits: Vec::new(),
            validation: None,
        })
        .unwrap();
    assert_eq!(preview.result.changes.len(), 4);
    let transaction_runtime = TransactionRuntime::default();
    let external = NoExternal;
    let coordinator = TransactionCoordinator::new(
        stores.control(),
        stores.secrets(),
        stores.runtime(),
        &external,
        &input,
        &transaction_runtime,
    );
    coordinator.reconcile_startup_and_open().unwrap();
    apply_preview(
        &stores,
        &planner,
        &coordinator,
        workspace,
        preview,
        "mvp-11-first-save",
    );

    let snapshot = hiroute_domain::ComputeManagementRepositoryPort::compute_management_snapshot(
        stores.control(),
        workspace,
    )
    .unwrap();
    assert_eq!(snapshot.sources.len(), 1);
    let source = &snapshot.sources[0];
    assert_eq!(source.models.len(), 2);
    assert_eq!(source.credentials.len(), 1);
    assert_eq!(
        stores
            .secrets()
            .generation(&source.credentials[0].credential)
            .unwrap(),
        1
    );

    let second = candidate("candidate/manual-secondary", "slot/secondary");
    registry.register_compute_candidate(second.clone()).unwrap();
    let second_preview = planner
        .preview(ComputeManagementChangeV2 {
            schema: "hiroute.compute-management-change/v2".into(),
            subject: ComputeManagementSubjectV2::SavedSource {
                source_id: source.source_id.clone(),
            },
            expected_revisions: snapshot.revisions,
            selected_model_refs: vec!["model/one".into(), "model/two".into()],
            intent: ComputeManagementIntentV2::SaveReady,
            key_edits: vec![ComputeKeyEditV2::Add {
                input_candidate: second.candidate,
            }],
            validation: None,
        })
        .unwrap();
    apply_preview(
        &stores,
        &planner,
        &coordinator,
        workspace,
        second_preview,
        "mvp-11-add-second-key",
    );
    let second_snapshot =
        hiroute_domain::ComputeManagementRepositoryPort::compute_management_snapshot(
            stores.control(),
            workspace,
        )
        .unwrap();
    let second_source = &second_snapshot.sources[0];
    assert_eq!(second_source.credentials.len(), 2);
    let primary_id = second_source.credentials[0].key_id.clone();
    let secondary_id = second_source.credentials[1].key_id.clone();

    let replacement = candidate("candidate/manual-replacement", "slot/replacement");
    registry
        .register_compute_candidate(replacement.clone())
        .unwrap();
    let combination_preview = planner
        .preview(ComputeManagementChangeV2 {
            schema: "hiroute.compute-management-change/v2".into(),
            subject: ComputeManagementSubjectV2::SavedSource {
                source_id: second_source.source_id.clone(),
            },
            expected_revisions: second_snapshot.revisions,
            selected_model_refs: vec!["model/one".into(), "model/two".into()],
            intent: ComputeManagementIntentV2::SaveReady,
            key_edits: vec![
                ComputeKeyEditV2::Replace {
                    key_id: primary_id.clone(),
                    expected_generation: 1,
                    input_candidate: replacement.candidate,
                },
                ComputeKeyEditV2::SetEnabled {
                    key_id: primary_id.clone(),
                    expected_generation: 1,
                    enabled: false,
                },
                ComputeKeyEditV2::SetOrder {
                    key_ids: vec![secondary_id.clone(), primary_id.clone()],
                },
            ],
            validation: None,
        })
        .unwrap();
    apply_preview(
        &stores,
        &planner,
        &coordinator,
        workspace,
        combination_preview,
        "mvp-11-combined-key-edit",
    );

    let final_snapshot =
        hiroute_domain::ComputeManagementRepositoryPort::compute_management_snapshot(
            stores.control(),
            workspace,
        )
        .unwrap();
    let final_source = &final_snapshot.sources[0];
    assert_eq!(final_source.credentials[0].key_id, secondary_id);
    assert!(final_source.credentials[0].enabled);
    assert_eq!(final_source.credentials[1].key_id, primary_id);
    assert!(!final_source.credentials[1].enabled);
    assert_eq!(final_source.credentials[1].credential.generation(), 2);
    assert_eq!(
        stores
            .secrets()
            .generation(&final_source.credentials[1].credential)
            .unwrap(),
        2
    );
    let compiled = compile_compute_management_source(final_source).unwrap();
    assert_eq!(compiled.len(), 2);
    assert_eq!(compiled[0].native_reasoning, compiled[1].native_reasoning);
    let hiroute_application::compute_management::ComputeManagementCredentialCompilationV2::Native {
        ordered,
    } = &compiled[0].credential
    else {
        panic!("native source must compile a native credential selection");
    };
    assert_eq!(ordered.len(), 1);
    let hiroute_domain::ComputeCredentialSelectionV2::Credential { credential_ref } = &ordered[0]
    else {
        panic!("authenticated source must compile an explicit credential");
    };
    assert_eq!(credential_ref.credential_id(), secondary_id);

    let public = query_compute_management(
        stores.control(),
        stores.runtime(),
        workspace,
        &ComputeManagementQueryV2::default(),
    )
    .unwrap();
    assert_eq!(public.sources[0].ready_model_count, 2);
    assert_eq!(public.sources[0].keys[0].key_id, secondary_id);
    let encoded = serde_json::to_string(&public).unwrap();
    for protected in [
        "management-secret-canary",
        "secondary-secret-canary",
        "replacement-secret-canary",
        "slot/primary",
        "slot/secondary",
        "slot/replacement",
    ] {
        assert!(!encoded.contains(protected));
    }
    assert!(!encoded.contains("owner_scope"));
    assert!(!encoded.contains("allowed_destinations"));
    assert!(!encoded.contains("native_recheck"));

    final_source.native_recheck.clone()
}

#[test]
fn editing_source_adds_endpoint_without_reentering_or_copying_the_saved_key() {
    let directory = tempdir().unwrap();
    let workspace = WorkspaceId::default();
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let registry = TrustedComputeCandidateRegistry::new();
    let first = candidate("candidate/endpoint-first", "slot/primary");
    registry.register_compute_candidate(first.clone()).unwrap();
    let input = ProtectedInput;
    let planner =
        ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
    let runtime = TransactionRuntime::default();
    let external = NoExternal;
    let coordinator = TransactionCoordinator::new(
        stores.control(),
        stores.secrets(),
        stores.runtime(),
        &external,
        &input,
        &runtime,
    );
    coordinator.reconcile_startup_and_open().unwrap();
    let initial = hiroute_domain::ComputeManagementRepositoryPort::compute_management_snapshot(
        stores.control(),
        &workspace,
    )
    .unwrap();
    let preview = planner
        .preview(ComputeManagementChangeV2 {
            schema: "hiroute.compute-management-change/v2".into(),
            subject: ComputeManagementSubjectV2::Candidate {
                candidate: first.candidate,
            },
            expected_revisions: initial.revisions,
            selected_model_refs: first
                .models
                .iter()
                .map(|model| model.model_ref.clone())
                .collect(),
            intent: ComputeManagementIntentV2::SaveReady,
            key_edits: Vec::new(),
            validation: None,
        })
        .unwrap();
    apply_preview(
        &stores,
        &planner,
        &coordinator,
        &workspace,
        preview,
        "endpoint-first-save",
    );

    let saved = hiroute_domain::ComputeManagementRepositoryPort::compute_management_snapshot(
        stores.control(),
        &workspace,
    )
    .unwrap();
    let source = &saved.sources[0];
    let original_binding_ids = source
        .models
        .iter()
        .map(|model| model.binding_id.clone())
        .collect::<Vec<_>>();
    let original_key_id = source.credentials[0].key_id.clone();
    let mut edited = candidate("candidate/endpoint-edit", "slot/unused");
    edited.existing_source_id = Some(source.source_id.clone());
    edited.trusted_lineage_digest = Some(source.lineage_digest.clone());
    edited.credential_binding = ComputeCredentialBindingV2::NativeSaved {
        credential_id: original_key_id.clone(),
        expected_generation: 1,
    };
    let mut messages = source.target.clone();
    messages.authority = "messages.example.test".into();
    messages.request_path = "/apps/anthropic/v1/messages".into();
    messages.upstream_protocol = UpstreamProtocol::Messages;
    messages.protocol_profile_id = "profile/messages".into();
    edited
        .additional_native_endpoints
        .push(hiroute_domain::ComputeNativeEndpointV3 {
            target: messages.clone(),
            authentication: GatewayAuthenticationSemanticsV1::ApiKeyHeader {
                header: "x-api-key".into(),
            },
            recheck: None,
        });
    registry.register_compute_candidate(edited.clone()).unwrap();
    let preview = planner
        .preview(ComputeManagementChangeV2 {
            schema: "hiroute.compute-management-change/v2".into(),
            subject: ComputeManagementSubjectV2::Candidate {
                candidate: edited.candidate,
            },
            expected_revisions: saved.revisions,
            selected_model_refs: edited
                .models
                .iter()
                .map(|model| model.model_ref.clone())
                .collect(),
            intent: ComputeManagementIntentV2::SaveReady,
            key_edits: Vec::new(),
            validation: None,
        })
        .unwrap();
    apply_preview(
        &stores,
        &planner,
        &coordinator,
        &workspace,
        preview,
        "endpoint-edit-save",
    );

    let after = hiroute_domain::ComputeManagementRepositoryPort::compute_management_snapshot(
        stores.control(),
        &workspace,
    )
    .unwrap();
    let updated = &after.sources[0];
    assert_eq!(updated.source_id, source.source_id);
    assert_eq!(
        updated
            .models
            .iter()
            .map(|model| model.binding_id.clone())
            .collect::<Vec<_>>(),
        original_binding_ids
    );
    assert_eq!(updated.credentials.len(), 1);
    assert_eq!(updated.credentials[0].key_id, original_key_id);
    assert_eq!(updated.credentials[0].credential.generation(), 2);
    assert!(
        updated.credentials[0]
            .credential
            .allowed_destinations()
            .contains(&messages.credential_destination().unwrap())
    );
    assert_eq!(
        stores
            .secrets()
            .generation(&updated.credentials[0].credential)
            .unwrap(),
        2
    );
}

#[test]
fn application_save_reaches_local_storage_secret_query_and_compilation() {
    let directory = tempdir().unwrap();
    let workspace = WorkspaceId::default();
    let expected_recheck = populate_saved_source(directory.path(), &workspace);
    let reopened = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let reopened_snapshot =
        hiroute_domain::ComputeManagementRepositoryPort::compute_management_snapshot(
            reopened.control(),
            &workspace,
        )
        .unwrap();
    assert_eq!(
        reopened_snapshot.sources[0].native_recheck,
        expected_recheck
    );
}

#[test]
fn discovered_candidate_config_change_between_prepare_and_preview_is_stale() {
    let directory = tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let registry = TrustedComputeCandidateRegistry::new();
    let evidence = CanonicalDigest::of_bytes(b"discovery/config/catalog-v1");
    let facts = discovered_candidate(evidence.clone());
    registry.register_compute_candidate(facts.clone()).unwrap();
    let input = DiscoveryProtectedInput::new(evidence);
    let planner =
        ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
    let revisions = hiroute_domain::ComputeManagementRepositoryPort::compute_management_snapshot(
        stores.control(),
        &WorkspaceId::default(),
    )
    .unwrap()
    .revisions;

    // The candidate represents a completed Prepare. A configuration-only change leaves the
    // credential bytes readable but invalidates the full evidence before Preview can seal a plan.
    input.evidence_valid.store(false, Ordering::Release);
    let result = planner.preview(discovery_change(&facts, revisions));
    assert!(matches!(
        result,
        Err(ComputeManagementPlanningErrorV2::PreviewStale)
    ));
    assert!(
        hiroute_domain::ComputeManagementRepositoryPort::compute_management_snapshot(
            stores.control(),
            &WorkspaceId::default(),
        )
        .unwrap()
        .sources
        .is_empty()
    );
}

#[test]
fn discovered_candidate_config_change_between_preview_and_apply_is_stale_before_admission() {
    let directory = tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let registry = TrustedComputeCandidateRegistry::new();
    let evidence = CanonicalDigest::of_bytes(b"discovery/config/catalog-v1");
    let facts = discovered_candidate(evidence.clone());
    registry.register_compute_candidate(facts.clone()).unwrap();
    let input = DiscoveryProtectedInput::new(evidence);
    let planner =
        ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
    let workspace = WorkspaceId::default();
    let revisions = hiroute_domain::ComputeManagementRepositoryPort::compute_management_snapshot(
        stores.control(),
        &workspace,
    )
    .unwrap()
    .revisions;
    let preview = planner
        .preview(discovery_change(&facts, revisions))
        .unwrap();
    let prepared = planner
        .prepare_apply(ComputeConnectionApplyRequestV1 {
            spec: preview.result.spec.clone(),
            accept_digest: preview.result.accept_digest.clone(),
            expected_revisions: preview.result.expected_revisions.clone(),
            idempotency_key: "discovery-preview-apply-stale".into(),
        })
        .unwrap();
    let transaction_runtime = TransactionRuntime::default();
    let external = NoExternal;
    let coordinator = TransactionCoordinator::new(
        stores.control(),
        stores.secrets(),
        stores.runtime(),
        &external,
        &input,
        &transaction_runtime,
    );
    coordinator.reconcile_startup_and_open().unwrap();

    // prepare_apply reproduced the accepted Preview while evidence was current. Admission must
    // revalidate again inside the writer boundary, after idempotency lookup and before begin.
    input.evidence_valid.store(false, Ordering::Release);
    let accepted = coordinator.accept_prepared(
        &workspace,
        &VerifiedPrincipal::for_local_control(),
        prepared,
    );
    assert!(matches!(
        accepted,
        Err(TransactionError::ChangePreviewStale)
    ));
    assert!(
        stores
            .control()
            .recoverable_operations()
            .unwrap()
            .is_empty()
    );
    assert!(
        hiroute_domain::ComputeManagementRepositoryPort::compute_management_snapshot(
            stores.control(),
            &workspace,
        )
        .unwrap()
        .sources
        .is_empty()
    );
}

mod disabled;
