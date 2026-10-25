//! Exercise real journal/receipt transactions and protected files, without model calls.
use super::*;
use hiroute_domain::{OperationState, OperationStepKind, OperationStepStatus};

fn open(root: &std::path::Path) -> (ControlStore, ManagedArtifactStore) {
    (
        ControlStore::open(
            &crate::test_storage_authority(),
            root.join("control.db"),
            root.join("backups"),
        )
        .unwrap(),
        ManagedArtifactStore::open_registered(
            &crate::test_storage_authority(),
            root.join("files"),
            root.join("restore"),
            [],
        )
        .unwrap(),
    )
}

fn admit(
    control: &ControlStore,
    artifacts: &ManagedArtifactStore,
    transaction: AgentConnectionTransactionKindV1,
    key: &str,
) -> OperationV1 {
    let prototype = agent_transaction_plan(transaction, None);
    let target = prototype
        .external()
        .iter()
        .find(|i| i.kind() == hiroute_domain::OwnedEffectKind::AgentArtifact)
        .unwrap()
        .target();
    let plan = agent_transaction_plan_payload(
        transaction,
        artifacts.current_external_fingerprint(target).unwrap(),
        key,
    );
    let mut operation = agent_operation(plan, key);
    operation.expected_revisions = control.current_revisions(&operation.workspace_id).unwrap();
    let token = format!("artifact-{key}");
    control
        .grant_apply_capability(&token, &operation, i64::MAX)
        .unwrap();
    let auth = control
        .verify_apply_authorization(
            &ProtectedApplyCapability::new(token).unwrap(),
            &operation.workspace_id,
            &operation.idempotency.principal,
            &operation.idempotency.operation_kind,
            &operation.accepted_digest,
            &operation.expected_revisions,
        )
        .unwrap();
    assert_eq!(
        control.begin_operation(&operation, &auth).unwrap(),
        BeginOperationOutcome::Created
    );
    operation
}

fn stage(artifacts: &ManagedArtifactStore, operation: &mut OperationV1) {
    let effects = operation
        .plan
        .external()
        .iter()
        .filter(|i| i.kind() == hiroute_domain::OwnedEffectKind::AgentArtifact)
        .map(|intent| {
            artifacts
                .apply_artifact(&operation.operation_id, intent)
                .unwrap()
        })
        .collect::<Vec<_>>();
    operation
        .steps
        .iter_mut()
        .find(|s| s.kind == OperationStepKind::ApplyAgentArtifacts)
        .unwrap()
        .effects = effects;
}

fn finish(control: &ControlStore, artifacts: &ManagedArtifactStore, operation: &mut OperationV1) {
    for effect in &operation
        .steps
        .iter()
        .find(|s| s.kind == OperationStepKind::ApplyAgentArtifacts)
        .unwrap()
        .effects
    {
        artifacts.activate_artifact(effect).unwrap();
    }
    operation.state = OperationState::Succeeded;
    for step in &mut operation.steps {
        step.status = OperationStepStatus::Applied;
    }
    control.finish_operation(operation).unwrap();
    control.complete_operation_finalization(operation).unwrap();
}

fn complete(
    control: &ControlStore,
    artifacts: &ManagedArtifactStore,
    transaction: AgentConnectionTransactionKindV1,
    key: &str,
) -> OperationV1 {
    let mut operation = admit(control, artifacts, transaction, key);
    stage(artifacts, &mut operation);
    finish(control, artifacts, &mut operation);
    operation
}

