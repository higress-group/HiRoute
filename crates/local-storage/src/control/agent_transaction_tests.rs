use std::collections::BTreeMap;
use std::fs;

use crate::test_tempdir as tempdir;
use hiroute_domain::{
    AgentConnectionControlIntentV1, AgentConnectionEffectRoleV1, AgentConnectionTransactionKindV1,
    AgentConnectionTransactionSubjectV1, BeginOperationOutcome, CanonicalDigest, ChangeSpecV1,
    CompensationOutcome, ControlRepositoryPort, EffectReconciliation, ExternalEffectIntentV1,
    IdempotencyScopeV1, OperationId, OperationV1, ProtectedApplyCapability, RevisionSetV1,
    TransactionPlanV1, WorkspaceId,
};
use serde_json::json;

use super::{ArtifactBackupAad, ControlStore, ManagedArtifactStore};

fn rewrite_marker_as_legacy(
    store: &ManagedArtifactStore,
    operation: &OperationId,
    effect_id: &str,
    schema: &str,
    backup_aad: ArtifactBackupAad,
) {
    let mut marker = store.load_marker(operation, effect_id).unwrap().unwrap();
    let before = store.validated_backup(&marker).unwrap().unwrap();
    marker.schema = schema.to_owned();
    marker.backup_aad = backup_aad;
    let sealed = store.seal_backup(&marker, &before).unwrap();
    fs::write(
        store
            .restore_root
            .join(marker.backup_name.as_ref().unwrap()),
        sealed,
    )
    .unwrap();
    let mut encoded = serde_json::to_value(&marker).unwrap();
    encoded.as_object_mut().unwrap().remove("backup_aad");
    fs::write(
        store.marker_path(operation, effect_id),
        serde_json::to_vec(&encoded).unwrap(),
    )
    .unwrap();
}

#[test]
#[cfg(unix)]
fn native_absence_effect_reopens_and_protected_record_rejects_rebinding() {
    use hiroute_domain::NativeAgentArtifactPort;
    use std::os::unix::fs::PermissionsExt;
    let directory = tempdir().unwrap();
    let root = directory.path().join("artifacts");
    let restore = directory.path().join("restore");
    let store =
        ManagedArtifactStore::open(&crate::test_storage_authority(), &root, &restore).unwrap();
    let prototype = agent_transaction_plan(AgentConnectionTransactionKindV1::Restore, None)
        .external()
        .iter()
        .find(|e| e.kind() == hiroute_domain::OwnedEffectKind::AgentArtifact)
        .unwrap()
        .clone();
    let target = root.join(prototype.target());
    crate::test_create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"owned Skill").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
    let plan = agent_transaction_plan(
        AgentConnectionTransactionKindV1::Restore,
        store
            .current_external_fingerprint(prototype.target())
            .unwrap(),
    );
    let intent = plan
        .external()
        .iter()
        .find(|e| e.kind() == hiroute_domain::OwnedEffectKind::AgentArtifact)
        .unwrap();
    let operation = OperationId::parse("op_99999999999999999999999999999999").unwrap();
    store
        .save_native_restore(&operation, intent, b"private-original-field")
        .unwrap();
    let effect = store
        .stage_native_target(&operation, intent, None, false)
        .unwrap();
    assert!(target.exists(), "stage does not activate deletion");
    drop(store);
    let reopened =
        ManagedArtifactStore::open(&crate::test_storage_authority(), &root, &restore).unwrap();
    assert_eq!(
        reopened
            .load_native_restore(&operation, intent)
            .unwrap()
            .unwrap()
            .as_slice(),
        b"private-original-field"
    );
    assert!(
        reopened
            .save_native_restore(&operation, intent, b"different")
            .is_err()
    );
    for entry in fs::read_dir(&restore).unwrap() {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        assert!(
            !bytes
                .windows(b"private-original-field".len())
                .any(|w| w == b"private-original-field")
        );
    }
    reopened.activate_artifact(&effect).unwrap();
    assert!(!target.exists());
    assert!(matches!(
        reopened.observe_artifact(&operation, intent).unwrap(),
        EffectReconciliation::Applied(_)
    ));
    reopened.activate_artifact(&effect).unwrap();
    assert_eq!(
        reopened.compensate_artifact(&effect).unwrap(),
        CompensationOutcome::Compensated
    );
    assert_eq!(fs::read(&target).unwrap(), b"owned Skill");
}

