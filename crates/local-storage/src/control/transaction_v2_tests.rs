use std::collections::BTreeMap;
use std::fs;

use crate::test_tempdir as tempdir;
use hiroute_domain::{
    BeginOperationOutcome, CanonicalDigest, CompensationOutcome, ControlRepositoryPort,
    EffectReconciliation, ExternalEffectIntentV1, IdempotencyScopeV1, OperationId, OperationV1,
    OwnedEffectKind, ProtectedApplyCapability, RevisionMismatch, RevisionSetV1, RuntimeMutationV1,
    SecretMutationV1, SecretStorePort, TransactionPlanV1, WorkspaceId,
};
use serde_json::json;

use super::{ControlStore, ManagedArtifactStore};
use crate::{LocalSecretStore, LocalStorageError, SqliteBackup};

fn operation(workspace: &WorkspaceId, key: &str, target: u64) -> OperationV1 {
    let request_digest = CanonicalDigest::of_bytes(format!("request-{key}").as_bytes());
    let accepted_digest = CanonicalDigest::of_bytes(b"accepted-change");
    let scope = IdempotencyScopeV1::new("interactive-user", "ApplySetup", key).unwrap();
    OperationV1::new(
        OperationId::derive(workspace, &scope, &request_digest),
        workspace.clone(),
        scope,
        request_digest,
        accepted_digest,
        RevisionSetV1 {
            target,
            dependencies: BTreeMap::new(),
        },
        TransactionPlanV1::from_registered_typed_planner(
            hiroute_domain::ChangeSpecV1 {
                schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
                command_id: "setup.apply".to_owned(),
                resource_id: Some(workspace.to_string()),
                desired_state: json!({"connection_option_id": "source-a"}),
            },
            json!({"connection_option_id": "source-a"}),
            None,
            Vec::new(),
            vec![
                RuntimeMutationV1::from_registered_planner(
                    "active/setup",
                    json!({"ready": true}),
                    0,
                )
                .unwrap(),
            ],
            registered_external_effects(),
        )
        .unwrap(),
    )
    .unwrap()
}

fn registered_external_effects() -> Vec<ExternalEffectIntentV1> {
    vec![
        ExternalEffectIntentV1::from_registered_adapter(
            "publication-setup",
            OwnedEffectKind::Publication,
            "publication/current",
            None,
            json!({"setup": "active"}),
            0o644,
            false,
        )
        .unwrap(),
        ExternalEffectIntentV1::from_registered_adapter(
            "agent-setup",
            OwnedEffectKind::AgentArtifact,
            "agents/codex",
            None,
            json!({"configured": true}),
            0o640,
            false,
        )
        .unwrap(),
    ]
}

fn authorize(
    control: &ControlStore,
    operation: &OperationV1,
    capability: &str,
    expires_at: i64,
) -> hiroute_domain::VerifiedApplyAuthorizationV1 {
    control
        .grant_apply_capability(capability, operation, expires_at)
        .unwrap();
    control
        .verify_apply_authorization(
            &ProtectedApplyCapability::new(capability.to_owned()).unwrap(),
            &operation.workspace_id,
            &operation.idempotency.principal,
            &operation.idempotency.operation_kind,
            &operation.accepted_digest,
            &operation.expected_revisions,
        )
        .unwrap()
}