#[test]
fn superseded_damage_is_outside_startup_but_strict_on_access() {
    let root = tempdir().unwrap();
    let (control, artifacts) = open(root.path());
    let old = complete(
        &control,
        &artifacts,
        AgentConnectionTransactionKindV1::Apply,
        "old",
    );
    let old_intent = old
        .plan
        .external()
        .iter()
        .find(|i| i.kind() == hiroute_domain::OwnedEffectKind::AgentArtifact)
        .unwrap();
    for n in 0..20 {
        complete(
            &control,
            &artifacts,
            AgentConnectionTransactionKindV1::Apply,
            &format!("replacement-{n}"),
        );
    }
    let dependencies = control.artifact_dependencies().unwrap();
    assert_eq!(
        dependencies.len(),
        1,
        "one current target, not 21 historical restore points"
    );
    fs::write(
        artifacts.marker_path(&old.operation_id, old_intent.effect_id()),
        b"damaged unrelated marker",
    )
    .unwrap();
    control.connection.borrow().execute("UPDATE operations SET operation_json='damaged unrelated history' WHERE operation_id=?1",[old.operation_id.as_str()]).unwrap();
    drop(artifacts);
    drop(control);
    // Negative control: the former production opener audited every retained marker.
    // The identical data still fails that explicit full audit.
    assert!(
        ManagedArtifactStore::open_with_external_targets(
            &crate::test_storage_authority(),
            root.path().join("files"),
            root.path().join("restore"),
            []
        )
        .is_err()
    );
    let (control, artifacts) = open(root.path());
    artifacts
        .validate_dependencies(&control.artifact_dependencies().unwrap())
        .unwrap();
    assert!(
        artifacts
            .observe_artifact(&old.operation_id, old_intent)
            .is_err()
    );
    assert!(control.load_operation(&old.operation_id).is_err());
    let current = control
        .current_agent_configuration_operations(&WorkspaceId::default())
        .unwrap();
    assert_eq!(current.len(), 1);
    assert_ne!(current[0].operation_id, old.operation_id);
    assert!(
        control
            .current_agent_configuration_operations(
                &WorkspaceId::parse("workspace/foreign").unwrap()
            )
            .unwrap()
            .is_empty()
    );
    control
        .connection
        .borrow()
        .execute(
            "DELETE FROM operation_steps WHERE operation_id=?1 AND step_kind='activate'",
            [current[0].operation_id.as_str()],
        )
        .unwrap();
    assert_eq!(
        control
            .current_agent_configuration_operations(&WorkspaceId::default())
            .unwrap()
            .len(),
        1,
        "optional completed service proof cannot disable current file/grant authority"
    );
    control
        .connection
        .borrow()
        .execute(
            "DELETE FROM operation_steps WHERE operation_id=?1 AND step_kind='apply_secrets'",
            [current[0].operation_id.as_str()],
        )
        .unwrap();
    assert!(
        control
            .current_agent_configuration_operations(&WorkspaceId::default())
            .is_err(),
        "active budget authority still requires its exact grant-step proof"
    );
}

