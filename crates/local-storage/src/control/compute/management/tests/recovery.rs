//! Real coordinator and three stores; SQLite faults are limited to exact deletion checkpoints.
use super::lifecycle::{edit, saved};
use super::*;
use hiroute_application_api::ComputeManagementEditV1;
use hiroute_domain::{
    ComputeManagementRepositoryPort, ComputeManagementSourceV2, VerifiedSecretSubjectV1,
};

fn assert_credentials_restored(stores: &LocalStorageSet, source: &ComputeManagementSourceV2) {
    for key in &source.credentials {
        let credential = &key.credential;
        let subject = VerifiedSecretSubjectV1::from_authenticated_transport(
            credential.subject(),
            credential.owner_scope(),
        )
        .unwrap();
        let secret = stores
            .secrets()
            .resolve_secret(
                &subject,
                credential,
                credential.purpose(),
                credential.allowed_destinations().iter().next().unwrap(),
                credential.generation(),
            )
            .unwrap();
        assert_eq!(
            stores.secrets().fingerprint(&secret).unwrap(),
            key.fingerprint
        );
    }
}

#[test]
fn compute_delete_failure_after_secret_activation_rolls_back_all_stores_and_survives_restart() {
    for (terminal_failure, blocked_secret) in [
        (false, None),
        (true, None),
        (false, Some(0)),
        (false, Some(1)),
    ] {
        let root = tempdir().unwrap();
        populate_saved_source(root.path(), &WorkspaceId::default());
        let before;
        let restored;
        let restored_revisions;
        let operation_id;
        {
            let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
            before = saved(&stores);
            let registry = TrustedComputeCandidateRegistry::new();
            let input = ProtectedInput;
            let planner = ComputeManagementPlanner::new(
                &registry,
                stores.control(),
                stores.secrets(),
                &input,
            );
            let preview = planner
                .preview(edit(&stores, &before, ComputeManagementEditV1::Delete, &[]))
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
            let prepared = planner
                .prepare_apply(ComputeConnectionApplyRequestV1 {
                    spec: preview.result.spec,
                    accept_digest: preview.result.accept_digest,
                    expected_revisions: preview.result.expected_revisions,
                    idempotency_key: "delete-rollback".into(),
                })
                .unwrap();
            let accepted = coordinator
                .accept_prepared(
                    &WorkspaceId::default(),
                    &VerifiedPrincipal::for_local_control(),
                    prepared,
                )
                .unwrap();
            operation_id = accepted.operation().operation_id.clone();
            let secrets = accepted.operation().plan.secrets();
            assert_eq!(secrets.len(), 2);
            let activated: Vec<_> = secrets
                .iter()
                .take(blocked_secret.unwrap_or(secrets.len()))
                .map(|secret| secret.credential().credential_id().to_owned())
                .collect();
            if let Some(index) = blocked_secret {
                let blocked = secrets[index].credential().credential_id();
                assert!(!blocked.contains('\''));
                stores.secrets().with_connection(|connection| connection.execute_batch(&format!("CREATE TRIGGER fail_secret_delete BEFORE DELETE ON secret_entries WHEN OLD.credential_id='{blocked}' BEGIN SELECT RAISE(ABORT, 'fixture_secret_delete_failure'); END;")).unwrap());
            }
            // Activation deletes Secret entries before trying the Control row. Failure here
            // exercises real coordinator compensation of already-activated credential deletes.
            stores.control().with_connection(|connection| connection.execute_batch(
            "CREATE TRIGGER fail_connection_delete BEFORE DELETE ON compute_management_sources BEGIN SELECT RAISE(ABORT, 'fixture_delete_failure'); END;").unwrap());
            if terminal_failure {
                stores.control().with_connection(|connection| connection.execute_batch("CREATE TRIGGER fail_rollback_terminal BEFORE UPDATE OF state ON operations WHEN NEW.state='rolled_back' BEGIN SELECT RAISE(ABORT, 'fixture_rollback_terminal'); END;").unwrap());
                assert!(coordinator.run(&operation_id).is_err());
            } else {
                let completed = coordinator.run(&operation_id).unwrap();
                assert_eq!(completed.state, OperationState::RolledBack);
            }
            restored = saved(&stores);
            let mut expected = before.clone();
            if !activated.is_empty() {
                expected.revision += 2;
            }
            for key in &mut expected.credentials {
                if !activated.contains(&key.key_id) {
                    continue;
                }
                let credential = &key.credential;
                key.credential = hiroute_domain::CredentialRefV1::new(
                    credential.credential_id(),
                    credential.owner_scope(),
                    credential.subject(),
                    credential.purpose(),
                    credential.allowed_destinations().iter().cloned(),
                    credential.generation() + 2,
                )
                .unwrap();
            }
            assert_eq!(restored, expected);
            if terminal_failure {
                super::compensation_guards::assert_compensation_owner_and_receipt_guards(
                    &stores,
                    &operation_id,
                    &before,
                    &restored,
                );
            }
            assert_credentials_restored(&stores, &restored);
            for key in &before.credentials {
                let credential = &key.credential;
                let subject = VerifiedSecretSubjectV1::from_authenticated_transport(
                    credential.subject(),
                    credential.owner_scope(),
                )
                .unwrap();
                assert_eq!(
                    stores
                        .secrets()
                        .resolve_secret(
                            &subject,
                            credential,
                            credential.purpose(),
                            credential.allowed_destinations().iter().next().unwrap(),
                            credential.generation()
                        )
                        .is_err(),
                    activated.contains(&key.key_id)
                );
            }
            let lease = stores
                .lease_native_credential_exact(&super::runtime_leases::frozen_request(
                    &restored, 0, 0,
                ))
                .unwrap()
                .unwrap();
            assert_eq!(
                lease.generation(),
                restored.credentials[0].credential.generation()
            );
            restored_revisions = stores
                .control()
                .current_revisions(&WorkspaceId::default())
                .unwrap();
            stores.secrets().with_connection(|connection| {
                connection
                    .execute_batch("DROP TRIGGER IF EXISTS fail_secret_delete;")
                    .unwrap()
            });
            stores.control().with_connection(|connection| {
            connection
                .execute_batch("DROP TRIGGER fail_connection_delete; DROP TRIGGER IF EXISTS fail_rollback_terminal;")
                .unwrap()
        });
        }
        let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
        let runtime = TransactionRuntime::default();
        let external = NoExternal;
        let input = ProtectedInput;
        let coordinator = TransactionCoordinator::new(
            stores.control(),
            stores.secrets(),
            stores.runtime(),
            &external,
            &input,
            &runtime,
        );
        coordinator.reconcile_startup_and_open().unwrap();
        assert_eq!(
            stores
                .control()
                .load_operation(&operation_id)
                .unwrap()
                .unwrap()
                .state,
            OperationState::RolledBack
        );
        assert_eq!(saved(&stores), restored);
        assert_credentials_restored(&stores, &restored);
        assert_eq!(
            stores
                .control()
                .current_revisions(&WorkspaceId::default())
                .unwrap(),
            restored_revisions
        );
        coordinator.reconcile_startup_and_open().unwrap();
        assert_eq!(
            stores
                .control()
                .current_revisions(&WorkspaceId::default())
                .unwrap(),
            restored_revisions
        );
    }
}