#[test]
#[cfg(unix)]
fn native_read_rejects_links_and_oversized_files_without_following() {
    use hiroute_domain::NativeAgentArtifactPort;
    use std::os::unix::fs::{PermissionsExt, symlink};
    let directory = tempdir().unwrap();
    let store = ManagedArtifactStore::open(
        &crate::test_storage_authority(),
        directory.path().join("artifacts"),
        directory.path().join("restore"),
    )
    .unwrap();
    let root = directory.path().join("artifacts");
    let target = root.join("native.toml");
    fs::write(&target, b"private").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    fs::hard_link(&target, root.join("other")).unwrap();
    assert!(store.read_native_target("native.toml").is_err());
    assert!(store.read_private_native_target("native.toml").is_err());
    fs::remove_file(&target).unwrap();
    symlink(root.join("other"), &target).unwrap();
    assert!(store.read_native_target("native.toml").is_err());
    assert!(store.read_private_native_target("native.toml").is_err());
    fs::remove_file(&target).unwrap();
    let file = fs::File::create(&target).unwrap();
    file.set_len(1024 * 1024 + 1).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(store.read_native_target("native.toml").is_err());
    assert!(store.read_private_native_target("native.toml").is_err());
}

#[test]
#[cfg(unix)]
fn native_and_private_reads_accept_accessible_modes_without_chmod() {
    use hiroute_domain::NativeAgentArtifactPort;
    use std::os::unix::fs::PermissionsExt;
    let directory = tempdir().unwrap();
    let path = directory.path().join("settings.json");
    let store = ManagedArtifactStore::open_with_external_target(
        &crate::test_storage_authority(),
        directory.path().join("artifacts"),
        directory.path().join("restore"),
        "native-settings",
        &path,
    )
    .unwrap();
    assert!(
        store
            .read_private_native_target("native-settings")
            .unwrap()
            .is_none()
    );
    let bytes = br#"{"providers":[{"apiKey":"fixture-local-bearer"}]}"#;
    fs::write(&path, bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        store
            .read_private_native_target("native-settings")
            .unwrap()
            .unwrap()
            .as_slice(),
        bytes
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        store
            .read_private_native_target("native-settings")
            .unwrap()
            .unwrap()
            .as_slice(),
        bytes
    );
    assert_eq!(
        store
            .read_native_target("native-settings")
            .unwrap()
            .unwrap()
            .as_slice(),
        bytes
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
        0o644
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

fn agent_transaction_plan(
    transaction: AgentConnectionTransactionKindV1,
    before: Option<CanonicalDigest>,
) -> TransactionPlanV1 {
    let spec = ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: transaction.command_id().to_owned(),
        resource_id: Some("agent-connection/codex-default".to_owned()),
        desired_state: match transaction {
            AgentConnectionTransactionKindV1::Settings => {
                panic!("legacy fixture does not model independent settings")
            }
            AgentConnectionTransactionKindV1::Apply => json!({
                "agent_id": "agent.codex",
                "profile_id": "default",
                "default_agent_plan_id": "plan.primary"
            }),
            AgentConnectionTransactionKindV1::Restore => json!({
                "agent_id": "agent.codex",
                "profile_id": "default",
                "restore_point_ref": "restore.codex.1"
            }),
        },
    };
    let subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
        "agent.codex",
        "default",
        "codex.profile.v1",
    )
    .unwrap();
    let control = AgentConnectionControlIntentV1::from_registered_planner(
        transaction,
        subject,
        &spec,
        &json!({"connection_revision": 4, "desired_digest": "connection.digest.v4"}),
    )
    .unwrap();
    let external = [
        AgentConnectionEffectRoleV1::GrantScopedPublication,
        AgentConnectionEffectRoleV1::ManagedConfiguration,
    ]
    .into_iter()
    .map(|role| {
        ExternalEffectIntentV1::from_agent_connection_planner(
            &control,
            role,
            if role == AgentConnectionEffectRoleV1::ManagedConfiguration {
                before.clone()
            } else {
                None
            },
            &json!({"role": format!("{role:?}"), "content_digest": "sha256:fixture"}),
            if role == AgentConnectionEffectRoleV1::ManagedConfiguration {
                0o640
            } else {
                0o644
            },
        )
        .unwrap()
    })
    .collect();
    TransactionPlanV1::from_agent_connection_planner(spec, control, external).unwrap()
}

