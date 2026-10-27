//! Exact bytes, rollback and current receipts remain authoritative on warm reads.
use super::*;
use rusqlite::{Connection, params};

fn count(control: &ControlStore) -> usize {
    control.completed_operation_cache.borrow().decodes
}

#[test]
fn completed_provenance_reuses_decode_but_not_mutable_returned_state() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let operation = complete(&control, "completed-decoder", &"x".repeat(128 * 1024));
    let before = count(&control);
    let mut first = control
        .load_operation(&operation.operation_id)
        .unwrap()
        .unwrap();
    first.safe_error_code = Some("caller-local-change".into());
    first.steps[0].attempts += 1;
    for _ in 0..6 {
        let current = control
            .load_operation(&operation.operation_id)
            .unwrap()
            .unwrap();
        assert_eq!(current.safe_error_code, operation.safe_error_code);
        assert_eq!(current.steps, operation.steps);
        assert!(control.operation_is_current(&current).unwrap());
    }
    assert_eq!(count(&control) - before, 1);
    let reopened = open(directory.path());
    reopened
        .load_operation(&operation.operation_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        count(&reopened),
        1,
        "no process-local proof survives reopen"
    );
}

#[test]
fn completed_decoder_observes_external_payload_damage_and_missing_journal() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let operation = complete(&control, "completed-payload", &"x".repeat(128 * 1024));
    control
        .load_operation(&operation.operation_id)
        .unwrap()
        .unwrap();
    let external = Connection::open(directory.path().join("control.db")).unwrap();
    let (digest, original): (String, String) = external.query_row(
        "SELECT digest,payload_json FROM operation_payloads WHERE operation_id=?1 ORDER BY digest LIMIT 1",
        [operation.operation_id.as_str()], |r| Ok((r.get(0)?,r.get(1)?)),
    ).unwrap();
    external
        .execute(
            "UPDATE operation_payloads SET payload_json='{}' WHERE operation_id=?1 AND digest=?2",
            params![operation.operation_id.as_str(), digest],
        )
        .unwrap();
    assert!(control.load_operation(&operation.operation_id).is_err());
    external
        .execute(
            "UPDATE operation_payloads SET payload_json=?3 WHERE operation_id=?1 AND digest=?2",
            params![operation.operation_id.as_str(), digest, original],
        )
        .unwrap();
    let restored = control
        .load_operation(&operation.operation_id)
        .unwrap()
        .unwrap();
    assert!(control.operation_is_current(&restored).unwrap());
    external
        .execute(
            "UPDATE operations SET operation_json='{}' WHERE operation_id=?1",
            [operation.operation_id.as_str()],
        )
        .unwrap();
    assert!(control.load_operation(&operation.operation_id).is_err());
}

#[test]
fn completed_decoder_does_not_replace_current_receipt_or_identity_checks() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let operation = complete(&control, "completed-currentness", "current");
    control
        .load_operation(&operation.operation_id)
        .unwrap()
        .unwrap();
    for step in &operation.steps {
        let original: String = control.with_connection(|db| {
            db.query_row(
                "SELECT step_json FROM operation_steps WHERE operation_id=?1 AND step_no=?2",
                params![operation.operation_id.as_str(), step.sequence],
                |row| row.get(0),
            )
            .unwrap()
        });
        control.with_connection(|db| {
            assert_eq!(db.execute(
                "UPDATE operation_steps SET step_json='{}' WHERE operation_id=?1 AND step_no=?2",
                params![operation.operation_id.as_str(), step.sequence],
            ).unwrap(), 1);
        });
        let warm = control
            .load_operation(&operation.operation_id)
            .unwrap()
            .unwrap();
        assert!(!control.operation_is_current(&warm).unwrap());
        let db = control.connection.borrow();
        assert_eq!(
            super::super::super::journal::is_current_artifact_step(&db, &warm).unwrap(),
            step.kind != hiroute_domain::OperationStepKind::ApplyAgentArtifacts,
            "only the current artifact receipt owns artifact authority: {:?}",
            step.kind,
        );
        assert_eq!(
            super::super::super::journal::is_current_grant_step(&db, &warm).unwrap(),
            step.kind != hiroute_domain::OperationStepKind::ApplySecrets,
            "only the current grant receipt owns grant authority: {:?}",
            step.kind,
        );
        drop(db);
        control.with_connection(|db| {
            db.execute(
                "UPDATE operation_steps SET step_json=?3 WHERE operation_id=?1 AND step_no=?2",
                params![operation.operation_id.as_str(), step.sequence, original],
            )
            .unwrap();
        });
        assert!(control.operation_is_current(&warm).unwrap());
    }
    let warm = control
        .load_operation(&operation.operation_id)
        .unwrap()
        .unwrap();
    control.with_connection(|db| {
        db.execute(
            "UPDATE operations SET accepted_change_digest='changed' WHERE operation_id=?1",
            [operation.operation_id.as_str()],
        )
        .unwrap()
    });
    assert!(!control.operation_is_current(&warm).unwrap());
    assert!(
        !super::super::super::journal::is_current_artifact_step(
            &control.connection.borrow(),
            &warm
        )
        .unwrap()
    );
}

#[test]
fn completed_decoder_checks_caller_transaction_bytes_after_savepoint_rollback() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let operation = complete(&control, "completed-rollback", "current");
    control
        .load_operation(&operation.operation_id)
        .unwrap()
        .unwrap();
    control.with_connection(|db| {
        db.execute_batch("BEGIN; SAVEPOINT changed").unwrap();
        db.execute(
            "UPDATE operation_payloads SET payload_json='{}' WHERE operation_id=?1",
            [operation.operation_id.as_str()],
        )
        .unwrap();
    });
    assert!(control.load_operation(&operation.operation_id).is_err());
    control.with_connection(|db| db.execute_batch("ROLLBACK TO changed").unwrap());
    let restored = control
        .load_operation(&operation.operation_id)
        .unwrap()
        .unwrap();
    assert!(control.operation_is_current(&restored).unwrap());
    control.with_connection(|db| db.execute_batch("ROLLBACK").unwrap());
    assert!(
        control
            .operation_is_current(
                &control
                    .load_operation(&operation.operation_id)
                    .unwrap()
                    .unwrap()
            )
            .unwrap()
    );
}

#[test]
fn completed_decoder_capacity_eviction_keeps_cold_validation() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let mut operations = Vec::new();
    for index in 0..34 {
        let operation = complete(&control, &format!("completed-capacity-{index}"), "bounded");
        control
            .load_operation(&operation.operation_id)
            .unwrap()
            .unwrap();
        operations.push(operation);
    }
    let before = count(&control);
    let latest = operations.last().unwrap();
    control
        .load_operation(&latest.operation_id)
        .unwrap()
        .unwrap();
    assert_eq!(count(&control), before);
    let first = control
        .load_operation(&operations[0].operation_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        count(&control),
        before + 1,
        "evicted provenance takes the cold reader"
    );
    assert!(control.operation_is_current(&first).unwrap());
}
