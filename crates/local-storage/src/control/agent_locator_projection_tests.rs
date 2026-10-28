use super::super::super::agent_locator;
use super::*;

fn install(control: &ControlStore) {
    let db = control.connection.borrow();
    let tx = db.unchecked_transaction().unwrap();
    crate::migrations::agent_locator_v33::remove_test_shape(&tx);
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
    db.execute_batch("BEGIN; CREATE TEMP TABLE agent_locator_heads(ordinal INTEGER)")
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

#[test]
fn exact_locator_skips_newer_unrelated_history_and_checks_current_bytes() {
    use super::super::super::{AgentLocatorResult, AgentLocatorScope};
    use hiroute_domain::AgentSettingsFacet;
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let target = directory.path().join("selected");
    let mut selected = target_operation(target.to_str().unwrap(), "selected-locator", false);
    persist(&control, &mut selected);
    let mut later = Vec::new();
    for index in 0..5 {
        let root = directory.path().join(format!("other-{index}"));
        let mut operation =
            target_operation(root.to_str().unwrap(), &format!("other-{index}"), false);
        persist(&control, &mut operation);
        later.push(operation);
    }
    install(&control);
    // One unrelated indexed optional step no longer validates as full history.
    control
        .connection
        .borrow()
        .execute(
            "UPDATE operation_steps SET step_json='{}' WHERE operation_id=?1 AND step_no=2",
            [later[3].operation_id.as_str()],
        )
        .unwrap();
    let context = selected.plan.spec().resource_id.as_deref().unwrap();
    let before = super::super::super::agent_read::DECODE_COUNT.with(std::cell::Cell::get);
    let found = control
        .find_agent_locator(
            &WorkspaceId::default(),
            AgentLocatorScope::ContextFacet {
                context_id: context,
                facet: AgentSettingsFacet::ManagementSkill,
            },
        )
        .unwrap();
    assert!(
        matches!(found, AgentLocatorResult::Found(ref operation) if operation.operation_id == selected.operation_id)
    );
    assert_eq!(
        super::super::super::agent_read::DECODE_COUNT.with(std::cell::Cell::get) - before,
        1
    );
    assert!(read(&control).is_err());
    // The selected row and both step copies remain proof obligations.
    control
        .connection
        .borrow()
        .execute(
            "UPDATE operation_steps SET step_json='{}' WHERE operation_id=?1 AND step_no=2",
            [selected.operation_id.as_str()],
        )
        .unwrap();
    assert!(
        control
            .find_agent_locator(
                &WorkspaceId::default(),
                AgentLocatorScope::ContextFacet {
                    context_id: context,
                    facet: AgentSettingsFacet::ManagementSkill,
                }
            )
            .is_err()
    );
}

#[test]
fn unknown_newer_member_fences_a_previous_hit_and_absence() {
    use super::super::super::AgentLocatorScope;
    use hiroute_domain::AgentSettingsFacet;
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let root = directory.path().join("selected");
    let mut selected = target_operation(root.to_str().unwrap(), "selected-before-unknown", false);
    let mut unknown = target_operation(
        directory.path().join("unknown").to_str().unwrap(),
        "unknown-newer",
        false,
    );
    persist(&control, &mut selected);
    persist(&control, &mut unknown);
    control
        .connection
        .borrow()
        .execute(
            "UPDATE operation_payloads SET payload_json='{}' WHERE operation_id=?1",
            [unknown.operation_id.as_str()],
        )
        .unwrap();
    install(&control);
    let context = selected.plan.spec().resource_id.as_deref().unwrap();
    assert!(
        control
            .find_agent_locator(
                &WorkspaceId::default(),
                AgentLocatorScope::ContextFacet {
                    context_id: context,
                    facet: AgentSettingsFacet::ManagementSkill,
                }
            )
            .is_err()
    );
    assert!(
        control
            .find_agent_locator(
                &WorkspaceId::default(),
                AgentLocatorScope::ContextFacet {
                    context_id: "agent-context/unrecorded",
                    facet: AgentSettingsFacet::Model,
                }
            )
            .is_err()
    );
}
