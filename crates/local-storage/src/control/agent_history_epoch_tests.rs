//! SQLite snapshot ownership, invalidation and rollback; no native authority is cached.
use super::*;

fn hashes(control: &ControlStore) -> usize {
    control.agent_history_cache.borrow().fingerprints
}

#[test]
fn unrelated_external_commits_recheck_exact_bytes_without_redecoding_history() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    complete(&control, "external-epoch", "");
    let original = read(&control).unwrap();
    let before = hashes(&control);
    let external = rusqlite::Connection::open(directory.path().join("control.db")).unwrap();
    external
        .execute_batch(
            "CREATE TABLE unrelated_probe(value INTEGER); INSERT INTO unrelated_probe VALUES(1)",
        )
        .unwrap();
    let next = read(&control).unwrap();
    assert_eq!(
        hashes(&control),
        before + 1,
        "external write invalidates the shortcut proof"
    );
    assert!(
        Arc::ptr_eq(&original[0], &next[0]),
        "unchanged exact closure retains only its decoded facts"
    );
    external
        .execute_batch("UPDATE operation_steps SET state='pending' WHERE step_no=3")
        .unwrap();
    assert_eq!(
        read(&control).unwrap_err().code,
        PortErrorCode::Corrupt,
        "even a non-retained publication step must still be revalidated"
    );
}

#[test]
fn different_store_identity_discards_even_byte_identical_projection() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    complete(&control, "store-identity", "");
    let old = read(&control).unwrap();
    control
        .connection
        .borrow()
        .execute(
            "UPDATE storage_meta SET store_uuid=?1 WHERE singleton=1",
            ["cache-replacement-fixture"],
        )
        .unwrap();
    let new = read(&control).unwrap();
    assert!(!Arc::ptr_eq(&old[0], &new[0]));
}

#[test]
fn changed_schema_and_local_writes_invalidate_without_discarding_unchanged_facts() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    complete(&control, "epoch", "");
    let original = read(&control).unwrap();
    for sql in [
        "CREATE TABLE harmless_probe(value INTEGER)",
        "CREATE TEMP TABLE harmless_temp_probe(value INTEGER)",
        "INSERT INTO harmless_probe VALUES(1)",
        "DROP TABLE harmless_temp_probe",
    ] {
        let before = hashes(&control);
        control.connection.borrow().execute_batch(sql).unwrap();
        let next = read(&control).unwrap();
        assert_eq!(hashes(&control), before + 1, "{sql}");
        assert!(Arc::ptr_eq(&original[0], &next[0]));
        read(&control).unwrap();
        assert_eq!(hashes(&control), before + 1, "{sql}: warm read");
    }
}

#[test]
fn caller_transaction_facts_never_escape_rollback_or_savepoint_rollback() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    complete(&control, "rollback-epoch", "");
    for (begin, rollback) in [
        ("BEGIN", "ROLLBACK"),
        (
            "SAVEPOINT outer_scope",
            "ROLLBACK TO outer_scope; RELEASE outer_scope",
        ),
    ] {
        let original = read(&control).unwrap();
        control.connection.borrow().execute_batch(begin).unwrap();
        control
            .connection
            .borrow()
            .execute_batch(
                "UPDATE operations SET generation=generation+1,
             operation_json=json_set(operation_json,'$.generation',generation+1)",
            )
            .unwrap();
        let transient = read(&control).unwrap();
        assert!(!Arc::ptr_eq(&original[0], &transient[0]));
        let changes = control.connection.borrow().total_changes();
        control.connection.borrow().execute_batch(rollback).unwrap();
        assert_eq!(control.connection.borrow().total_changes(), changes);
        let restored = read(&control).unwrap();
        assert!(!Arc::ptr_eq(&transient[0], &restored[0]));
        let before = hashes(&control);
        let warm = read(&control).unwrap();
        assert!(Arc::ptr_eq(&restored[0], &warm[0]));
        assert_eq!(hashes(&control), before);
    }
    // A failed caller read must not leave an epoch that legitimizes the next read.
    control
        .connection
        .borrow()
        .execute_batch("BEGIN; UPDATE operation_steps SET state='pending' WHERE step_no=0")
        .unwrap();
    assert!(read(&control).is_err());
    control
        .connection
        .borrow()
        .execute_batch("ROLLBACK")
        .unwrap();
    assert_eq!(read(&control).unwrap().len(), 1);
}

