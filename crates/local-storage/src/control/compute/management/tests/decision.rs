//! Exercise journaled decisions and exact leases through the real three-store coordinator.
use super::lifecycle::saved;
use super::*;
use hiroute_application_api::{ApplyRequestV1, PreviewRequestV1};
use hiroute_domain::{
    CHANGE_SPEC_SCHEMA_V1, ChangeSpecV1, DecisionAuthenticationV2, DecisionConnectionV2,
    DecisionCredentialBindingV1, DecisionCredentialLeaseRequestV1,
    DecisionExpectedSourceRevisionV1, DecisionServiceChangeV2, DecisionServiceOperationInputV2,
    DecisionServiceSchemaV2, DecisionServiceV2, DecisionWireProtocolV2,
};

fn service(
    source: &hiroute_domain::ComputeManagementSourceV2,
    id: &str,
    model: &str,
) -> DecisionServiceV2 {
    DecisionServiceV2 {
        schema: DecisionServiceSchemaV2::V2,
        id: id.into(),
        revision: 1,
        name: id.into(),
        connection: DecisionConnectionV2::Native {
            provider: "fixture".into(),
            wire_protocol: DecisionWireProtocolV2::OpenaiDecisions,
            model: model.into(),
            endpoint: "https://decision.example.test/v1/decisions".into(),
            timeout_ms: 3000,
            authentication: DecisionAuthenticationV2::SharedSource {
                source_id: source.source_id.clone(),
                source_lineage_digest: source.lineage_digest.clone(),
            },
        },
    }
}

fn run(
    stores: &LocalStorageSet,
    input: DecisionServiceOperationInputV2,
    key: &str,
    fail_activation: bool,
) -> OperationV1 {
    let external = NoExternal;
    let protected = ProtectedInput;
    let runtime = TransactionRuntime::default();
    let coordinator = TransactionCoordinator::new(
        stores.control(),
        stores.secrets(),
        stores.runtime(),
        &external,
        &protected,
        &runtime,
    );
    coordinator.reconcile_startup_and_open().unwrap();
    let spec = ChangeSpecV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        command_id: "decision.connections.apply".into(),
        resource_id: Some(input.change.id.clone()),
        desired_state: serde_json::to_value(input).unwrap(),
    };
    let preview = coordinator
        .preview(&WorkspaceId::default(), PreviewRequestV1::new(spec))
        .unwrap();
    let accepted = coordinator
        .accept(
            &WorkspaceId::default(),
            &VerifiedPrincipal::for_local_control(),
            ApplyRequestV1 {
                schema_version: CHANGE_SPEC_SCHEMA_V1,
                spec: preview.normalized_spec,
                accept_digest: preview.change_digest,
                expected_revisions: preview.expected_revisions,
                idempotency_key: key.into(),
                apply_capability: None,
            },
        )
        .unwrap();
    if fail_activation {
        stores.control().with_connection(|connection| connection.execute_batch(
            "CREATE TEMP TRIGGER fail_decision_activation BEFORE UPDATE OF source_json ON main.compute_management_sources WHEN instr(NEW.source_json,'\"decision_targets\"') > 0 BEGIN SELECT RAISE(ABORT, 'fixture_decision_activation_failure'); END;").unwrap());
    }
    let result = coordinator.run_accepted(accepted).unwrap();
    if fail_activation {
        stores.control().with_connection(|connection| {
            connection
                .execute_batch("DROP TRIGGER fail_decision_activation;")
                .unwrap()
        });
    }
    result
}

fn add(
    source: &hiroute_domain::ComputeManagementSourceV2,
    service: DecisionServiceV2,
) -> DecisionServiceOperationInputV2 {
    DecisionServiceOperationInputV2 {
        change: DecisionServiceChangeV2 {
            id: service.id.clone(),
            expected_revision: 0,
            service: Some(service),
            expected_source_revisions: vec![DecisionExpectedSourceRevisionV1 {
                source_id: source.source_id.clone(),
                revision: source.revision,
            }],
        },
        input_slot: None,
    }
}

