use super::*;
use hiroute_domain::{
    ComputeManagementRepositoryPort, ComputeManagementSourceV2, ConnectorRuntimeKind,
    GatewayOperationalTargetV1, NATIVE_CREDENTIAL_LEASE_REQUEST_SCHEMA_V1,
    NativeCredentialLeaseRequestV1,
};

fn save(
    stores: &LocalStorageSet,
    registry: &TrustedComputeCandidateRegistry,
    subject: ComputeManagementSubjectV2,
    intent: ComputeManagementIntentV2,
    key_edits: Vec<ComputeKeyEditV2>,
    key: &str,
) -> ComputeManagementSourceV2 {
    let workspace = WorkspaceId::default();
    let snapshot = stores
        .control()
        .compute_management_snapshot(&workspace)
        .unwrap();
    let input = ProtectedInput;
    let planner =
        ComputeManagementPlanner::new(registry, stores.control(), stores.secrets(), &input);
    let preview = planner
        .preview(ComputeManagementChangeV2 {
            edit: None,
            schema: "hiroute.compute-management-change/v2".into(),
            subject,
            expected_revisions: snapshot.revisions,
            selected_model_refs: vec!["model/one".into(), "model/two".into()],
            intent,
            key_edits,
            validation: None,
        })
        .unwrap();
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
    apply_preview(stores, &planner, &coordinator, &workspace, preview, key);
    stores
        .control()
        .compute_management_snapshot(&workspace)
        .unwrap()
        .sources
        .remove(0)
}

fn frozen_request(
    source: &ComputeManagementSourceV2,
    model: usize,
    credential: usize,
) -> NativeCredentialLeaseRequestV1 {
    let target = GatewayOperationalTargetV1::UserConfiguredNative {
        uri: "https://api.example.test/v1/responses".into(),
    };
    NativeCredentialLeaseRequestV1 {
        schema_version: NATIVE_CREDENTIAL_LEASE_REQUEST_SCHEMA_V1.into(),
        stable_binding_id: source.models[model].binding_id.clone(),
        credential_id: source.credentials[credential].key_id.clone(),
        credential_destination_ref: source.target.credential_destination().unwrap(),
        excluded_key_ids: Vec::new(),
        connector_runtime: ConnectorRuntimeKind::BuiltinNative,
        connector_id: "builtin-openai".into(),
        upstream_protocol: UpstreamProtocol::Responses,
        upstream_model_id: source.models[model].upstream_model_id.clone(),
        native_transport_model: source.models[model].upstream_model_id.clone(),
        logical_endpoint: target.uri().into(),
        operational_target_digest: CanonicalDigest::of(&target).unwrap(),
        operational_target: target,
        request_path: "/v1/responses".into(),
        runtime_epoch: None,
        target_epoch: None,
        protocol_profile_digest: CanonicalDigest::of_bytes(b"frozen-managed-profile"),
        authentication: source.authentication.clone(),
    }
}

#[test]
fn frozen_native_requests_obey_current_source_and_key_switches_after_restart() {
    let root = tempdir().unwrap();
    let registry = TrustedComputeCandidateRegistry::new();
    let primary = candidate("candidate/lease-primary", "slot/primary");
    let secondary = candidate("candidate/lease-secondary", "slot/secondary");
    registry
        .register_compute_candidate(primary.clone())
        .unwrap();
    registry
        .register_compute_candidate(secondary.clone())
        .unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let source = save(
        &stores,
        &registry,
        ComputeManagementSubjectV2::Candidate {
            candidate: primary.candidate,
        },
        ComputeManagementIntentV2::SaveReady,
        Vec::new(),
        "lease-create",
    );
    let subject = ComputeManagementSubjectV2::SavedSource {
        source_id: source.source_id.clone(),
    };
    let source = save(
        &stores,
        &registry,
        subject.clone(),
        ComputeManagementIntentV2::SaveReady,
        vec![ComputeKeyEditV2::Add {
            input_candidate: secondary.candidate,
        }],
        "lease-add-key",
    );
    let requests = [
        frozen_request(&source, 0, 0),
        frozen_request(&source, 1, 0),
        frozen_request(&source, 0, 1),
    ];
    for request in &requests {
        assert!(
            stores
                .lease_native_credential_exact(request)
                .unwrap()
                .is_some()
        );
    }
    let edit = |index: usize, enabled| ComputeKeyEditV2::SetEnabled {
        key_id: source.credentials[index].key_id.clone(),
        expected_generation: 1,
        enabled,
    };
    save(
        &stores,
        &registry,
        subject.clone(),
        ComputeManagementIntentV2::SaveReady,
        vec![edit(0, false)],
        "lease-disable-first-key",
    );
    for request in &requests[..2] {
        assert!(
            stores
                .lease_native_credential_exact(request)
                .unwrap()
                .is_none(),
            "the same disabled key must be unavailable to both frozen model bindings"
        );
        assert!(
            stores
                .secrets()
                .lease_native_credential_exact(request)
                .unwrap()
                .is_some(),
            "the old Secret-only production path still leases it, proving this regression"
        );
    }
    assert!(
        stores
            .lease_native_credential_exact(&requests[2])
            .unwrap()
            .is_some()
    );
    save(
        &stores,
        &registry,
        subject.clone(),
        ComputeManagementIntentV2::SaveDisabled,
        Vec::new(),
        "lease-disable-source",
    );
    for request in &requests {
        assert!(
            stores
                .lease_native_credential_exact(request)
                .unwrap()
                .is_none()
        );
    }
    drop(stores);
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    for request in &requests {
        assert!(
            stores
                .lease_native_credential_exact(request)
                .unwrap()
                .is_none()
        );
    }
    save(
        &stores,
        &registry,
        subject.clone(),
        ComputeManagementIntentV2::SaveReady,
        Vec::new(),
        "lease-reenable-source",
    );
    assert!(
        stores
            .lease_native_credential_exact(&requests[0])
            .unwrap()
            .is_none()
    );
    assert!(
        stores
            .lease_native_credential_exact(&requests[2])
            .unwrap()
            .is_some()
    );
    save(
        &stores,
        &registry,
        subject.clone(),
        ComputeManagementIntentV2::SaveDisabled,
        vec![edit(1, false)],
        "lease-disable-last-key",
    );
    for request in &requests {
        assert!(
            stores
                .lease_native_credential_exact(request)
                .unwrap()
                .is_none()
        );
    }
    save(
        &stores,
        &registry,
        subject,
        ComputeManagementIntentV2::SaveReady,
        vec![edit(0, true), edit(1, true)],
        "lease-reenable-keys",
    );
    for request in &requests {
        let lease = stores
            .lease_native_credential_exact(request)
            .unwrap()
            .unwrap();
        assert_eq!(lease.generation(), 1);
        assert_eq!(lease.credential_id(), request.credential_id);
    }
}
