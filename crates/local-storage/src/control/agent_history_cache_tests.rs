//! Real SQLite journal/proof validation; no status response is stubbed or cached.
use super::*;
use hiroute_domain::PortErrorCode;
use std::sync::Arc;

const KINDS: &[&str] = &["ApplyAgentConnectionChange", "ApplyAgentConnectionRestore"];
#[path = "completed_operation_cache_tests.rs"]
mod completed_decoder;
#[path = "agent_history_epoch_tests.rs"]
mod epoch;
fn complete(control: &ControlStore, key: &str, payload: &str) -> OperationV1 {
    let mut operation = agent_operation(
        agent_transaction_plan_payload(AgentConnectionTransactionKindV1::Apply, None, payload),
        key,
    );
    persist(control, &mut operation);
    operation
}
fn persist(control: &ControlStore, operation: &mut OperationV1) {
    persist_with_history_padding(control, operation, None);
}
fn persist_with_history_padding(
    control: &ControlStore,
    operation: &mut OperationV1,
    padding: Option<&str>,
) {
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
    add_component_file_receipts(operation);
    if let Some(padding) = padding {
        // A valid receipt's unconsumed compensation data must be authenticated
        // but is not a current-status or grant fact.
        let receipt = &mut operation
            .steps
            .iter_mut()
            .find(|step| step.kind == hiroute_domain::OperationStepKind::ApplyAgentArtifacts)
            .unwrap()
            .effects[0];
        Arc::make_mut(&mut receipt.compensation)["retained_history_fixture"] = padding.into();
    }
    control.finish_operation(operation).unwrap();
    // The component fixture has no external finalizer; mark its retained journal settled.
    control.complete_operation_finalization(operation).unwrap();
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
fn newest_matching_fact_stops_before_unneeded_history_but_rechecks_current_bytes() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let old = complete(&control, "unneeded-old", &"p".repeat(128 * 1024));
    let selected = complete(&control, "current-selected", "current");
    let find = || {
        control.find_verified_agent_operation(&WorkspaceId::default(), KINDS, |fact| {
            Ok(fact.operation_id == selected.operation_id)
        })
    };
    let decodes = || super::super::agent_read::DECODE_COUNT.with(std::cell::Cell::get);
    let before = decodes();
    assert_eq!(find().unwrap().unwrap().operation_id, selected.operation_id);
    assert_eq!(decodes() - before, 1);
    assert_eq!(find().unwrap().unwrap().operation_id, selected.operation_id);
    assert_eq!(decodes() - before, 1);
    let external = rusqlite::Connection::open(directory.path().join("control.db")).unwrap();
    external
        .execute(
            "UPDATE operations SET operation_json='{}' WHERE operation_id=?1",
            [old.operation_id.as_str()],
        )
        .unwrap();
    assert_eq!(find().unwrap().unwrap().operation_id, selected.operation_id);
    assert!(
        read(&control).is_err(),
        "explicit whole-history reads remain strict"
    );
    assert!(
        control
            .find_verified_agent_operation(&WorkspaceId::default(), KINDS, |_| Ok(false))
            .is_err(),
        "no match must not skip a corrupt visited row"
    );
    external
        .execute(
            "UPDATE operation_steps SET step_json='{}' WHERE operation_id=?1 AND step_no=0",
            [selected.operation_id.as_str()],
        )
        .unwrap();
    assert!(
        find().is_err(),
        "a warm result cannot authorize changed current journal bytes"
    );
}

#[test]
fn newest_matching_fact_preserves_no_match_and_current_membership() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    complete(&control, "first", "");
    assert!(
        control
            .find_verified_agent_operation(&WorkspaceId::default(), KINDS, |_| Ok(false))
            .unwrap()
            .is_none()
    );
    let next = complete(&control, "next", "");
    let find =
        || control.find_verified_agent_operation(&WorkspaceId::default(), KINDS, |_| Ok(true));
    assert_eq!(find().unwrap().unwrap().operation_id, next.operation_id);
    let external = rusqlite::Connection::open(directory.path().join("control.db")).unwrap();
    external
        .execute(
            "UPDATE operations SET accepted_change_digest=?2 WHERE operation_id=?1",
            rusqlite::params![
                next.operation_id.as_str(),
                CanonicalDigest::of_bytes(b"substituted").as_str()
            ],
        )
        .unwrap();
    assert!(
        find().is_err(),
        "selected indexed identity must match the authenticated row"
    );
}