fn agent_operation(plan: TransactionPlanV1, key: &str) -> OperationV1 {
    let workspace = WorkspaceId::default();
    let request_digest = CanonicalDigest::of_bytes(format!("agent-request-{key}").as_bytes());
    let accepted_digest = CanonicalDigest::of_bytes(b"agent-change");
    let scope =
        IdempotencyScopeV1::new("interactive-user", "ApplyAgentConnectionChange", key).unwrap();
    OperationV1::new(
        OperationId::derive(&workspace, &scope, &request_digest),
        workspace,
        scope,
        request_digest,
        accepted_digest,
        RevisionSetV1 {
            target: 0,
            dependencies: BTreeMap::new(),
        },
        plan,
    )
    .unwrap()
}

#[test]
fn succeeded_agent_read_projection_checks_sql_identity_and_both_step_copies() {
    let directory = tempdir().unwrap();
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        directory.path().join("control.db"),
        directory.path().join("backups"),
    )
    .unwrap();
    let mut operation = agent_operation(
        agent_transaction_plan(AgentConnectionTransactionKindV1::Apply, None),
        "read-facts",
    );
    control
        .grant_apply_capability("read-facts-capability", &operation, i64::MAX)
        .unwrap();
    let authorization = control
        .verify_apply_authorization(
            &ProtectedApplyCapability::new("read-facts-capability".into()).unwrap(),
            &operation.workspace_id,
            &operation.idempotency.principal,
            &operation.idempotency.operation_kind,
            &operation.accepted_digest,
            &operation.expected_revisions,
        )
        .unwrap();
    control.begin_operation(&operation, &authorization).unwrap();
    operation.state = hiroute_domain::OperationState::Succeeded;
    for step in &mut operation.steps {
        step.status = hiroute_domain::OperationStepStatus::Applied;
    }
    control.save_operation(&mut operation).unwrap();
    let facts = control
        .succeeded_agent_operations_for_kind(&operation.workspace_id, "ApplyAgentConnectionChange")
        .unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].operation_id, operation.operation_id);
    assert_eq!(facts[0].plan.spec(), operation.plan.spec());
    assert_eq!(facts[0].plan.external(), operation.plan.external());
    control.connection.borrow().execute("UPDATE operation_steps SET step_json=json_set(step_json,'$.step.attempts',99) WHERE step_no=2", []).unwrap();
    assert_eq!(
        control
            .succeeded_agent_operations_for_kind(
                &operation.workspace_id,
                "ApplyAgentConnectionChange"
            )
            .unwrap_err()
            .code,
        hiroute_domain::PortErrorCode::Corrupt
    );
    control.connection.borrow().execute("UPDATE operation_steps SET step_json=json_set(step_json,'$.step.attempts',0) WHERE step_no=2", []).unwrap();
    assert!(
        control
            .succeeded_agent_operations_for_kind(
                &operation.workspace_id,
                "ApplyAgentConnectionChange"
            )
            .is_ok()
    );
    control
        .connection
        .borrow()
        .execute("UPDATE operations SET generation=generation+1", [])
        .unwrap();
    assert_eq!(
        control
            .succeeded_agent_operations_for_kind(
                &operation.workspace_id,
                "ApplyAgentConnectionChange"
            )
            .unwrap_err()
            .code,
        hiroute_domain::PortErrorCode::Corrupt
    );
}