#[test]
fn capability_is_exact_one_shot_and_consumed_with_operation_admission() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("data");
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        root.join("control.db"),
        root.join("backups"),
    )
    .unwrap();
    let workspace = WorkspaceId::default();
    let operation = operation(&workspace, "idem-a", 0);
    let authorization = authorize(&control, &operation, "capability-a", i64::MAX);
    assert_eq!(
        control.begin_operation(&operation, &authorization).unwrap(),
        BeginOperationOutcome::Created
    );
    assert!(control.writer_recovery_required().unwrap());
    assert!(
        control
            .verify_apply_authorization(
                &ProtectedApplyCapability::new("capability-a".to_owned()).unwrap(),
                &workspace,
                "interactive-user",
                "ApplySetup",
                &operation.accepted_digest,
                &operation.expected_revisions,
            )
            .is_err()
    );
    assert_eq!(
        control.begin_operation(&operation, &authorization).unwrap(),
        BeginOperationOutcome::ExistingSame(Box::new(operation.clone()))
    );
    control.with_connection(|connection| {
        let consumed: String = connection
            .query_row(
                "SELECT consumed_operation_id FROM apply_capabilities",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(consumed, operation.operation_id.as_str());
        let claim: String = connection
            .query_row("SELECT operation_id FROM writer_claim", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(claim, operation.operation_id.as_str());
    });
}

#[test]
fn capability_missing_wrong_expired_revoked_and_wrong_digest_fail_closed() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("data");
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        root.join("control.db"),
        root.join("backups"),
    )
    .unwrap();
    let operation = operation(&WorkspaceId::default(), "idem-a", 0);
    let verify = |raw: &str, accepted: &CanonicalDigest| {
        control.verify_apply_authorization(
            &ProtectedApplyCapability::new(raw.to_owned()).unwrap(),
            &operation.workspace_id,
            "interactive-user",
            "ApplySetup",
            accepted,
            &operation.expected_revisions,
        )
    };
    assert!(verify("missing", &operation.accepted_digest).is_err());
    control
        .grant_apply_capability("correct", &operation, i64::MAX)
        .unwrap();
    assert!(verify("wrong", &operation.accepted_digest).is_err());
    assert!(verify("correct", &CanonicalDigest::of_bytes(b"wrong")).is_err());
    let capability = ProtectedApplyCapability::new("correct".to_owned()).unwrap();
    assert!(
        control
            .verify_apply_authorization(
                &capability,
                &operation.workspace_id,
                "desktop",
                "ApplySetup",
                &operation.accepted_digest,
                &operation.expected_revisions,
            )
            .is_err()
    );
    assert!(
        control
            .verify_apply_authorization(
                &capability,
                &operation.workspace_id,
                "interactive-user",
                "ApplyRouting",
                &operation.accepted_digest,
                &operation.expected_revisions,
            )
            .is_err()
    );
    assert!(
        control
            .verify_apply_authorization(
                &capability,
                &WorkspaceId::parse("personal/other").unwrap(),
                "interactive-user",
                "ApplySetup",
                &operation.accepted_digest,
                &operation.expected_revisions,
            )
            .is_err()
    );
    assert!(
        control
            .verify_apply_authorization(
                &capability,
                &operation.workspace_id,
                "interactive-user",
                "ApplySetup",
                &operation.accepted_digest,
                &RevisionSetV1 {
                    target: 1,
                    dependencies: BTreeMap::new(),
                },
            )
            .is_err()
    );
    control.with_connection(|connection| {
        connection
            .execute(
                "UPDATE apply_capabilities SET capability_scope = 'wrong-scope'",
                [],
            )
            .unwrap();
    });
    assert!(verify("correct", &operation.accepted_digest).is_err());
    control.with_connection(|connection| {
        connection
            .execute(
                "UPDATE apply_capabilities
                 SET capability_scope = 'apply:one-shot', revoked = 1",
                [],
            )
            .unwrap();
    });
    assert!(verify("correct", &operation.accepted_digest).is_err());
    control.with_connection(|connection| {
        connection
            .execute(
                "UPDATE apply_capabilities SET revoked = 0, expires_at = 0",
                [],
            )
            .unwrap();
    });
    assert!(verify("correct", &operation.accepted_digest).is_err());
}

#[test]
fn revision_failure_rolls_back_capability_consumption_and_writer_claim() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("data");
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        root.join("control.db"),
        root.join("backups"),
    )
    .unwrap();
    let workspace = WorkspaceId::default();
    let operation = operation(&workspace, "idem-a", 0);
    let authorization = authorize(&control, &operation, "capability-a", i64::MAX);
    control
        .set_dependency_revision(&workspace, "prices", 1)
        .unwrap();
    assert_eq!(
        control.begin_operation(&operation, &authorization).unwrap(),
        BeginOperationOutcome::RevisionChanged(RevisionMismatch::Dependencies)
    );
    assert!(!control.writer_recovery_required().unwrap());
    control.with_connection(|connection| {
        let consumed: Option<String> = connection
            .query_row(
                "SELECT consumed_operation_id FROM apply_capabilities",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(consumed.is_none());
        let operations: u64 = connection
            .query_row("SELECT count(*) FROM operations", [], |row| row.get(0))
            .unwrap();
        assert_eq!(operations, 0);
    });
}