#[test]
fn compute_key_edit_rollback_preserves_authority_and_restores_forward_references() {
    for action in ["replace", "remove", "rebind"] {
        let root = tempdir().unwrap();
        populate_saved_source(root.path(), &WorkspaceId::default());
        let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
        let before = saved(&stores);
        let registry = TrustedComputeCandidateRegistry::new();
        let mut change = edit(&stores, &before, ComputeManagementEditV1::Delete, &[]);
        change.edit = None;
        change.selected_model_refs = before
            .models
            .iter()
            .map(|model| model.model_ref.clone())
            .collect();
        let key = &before.credentials[0];
        match action {
            "replace" => {
                let replacement = candidate("candidate/rollback-replace", "slot/primary");
                registry
                    .register_compute_candidate(replacement.clone())
                    .unwrap();
                change.key_edits.push(ComputeKeyEditV2::Replace {
                    key_id: key.key_id.clone(),
                    expected_generation: key.credential.generation(),
                    input_candidate: replacement.candidate,
                });
            }
            "remove" => change.key_edits.push(ComputeKeyEditV2::Remove {
                key_id: key.key_id.clone(),
                expected_generation: key.credential.generation(),
            }),
            _ => {
                let mut edited = candidate("candidate/rollback-endpoint", "slot/unused");
                edited.existing_source_id = Some(before.source_id.clone());
                edited.trusted_lineage_digest = Some(before.lineage_digest.clone());
                edited.credential_binding = ComputeCredentialBindingV2::NativeSaved {
                    credential_id: key.key_id.clone(),
                    expected_generation: key.credential.generation(),
                };
                let mut target = before.target.clone();
                target.authority = "rollback-messages.example.test".into();
                target.request_path = "/v1/messages".into();
                target.upstream_protocol = UpstreamProtocol::Messages;
                target.protocol_profile_id = "profile/messages".into();
                edited
                    .additional_native_endpoints
                    .push(hiroute_domain::ComputeNativeEndpointV3 {
                        target,
                        authentication: GatewayAuthenticationSemanticsV1::ApiKeyHeader {
                            header: "x-api-key".into(),
                        },
                        recheck: None,
                    });
                registry.register_compute_candidate(edited.clone()).unwrap();
                change.selected_model_refs = edited
                    .models
                    .iter()
                    .map(|model| model.model_ref.clone())
                    .collect();
                change.subject = ComputeManagementSubjectV2::Candidate {
                    candidate: edited.candidate,
                };
            }
        }
        let input = ProtectedInput;
        let planner =
            ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
        let preview = planner.preview(change).unwrap();
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
        let prepared = planner
            .prepare_apply(ComputeConnectionApplyRequestV1 {
                spec: preview.result.spec,
                accept_digest: preview.result.accept_digest,
                expected_revisions: preview.result.expected_revisions,
                idempotency_key: format!("rollback-{action}"),
            })
            .unwrap();
        let accepted = coordinator
            .accept_prepared(
                &WorkspaceId::default(),
                &VerifiedPrincipal::for_local_control(),
                prepared,
            )
            .unwrap();
        let secrets = accepted.operation().plan.secrets();
        assert_eq!(secrets.len(), if action == "rebind" { 2 } else { 1 });
        let expected_kind = match action {
            "replace" => hiroute_domain::SecretMutationKind::Upsert,
            "remove" => hiroute_domain::SecretMutationKind::Delete,
            _ => hiroute_domain::SecretMutationKind::Rebind,
        };
        assert!(secrets.iter().all(|secret| secret.kind() == expected_kind));
        // Fail the desired activation, while allowing the distinct forward compensation revision.
        stores.control().with_connection(|connection| connection.execute_batch(&format!("CREATE TRIGGER fail_source_edit BEFORE UPDATE ON compute_management_sources WHEN NEW.revision={} BEGIN SELECT RAISE(ABORT, 'fixture_source_edit'); END;", before.revision + 1)).unwrap());
        assert_eq!(
            coordinator
                .run(&accepted.operation().operation_id)
                .unwrap()
                .state,
            OperationState::RolledBack,
            "{action}"
        );
        let restored = saved(&stores);
        let mut expected = before.clone();
        expected.revision += 2;
        for key in &mut expected.credentials {
            if !secrets
                .iter()
                .any(|secret| secret.credential().credential_id() == key.key_id)
            {
                continue;
            }
            let credential = &key.credential;
            key.credential = hiroute_domain::CredentialRefV1::new(
                credential.credential_id(),
                credential.owner_scope(),
                credential.subject(),
                credential.purpose(),
                credential.allowed_destinations().iter().cloned(),
                credential.generation() + 2,
            )
            .unwrap();
        }
        assert_eq!(restored, expected, "{action}");
        assert_credentials_restored(&stores, &restored);
        assert!(
            stores
                .lease_native_credential_exact(&super::runtime_leases::frozen_request(
                    &restored, 0, 0
                ))
                .unwrap()
                .is_some(),
            "{action}"
        );
    }
}

