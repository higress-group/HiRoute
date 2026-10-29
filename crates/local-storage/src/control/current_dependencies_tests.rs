//! Real journal admission/finalization transactions and damage controls.
use super::*;
use crate::control::current_dependencies::validate_recovery_members;
use hiroute_domain::{OperationState, OperationStepStatus};

fn admitted(control: &ControlStore, key: &str) -> OperationV1 {
    let op = operation(&WorkspaceId::default(), key, 0);
    let authorization = authorize(control, &op, key, i64::MAX);
    control.begin_operation(&op, &authorization).unwrap();
    op
}

#[test]
fn other_recovery_reads_validate_current_without_redecoding_it() {
    let root = tempdir().unwrap();
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        root.path().join("control.db"),
        root.path().join("backups"),
    )
    .unwrap();
    let op = admitted(&control, "current-recovery-read");
    let count = || crate::control::recovery_reads::DECODE_COUNT.with(std::cell::Cell::get);
    let before = count();
    assert!(
        control
            .other_recoverable_operations(&op)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        count(),
        before,
        "the current journal must not be reconstructed and then discarded"
    );
    assert_eq!(control.recoverable_operations().unwrap(), vec![op.clone()]);
    assert_eq!(
        count(),
        before + 1,
        "independent recovery still fully decodes the writer"
    );
    let mut stale = op.clone();
    stale.generation += 1;
    assert!(control.other_recoverable_operations(&stale).is_err());
    control
        .connection
        .borrow()
        .execute(
            "DELETE FROM operation_steps WHERE operation_id=?1 AND step_no=0",
            [op.operation_id.as_str()],
        )
        .unwrap();
    assert!(
        control.other_recoverable_operations(&op).is_err(),
        "even the excluded current journal must retain every indexed step"
    );
}

#[test]
fn other_recovery_reads_preserve_parked_members_and_reject_damage() {
    let root = tempdir().unwrap();
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        root.path().join("control.db"),
        root.path().join("backups"),
    )
    .unwrap();
    let mut parked = admitted(&control, "parked-before-current");
    parked.state = OperationState::Activating;
    parked
        .step_mut(hiroute_domain::OperationStepKind::Activate)
        .terminal_result = Some(
        serde_json::to_string(&hiroute_domain::SettingsServiceCompletionV1 {
            schema: hiroute_domain::SETTINGS_SERVICE_COMPLETION_SCHEMA.into(),
            publication_revision: 1,
            publication_digest: CanonicalDigest::of_bytes(b"publication"),
            completed_effects_digest: CanonicalDigest::of_bytes(b"completed"),
        })
        .unwrap(),
    );
    control.save_operation_tail(&mut parked).unwrap();
    let current = admitted(&control, "new-current");
    assert_eq!(
        control.other_recoverable_operations(&current).unwrap(),
        vec![parked.clone()]
    );
    control
        .connection
        .borrow()
        .execute(
            "UPDATE operations SET operation_json='corrupt pending journal' WHERE operation_id=?1",
            [parked.operation_id.as_str()],
        )
        .unwrap();
    assert!(control.other_recoverable_operations(&current).is_err());
}

#[test]
fn current_recovery_exclusion_never_bypasses_membership_coverage() {
    let root = tempdir().unwrap();
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        root.path().join("control.db"),
        root.path().join("backups"),
    )
    .unwrap();
    let current = admitted(&control, "current-coverage");
    control
        .connection
        .borrow()
        .execute("DROP TRIGGER operation_recovery_insert", [])
        .unwrap();
    assert!(control.other_recoverable_operations(&current).is_err());
}

#[test]
fn terminal_journal_remains_pending_until_real_finalizer_acknowledges_it() {
    let root = tempdir().unwrap();
    let path = root.path().join("control.db");
    let backup = root.path().join("backups");
    let control = ControlStore::open(&crate::test_storage_authority(), &path, &backup).unwrap();
    let mut op = admitted(&control, "pending-terminal");
    assert_eq!(
        validate_recovery_members(&control.connection.borrow()).unwrap(),
        vec![op.operation_id.to_string()]
    );
    // This is a storage transaction test; it does not pretend the external effects ran.
    op.state = OperationState::RolledBack;
    for step in &mut op.steps {
        step.status = OperationStepStatus::Compensated;
    }
    control.finish_operation(&mut op).unwrap();
    assert_eq!(
        control.pending_finalization_operations().unwrap(),
        vec![op.clone()]
    );
    assert!(control.operation_finalization_pending(&op).unwrap());
    let mut stale = op.clone();
    stale.generation += 1;
    assert!(control.operation_finalization_pending(&stale).is_err());
    drop(control);
    let control = ControlStore::open(&crate::test_storage_authority(), &path, &backup).unwrap();
    assert!(control.operation_finalization_pending(&op).unwrap());
    assert_eq!(
        control.pending_finalization_operations().unwrap(),
        vec![op.clone()]
    );
    control.complete_operation_finalization(&op).unwrap();
    assert!(!control.operation_finalization_pending(&op).unwrap());
    control.complete_operation_finalization(&op).unwrap();
    assert!(
        validate_recovery_members(&control.connection.borrow())
            .unwrap()
            .is_empty()
    );
    assert_eq!(control.load_operation(&op.operation_id).unwrap(), Some(op));
}