#[test]
fn concurrent_admission_keeps_the_second_capability_unconsumed() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("data");
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        root.join("control.db"),
        root.join("backups"),
    )
    .unwrap();
    let workspace = WorkspaceId::default();
    let first = operation(&workspace, "idem-a", 0);
    let second = operation(&workspace, "idem-b", 0);
    let first_authorization = authorize(&control, &first, "capability-a", i64::MAX);
    let second_authorization = authorize(&control, &second, "capability-b", i64::MAX);

    assert_eq!(
        control
            .begin_operation(&first, &first_authorization)
            .unwrap(),
        BeginOperationOutcome::Created
    );
    assert_eq!(
        control
            .writer_claim_operation()
            .unwrap()
            .unwrap()
            .operation_id,
        first.operation_id
    );
    assert_eq!(
        control
            .begin_operation(&second, &second_authorization)
            .err()
            .unwrap()
            .code,
        hiroute_domain::PortErrorCode::Conflict
    );
    control.with_connection(|connection| {
        let consumed: Option<String> = connection
            .query_row(
                "SELECT consumed_operation_id FROM apply_capabilities
                 WHERE capability_digest = ?1",
                [CanonicalDigest::of_bytes(b"capability-b").as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(consumed.is_none());
        let operations: u64 = connection
            .query_row("SELECT count(*) FROM operations", [], |row| row.get(0))
            .unwrap();
        assert_eq!(operations, 1);
    });
}

#[test]
fn control_stage_is_invisible_and_rollback_moves_revision_forward() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("data");
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        root.join("control.db"),
        root.join("backups"),
    )
    .unwrap();
    let workspace = WorkspaceId::default();
    let operation = operation(&workspace, "idem-a", 0);
    let authorization = authorize(&control, &operation, "capability-a", i64::MAX);
    control.begin_operation(&operation, &authorization).unwrap();
    let effect = control
        .apply_control(
            &operation.operation_id,
            &workspace,
            0,
            &json!({"ready": true}),
        )
        .unwrap();
    assert!(matches!(
        control
            .observe_control(&operation.operation_id, &workspace)
            .unwrap(),
        EffectReconciliation::Staged(_)
    ));
    assert!(control.desired_state(&workspace).unwrap().is_none());
    assert_eq!(control.current_revisions(&workspace).unwrap().target, 0);
    control.activate_control(&effect).unwrap();
    assert_eq!(
        control.desired_state(&workspace).unwrap(),
        Some(json!({"ready": true}))
    );
    assert_eq!(control.current_revisions(&workspace).unwrap().target, 1);
    assert_eq!(
        control.compensate_control(&effect).unwrap(),
        CompensationOutcome::Compensated
    );
    assert!(control.desired_state(&workspace).unwrap().is_none());
    assert_eq!(control.current_revisions(&workspace).unwrap().target, 2);
}

fn artifact_intent(before: Option<CanonicalDigest>) -> ExternalEffectIntentV1 {
    ExternalEffectIntentV1::from_registered_adapter(
        "agent-setup",
        OwnedEffectKind::AgentArtifact,
        "agents/codex",
        before,
        json!({"configured": true}),
        0o640,
        false,
    )
    .unwrap()
}

#[test]
fn artifact_stage_is_invisible_and_mode_is_owned_and_conditionally_restored() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    for before_mode in [0o644, 0o640, 0o666, 0o1666] {
        let directory = tempdir().unwrap();
        let root = directory.path().join("artifacts");
        let restore = directory.path().join("restore");
        crate::test_create_dir_all(root.join("agents")).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(root.join("agents"), fs::Permissions::from_mode(0o700)).unwrap();
        let target = root.join("agents/codex");
        fs::write(&target, b"before").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(before_mode)).unwrap();
        let store =
            ManagedArtifactStore::open(&crate::test_storage_authority(), &root, &restore).unwrap();
        let before = store.current_external_fingerprint("agents/codex").unwrap();
        let intent = artifact_intent(before);
        let operation = OperationId::parse("op_11111111111111111111111111111111").unwrap();
        let effect = store.apply_artifact(&operation, &intent).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"before");
        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o7777, before_mode);
        assert!(matches!(
            store.observe_artifact(&operation, &intent).unwrap(),
            EffectReconciliation::Staged(_)
        ));
        store.activate_artifact(&effect).unwrap();
        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o7777, 0o640);
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            store.observe_artifact(&operation, &intent).unwrap(),
            EffectReconciliation::OwnershipLost(_)
        ));
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        assert_eq!(
            store.compensate_artifact(&effect).unwrap(),
            CompensationOutcome::Compensated
        );
        assert_eq!(fs::read(&target).unwrap(), b"before");
        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o7777, before_mode);
    }
}

