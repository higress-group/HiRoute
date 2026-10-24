//! Real SQLite journal/proof validation; no status response is stubbed or cached.
use super::*;
use std::sync::Arc;

const KINDS: &[&str] = &["ApplyAgentConnectionChange", "ApplyAgentConnectionRestore"];
fn complete(control: &ControlStore, key: &str, payload: &str) -> OperationV1 {
    let mut operation = agent_operation(
        agent_transaction_plan_payload(AgentConnectionTransactionKindV1::Apply, None, payload),
        key,
    );
    let token = format!("capability-{key}");
    control
        .grant_apply_capability(&token, &operation, i64::MAX)
        .unwrap();
    let authorization = control
        .verify_apply_authorization(
            &ProtectedApplyCapability::new(token).unwrap(),
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
    control.finish_operation(&mut operation).unwrap();
    operation
}
fn open(directory: &std::path::Path) -> ControlStore {
    ControlStore::open(
        &crate::test_storage_authority(),
        directory.join("control.db"),
        directory.join("backups"),
    )
    .unwrap()
}
fn read(
    control: &ControlStore,
) -> hiroute_domain::PortResult<Vec<Arc<super::super::SucceededAgentOperationV1>>> {
    control.verified_agent_history_for_kinds(&WorkspaceId::default(), KINDS)
}

#[test]
fn large_history_validates_once_then_shares_facts_without_scan_thrash() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    // Two planner effects carry a realistic large immutable publication payload.
    let payload = "p".repeat(512 * 1024);
    for index in 0..21 {
        complete(&control, &format!("history-{index}"), &payload);
    }
    let size: u64 = control
        .connection
        .borrow()
        .query_row(
            "SELECT sum(length(operation_json)) FROM operations",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(size > 21 * 1024 * 1024);
    let cold = read(&control).unwrap();
    assert_eq!(control.agent_history_cache.borrow().decodes, 21);
    // Model, collaboration and management reads across multiple Agents share
    // this same immutable history; filtering belongs after strict verification.
    for _ in 0..7 * 3 {
        let warm = read(&control).unwrap();
        assert_eq!(warm.len(), 21);
        assert!(cold.iter().zip(&warm).all(|(a, b)| Arc::ptr_eq(a, b)));
    }
    assert_eq!(control.agent_history_cache.borrow().decodes, 21);
    let strict = control
        .succeeded_agent_operations_for_kinds(&WorkspaceId::default(), KINDS)
        .unwrap();
    assert_eq!(format!("{cold:?}"), format!("{strict:?}"));
    drop(control);
    let reopened = open(directory.path());
    assert_eq!(reopened.agent_history_cache.borrow().decodes, 0);
    assert_eq!(read(&reopened).unwrap().len(), 21);
    assert_eq!(reopened.agent_history_cache.borrow().decodes, 21);
}

#[test]
fn warmed_history_rejects_external_row_and_step_changes_and_valid_raw_substitution() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    complete(&control, "a", "");
    complete(&control, "b", "");
    read(&control).unwrap();
    let external = rusqlite::Connection::open(directory.path().join("control.db")).unwrap();
    for sql in [
        "UPDATE operations SET generation=generation+1",
        "UPDATE operations SET principal='other'",
        "UPDATE operation_steps SET step_json=json_set(step_json,'$.step.attempts',99) WHERE step_no=2",
        "UPDATE operation_steps SET state='pending' WHERE step_no=1",
        "DELETE FROM operation_steps WHERE step_no=5",
        "UPDATE operations SET operation_json=(SELECT operation_json FROM operations ORDER BY rowid ASC LIMIT 1) WHERE rowid=(SELECT max(rowid) FROM operations)",
        "UPDATE operations SET operation_json=json_set(operation_json,'$.steps[0].extra','unknown')",
    ] {
        let before = read(&control).unwrap();
        external.execute_batch("BEGIN").unwrap();
        external.execute_batch(sql).unwrap();
        // Another connection's uncommitted write cannot split our snapshot.
        let unchanged = read(&control).unwrap();
        assert!(
            before
                .iter()
                .zip(&unchanged)
                .all(|(a, b)| Arc::ptr_eq(a, b))
        );
        external.execute_batch("ROLLBACK").unwrap();
        // Repeat inside the current connection's transaction: the new exact
        // closure is seen, and the original reader's failure is not bypassed.
        control.connection.borrow().execute_batch("BEGIN").unwrap();
        control.connection.borrow().execute_batch(sql).unwrap();
        assert_eq!(
            read(&control).unwrap_err().code,
            hiroute_domain::PortErrorCode::Corrupt,
            "{sql}"
        );
        control
            .connection
            .borrow()
            .execute_batch("ROLLBACK")
            .unwrap();
        assert_eq!(read(&control).unwrap().len(), 2);
    }
    external
        .execute_batch("UPDATE operation_steps SET state='pending' WHERE step_no=1")
        .unwrap();
    assert_eq!(
        read(&control).unwrap_err().code,
        hiroute_domain::PortErrorCode::Corrupt
    );
}

#[test]
fn history_membership_and_valid_same_connection_writes_are_not_cached() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let mut first = complete(&control, "first", "");
    let original = read(&control).unwrap();
    complete(&control, "second", "");
    let appended = read(&control).unwrap();
    assert_eq!(appended.len(), 2);
    assert_eq!(appended[1].operation_id, first.operation_id);
    assert!(Arc::ptr_eq(&appended[1], &original[0]));
    control.save_operation(&mut first).unwrap();
    let changed = read(&control).unwrap();
    assert!(
        !Arc::ptr_eq(&changed[1], &original[0]),
        "generation change reused old proof"
    );
    control
        .connection
        .borrow()
        .execute(
            "DELETE FROM operation_steps WHERE operation_id=?1",
            [first.operation_id.as_str()],
        )
        .unwrap();
    control
        .connection
        .borrow()
        .execute(
            "DELETE FROM operations WHERE operation_id=?1",
            [first.operation_id.as_str()],
        )
        .unwrap();
    assert_eq!(read(&control).unwrap().len(), 1);
}