#[test]
fn agent_connection_transaction_journal_reopens_with_exact_typed_plan() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("data");
    let database = root.join("control.db");
    let backups = root.join("backups");
    let control =
        ControlStore::open(&crate::test_storage_authority(), &database, &backups).unwrap();
    let operation = agent_operation(
        agent_transaction_plan(AgentConnectionTransactionKindV1::Apply, None),
        "agent-journal",
    );
    control
        .grant_apply_capability("agent-capability", &operation, i64::MAX)
        .unwrap();
    let authorization = control
        .verify_apply_authorization(
            &ProtectedApplyCapability::new("agent-capability".to_owned()).unwrap(),
            &operation.workspace_id,
            &operation.idempotency.principal,
            &operation.idempotency.operation_kind,
            &operation.accepted_digest,
            &operation.expected_revisions,
        )
        .unwrap();
    assert_eq!(
        control.begin_operation(&operation, &authorization).unwrap(),
        BeginOperationOutcome::Created
    );
    drop(control);

    let reopened =
        ControlStore::open(&crate::test_storage_authority(), &database, &backups).unwrap();
    assert!(reopened.writer_recovery_required().unwrap());
    assert_eq!(
        reopened
            .load_operation(&operation.operation_id)
            .unwrap()
            .unwrap(),
        operation
    );
}

#[test]
fn agent_connection_transaction_artifact_restart_preserves_compensation_ownership() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().unwrap();
    let root = directory.path().join("artifacts");
    let restore = directory.path().join("restore");
    let store =
        ManagedArtifactStore::open(&crate::test_storage_authority(), &root, &restore).unwrap();
    let prototype = agent_transaction_plan(AgentConnectionTransactionKindV1::Restore, None)
        .external()
        .iter()
        .find(|effect| effect.kind() == hiroute_domain::OwnedEffectKind::AgentArtifact)
        .unwrap()
        .clone();
    let target = root.join(prototype.target());
    crate::test_create_dir_all(target.parent().unwrap()).unwrap();
    fs::set_permissions(target.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(&target, b"before").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
    let before = store
        .current_external_fingerprint(prototype.target())
        .unwrap();
    let plan = agent_transaction_plan(AgentConnectionTransactionKindV1::Restore, before);
    let intent = plan
        .external()
        .iter()
        .find(|effect| effect.kind() == hiroute_domain::OwnedEffectKind::AgentArtifact)
        .unwrap()
        .clone();
    let operation = OperationId::parse("op_77777777777777777777777777777777").unwrap();
    let effect = store.apply_artifact(&operation, &intent).unwrap();
    drop(store);

    let reopened =
        ManagedArtifactStore::open(&crate::test_storage_authority(), &root, &restore).unwrap();
    assert!(matches!(
        reopened.observe_artifact(&operation, &intent).unwrap(),
        EffectReconciliation::Staged(_)
    ));
    reopened.activate_artifact(&effect).unwrap();
    drop(reopened);

    let reopened =
        ManagedArtifactStore::open(&crate::test_storage_authority(), &root, &restore).unwrap();
    assert!(matches!(
        reopened.observe_artifact(&operation, &intent).unwrap(),
        EffectReconciliation::Applied(_)
    ));
    assert_eq!(
        reopened.compensate_artifact(&effect).unwrap(),
        CompensationOutcome::Compensated
    );
    assert_eq!(fs::read(&target).unwrap(), b"before");
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o644
    );

    let second_operation = OperationId::parse("op_88888888888888888888888888888888").unwrap();
    let before = reopened
        .current_external_fingerprint(intent.target())
        .unwrap();
    let second_plan = agent_transaction_plan(AgentConnectionTransactionKindV1::Restore, before);
    let second_intent = second_plan
        .external()
        .iter()
        .find(|candidate| candidate.kind() == hiroute_domain::OwnedEffectKind::AgentArtifact)
        .unwrap();
    let second_effect = reopened
        .apply_artifact(&second_operation, second_intent)
        .unwrap();
    reopened.activate_artifact(&second_effect).unwrap();
    fs::write(&target, b"user-edit").unwrap();
    assert!(matches!(
        reopened
            .observe_artifact(&second_operation, second_intent)
            .unwrap(),
        EffectReconciliation::OwnershipLost(_)
    ));
    assert_eq!(
        reopened.compensate_artifact(&second_effect).unwrap(),
        CompensationOutcome::OwnershipLost
    );
    assert_eq!(fs::read(&target).unwrap(), b"user-edit");
}