#[test]
fn restore_markers_with_missing_key_are_locked_without_reinitialization() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().unwrap();
    let root = directory.path().join("artifacts");
    let restore = directory.path().join("restore");
    crate::test_create_dir_all(root.join("agents")).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(root.join("agents"), fs::Permissions::from_mode(0o700)).unwrap();
    let store =
        ManagedArtifactStore::open(&crate::test_storage_authority(), &root, &restore).unwrap();
    let intent = artifact_intent(None);
    let operation = OperationId::parse("op_22222222222222222222222222222222").unwrap();
    store.apply_artifact(&operation, &intent).unwrap();
    drop(store);
    fs::remove_file(restore.join(".restore-key")).unwrap();
    let entries_before = fs::read_dir(&restore).unwrap().count();
    assert!(matches!(
        ManagedArtifactStore::open(&crate::test_storage_authority(), &root, &restore),
        Err(LocalStorageError::Locked)
    ));
    assert_eq!(fs::read_dir(&restore).unwrap().count(), entries_before);
    assert!(!restore.join(".restore-key").exists());

    let other_root = directory.path().join("other-artifacts");
    let other_restore = directory.path().join("other-restore");
    let other = ManagedArtifactStore::open(
        &crate::test_storage_authority(),
        &other_root,
        &other_restore,
    )
    .unwrap();
    drop(other);
    fs::copy(
        other_restore.join(".restore-key"),
        restore.join(".restore-key"),
    )
    .unwrap();
    assert!(matches!(
        ManagedArtifactStore::open(&crate::test_storage_authority(), &root, &restore),
        Err(LocalStorageError::Locked)
    ));
}

#[test]
fn sensitive_artifacts_require_owner_only_mode() {
    let intent = ExternalEffectIntentV1::from_registered_adapter(
        "agent-sensitive",
        OwnedEffectKind::AgentArtifact,
        "agents/secret",
        None,
        json!({"configured": true}),
        0o640,
        true,
    );
    assert!(intent.is_err());
}