#[test]
fn current_marker_backup_journal_and_coverage_damage_remain_fatal() {
    for damage in [
        "marker",
        "backup",
        "journal",
        "generation",
        "state",
        "identity-row",
        "file-step-copy",
        "file-step-row",
        "file-step-unfinished",
        "input",
        "head",
        "coverage",
        "identity",
    ] {
        let root = tempdir().unwrap();
        let (control, artifacts) = open(root.path());
        complete(
            &control,
            &artifacts,
            AgentConnectionTransactionKindV1::Apply,
            "first",
        );
        let current = complete(
            &control,
            &artifacts,
            AgentConnectionTransactionKindV1::Apply,
            "current",
        );
        let intent = current
            .plan
            .external()
            .iter()
            .find(|i| i.kind() == hiroute_domain::OwnedEffectKind::AgentArtifact)
            .unwrap();
        match damage {
            "marker" => {
                fs::remove_file(artifacts.marker_path(&current.operation_id, intent.effect_id()))
                    .unwrap()
            }
            "backup" => {
                let marker = artifacts
                    .load_marker(&current.operation_id, intent.effect_id())
                    .unwrap()
                    .unwrap();
                fs::write(
                    artifacts.restore_root.join(marker.backup_name.unwrap()),
                    b"corrupt",
                )
                .unwrap();
            }
            "journal" => {
                control
                    .connection
                    .borrow()
                    .execute(
                        "UPDATE operations SET operation_json='corrupt' WHERE operation_id=?1",
                        [current.operation_id.as_str()],
                    )
                    .unwrap();
            }
            "generation" => {
                control
                    .connection
                    .borrow()
                    .execute(
                        "UPDATE operations SET generation=generation+1 WHERE operation_id=?1",
                        [current.operation_id.as_str()],
                    )
                    .unwrap();
            }
            "state" => {
                control
                    .connection
                    .borrow()
                    .execute(
                        "UPDATE operations SET state='rolled_back' WHERE operation_id=?1",
                        [current.operation_id.as_str()],
                    )
                    .unwrap();
            }
            "identity-row" => {
                control
                    .connection
                    .borrow()
                    .execute(
                        "UPDATE operations SET principal='other-principal' WHERE operation_id=?1",
                        [current.operation_id.as_str()],
                    )
                    .unwrap();
            }
            "file-step-row" => {
                control.connection.borrow().execute("DELETE FROM operation_steps WHERE operation_id=?1 AND step_kind='apply_agent_artifacts'", [current.operation_id.as_str()]).unwrap();
            }
            "file-step-copy" => {
                control.connection.borrow().execute("UPDATE operation_steps SET step_json=json_set(step_json,'$.step.attempts',55) WHERE operation_id=?1 AND step_kind='apply_agent_artifacts'",[current.operation_id.as_str()]).unwrap();
            }
            "file-step-unfinished" => {
                control.connection.borrow().execute("UPDATE operation_steps SET state='pending',step_json=json_set(step_json,'$.step.status','pending') WHERE operation_id=?1 AND step_kind='apply_agent_artifacts'",[current.operation_id.as_str()]).unwrap();
                let step = current
                    .steps
                    .iter()
                    .find(|s| s.kind == OperationStepKind::ApplyAgentArtifacts)
                    .unwrap()
                    .sequence;
                control.connection.borrow().execute("UPDATE operations SET operation_json=json_set(operation_json,?2,'pending') WHERE operation_id=?1",rusqlite::params![current.operation_id.as_str(),format!("$.steps[{step}].status")]).unwrap();
            }
            "input" => {
                control
                    .connection
                    .borrow()
                    .execute(
                        "DELETE FROM operation_payloads WHERE operation_id=?1",
                        [current.operation_id.as_str()],
                    )
                    .unwrap();
            }
            "head" => {
                control
                    .connection
                    .borrow()
                    .execute("DELETE FROM artifact_heads", [])
                    .unwrap();
            }
            "coverage" => {
                control
                    .connection
                    .borrow()
                    .execute("DELETE FROM artifact_head_coverage", [])
                    .unwrap();
            }
            "identity" => {
                control
                    .connection
                    .borrow()
                    .execute(
                        "UPDATE artifact_head_coverage SET store_uuid='other-store'",
                        [],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let admitted = control
            .artifact_dependencies()
            .ok()
            .is_some_and(|d| artifacts.validate_dependencies(&d).is_ok());
        assert!(!admitted, "{damage} must not become an empty recovery set");
    }
}

#[test]
fn unfinished_effect_and_admission_basis_are_strict() {
    for damage in ["marker", "transition"] {
        let root = tempdir().unwrap();
        let (control, artifacts) = open(root.path());
        let mut pending = admit(
            &control,
            &artifacts,
            AgentConnectionTransactionKindV1::Apply,
            "pending",
        );
        // Not-yet-staged absence is legitimate; no invented marker is required.
        artifacts
            .validate_dependencies(&control.artifact_dependencies().unwrap())
            .unwrap();
        stage(&artifacts, &mut pending);
        control.save_operation(&mut pending).unwrap();
        if damage == "marker" {
            let effect = &pending
                .steps
                .iter()
                .find(|s| s.kind == OperationStepKind::ApplyAgentArtifacts)
                .unwrap()
                .effects[0];
            fs::write(
                artifacts.marker_path(&pending.operation_id, &effect.effect_id),
                b"corrupt",
            )
            .unwrap();
        } else {
            control
                .connection
                .borrow()
                .execute("DELETE FROM artifact_head_transitions", [])
                .unwrap();
        }
        let admitted = control
            .artifact_dependencies()
            .ok()
            .is_some_and(|d| artifacts.validate_dependencies(&d).is_ok());
        assert!(!admitted, "{damage}");
    }
}

#[test]
fn owner_update_and_terminal_release_share_transaction_and_rollback_preserves_owner() {
    let root = tempdir().unwrap();
    let (control, artifacts) = open(root.path());
    let original = complete(
        &control,
        &artifacts,
        AgentConnectionTransactionKindV1::Apply,
        "original",
    );
    let mut pending = admit(
        &control,
        &artifacts,
        AgentConnectionTransactionKindV1::Apply,
        "rollback",
    );
    stage(&artifacts, &mut pending);
    for effect in &pending
        .steps
        .iter()
        .find(|s| s.kind == OperationStepKind::ApplyAgentArtifacts)
        .unwrap()
        .effects
    {
        artifacts.compensate_artifact(effect).unwrap();
    }
    pending.state = OperationState::RolledBack;
    for step in &mut pending.steps {
        step.status = OperationStepStatus::Compensated;
    }
    control.finish_operation(&mut pending).unwrap();
    control.complete_operation_finalization(&pending).unwrap();
    let deps = control.artifact_dependencies().unwrap();
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].operation, original.operation_id);
    artifacts.validate_dependencies(&deps).unwrap();
    let mut next = admit(
        &control,
        &artifacts,
        AgentConnectionTransactionKindV1::Apply,
        "next",
    );
    stage(&artifacts, &mut next);
    next.state = OperationState::Succeeded;
    for step in &mut next.steps {
        step.status = OperationStepStatus::Applied;
    }
    control.connection.borrow().execute_batch("CREATE TRIGGER refuse_terminal BEFORE UPDATE OF state ON operations WHEN NEW.state='succeeded' BEGIN SELECT RAISE(ABORT,'injected after head update'); END").unwrap();
    assert!(control.finish_operation(&mut next).is_err());
    assert_eq!(
        control
            .connection
            .borrow()
            .query_row("SELECT owner_operation_id FROM artifact_heads", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
        original.operation_id.to_string()
    );
    assert_eq!(
        control
            .connection
            .borrow()
            .query_row("SELECT operation_id FROM writer_claim", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        next.operation_id.to_string()
    );
    control
        .connection
        .borrow()
        .execute_batch("DROP TRIGGER refuse_terminal")
        .unwrap();
    finish(&control, &artifacts, &mut next);
    control.complete_operation_finalization(&next).unwrap();
    assert_eq!(
        control.artifact_dependencies().unwrap()[0].operation,
        next.operation_id
    );
}

#[test]
fn successful_restore_retires_only_the_admitted_current_owner() {
    let root = tempdir().unwrap();
    let (control, artifacts) = open(root.path());
    complete(
        &control,
        &artifacts,
        AgentConnectionTransactionKindV1::Apply,
        "first",
    );
    let current = complete(
        &control,
        &artifacts,
        AgentConnectionTransactionKindV1::Apply,
        "replacement",
    );
    let mut restore = admit(
        &control,
        &artifacts,
        AgentConnectionTransactionKindV1::Restore,
        "restore",
    );
    let basis: String = control
        .connection
        .borrow()
        .query_row(
            "SELECT before_json FROM artifact_head_transitions WHERE operation_id=?1",
            [restore.operation_id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(basis.contains(current.operation_id.as_str()));
    stage(&artifacts, &mut restore);
    finish(&control, &artifacts, &mut restore);
    assert!(control.artifact_dependencies().unwrap().is_empty());
    assert_eq!(
        control
            .connection
            .borrow()
            .query_row("SELECT count(*) FROM artifact_head_transitions", [], |r| {
                r.get::<_, usize>(0)
            })
            .unwrap(),
        0
    );
}

#[test]
fn missing_success_receipt_cannot_replace_a_current_owner() {
    let root = tempdir().unwrap();
    let (control, artifacts) = open(root.path());
    let current = complete(
        &control,
        &artifacts,
        AgentConnectionTransactionKindV1::Apply,
        "owned",
    );
    let mut next = admit(
        &control,
        &artifacts,
        AgentConnectionTransactionKindV1::Restore,
        "no-receipt",
    );
    next.state = OperationState::Succeeded;
    for step in &mut next.steps {
        step.status = OperationStepStatus::Applied;
    }
    assert!(control.finish_operation(&mut next).is_err());
    assert_eq!(
        control
            .connection
            .borrow()
            .query_row("SELECT owner_operation_id FROM artifact_heads", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
        current.operation_id.to_string()
    );
    assert!(
        control
            .load_operation(&next.operation_id)
            .unwrap()
            .unwrap()
            .state
            != OperationState::Succeeded
    );
}

#[test]
fn unrelated_completed_step_proof_is_optional_but_pending_journal_stays_strict() {
    let root = tempdir().unwrap();
    let (control, artifacts) = open(root.path());
    let current = complete(
        &control,
        &artifacts,
        AgentConnectionTransactionKindV1::Apply,
        "owned",
    );
    // The file step and its protected material remain exact. Another completed
    // step's two copies disagree, as happens when optional attribution proof is lost.
    control.connection.borrow().execute("UPDATE operations SET operation_json=json_set(operation_json,'$.steps[1].attempts',7) WHERE operation_id=?1",[current.operation_id.as_str()]).unwrap();
    let changed = control
        .load_operation(&current.operation_id)
        .unwrap()
        .unwrap();
    assert!(
        !control.operation_is_current(&changed).unwrap(),
        "execution must not accept this whole journal"
    );
    artifacts
        .validate_dependencies(&control.artifact_dependencies().unwrap())
        .unwrap();
    // Both admission and terminal owner replacement authenticate the previous
    // current file without borrowing unrelated completed-step proof.
    let mut replacement = admit(
        &control,
        &artifacts,
        AgentConnectionTransactionKindV1::Apply,
        "replacement",
    );
    artifacts
        .validate_dependencies(&control.artifact_dependencies().unwrap())
        .unwrap();
    stage(&artifacts, &mut replacement);
    finish(&control, &artifacts, &mut replacement);
    assert_eq!(
        control.artifact_dependencies().unwrap()[0].operation,
        replacement.operation_id
    );
    let pending = admit(
        &control,
        &artifacts,
        AgentConnectionTransactionKindV1::Apply,
        "pending",
    );
    control.connection.borrow().execute("UPDATE operations SET operation_json=json_set(operation_json,'$.steps[1].attempts',7) WHERE operation_id=?1",[pending.operation_id.as_str()]).unwrap();
    assert!(
        control.artifact_dependencies().is_err(),
        "unresolved operation requires every recovery step"
    );
}

#[test]
fn direct_restore_input_uses_exact_file_proof_not_other_completed_steps() {
    let root = tempdir().unwrap();
    let (control, artifacts) = open(root.path());
    let current = complete(
        &control,
        &artifacts,
        AgentConnectionTransactionKindV1::Apply,
        "owned",
    );
    control.connection.borrow().execute("UPDATE operations SET operation_json=json_set(operation_json,'$.steps[1].attempts',7) WHERE operation_id=?1",[current.operation_id.as_str()]).unwrap();
    let pending = admit(
        &control,
        &artifacts,
        AgentConnectionTransactionKindV1::Restore,
        "restore",
    );
    let envelope = AgentConnectionControlIntentV1::from_registered_planner(
        AgentConnectionTransactionKindV1::Restore,
        AgentConnectionTransactionSubjectV1::from_registered_profile(
            "agent.codex",
            "default",
            "codex.profile.v1",
        )
        .unwrap(),
        pending.plan.spec(),
        &json!({"connection_revision":4,"desired_digest":"connection.digest.v4"}),
    )
    .unwrap();
    // Exercise the dependency selector with the native planner's direct-reference
    // envelope. Native rendering/backup content is covered by daemon process cases.
    let intent = ExternalEffectIntentV1::from_agent_connection_planner(&envelope,
        AgentConnectionEffectRoleV1::ManagedConfiguration, None,
        &json!({"schema":"hiroute.settings-codex-model-file/v1", "change":{"action":"restore","original_operation":current.operation_id}}), 0o640).unwrap();
    let mut selected = BTreeMap::new();
    crate::control::artifact_dependencies::add_predecessor(
        &control,
        &mut selected,
        &pending,
        &intent,
    )
    .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(
        selected.values().next().unwrap().operation,
        current.operation_id
    );
    artifacts
        .validate_dependencies(&selected.into_values().collect::<Vec<_>>())
        .unwrap();
    control.connection.borrow().execute("UPDATE operations SET operation_json=json_set(operation_json,'$.steps[4].effects',json('[]')) WHERE operation_id=?1",[current.operation_id.as_str()]).unwrap();
    assert!(
        crate::control::artifact_dependencies::add_predecessor(
            &control,
            &mut BTreeMap::new(),
            &pending,
            &intent
        )
        .is_err(),
        "a damaged required file proof still refuses restore"
    );
}

#[test]
fn missing_optional_completed_row_does_not_pin_file_but_pending_rows_are_required() {
    let root = tempdir().unwrap();
    let (control, artifacts) = open(root.path());
    let current = complete(
        &control,
        &artifacts,
        AgentConnectionTransactionKindV1::Apply,
        "owned",
    );
    control
        .connection
        .borrow()
        .execute(
            "DELETE FROM operation_steps WHERE operation_id=?1 AND step_kind='apply_secrets'",
            [current.operation_id.as_str()],
        )
        .unwrap();
    artifacts
        .validate_dependencies(&control.artifact_dependencies().unwrap())
        .unwrap();
    let mut replacement = admit(
        &control,
        &artifacts,
        AgentConnectionTransactionKindV1::Apply,
        "replacement",
    );
    stage(&artifacts, &mut replacement);
    finish(&control, &artifacts, &mut replacement);
    let pending = admit(
        &control,
        &artifacts,
        AgentConnectionTransactionKindV1::Apply,
        "pending",
    );
    control
        .connection
        .borrow()
        .execute(
            "DELETE FROM operation_steps WHERE operation_id=?1 AND step_kind='apply_secrets'",
            [pending.operation_id.as_str()],
        )
        .unwrap();
    assert!(
        control.artifact_dependencies().is_err(),
        "all pending step rows are required"
    );
}