#[test]
fn external_commit_after_snapshot_start_is_visible_on_the_next_read_only() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let old = complete(&control, "snapshot-old", "");
    complete(&control, "snapshot-new", "");
    read(&control).unwrap();
    let external = rusqlite::Connection::open(directory.path().join("control.db")).unwrap();
    let mut calls = 0;
    let found = control
        .find_verified_agent_operation(&WorkspaceId::default(), KINDS, |_| {
            calls += 1;
            if calls == 1 {
                // The first selected row establishes the read snapshot. A later
                // writer cannot replace its epoch or the second row's old bytes.
                external.execute(
                    "UPDATE operation_steps SET step_json='{}' WHERE operation_id=?1 AND step_no=0",
                    [old.operation_id.as_str()],
                ).unwrap();
            }
            Ok(false)
        })
        .unwrap();
    assert!(found.is_none());
    assert_eq!(calls, 2);
    // This commit predates the next read snapshot, which must reject its bytes.
    assert_eq!(read(&control).unwrap_err().code, PortErrorCode::Corrupt);
}

#[test]
fn transient_valid_facts_are_not_reused_after_read_transaction_errors() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    complete(&control, "old", "");
    let newest = complete(&control, "new", "");
    read(&control).unwrap();
    control.connection.borrow().execute_batch("BEGIN").unwrap();
    control
        .connection
        .borrow()
        .execute_batch(
            "UPDATE operations SET generation=generation+1,
         operation_json=json_set(operation_json,'$.generation',generation+1)",
        )
        .unwrap();
    let mut captured = None;
    let failed = control.find_verified_agent_operation(&WorkspaceId::default(), KINDS, |fact| {
        if fact.operation_id == newest.operation_id {
            captured = Some(fact.operation_id.clone());
        }
        Err(hiroute_domain::PortError::new(
            PortErrorCode::Unavailable,
            "test.read.interrupted",
        ))
    });
    assert!(failed.is_err());
    assert_eq!(captured, Some(newest.operation_id));
    control
        .connection
        .borrow()
        .execute_batch("ROLLBACK")
        .unwrap();
    let before = hashes(&control);
    assert_eq!(read(&control).unwrap().len(), 2);
    assert_eq!(hashes(&control), before + 2);
}

#[test]
fn a_selection_callback_write_invalidates_remaining_facts_and_rollback_proofs() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let old = complete(&control, "callback-old", "");
    complete(&control, "callback-new", "");
    read(&control).unwrap();
    let before = hashes(&control);
    let mut calls = 0;
    let failed = control.find_verified_agent_operation(&WorkspaceId::default(), KINDS, |_| {
        calls += 1;
        if calls == 1 {
            control
                .connection
                .borrow()
                .execute(
                    "UPDATE operations SET generation=generation+1,
                 operation_json=json_set(operation_json,'$.generation',generation+1)
                 WHERE operation_id=?1",
                    [old.operation_id.as_str()],
                )
                .unwrap();
            Ok(false)
        } else {
            Err(hiroute_domain::PortError::new(
                PortErrorCode::Unavailable,
                "test.callback.failed",
            ))
        }
    });
    assert!(failed.is_err());
    assert_eq!(calls, 2);
    assert_eq!(
        hashes(&control),
        before + 1,
        "callback writes must be revalidated"
    );
    read(&control).unwrap();
    assert_eq!(
        hashes(&control),
        before + 3,
        "rolled-back facts cannot mint reuse proofs"
    );
}