#[test]
fn empty_or_replaced_coverage_cannot_hide_unfinished_work() {
    for damage in [
        "DELETE FROM operation_recovery_members",
        "DELETE FROM operation_recovery_coverage",
        "UPDATE operation_recovery_coverage SET store_uuid='different-store'",
        "DROP TRIGGER operation_recovery_insert",
    ] {
        let root = tempdir().unwrap();
        let control = ControlStore::open(
            &crate::test_storage_authority(),
            root.path().join("control.db"),
            root.path().join("backups"),
        )
        .unwrap();
        let mut op = admitted(&control, "lost-member");
        control.connection.borrow().execute_batch(damage).unwrap();
        assert!(
            validate_recovery_members(&control.connection.borrow()).is_err(),
            "{damage}"
        );
        assert!(control.recoverable_operations().is_err(), "{damage}");
        if !damage.starts_with("DROP TRIGGER") {
            op.steps[0].attempts += 1;
            assert!(
                control.save_operation(&mut op).is_err(),
                "write after {damage}"
            );
        }
    }
}

#[test]
fn failed_finalization_transaction_keeps_the_member_and_coverage() {
    let root = tempdir().unwrap();
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        root.path().join("control.db"),
        root.path().join("backups"),
    )
    .unwrap();
    let op = admitted(&control, "rollback-coverage");
    {
        let mut db = control.connection.borrow_mut();
        let tx = db.transaction().unwrap();
        tx.execute("UPDATE operations SET state='rolled_back',finalization_pending=0 WHERE operation_id=?1", [op.operation_id.as_str()]).unwrap();
        // The real terminal transaction also releases its writer claim before commit.
        tx.execute(
            "DELETE FROM writer_claim WHERE operation_id=?1",
            [op.operation_id.as_str()],
        )
        .unwrap();
        assert!(validate_recovery_members(&tx).unwrap().is_empty());
        // Deliberate rollback models a failure before commit, not a partially published index.
    }
    assert_eq!(
        validate_recovery_members(&control.connection.borrow()).unwrap(),
        vec![op.operation_id.to_string()]
    );
    assert_eq!(control.load_operation(&op.operation_id).unwrap(), Some(op));
}

#[test]
fn large_effect_payload_is_immutable_while_progress_updates_stay_small() {
    let root = tempdir().unwrap();
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        root.path().join("control.db"),
        root.path().join("backups"),
    )
    .unwrap();
    let mut op = admitted(&control, "partitioned-journal");
    op.steps[0].effects.push(hiroute_domain::OwnedEffectV1 {
        effect_id: "large-effect".into(),
        kind: OwnedEffectKind::Control,
        target: "large-effect".into(),
        before_fingerprint: None,
        after_fingerprint: None,
        compensation: json!({"retained":"a".repeat(512*1024)}).into(),
    });
    control.save_operation(&mut op).unwrap();
    let payloads = || {
        control
            .connection
            .borrow()
            .query_row(
                "SELECT count(*),sum(length(payload_json)) FROM operation_payloads",
                [],
                |r| Ok((r.get::<_, u64>(0)?, r.get::<_, u64>(1)?)),
            )
            .unwrap()
    };
    let original = payloads();
    assert!(original.1 > 512 * 1024);
    for _ in 0..6 {
        op.steps[0].attempts += 1;
        control.save_operation(&mut op).unwrap();
        assert!(control.operation_is_current(&op).unwrap());
        assert_eq!(payloads(), original);
        let bytes: u64 = control.connection.borrow().query_row("SELECT length(operation_json)+(SELECT sum(length(step_json)) FROM operation_steps) FROM operations", [], |r| r.get(0)).unwrap();
        assert!(bytes < 32 * 1024, "progress rewrote {bytes} bytes");
    }
    assert_eq!(
        control.load_operation(&op.operation_id).unwrap(),
        Some(op.clone())
    );
    // A matching reference/digest column cannot hide changed retained bytes.
    control
        .connection
        .borrow()
        .execute(
            "UPDATE operation_payloads SET payload_json='{}' WHERE length(payload_json)>500000",
            [],
        )
        .unwrap();
    assert!(!control.operation_is_current(&op).unwrap());
    assert!(control.load_operation(&op.operation_id).is_err());
}

#[test]
fn finalization_lookup_refuses_missing_durable_membership() {
    for damage in [
        "DELETE FROM operation_recovery_members",
        "DELETE FROM operation_recovery_coverage",
        "UPDATE operation_recovery_coverage SET store_uuid='foreign-store'",
    ] {
        let root = tempdir().unwrap();
        let control = ControlStore::open(
            &crate::test_storage_authority(),
            root.path().join("control.db"),
            root.path().join("backups"),
        )
        .unwrap();
        let mut op = admitted(&control, "pending-finalizer");
        op.state = OperationState::RolledBack;
        for step in &mut op.steps {
            step.status = OperationStepStatus::Compensated;
        }
        control.finish_operation(&mut op).unwrap();
        assert!(control.operation_finalization_pending(&op).unwrap());
        control.connection.borrow().execute_batch(damage).unwrap();
        assert!(
            control.operation_finalization_pending(&op).is_err(),
            "{damage}"
        );
    }
}