#[test]
#[cfg(unix)]
fn managed_artifact_v2_backup_migrates_to_v4_and_restores_exact_bytes() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().unwrap();
    let root = directory.path().join("artifacts");
    let restore = directory.path().join("restore");
    let store =
        ManagedArtifactStore::open(&crate::test_storage_authority(), &root, &restore).unwrap();
    let prototype = agent_transaction_plan(AgentConnectionTransactionKindV1::Restore, None)
        .external()
        .iter()
        .find(|effect| effect.kind() == hiroute_domain::OwnedEffectKind::AgentArtifact)
        .unwrap()
        .clone();
    let target = root.join(prototype.target());
    crate::test_create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"v2-before").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
    let plan = agent_transaction_plan(
        AgentConnectionTransactionKindV1::Restore,
        store
            .current_external_fingerprint(prototype.target())
            .unwrap(),
    );
    let intent = plan
        .external()
        .iter()
        .find(|effect| effect.kind() == hiroute_domain::OwnedEffectKind::AgentArtifact)
        .unwrap();
    let operation = OperationId::parse("op_12121212121212121212121212121212").unwrap();
    let effect = store.apply_artifact(&operation, intent).unwrap();
    rewrite_marker_as_legacy(
        &store,
        &operation,
        intent.effect_id(),
        "hiroute.managed-artifact-marker/v2",
        ArtifactBackupAad::LegacyV2,
    );
    drop(store);

    let reopened =
        ManagedArtifactStore::open(&crate::test_storage_authority(), &root, &restore).unwrap();
    let marker = reopened
        .load_marker(&operation, intent.effect_id())
        .unwrap()
        .unwrap();
    assert_eq!(marker.schema, "hiroute.managed-artifact-marker/v4");
    assert_eq!(marker.backup_aad, ArtifactBackupAad::LegacyV2);
    reopened.activate_artifact(&effect).unwrap();
    assert_eq!(
        reopened.compensate_artifact(&effect).unwrap(),
        CompensationOutcome::Compensated
    );
    assert_eq!(fs::read(target).unwrap(), b"v2-before");
}

#[test]
#[cfg(unix)]
fn managed_delete_v3_backup_migrates_to_v4_and_restores_exact_bytes() {
    use hiroute_domain::NativeAgentArtifactPort;
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().unwrap();
    let root = directory.path().join("artifacts");
    let restore = directory.path().join("restore");
    let store =
        ManagedArtifactStore::open(&crate::test_storage_authority(), &root, &restore).unwrap();
    let prototype = agent_transaction_plan(AgentConnectionTransactionKindV1::Restore, None)
        .external()
        .iter()
        .find(|effect| effect.kind() == hiroute_domain::OwnedEffectKind::AgentArtifact)
        .unwrap()
        .clone();
    let target = root.join(prototype.target());
    crate::test_create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"v3-delete-before").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
    let plan = agent_transaction_plan(
        AgentConnectionTransactionKindV1::Restore,
        store
            .current_external_fingerprint(prototype.target())
            .unwrap(),
    );
    let intent = plan
        .external()
        .iter()
        .find(|effect| effect.kind() == hiroute_domain::OwnedEffectKind::AgentArtifact)
        .unwrap();
    let operation = OperationId::parse("op_34343434343434343434343434343434").unwrap();
    store
        .save_native_restore(&operation, intent, b"protected-fields")
        .unwrap();
    let effect = store
        .stage_native_target(&operation, intent, None, false)
        .unwrap();
    rewrite_marker_as_legacy(
        &store,
        &operation,
        intent.effect_id(),
        "hiroute.managed-artifact-marker/v3",
        ArtifactBackupAad::LegacyV3,
    );
    drop(store);

    let reopened =
        ManagedArtifactStore::open(&crate::test_storage_authority(), &root, &restore).unwrap();
    let marker = reopened
        .load_marker(&operation, intent.effect_id())
        .unwrap()
        .unwrap();
    assert_eq!(marker.schema, "hiroute.managed-artifact-marker/v4");
    assert_eq!(marker.backup_aad, ArtifactBackupAad::LegacyV3);
    reopened.activate_artifact(&effect).unwrap();
    assert!(!target.exists());
    assert_eq!(
        reopened.compensate_artifact(&effect).unwrap(),
        CompensationOutcome::Compensated
    );
    assert_eq!(fs::read(target).unwrap(), b"v3-delete-before");
}

#[path = "../agents/collaboration_store_tests.rs"]
mod collaboration_store_tests;

#[path = "../agents/plan_references_tests.rs"]
mod plan_references_tests;

#[path = "delegation_tests.rs"]
mod delegation_tests;
