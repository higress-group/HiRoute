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
    drop(control);
    let control = ControlStore::open(&crate::test_storage_authority(), &path, &backup).unwrap();
    assert_eq!(
        control.pending_finalization_operations().unwrap(),
        vec![op.clone()]
    );
    control.complete_operation_finalization(&op).unwrap();
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
