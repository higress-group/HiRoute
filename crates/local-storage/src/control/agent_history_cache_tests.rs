//! Real SQLite journal/proof validation; no status response is stubbed or cached.
use super::*;
use hiroute_domain::PortErrorCode;
use std::sync::Arc;

const KINDS: &[&str] = &["ApplyAgentConnectionChange", "ApplyAgentConnectionRestore"];
fn complete(control: &ControlStore, key: &str, payload: &str) -> OperationV1 {
    let mut operation = agent_operation(
        agent_transaction_plan_payload(AgentConnectionTransactionKindV1::Apply, None, payload),
        key,
    );
    persist(control, &mut operation);
    operation
}
fn persist(control: &ControlStore, operation: &mut OperationV1) {
    let key = &operation.idempotency.key;
    let token = format!("capability-{key}");
    control
        .grant_apply_capability(&token, operation, i64::MAX)
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
    control.begin_operation(operation, &authorization).unwrap();
    operation.state = hiroute_domain::OperationState::Succeeded;
    for step in &mut operation.steps {
        step.status = hiroute_domain::OperationStepStatus::Applied;
    }
    control.finish_operation(operation).unwrap();
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
fn budget_and_attribution_reads_reuse_the_same_verified_history() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    for key in ["budget-first", "budget-second"] {
        complete(&control, key, &"p".repeat(128 * 1024));
    }
    let decodes = || super::super::agent_read::DECODE_COUNT.with(std::cell::Cell::get);
    let before = decodes();
    let cold = read(&control).unwrap();
    assert_eq!(decodes() - before, 2);
    let expected: Vec<_> = cold.iter().map(|fact| fact.operation_id.clone()).collect();
    for _ in 0..3 {
        let budget = control
            .succeeded_agent_operations_for_kind(&WorkspaceId::default(), KINDS[0])
            .unwrap();
        let (attribution, rejected) = control
            .agent_attribution_recovery_candidates(&WorkspaceId::default())
            .unwrap();
        assert_eq!(rejected, 0);
        for facts in [&budget, &attribution] {
            assert_eq!(
                facts
                    .iter()
                    .map(|fact| fact.operation_id.clone())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
    assert_eq!(
        decodes() - before,
        2,
        "read-only consumers revalidated warm history"
    );
}

#[test]
fn attribution_skips_changed_proofs_while_budget_reads_remain_strict() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let valid = complete(&control, "valid", "");
    let invalid = complete(&control, "invalid", "");
    read(&control).unwrap();
    let external = rusqlite::Connection::open(directory.path().join("control.db")).unwrap();
    external
        .execute(
            "UPDATE operation_steps SET state='pending' WHERE operation_id=?1 AND step_no=5",
            [invalid.operation_id.as_str()],
        )
        .unwrap();
    for _ in 0..2 {
        let (facts, rejected) = control
            .agent_attribution_recovery_candidates(&WorkspaceId::default())
            .unwrap();
        assert_eq!(rejected, 1);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].operation_id, valid.operation_id);
        assert_eq!(
            control
                .succeeded_agent_operations_for_kind(&WorkspaceId::default(), KINDS[0])
                .unwrap_err()
                .code,
            PortErrorCode::Corrupt
        );
    }
    external
        .execute(
            "UPDATE operation_steps SET state='applied' WHERE operation_id=?1 AND step_no=5",
            [invalid.operation_id.as_str()],
        )
        .unwrap();
    external
        .execute(
            "UPDATE operations SET generation=-1 WHERE operation_id=?1",
            [invalid.operation_id.as_str()],
        )
        .unwrap();
    let (facts, rejected) = control
        .agent_attribution_recovery_candidates(&WorkspaceId::default())
        .unwrap();
    assert_eq!(rejected, 1, "Unreadable row metadata was not counted");
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].operation_id, valid.operation_id);
    assert_eq!(read(&control).unwrap_err().code, PortErrorCode::Corrupt);
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
    eprintln!(
        "agent_history: raw_bytes={size}, entries=21, cold_decodes=21, warm_scans=21, additional_warm_decodes=0; Arc identity retained"
    );
    drop(control);
    let reopened = open(directory.path());
    assert_eq!(reopened.agent_history_cache.borrow().decodes, 0);
    assert_eq!(read(&reopened).unwrap().len(), 21);
    assert_eq!(reopened.agent_history_cache.borrow().decodes, 21);
}

#[test]
fn cache_capacity_and_oversized_entries_fall_back_without_eviction_thrash() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    complete(&control, "first", "");
    complete(&control, "second", "");
    // The same admission logic with tiny test-only limits avoids allocating
    // hundreds of megabytes merely to test the fallback boundary.
    control.agent_history_cache.borrow_mut().budget_override = Some((1, usize::MAX));
    let first = read(&control).unwrap();
    for _ in 0..3 {
        let next = read(&control).unwrap();
        assert_eq!(next.len(), 2);
        assert!(Arc::ptr_eq(&first[0], &next[0]));
        assert!(!Arc::ptr_eq(&first[1], &next[1]));
    }
    assert_eq!(control.agent_history_cache.borrow().decodes, 5);
    *control.agent_history_cache.borrow_mut() = Default::default();
    control.agent_history_cache.borrow_mut().budget_override = Some((128, 1));
    let first = read(&control).unwrap();
    let second = read(&control).unwrap();
    assert_eq!(first.len(), 2);
    assert!(first.iter().zip(second).all(|(a, b)| !Arc::ptr_eq(a, &b)));
    assert_eq!(control.agent_history_cache.borrow().decodes, 4);
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