#[test]
fn current_lookup_does_not_keep_unmatched_history_alive_until_scan_end() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let oldest = complete(&control, "lookup-oldest", "");
    complete(&control, "lookup-newest", "");
    let history = read(&control).unwrap();
    let newest = Arc::downgrade(&history[0]);
    drop(history);
    let found = control
        .find_verified_agent_operation(&WorkspaceId::default(), KINDS, |fact| {
            if fact.operation_id == oldest.operation_id {
                let previous = newest.upgrade().unwrap();
                assert_eq!(
                    Arc::strong_count(&previous),
                    2,
                    "only the cache and this assertion may retain the unmatched fact"
                );
                return Ok(true);
            }
            Ok(false)
        })
        .unwrap()
        .unwrap();
    assert_eq!(found.operation_id, oldest.operation_id);
}

#[test]
fn current_match_replaces_old_full_cache_without_bulk_scan_eviction() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    complete(&control, "capacity-old", "");
    control.agent_history_cache.borrow_mut().budget_override = Some((1, usize::MAX));
    drop(read(&control).unwrap());
    let latest = complete(&control, "capacity-current", "");
    let current = control
        .find_verified_agent_operation(&WorkspaceId::default(), KINDS, |fact| {
            Ok(fact.operation_id == latest.operation_id)
        })
        .unwrap()
        .unwrap();
    for _ in 0..3 {
        let full = read(&control).unwrap();
        assert!(
            Arc::ptr_eq(&current, &full[0]),
            "bulk history must not evict the current fact"
        );
        let found = control
            .find_verified_agent_operation(&WorkspaceId::default(), KINDS, |fact| {
                Ok(fact.operation_id == latest.operation_id)
            })
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&current, &found));
    }
}

#[test]
fn compact_status_projection_still_authenticates_every_discarded_step() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let operation = complete(&control, "compact-projection", "retained-plan");
    let current = read(&control).unwrap();
    assert_eq!(
        current[0].grant_effects(),
        operation
            .step(hiroute_domain::OperationStepKind::ApplySecrets)
            .effects
    );
    for step in 0..6 {
        let db = control.connection.borrow();
        db.execute_batch("BEGIN").unwrap();
        db.execute(
            "UPDATE operation_steps SET step_json='{}' WHERE step_no=?1",
            [step],
        )
        .unwrap();
        drop(db);
        assert_eq!(
            read(&control).unwrap_err().code,
            PortErrorCode::Corrupt,
            "step {step}"
        );
        control
            .connection
            .borrow()
            .execute_batch("ROLLBACK")
            .unwrap();
        assert_eq!(read(&control).unwrap().len(), 1);
    }
}