#[test]
fn one_random_secret_sentinel_never_reaches_journal_databases_backup_or_artifacts() {
    use std::os::unix::fs::PermissionsExt;

    let sentinel = (0_u16..97)
        .map(|index| ((index * 73 + 19) % 251) as u8)
        .collect::<Vec<_>>();
    let directory = tempdir().unwrap();
    let root = directory.path().join("data");
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        root.join("control.db"),
        root.join("migration"),
    )
    .unwrap();
    let secrets = LocalSecretStore::open(
        &crate::test_storage_authority(),
        root.join("secrets.db"),
        root.join("master-key"),
        root.join("migration"),
    )
    .unwrap();
    let protected = hiroute_domain::ProtectedSecret::new(sentinel.clone()).unwrap();
    let credential = hiroute_domain::CredentialRefV1::new(
        "credential/source-a",
        "connection/source-a",
        "hirouted",
        "provider-auth",
        ["provider-api".to_owned()],
        0,
    )
    .unwrap();
    let mutation = SecretMutationV1::upsert(
        credential,
        0,
        "primary",
        Some(secrets.fingerprint(&protected).unwrap()),
    )
    .unwrap();
    let workspace = WorkspaceId::default();
    let request_digest = CanonicalDigest::of_bytes(b"sentinel-request");
    let accepted_digest = CanonicalDigest::of_bytes(b"sentinel-change");
    let scope = IdempotencyScopeV1::new("interactive-user", "ApplySetup", "sentinel-idem").unwrap();
    let mut operation = OperationV1::new(
        OperationId::derive(&workspace, &scope, &request_digest),
        workspace.clone(),
        scope,
        request_digest,
        accepted_digest,
        RevisionSetV1 {
            target: 0,
            dependencies: BTreeMap::new(),
        },
        TransactionPlanV1::from_registered_typed_planner(
            hiroute_domain::ChangeSpecV1 {
                schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
                command_id: "setup.apply".to_owned(),
                resource_id: Some(workspace.to_string()),
                desired_state: json!({
                    "connection_option_id": "source-a",
                    "secret": {"input_slot": "primary", "expected_generation": 0}
                }),
            },
            json!({"connection_option_id": "source-a"}),
            None,
            vec![mutation.clone()],
            vec![
                RuntimeMutationV1::from_registered_planner(
                    "active/setup",
                    json!({"ready": true}),
                    0,
                )
                .unwrap(),
            ],
            registered_external_effects(),
        )
        .unwrap(),
    )
    .unwrap();
    let authorization = authorize(&control, &operation, "sentinel-capability", i64::MAX);
    control.begin_operation(&operation, &authorization).unwrap();
    let secret_effect = secrets
        .apply_secret(&operation.operation_id, &mutation, Some(&protected))
        .unwrap();
    secrets.activate_secret(&secret_effect).unwrap();
    control.save_operation(&mut operation).unwrap();
    secrets.checkpoint().unwrap();
    let backup = secrets
        .with_connection(|connection| {
            SqliteBackup::create(
                &crate::test_storage_authority(),
                connection,
                root.join("sentinel-backup/secrets.db"),
            )
        })
        .unwrap();
    assert!(backup.manifest().key_id.is_some());

    let artifact_root = root.join("artifacts");
    crate::test_create_dir_all(artifact_root.join("agents")).unwrap();
    fs::set_permissions(&artifact_root, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(
        artifact_root.join("agents"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let artifacts = ManagedArtifactStore::open(
        &crate::test_storage_authority(),
        &artifact_root,
        root.join("restore"),
    )
    .unwrap();
    let intent = artifact_intent(None);
    let artifact_effect = artifacts
        .apply_artifact(&operation.operation_id, &intent)
        .unwrap();
    artifacts.activate_artifact(&artifact_effect).unwrap();

    for output in [
        serde_json::to_vec(&operation).unwrap(),
        serde_json::to_vec(&secret_effect).unwrap(),
        serde_json::to_vec(backup.manifest()).unwrap(),
    ] {
        assert!(
            !output
                .windows(sentinel.len())
                .any(|window| window == sentinel)
        );
    }
    fn scan(path: &std::path::Path, sentinel: &[u8]) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let metadata = entry.metadata().unwrap();
            if metadata.is_dir() {
                scan(&entry.path(), sentinel);
            } else {
                let bytes = fs::read(entry.path()).unwrap();
                assert!(
                    !bytes
                        .windows(sentinel.len())
                        .any(|window| window == sentinel),
                    "plaintext sentinel leaked to {}",
                    entry.path().display()
                );
            }
        }
    }
    scan(&root, &sentinel);
}

#[test]
fn status_projection_is_scoped_fresh_and_cannot_validate_executable_content() {
    let root = tempdir().unwrap();
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        root.path().join("control.db"),
        root.path().join("backups"),
    )
    .unwrap();
    let mut op = operation(&WorkspaceId::default(), "status-projection", 0);
    let auth = authorize(&control, &op, "status-projection-capability", i64::MAX);
    control.begin_operation(&op, &auth).unwrap();
    op.state = hiroute_domain::OperationState::CompilingPublication;
    op.safe_error_code = Some("INJECTED_FAILURE".into());
    control.save_operation(&mut op).unwrap();
    let status = control
        .operation_status_for_idempotency(&op.workspace_id, &op.idempotency)
        .unwrap()
        .unwrap();
    assert_eq!(status.operation_id, op.operation_id);
    assert_eq!(status.state, op.state);
    assert_eq!(status.generation, op.generation);
    assert_eq!(status.accepted_digest, op.accepted_digest);
    assert_eq!(status.safe_error_code, op.safe_error_code);
    for scope in [
        IdempotencyScopeV1::new("desktop", "ApplySetup", "status-projection").unwrap(),
        IdempotencyScopeV1::new("interactive-user", "ApplyOther", "status-projection").unwrap(),
        IdempotencyScopeV1::new("interactive-user", "ApplySetup", "other").unwrap(),
    ] {
        assert!(
            control
                .operation_status_for_idempotency(&op.workspace_id, &scope)
                .unwrap()
                .is_none()
        );
    }
    control.connection.borrow().execute(
        "UPDATE operations SET operation_json=json_set(operation_json, '$.plan.spec.command_id', 'corrupt')", [],
    ).unwrap();
    assert_eq!(
        control
            .operation_status_for_idempotency(&op.workspace_id, &op.idempotency)
            .unwrap()
            .unwrap(),
        status
    );
    assert!(control.load_operation(&op.operation_id).is_err());
    control
        .connection
        .borrow()
        .execute(
            "UPDATE operations SET request_digest=?1",
            [CanonicalDigest::of_bytes(b"changed").as_str()],
        )
        .unwrap();
    assert!(
        control
            .operation_status_for_idempotency(&op.workspace_id, &op.idempotency)
            .is_err()
    );
}