#[test]
fn decision_membership_issues_exact_leases_and_shares_only_the_destination() {
    let root = tempdir().unwrap();
    populate_saved_source(root.path(), &WorkspaceId::default());
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let before = saved(&stores);
    let first = service(&before, "decision/first", "decision-model-a");
    assert_eq!(
        run(
            &stores,
            add(&before, first.clone()),
            "decision-first",
            false
        )
        .state,
        OperationState::Succeeded
    );
    let current = saved(&stores);
    assert_eq!(
        current.credentials[0].credential.generation(),
        before.credentials[0].credential.generation() + 1
    );
    let binding = DecisionCredentialBindingV1::for_service(&first)
        .unwrap()
        .unwrap();
    let admission = stores.admit_decision_credentials(&binding).unwrap();
    assert!(!admission.keys.is_empty());
    let request = DecisionCredentialLeaseRequestV1 {
        binding: binding.clone(),
        admitted_keys: admission.keys.clone(),
        excluded_key_ids: vec![],
        key_id: admission.keys[0].key_id.clone(),
    };
    let lease = stores
        .lease_decision_credential_exact(&request)
        .unwrap()
        .unwrap();
    assert_eq!(lease.generation(), admission.keys[0].generation);
    let mut forged = request.clone();
    forged.binding.model = "different-model".into();
    assert!(stores.lease_decision_credential_exact(&forged).is_err());
    forged = request.clone();
    forged.binding.endpoint = "https://unregistered.example.test/v1/decisions".into();
    assert!(stores.lease_decision_credential_exact(&forged).is_err());

    let second = service(&current, "decision/second", "decision-model-b");
    assert_eq!(
        run(
            &stores,
            add(&current, second.clone()),
            "decision-second",
            false
        )
        .state,
        OperationState::Succeeded
    );
    let next = saved(&stores);
    assert_eq!(next.credentials, current.credentials);
    assert_eq!(next.decision_targets[0].consumers.len(), 2);
    assert!(
        stores
            .lease_decision_credential_exact(&request)
            .unwrap()
            .is_some()
    );
    let second_binding = DecisionCredentialBindingV1::for_service(&second)
        .unwrap()
        .unwrap();
    assert_ne!(
        binding.consumer_binding_id,
        second_binding.consumer_binding_id
    );
    assert_eq!(binding.destination_id, second_binding.destination_id);

    let delete = DecisionServiceOperationInputV2 {
        change: DecisionServiceChangeV2 {
            id: first.id.clone(),
            expected_revision: 1,
            service: None,
            expected_source_revisions: vec![DecisionExpectedSourceRevisionV1 {
                source_id: next.source_id.clone(),
                revision: next.revision,
            }],
        },
        input_slot: None,
    };
    assert_eq!(
        run(&stores, delete, "decision-delete-first", false).state,
        OperationState::Succeeded
    );
    let retained = saved(&stores);
    assert_eq!(retained.credentials, next.credentials);
    assert_eq!(retained.decision_targets[0].consumers.len(), 1);
    assert!(stores.lease_decision_credential_exact(&request).is_err());
    assert!(stores.admit_decision_credentials(&second_binding).is_ok());
}

#[test]
fn failed_decision_activation_restores_forward_secret_generations_and_old_routes_after_restart() {
    let root = tempdir().unwrap();
    populate_saved_source(root.path(), &WorkspaceId::default());
    let before;
    let restored;
    {
        let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
        before = saved(&stores);
        let decision = service(&before, "decision/rollback", "decision-model");
        let result = run(&stores, add(&before, decision), "decision-rollback", true);
        assert_eq!(result.state, OperationState::RolledBack);
        restored = saved(&stores);
        assert!(restored.decision_targets.is_empty());
        assert_eq!(restored.revision, before.revision + 2);
        for (old, new) in before.credentials.iter().zip(&restored.credentials) {
            assert_eq!(new.credential.generation(), old.credential.generation() + 2);
            assert_eq!(
                new.credential.allowed_destinations(),
                old.credential.allowed_destinations()
            );
        }
        assert!(
            stores
                .lease_native_credential_exact(&super::runtime_leases::frozen_request(
                    &restored, 0, 0
                ))
                .unwrap()
                .is_some()
        );
        assert!(
            stores
                .control()
                .decision_service_record(&WorkspaceId::default(), "decision/rollback", 1)
                .unwrap()
                .is_none()
        );
    }
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    assert_eq!(saved(&stores), restored);
    assert!(
        stores
            .lease_native_credential_exact(&super::runtime_leases::frozen_request(&restored, 0, 0))
            .unwrap()
            .is_some()
    );
}