#[test]
fn compute_delete_activated_before_terminal_journal_failure_completes_once_on_startup() {
    let root = tempdir().unwrap();
    populate_saved_source(root.path(), &WorkspaceId::default());
    let before;
    let operation_id;
    {
        let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
        before = saved(&stores);
        let registry = TrustedComputeCandidateRegistry::new();
        let input = ProtectedInput;
        let planner =
            ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
        let preview = planner
            .preview(edit(&stores, &before, ComputeManagementEditV1::Delete, &[]))
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
        let prepared = planner
            .prepare_apply(ComputeConnectionApplyRequestV1 {
                spec: preview.result.spec,
                accept_digest: preview.result.accept_digest,
                expected_revisions: preview.result.expected_revisions,
                idempotency_key: "delete-recovery".into(),
            })
            .unwrap();
        let accepted = coordinator
            .accept_prepared(
                &WorkspaceId::default(),
                &VerifiedPrincipal::for_local_control(),
                prepared,
            )
            .unwrap();
        operation_id = accepted.operation().operation_id.clone();
        stores.control().with_connection(|connection| connection.execute_batch(
            "CREATE TRIGGER fail_delete_terminal BEFORE UPDATE OF state ON operations WHEN NEW.state='succeeded' BEGIN SELECT RAISE(ABORT, 'fixture_terminal_failure'); END;").unwrap());
        assert!(coordinator.run(&operation_id).is_err());
        assert!(
            stores
                .control()
                .compute_management_source(&before.source_id)
                .unwrap()
                .is_none()
        );
        stores.control().with_connection(|connection| {
            connection
                .execute_batch("DROP TRIGGER fail_delete_terminal;")
                .unwrap()
        });
    }
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let runtime = TransactionRuntime::default();
    let external = NoExternal;
    let input = ProtectedInput;
    let coordinator = TransactionCoordinator::new(
        stores.control(),
        stores.secrets(),
        stores.runtime(),
        &external,
        &input,
        &runtime,
    );
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
    assert!(
        stores
            .control()
            .compute_management_source(&before.source_id)
            .unwrap()
            .is_none()
    );
    for key in &before.credentials {
        assert_eq!(
            stores.secrets().generation(&key.credential).unwrap(),
            key.credential.generation() + 1
        );
        assert!(!stores.secrets().with_connection(|connection| {
            connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM secret_entries WHERE credential_id=?1)",
                    [&key.key_id],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap()
        }));
    }
    let revisions = stores
        .control()
        .current_revisions(&WorkspaceId::default())
        .unwrap();
    coordinator.reconcile_startup_and_open().unwrap();
    assert_eq!(
        stores
            .control()
            .current_revisions(&WorkspaceId::default())
            .unwrap(),
        revisions
    );
}