#[test]
fn journal_patch_preserves_cold_recovery_and_rejects_changed_inputs_and_stale_writers() {
    let root = tempdir().unwrap();
    let db = root.path().join("control.db");
    let backups = root.path().join("backups");
    let control = ControlStore::open(&crate::test_storage_authority(), &db, &backups).unwrap();
    let mut op = operation(&WorkspaceId::default(), "journal-patch", 0);
    let auth = authorize(&control, &op, "journal-patch-capability", i64::MAX);
    control.begin_operation(&op, &auth).unwrap();
    assert!(control.operation_is_current(&op).unwrap());
    let stale = op.clone();
    let before = serde_json::to_value(&op).unwrap();
    op.state = hiroute_domain::OperationState::CompilingPublication;
    op.steps[3].attempts += 1;
    op.safe_error_code = Some("INJECTED_FAILURE".into());
    control.save_operation(&mut op).unwrap();
    assert_eq!(op.generation, 1);
    assert!(control.operation_is_current(&op).unwrap());
    assert!(!control.operation_is_current(&stale).unwrap());
    assert_eq!(
        control.load_operation(&op.operation_id).unwrap().unwrap(),
        op
    );
    assert_eq!(serde_json::to_value(&op).unwrap()["plan"], before["plan"]);
    assert!(control.save_operation(&mut stale.clone()).is_err());
    let mut altered = op.clone();
    altered.accepted_digest = CanonicalDigest::of_bytes(b"changed confirmation");
    assert!(control.save_operation(&mut altered).is_err());
    altered = op.clone();
    altered.steps[0].deterministic_input_digest = CanonicalDigest::of_bytes(b"changed step");
    assert!(control.save_operation(&mut altered).is_err());
    altered = op.clone();
    // The helper's key changes only idempotency, not its plan. Use a different resource so this
    // checks a genuinely changed immutable plan against the established checkpoint.
    altered.plan = operation(
        &WorkspaceId::parse("personal/other").unwrap(),
        "different-plan",
        0,
    )
    .plan;
    assert_ne!(altered.plan, op.plan);
    assert!(control.operation_is_current(&altered).is_err());
    drop(control);
    let control = ControlStore::open(&crate::test_storage_authority(), &db, &backups).unwrap();
    assert_eq!(control.recoverable_operations().unwrap(), vec![op.clone()]);
    let mut recovered = control.load_operation(&op.operation_id).unwrap().unwrap();
    recovered.safe_error_code = None;
    control.save_operation(&mut recovered).unwrap();
    assert_eq!(recovered.generation, 2);
    assert_eq!(
        control.load_operation(&op.operation_id).unwrap().unwrap(),
        recovered
    );
    control.connection.borrow().execute(
        "UPDATE operations SET operation_json=json_set(operation_json, '$.plan.spec.command_id', 'corrupt')", [],
    ).unwrap();
    assert!(control.load_operation(&op.operation_id).is_err());
    assert!(!control.operation_is_current(&recovered).unwrap());
    assert!(control.save_operation(&mut recovered).is_err());
}