#[test]
fn large_discarded_receipt_does_not_fill_the_status_projection_budget() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let mut operation = agent_operation(
        agent_transaction_plan_payload(AgentConnectionTransactionKindV1::Apply, None, ""),
        "compact-receipt",
    );
    persist_with_history_padding(&control, &mut operation, Some(&"p".repeat(2 * 1024 * 1024)));
    control.agent_history_cache.borrow_mut().budget_override = Some((1, 512 * 1024));
    let cold = read(&control).unwrap();
    let warm = read(&control).unwrap();
    assert!(
        Arc::ptr_eq(&cold[0], &warm[0]),
        "discarded compensation is not retained cache memory"
    );
    assert!(!format!("{:?}", cold[0]).contains("retained_history_fixture"));
    assert_eq!(cold[0].plan.spec(), operation.plan.spec());
    // Strict execution still returns the full original compensation.
    let strict = control
        .load_operation(&operation.operation_id)
        .unwrap()
        .unwrap();
    assert_eq!(strict.steps, operation.steps);
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
            "SELECT (SELECT sum(length(operation_json)) FROM operations) +
                    (SELECT sum(length(payload_json)) FROM operation_payloads)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(size > 21 * 1024 * 1024);
    let cold = read(&control).unwrap();
    assert_eq!(control.agent_history_cache.borrow().decodes, 21);
    assert_eq!(control.agent_history_cache.borrow().fingerprints, 21);
    // Model, collaboration and management reads across multiple Agents share
    // this same immutable history; filtering belongs after strict verification.
    for _ in 0..7 * 3 {
        let warm = read(&control).unwrap();
        assert_eq!(warm.len(), 21);
        assert!(cold.iter().zip(&warm).all(|(a, b)| Arc::ptr_eq(a, b)));
    }
    assert_eq!(control.agent_history_cache.borrow().decodes, 21);
    assert_eq!(
        control.agent_history_cache.borrow().fingerprints,
        21,
        "unchanged read snapshots must not reread/hash every retained payload per facet"
    );
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
    // Remove the complete isolated fixture closure, preserving FK enforcement.
    control
        .connection
        .borrow()
        .execute(
            "DELETE FROM operation_payloads WHERE operation_id=?1",
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
fn native_target_discovery_reads_only_current_targets_and_keeps_unfinished_tails() {
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
    assert_eq!(
        super::super::native_targets::TARGET_READS.with(std::cell::Cell::get),
        1
    );
    for _ in 0..3 {
        assert_eq!(
            control
                .agent_settings_native_targets(&WorkspaceId::default())
                .unwrap(),
            targets
        );
    }
    assert_eq!(
        super::super::native_targets::TARGET_READS.with(std::cell::Cell::get),
        4
    );

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

#[test]
fn native_target_heads_ignore_unreferenced_corruption_but_require_the_current_owner() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let path = directory.path().join("hermes-selected");
    let root = path.to_str().unwrap();
    let mut old = target_operation(root, "old-native-target", false);
    persist(&control, &mut old);
    let mut current = target_operation(root, "current-native-target", true);
    persist(&control, &mut current);
    control
        .connection
        .borrow()
        .execute(
            "UPDATE operations SET operation_json='invalid' WHERE operation_id=?1",
            [old.operation_id.as_str()],
        )
        .unwrap();
    assert_eq!(
        control
            .agent_settings_native_targets(&WorkspaceId::default())
            .unwrap()[0]
            .config_root(),
        root
    );
    control.validate_native_target_heads().unwrap();
    assert_eq!(read(&control).unwrap_err().code, PortErrorCode::Corrupt);
    control
        .connection
        .borrow()
        .execute(
            "UPDATE operations SET operation_json='invalid' WHERE operation_id=?1",
            [current.operation_id.as_str()],
        )
        .unwrap();
    assert!(control.validate_native_target_heads().is_err());
}

#[test]
fn native_target_coverage_and_current_owner_damage_never_become_empty_discovery() {
    for damage in [
        "DELETE FROM agent_native_target_coverage",
        "UPDATE agent_native_target_coverage SET store_uuid='wrong-store'",
        "DELETE FROM agent_native_target_heads",
        "UPDATE agent_native_target_heads SET context_id='wrong-context'",
    ] {
        let directory = tempdir().unwrap();
        let control = open(directory.path());
        let path = directory.path().join("hermes-selected");
        let mut current = target_operation(path.to_str().unwrap(), "coverage-native-target", false);
        persist(&control, &mut current);
        control.connection.borrow().execute_batch(damage).unwrap();
        assert!(
            control
                .agent_settings_native_targets(&WorkspaceId::default())
                .is_err(),
            "{damage}"
        );
    }
}

#[test]
fn legacy_native_target_initialization_preserves_journal_bytes_and_latest_reference() {
    let directory = tempdir().unwrap();
    let control = open(directory.path());
    let path = directory.path().join("hermes-selected");
    for key in ["legacy-first", "legacy-second"] {
        let mut operation = target_operation(path.to_str().unwrap(), key, false);
        persist(&control, &mut operation);
    }
    let bytes = |db: &rusqlite::Connection| {
        db.prepare("SELECT operation_json FROM operations ORDER BY rowid")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    // The supported source stores inline immutable input. Export the fixture into
    // that exact representation before exercising its semantic conversion.
    {
        let db = control.connection.borrow();
        let rows = db
            .prepare("SELECT operation_id,operation_json FROM operations ORDER BY rowid")
            .unwrap()
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for (id, encoded) in rows {
            let inline = super::super::payloads::expand(&db, &encoded).unwrap();
            db.execute(
                "UPDATE operations SET operation_json=?2 WHERE operation_id=?1",
                rusqlite::params![id, inline.as_str()],
            )
            .unwrap();
        }
    }
    let before = bytes(&control.connection.borrow());
    // Reproduce the format initializer's one-time semantic phase on retained data;
    // remove only metadata it owns, retaining all immutable journal/payload bytes.
    let db = control.connection.borrow();
    for outcome in ["ROLLBACK", "COMMIT"] {
        db.execute_batch(
        "BEGIN; DROP TRIGGER operation_recovery_insert; DROP TRIGGER operation_recovery_guard;
        DROP TRIGGER operation_recovery_update; DELETE FROM operation_recovery_members;
        DELETE FROM operation_recovery_coverage; DELETE FROM agent_native_target_heads;
        DELETE FROM agent_native_target_coverage; DELETE FROM publication_grant_proofs;
        DELETE FROM artifact_head_transitions; DELETE FROM artifact_heads; DELETE FROM artifact_head_coverage;",
    )
    .unwrap();
        super::super::current_dependencies::initialize(&db).unwrap();
        db.execute_batch(crate::migrations::current_dependencies_v32::TRIGGERS)
            .unwrap();
        db.execute_batch(outcome).unwrap();
        assert_eq!(
            db.query_row("SELECT head_count FROM artifact_head_coverage", [], |r| {
                r.get::<_, usize>(0)
            })
            .unwrap(),
            0, // Explicit native-target hints here declare no file effects.
        );
    }
    assert_eq!(bytes(&db), before);
    drop(db);
    assert_eq!(
        control
            .agent_settings_native_targets(&WorkspaceId::default())
            .unwrap()
            .len(),
        1
    );
}