#[test]
fn decision_activation_before_terminal_failure_completes_once_after_restart() {
    let root = tempdir().unwrap();
    populate_saved_source(root.path(), &WorkspaceId::default());
    let operation_id;
    let decision;
    let activated;
    {
        let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
        let before = saved(&stores);
        decision = service(&before, "decision/interrupted", "decision-model");
        let input = add(&before, decision.clone());
        let external = NoExternal;
        let protected = ProtectedInput;
        let runtime = TransactionRuntime::default();
        let coordinator = TransactionCoordinator::new(
            stores.control(),
            stores.secrets(),
            stores.runtime(),
            &external,
            &protected,
            &runtime,
        );
        coordinator.reconcile_startup_and_open().unwrap();
        let spec = ChangeSpecV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            command_id: "decision.connections.apply".into(),
            resource_id: Some(decision.id.clone()),
            desired_state: serde_json::to_value(input).unwrap(),
        };
        let preview = coordinator
            .preview(&WorkspaceId::default(), PreviewRequestV1::new(spec))
            .unwrap();
        let accepted = coordinator
            .accept(
                &WorkspaceId::default(),
                &VerifiedPrincipal::for_local_control(),
                ApplyRequestV1 {
                    schema_version: CHANGE_SPEC_SCHEMA_V1,
                    spec: preview.normalized_spec,
                    accept_digest: preview.change_digest,
                    expected_revisions: preview.expected_revisions,
                    idempotency_key: "decision-interrupted".into(),
                    apply_capability: None,
                },
            )
            .unwrap();
        operation_id = accepted.operation().operation_id.clone();
        let fault = crate::control::terminal_fault::inject(OperationState::Succeeded);
        assert!(coordinator.run_accepted(accepted).is_err());
        drop(fault);
        activated = saved(&stores);
        assert_eq!(activated.revision, before.revision + 1);
        assert_eq!(activated.decision_targets[0].consumers.len(), 1);
        assert!(
            stores
                .control()
                .decision_service_record(&WorkspaceId::default(), &decision.id, 1)
                .unwrap()
                .is_none()
        );
    }
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let external = NoExternal;
    let protected = ProtectedInput;
    let runtime = TransactionRuntime::default();
    let coordinator = TransactionCoordinator::new(
        stores.control(),
        stores.secrets(),
        stores.runtime(),
        &external,
        &protected,
        &runtime,
    );
    for _ in 0..2 {
        coordinator.reconcile_startup_and_open().unwrap();
        assert_eq!(
            stores
                .control()
                .load_operation(&operation_id)
                .unwrap()
                .unwrap()
                .state,
            OperationState::Succeeded
        );
        assert_eq!(saved(&stores), activated);
        assert_eq!(
            stores
                .control()
                .decision_service_record(&WorkspaceId::default(), &decision.id, 1)
                .unwrap(),
            Some(hiroute_domain::DecisionServiceRecord::V2(decision.clone()))
        );
        let binding = DecisionCredentialBindingV1::for_service(&decision)
            .unwrap()
            .unwrap();
        let admission = stores.admit_decision_credentials(&binding).unwrap();
        let request = DecisionCredentialLeaseRequestV1 {
            binding,
            key_id: admission.keys[0].key_id.clone(),
            admitted_keys: admission.keys,
            excluded_key_ids: vec![],
        };
        assert!(
            stores
                .lease_decision_credential_exact(&request)
                .unwrap()
                .is_some()
        );
    }
}