#[test]
fn claimed_needs_attention_is_reloaded_and_a_proven_rollback_releases_its_writer() {
    let root = tempdir().unwrap();
    let db = root.path().join("control.db");
    let backups = root.path().join("backups");
    let control = ControlStore::open(&crate::test_storage_authority(), &db, &backups).unwrap();
    let mut operation = operation(&WorkspaceId::default(), "claimed-attention", 0);
    let authorization = authorize(
        &control,
        &operation,
        "claimed-attention-capability",
        i64::MAX,
    );
    control.begin_operation(&operation, &authorization).unwrap();
    operation
        .transition(hiroute_domain::OperationState::RollingBack)
        .unwrap();
    operation
        .transition(hiroute_domain::OperationState::NeedsAttention)
        .unwrap();
    control.finish_operation(&mut operation).unwrap();
    assert!(control.writer_recovery_required().unwrap());
    drop(control);

    let control = ControlStore::open(&crate::test_storage_authority(), &db, &backups).unwrap();
    assert_eq!(
        control.recoverable_operations().unwrap(),
        vec![operation.clone()]
    );
    operation
        .transition(hiroute_domain::OperationState::RolledBack)
        .unwrap();
    control.finish_operation(&mut operation).unwrap();
    assert!(!control.writer_recovery_required().unwrap());
    assert!(control.recoverable_operations().unwrap().is_empty());
}

#[test]
fn journal_rollback_and_commit_failure_do_not_advance_the_owned_checkpoint() {
    let root = tempdir().unwrap();
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        root.path().join("control.db"),
        root.path().join("backups"),
    )
    .unwrap();
    let mut op = operation(&WorkspaceId::default(), "journal-commit-failure", 0);
    let auth = authorize(&control, &op, "journal-commit-failure-capability", i64::MAX);
    control.begin_operation(&op, &auth).unwrap();
    let original = op.clone();
    op.state = hiroute_domain::OperationState::CompilingPublication;
    op.steps[3].attempts += 1;
    {
        let mut connection = control.connection.borrow_mut();
        let tx = connection.transaction().unwrap();
        assert_eq!(super::journal::save(&tx, &op).unwrap(), 1);
        tx.rollback().unwrap();
    }
    assert_eq!(op.generation, 0);
    assert_eq!(
        control.load_operation(&op.operation_id).unwrap().unwrap(),
        original
    );
    control.connection.borrow().execute_batch(
        "CREATE TABLE journal_commit_parent(id INTEGER PRIMARY KEY);
         CREATE TABLE journal_commit_failure(id INTEGER REFERENCES journal_commit_parent(id) DEFERRABLE INITIALLY DEFERRED);
         CREATE TRIGGER fail_journal_commit AFTER UPDATE ON operations BEGIN INSERT INTO journal_commit_failure VALUES(99); END;"
    ).unwrap();
    assert!(control.save_operation(&mut op).is_err());
    assert_eq!(op.generation, 0);
    assert!(!op.journal_is_committed().unwrap());
    assert!(!control.operation_is_current(&op).unwrap());
    assert_eq!(
        control.load_operation(&op.operation_id).unwrap().unwrap(),
        original
    );
    control
        .connection
        .borrow()
        .execute("DROP TRIGGER fail_journal_commit", [])
        .unwrap();
    control.save_operation(&mut op).unwrap();
    assert_eq!(op.generation, 1);
    assert_eq!(
        control.load_operation(&op.operation_id).unwrap().unwrap(),
        op
    );
    let step_json: String = control
        .connection
        .borrow()
        .query_row(
            "SELECT step_json FROM operation_steps WHERE operation_id=?1 AND step_no=3",
            [op.operation_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&step_json).unwrap()["step"],
        serde_json::to_value(&op.steps[3]).unwrap()
    );
}

