//! Real coordinator and three stores; SQLite faults are limited to exact deletion checkpoints.
use super::*;
use super::lifecycle::{edit, saved};
use hiroute_application_api::ComputeManagementEditV1;
use hiroute_domain::{ComputeManagementRepositoryPort, ComputeManagementSourceV2, VerifiedSecretSubjectV1};

fn assert_credentials_restored(stores: &LocalStorageSet, source: &ComputeManagementSourceV2) {
    for key in &source.credentials {
        let credential = &key.credential;
        let subject = VerifiedSecretSubjectV1::from_authenticated_transport(credential.subject(), credential.owner_scope()).unwrap();
        let secret = stores.secrets().resolve_secret(&subject, credential, credential.purpose(),
            credential.allowed_destinations().iter().next().unwrap(), credential.generation()).unwrap();
        assert_eq!(stores.secrets().fingerprint(&secret).unwrap(), key.fingerprint);
    }
}

#[test]
fn compute_delete_failure_after_secret_activation_rolls_back_all_stores_and_survives_restart() {
    let root = tempdir().unwrap();
    populate_saved_source(root.path(), &WorkspaceId::default());
    let before;
    {
        let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
        before = saved(&stores);
        let registry = TrustedComputeCandidateRegistry::new();
        let input = ProtectedInput;
        let planner = ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
        let preview = planner.preview(edit(&stores, &before, ComputeManagementEditV1::Delete, &[])).unwrap();
        let runtime = TransactionRuntime::default();
        let external = NoExternal;
        let coordinator = TransactionCoordinator::new(stores.control(), stores.secrets(), stores.runtime(), &external, &input, &runtime);
        coordinator.reconcile_startup_and_open().unwrap();
        let prepared = planner.prepare_apply(ComputeConnectionApplyRequestV1 { spec:preview.result.spec,
            accept_digest:preview.result.accept_digest, expected_revisions:preview.result.expected_revisions,
            idempotency_key:"delete-rollback".into() }).unwrap();
        let accepted = coordinator.accept_prepared(&WorkspaceId::default(), &VerifiedPrincipal::for_local_control(), prepared).unwrap();
        // Activation deletes Secret entries before trying the Control row. Failure here
        // exercises real coordinator compensation of already-activated credential deletes.
        stores.control().with_connection(|connection| connection.execute_batch(
            "CREATE TRIGGER fail_connection_delete BEFORE DELETE ON compute_management_sources BEGIN SELECT RAISE(ABORT, 'fixture_delete_failure'); END;").unwrap());
        let completed = coordinator.run(&accepted.operation().operation_id).unwrap();
        assert_eq!(completed.state, OperationState::RolledBack);
        assert_eq!(saved(&stores), before);
        assert_credentials_restored(&stores, &before);
        stores.control().with_connection(|connection| connection.execute_batch("DROP TRIGGER fail_connection_delete;").unwrap());
    }
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let runtime = TransactionRuntime::default();
    let external = NoExternal;
    let input = ProtectedInput;
    let coordinator = TransactionCoordinator::new(stores.control(), stores.secrets(), stores.runtime(), &external, &input, &runtime);
    coordinator.reconcile_startup_and_open().unwrap();
    assert_eq!(saved(&stores), before);
    assert_credentials_restored(&stores, &before);
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
        let planner = ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
        let preview = planner.preview(edit(&stores, &before, ComputeManagementEditV1::Delete, &[])).unwrap();
        let runtime = TransactionRuntime::default();
        let external = NoExternal;
        let coordinator = TransactionCoordinator::new(stores.control(), stores.secrets(), stores.runtime(), &external, &input, &runtime);
        coordinator.reconcile_startup_and_open().unwrap();
        let prepared = planner.prepare_apply(ComputeConnectionApplyRequestV1 { spec:preview.result.spec,
            accept_digest:preview.result.accept_digest, expected_revisions:preview.result.expected_revisions,
            idempotency_key:"delete-recovery".into() }).unwrap();
        let accepted = coordinator.accept_prepared(&WorkspaceId::default(), &VerifiedPrincipal::for_local_control(), prepared).unwrap();
        operation_id = accepted.operation().operation_id.clone();
        stores.control().with_connection(|connection| connection.execute_batch(
            "CREATE TRIGGER fail_delete_terminal BEFORE UPDATE OF state ON operations WHEN NEW.state='succeeded' BEGIN SELECT RAISE(ABORT, 'fixture_terminal_failure'); END;").unwrap());
        assert!(coordinator.run(&operation_id).is_err());
        assert!(stores.control().compute_management_source(&before.source_id).unwrap().is_none());
        stores.control().with_connection(|connection| connection.execute_batch("DROP TRIGGER fail_delete_terminal;").unwrap());
    }
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let runtime = TransactionRuntime::default();
    let external = NoExternal;
    let input = ProtectedInput;
    let coordinator = TransactionCoordinator::new(stores.control(), stores.secrets(), stores.runtime(), &external, &input, &runtime);
    coordinator.reconcile_startup_and_open().unwrap();
    assert_eq!(stores.control().load_operation(&operation_id).unwrap().unwrap().state, OperationState::Succeeded);
    assert!(stores.control().compute_management_source(&before.source_id).unwrap().is_none());
    for key in &before.credentials {
        assert_eq!(stores.secrets().generation(&key.credential).unwrap(), key.credential.generation() + 1);
        assert!(!stores.secrets().with_connection(|connection| connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM secret_entries WHERE credential_id=?1)", [&key.key_id], |row| row.get::<_,bool>(0)).unwrap()));
    }
    let revisions = stores.control().current_revisions(&WorkspaceId::default()).unwrap();
    coordinator.reconcile_startup_and_open().unwrap();
    assert_eq!(stores.control().current_revisions(&WorkspaceId::default()).unwrap(), revisions);
}
