use super::super::super::agent_locator;
use super::*;

fn install(control: &ControlStore) {
    let db = control.connection.borrow();
    let tx = db.unchecked_transaction().unwrap();
    tx.execute_batch(crate::migrations::agent_locator_v33::CONTROL)
        .unwrap();
    agent_locator::initialize(&tx).unwrap();
    tx.commit().unwrap();
}

#[test]
fn initial_projection_keeps_restore_order_and_isolates_unrelated_optional_damage() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let root = directory.path().join("hermes");
    let other_root = directory.path().join("other");
    let mut first = target_operation(root.to_str().unwrap(), "locator-first", false);
    let mut later = target_operation(root.to_str().unwrap(), "locator-restore", true);
    let mut other = target_operation(other_root.to_str().unwrap(), "locator-other", false);
    persist(&control, &mut first);
    persist(&control, &mut later);
    persist(&control, &mut other);
    // v32 was already admitted. This optional historical step is not a Plan input.
    control
        .connection
        .borrow()
        .execute(
            "UPDATE operation_steps SET step_json='{}' WHERE operation_id=?1 AND step_no=1",
            [first.operation_id.as_str()],
        )
        .unwrap();
    assert!(read(&control).is_err());
    install(&control);
    let db = control.connection.borrow();
    agent_locator::validate_coverage(&db).unwrap();
    let context = first.plan.spec().resource_id.as_ref().unwrap();
    let (id, action): (String, String) = db.query_row(
        "SELECT m.operation_id,h.action FROM agent_locator_heads h JOIN agent_locator_members m ON m.ordinal=h.ordinal
         WHERE h.workspace_id=?1 AND h.scope='management' AND h.context_id=?2 AND h.grant_key=''",
        rusqlite::params![WorkspaceId::DEFAULT, context], |r| Ok((r.get(0)?,r.get(1)?)),
    ).unwrap();
    assert_eq!(id, later.operation_id.as_str());
    assert_eq!(action, "restore");
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_locator_members", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        3
    );
}

#[test]
fn coverage_rejects_projection_or_producer_damage_and_keeps_unknown_events() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let mut op = target_operation(
        directory.path().join("hermes").to_str().unwrap(),
        "locator-coverage",
        false,
    );
    persist(&control, &mut op);
    install(&control);
    let db = control.connection.borrow();
    agent_locator::validate_coverage(&db).unwrap();
    db.execute_batch("BEGIN").unwrap();
    db.execute("DELETE FROM agent_locator_heads", []).unwrap();
    assert!(agent_locator::validate_coverage(&db).is_err());
    db.execute_batch("ROLLBACK").unwrap();
    db.execute_batch("BEGIN").unwrap();
    db.execute_batch("DROP TRIGGER agent_locator_operation_update")
        .unwrap();
    assert!(agent_locator::validate_coverage(&db).is_err());
    db.execute_batch("ROLLBACK").unwrap();
    db.execute(
        "UPDATE operation_steps SET step_json='{}' WHERE operation_id=?1 AND step_no=1",
        [op.operation_id.as_str()],
    )
    .unwrap();
    assert!(agent_locator::validate_coverage(&db).is_ok());
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_locator_dirty WHERE operation_id=?1",
            [op.operation_id.as_str()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    db.execute_batch("BEGIN").unwrap();
    db.execute("DELETE FROM agent_locator_dirty", []).unwrap();
    assert!(agent_locator::validate_coverage(&db).is_err());
    db.execute_batch("ROLLBACK").unwrap();
}