#[test]
fn current_operation_rejects_uncommitted_journal_and_forged_acknowledgement() {
    let root = tempdir().unwrap();
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        root.path().join("control.db"),
        root.path().join("backups"),
    )
    .unwrap();
    let mut op = operation(&WorkspaceId::default(), "current-journal", 0);
    let auth = authorize(&control, &op, "current-journal-capability", i64::MAX);
    control.begin_operation(&op, &auth).unwrap();
    let original = op.clone();
    for field in 0..5 {
        let mut modified = op.clone();
        match field {
            0 => modified.steps[0].attempts += 1,
            1 => modified.steps[0].terminal_result = Some("uncommitted".into()),
            2 => modified.safe_error_code = Some("UNCOMMITTED".into()),
            3 => modified.state = hiroute_domain::OperationState::CompilingPublication,
            _ => modified.steps[3]
                .effects
                .push(hiroute_domain::OwnedEffectV1 {
                    effect_id: "forged-publication".into(),
                    kind: OwnedEffectKind::Publication,
                    target: "publication/current".into(),
                    before_fingerprint: None,
                    after_fingerprint: Some(CanonicalDigest::of_bytes(b"forged")),
                    compensation: std::sync::Arc::new(json!({"forged": true})),
                }),
        }
        assert!(!modified.journal_is_committed().unwrap());
        assert!(!control.operation_is_current(&modified).unwrap());
    }
    // A caller can invoke acknowledge, but cannot manufacture the matching durable journal.
    control.save_operation(&mut op).unwrap();
    let mut forged = original;
    forged.steps[0].attempts += 1;
    forged.acknowledge_journal_commit(op.generation);
    assert!(forged.journal_is_committed().unwrap());
    assert!(!control.operation_is_current(&forged).unwrap());
    assert!(control.operation_is_current(&op).unwrap());
    control.connection.borrow().execute(
        "UPDATE operation_steps SET step_json=json_set(step_json, '$.step.attempts', 99) WHERE step_no=0", [],
    ).unwrap();
    assert!(!control.operation_is_current(&op).unwrap());
}

#[test]
fn current_operation_rejects_durable_identity_and_step_index_corruption() {
    let root = tempdir().unwrap();
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        root.path().join("control.db"),
        root.path().join("backups"),
    )
    .unwrap();
    let op = operation(&WorkspaceId::default(), "current-durable-identity", 0);
    let auth = authorize(
        &control,
        &op,
        "current-durable-identity-capability",
        i64::MAX,
    );
    control.begin_operation(&op, &auth).unwrap();
    assert!(control.operation_is_current(&op).unwrap());

    control.connection.borrow().execute(
        "UPDATE operations SET operation_json=json_set(operation_json, '$.request_digest', 'sha256:forged') WHERE operation_id=?1",
        [op.operation_id.as_str()],
    ).unwrap();
    assert!(!control.operation_is_current(&op).unwrap());
    control.connection.borrow().execute(
        "UPDATE operations SET operation_json=json_set(operation_json, '$.request_digest', ?2) WHERE operation_id=?1",
        rusqlite::params![op.operation_id.as_str(), op.request_digest.as_str()],
    ).unwrap();
    assert!(control.operation_is_current(&op).unwrap());

    control
        .connection
        .borrow()
        .execute(
            "UPDATE operation_steps SET state='applied' WHERE operation_id=?1 AND step_no=0",
            [op.operation_id.as_str()],
        )
        .unwrap();
    assert!(!control.operation_is_current(&op).unwrap());
}

#[test]
fn current_operation_allows_only_the_committed_parked_settings_tail_without_a_writer() {
    let root = tempdir().unwrap();
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        root.path().join("control.db"),
        root.path().join("backups"),
    )
    .unwrap();
    let mut op = operation(&WorkspaceId::default(), "parked-tail", 0);
    let auth = authorize(&control, &op, "parked-tail-capability", i64::MAX);
    control.begin_operation(&op, &auth).unwrap();
    op.state = hiroute_domain::OperationState::Activating;
    op.step_mut(hiroute_domain::OperationStepKind::Activate)
        .terminal_result = Some(
        serde_json::to_string(&hiroute_domain::SettingsServiceCompletionV1 {
            schema: hiroute_domain::SETTINGS_SERVICE_COMPLETION_SCHEMA.into(),
            publication_revision: 1,
            publication_digest: CanonicalDigest::of_bytes(b"publication"),
            completed_effects_digest: CanonicalDigest::of_bytes(b"completed"),
        })
        .unwrap(),
    );
    control.save_operation_tail(&mut op).unwrap();
    assert!(!control.writer_recovery_required().unwrap());
    assert!(control.operation_is_current(&op).unwrap());

    let mut uncommitted = op.clone();
    uncommitted.safe_error_code = Some("UNCOMMITTED".into());
    assert!(!control.operation_is_current(&uncommitted).unwrap());
    control
        .connection
        .borrow()
        .execute(
            "INSERT INTO writer_claim(singleton, operation_id, admitted_at) VALUES(1, 'operation/other', unixepoch())",
            [],
        )
        .unwrap();
    assert!(!control.operation_is_current(&op).unwrap());
}

#[path = "journal_order_tests.rs"]
mod journal_order_tests;
