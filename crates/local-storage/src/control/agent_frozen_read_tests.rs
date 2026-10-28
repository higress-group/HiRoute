use super::super::super::{
    agent_frozen_read::FrozenAgentJournal, agent_history_cache::AgentHistoryRow,
};
use super::*;

fn capture(control: &ControlStore, id: &OperationId) -> FrozenAgentJournal {
    let db = control.connection.borrow();
    let tx = db.unchecked_transaction().unwrap();
    let row = tx.query_row(
        "SELECT operation_id,workspace_id,principal,operation_kind,idempotency_key,request_digest,accepted_change_digest,state,generation,operation_json FROM operations WHERE operation_id=?1",
        [id.as_str()], AgentHistoryRow::read,
    ).unwrap();
    let frozen = FrozenAgentJournal::capture(&tx, row).unwrap();
    tx.commit().unwrap();
    frozen
}

#[test]
fn frozen_selected_journal_uses_exact_owner_and_only_referenced_payloads() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let mut selected = target_operation(
        directory.path().join("selected").to_str().unwrap(),
        "frozen-selected",
        false,
    );
    let mut unrelated = target_operation(
        directory.path().join("other").to_str().unwrap(),
        "frozen-unrelated",
        false,
    );
    persist(&control, &mut selected);
    persist(&control, &mut unrelated);
    // A valid but unreferenced owner payload is not part of the selected closure.
    let unused = "{\"unused\":true}";
    let unused_digest = CanonicalDigest::of_bytes(unused.as_bytes());
    control
        .connection
        .borrow()
        .execute(
            "INSERT INTO operation_payloads(operation_id,digest,payload_json) VALUES(?1,?2,?3)",
            rusqlite::params![
                selected.operation_id.as_str(),
                unused_digest.as_str(),
                unused
            ],
        )
        .unwrap();
    let frozen = capture(&control, &selected.operation_id);
    assert_eq!(frozen.payload_count(), 1);
    let verified = frozen.verify().unwrap();
    assert_eq!(verified.operation_id, selected.operation_id);
    assert_eq!(verified.plan.spec(), selected.plan.spec());
    // A captured closure is immutable, not a current-state promise: a real caller
    // must perform the separate post-verification physical/business fence.
    control
        .connection
        .borrow()
        .execute(
            "UPDATE operation_steps SET step_json='{}' WHERE operation_id=?1 AND step_no=2",
            [selected.operation_id.as_str()],
        )
        .unwrap();
    assert_eq!(frozen.verify().unwrap().operation_id, selected.operation_id);
    assert!(read(&control).is_err());
}

#[test]
fn frozen_selected_journal_rejects_another_valid_row_substitution() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let path = directory.path().join("same-context");
    let mut first = target_operation(path.to_str().unwrap(), "frozen-first", false);
    let mut second = target_operation(path.to_str().unwrap(), "frozen-second", false);
    persist(&control, &mut first);
    persist(&control, &mut second);
    control.connection.borrow().execute(
        "UPDATE operations SET operation_json=(SELECT operation_json FROM operations WHERE operation_id=?2) WHERE operation_id=?1",
        rusqlite::params![first.operation_id.as_str(), second.operation_id.as_str()],
    ).unwrap();
    let frozen = capture(&control, &first.operation_id);
    assert!(frozen.verify().is_err());
}