fn target_operation(root: &str, key: &str, restore: bool) -> OperationV1 {
    let context = format!(
        "agent-context/hermes/{}",
        CanonicalDigest::of_bytes(root.as_bytes()).as_str()
    );
    let spec = hiroute_domain::ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: "agents.settings.apply".into(),
        resource_id: Some(context.clone()),
        desired_state: json!({
            "schema_version":{"major":2,"minor":0}, "context_id":context,
            "native_target":{"kind":"hermes","config_root":root},
            "management_skill": if restore {
                json!({"intent":"restore","restore_point_ref":"restore.fixture"})
            } else { json!({"intent":"configure","settings":{}}) }
        }),
    };
    let subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
        "agent.hermes",
        "default",
        "hermes.profile.v1",
    )
    .unwrap();
    // A settings Skill reference can legitimately require no file effect.
    let control = AgentConnectionControlIntentV1::from_settings_planner(
        subject,
        &spec,
        false,
        &json!({"revision":1,"payload":"p".repeat(512 * 1024)}),
    )
    .unwrap();
    let workspace = WorkspaceId::default();
    let scope = IdempotencyScopeV1::new(
        "interactive-user",
        if restore {
            "ApplyAgentConnectionRestore"
        } else {
            "ApplyAgentConnectionChange"
        },
        key,
    )
    .unwrap();
    let request = CanonicalDigest::of_bytes(format!("agent-request-{key}").as_bytes());
    OperationV1::new(
        OperationId::derive(&workspace, &scope, &request),
        workspace,
        scope,
        request,
        CanonicalDigest::of_bytes(b"agent-change"),
        RevisionSetV1 {
            target: 0,
            dependencies: BTreeMap::new(),
        },
        TransactionPlanV1::from_agent_connection_planner(spec, control, vec![]).unwrap(),
    )
    .unwrap()
}

#[test]
fn native_target_discovery_reuses_verified_successes_and_keeps_unfinished_tails() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let main_path = directory.path().join("hermes-main");
    let main_root = main_path.to_str().unwrap();
    let pending_path = directory.path().join("hermes-pending");
    let pending_root = pending_path.to_str().unwrap();
    for index in 0..21 {
        let mut operation = target_operation(main_root, &format!("target-{index}"), index % 2 == 0);
        persist(&control, &mut operation);
    }
    let targets = control
        .agent_settings_native_targets(&WorkspaceId::default())
        .unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].config_root(), main_root);
    assert_eq!(control.agent_history_cache.borrow().decodes, 21);
    for _ in 0..3 {
        assert_eq!(
            control
                .agent_settings_native_targets(&WorkspaceId::default())
                .unwrap(),
            targets
        );
    }
    assert_eq!(control.agent_history_cache.borrow().decodes, 21);

    let pending = target_operation(pending_root, "pending-target", false);
    let token = "capability-pending-target";
    control
        .grant_apply_capability(token, &pending, i64::MAX)
        .unwrap();
    let authorization = control
        .verify_apply_authorization(
            &ProtectedApplyCapability::new(token.to_owned()).unwrap(),
            &pending.workspace_id,
            &pending.idempotency.principal,
            &pending.idempotency.operation_kind,
            &pending.accepted_digest,
            &pending.expected_revisions,
        )
        .unwrap();
    control.begin_operation(&pending, &authorization).unwrap();
    let targets = control
        .agent_settings_native_targets(&WorkspaceId::default())
        .unwrap();
    assert_eq!(targets.len(), 2);
    assert!(
        targets
            .iter()
            .any(|target| target.config_root() == pending_root)
    );
    // An existing read transaction is supported; no nested BEGIN or cached tail.
    control.connection.borrow().execute_batch("BEGIN").unwrap();
    control
        .connection
        .borrow()
        .execute(
            "UPDATE operations SET operation_json='invalid' WHERE operation_id=?1",
            [pending.operation_id.as_str()],
        )
        .unwrap();
    assert_eq!(
        control
            .agent_settings_native_targets(&WorkspaceId::default())
            .unwrap_err()
            .code,
        PortErrorCode::Corrupt
    );
    control
        .connection
        .borrow()
        .execute_batch("ROLLBACK")
        .unwrap();
    assert_eq!(
        control
            .agent_settings_native_targets(&WorkspaceId::default())
            .unwrap()
            .len(),
        2
    );
    control.connection.borrow().execute_batch("UPDATE operation_steps SET state='pending' WHERE step_no=5 AND operation_id != (SELECT operation_id FROM operations ORDER BY rowid DESC LIMIT 1)").unwrap();
    // Discovery is a path hint. It retains readable immutable inputs when
    // provenance is missing; the independent authority reader stays strict.
    assert_eq!(
        control
            .agent_settings_native_targets(&WorkspaceId::default())
            .unwrap(),
        targets
    );
    assert_eq!(read(&control).unwrap_err().code, PortErrorCode::Corrupt);
}
