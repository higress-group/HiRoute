//! Attribution recovery is best effort; publication activation remains strict.
use super::*;
use hiroute_domain::{ContentMode, ObservationQueryPort, PortErrorCode, SessionId};

fn configured() -> (
    tempfile::TempDir,
    ProductionControlRuntime,
    OperationId,
    OperationId,
    hiroute_domain::AgentAccessGrantRefV1,
    String,
) {
    let root = tempfile::tempdir().unwrap();
    let (runtime, _) = open_with_codex_fixture_surfaces(root.path());
    ensure_target_cache(&runtime.adapter);
    runtime.adapter.reconcile_startup_and_open().unwrap();
    let service = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()));
    let context = runtime
        .adapter
        .settings_context(crate::control::runtime::settings_facts::SettingsAgentClass::Codex);
    let publication = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap()
        .verify()
        .unwrap();
    let plans = publication.published_agent_plans().unwrap();
    let plan = plans
        .iter()
        .find(|plan| {
            plan.active
                && plan
                    .supported_ingress
                    .contains(&AgentIngressProtocolV1::Responses)
        })
        .unwrap()
        .agent_plan_id
        .as_str()
        .to_owned();
    let spec = json!({
        "schema_version":{"major":2,"minor":0}, "context_id":context,
        "model":{"intent":"configure","settings":{
            "mode":"codex_default","native_model_mode":"hiroute_only","fixed_models":[],
            "allowed_plan_ids":[plan],
            "default_selection":{"kind":"plan","plan_id":plan}
        }}
    });
    let first = apply(&service, &runtime, spec.clone(), "attribution-old");
    let last = apply(&service, &runtime, spec, "attribution-current");
    let grant = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .secrets()
        .inspect_agent_access_grant(WorkspaceId::DEFAULT, &format!("agent-connection/{context}"))
        .unwrap()
        .unwrap();
    (
        root,
        runtime,
        OperationId::parse(first["operation_id"].as_str().unwrap()).unwrap(),
        OperationId::parse(last["operation_id"].as_str().unwrap()).unwrap(),
        grant,
        plan,
    )
}

fn invalidate_operation(root: &std::path::Path, id: &OperationId) {
    // Remove a historical ensure receipt while keeping immutable command inputs valid.
    // The attribution reader cannot authenticate this journal; ordinary operation
    // decoding still succeeds. No production record is rewritten by the repair.
    rusqlite::Connection::open(root.join("live/control.db"))
        .unwrap()
        .execute(
            "UPDATE operations SET operation_json=json_set(operation_json,
             '$.steps[1].effects',json('[]')) WHERE operation_id=?1",
            [id.as_str()],
        )
        .unwrap();
}

fn clear_grant_mappings(root: &std::path::Path) {
    rusqlite::Connection::open(root.join("observation/activity.db"))
        .unwrap()
        .execute("DELETE FROM observation_agent_grants_v1", [])
        .unwrap();
}

fn assert_unlinked_session(runtime: &ProductionControlRuntime, suffix: &str) {
    let session = runtime
        .adapter
        .delegation_observation
        .get_session(
            &WorkspaceId::default(),
            &SessionId::parse(format!("session-{suffix}")).unwrap(),
            ContentMode::None,
        )
        .unwrap();
    assert!(session.summary.agent_id.is_empty());
}

#[test]
fn startup_skips_unverifiable_history_without_backfilling_old_sessions() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::grant_attribution_tests::startup_skips_unverifiable_history_without_backfilling_old_sessions",
    ) {
        return;
    }
    let (root, runtime, historical, _, grant, plan) = configured();
    invalidate_operation(root.path(), &historical);
    assert!(
        runtime
            .adapter
            .stores_lock()
            .unwrap()
            .control()
            .load_operation(&historical)
            .unwrap()
            .is_some(),
        "the fixture must not corrupt the independent immutable-operation contract"
    );
    // This was the old all-or-nothing startup query. Its failure must not disappear
    // from strict execution/recovery callers when attribution becomes best effort.
    assert_eq!(
        runtime
            .adapter
            .stores_lock()
            .unwrap()
            .control()
            .succeeded_agent_operations_for_kind(
                &WorkspaceId::default(),
                "ApplyAgentConnectionChange",
            )
            .unwrap_err()
            .code,
        PortErrorCode::Corrupt,
    );
    rusqlite::Connection::open(root.path().join("observation/activity.db"))
        .unwrap()
        .execute("DELETE FROM observation_agent_grants_v1", [])
        .unwrap();
    record_attributed_session(&runtime, &grant, "unmapped-before-recovery", &plan);
    let scanner = runtime.adapter.scanner.clone();
    drop(runtime);
    let runtime = open_with_scanner(root.path(), scanner);
    let diagnostics = hiroute_diagnostics::runtime::DiagnosticRuntime::start(
        hiroute_diagnostics::runtime::RuntimeConfig {
            root: root.path().join("diagnostic-fixture"),
            role: hiroute_diagnostics::event::ProcessRole::Daemon,
            component: hiroute_diagnostics::record::Component::Daemon,
            parent_session_id: None,
            level_override: Some(hiroute_diagnostics::DiagnosticLevel::Info),
        },
    );
    runtime.set_diagnostics(diagnostics.port());
    // Match the production startup sequence. Coordinator reconciliation alone
    // does not restore the publication or register its observation ownership.
    runtime.adapter.restore_active_publication().unwrap();
    runtime.adapter.reconcile_startup_and_open().unwrap();
    runtime
        .adapter
        .finish_startup_publication_recovery()
        .unwrap();
    record_attributed_session(&runtime, &grant, "mapped-after-recovery", &plan);
    assert_attributed_session(&runtime, "mapped-after-recovery");
    let old = runtime
        .adapter
        .delegation_observation
        .get_session(
            &WorkspaceId::default(),
            &SessionId::parse("session-unmapped-before-recovery").unwrap(),
            ContentMode::None,
        )
        .unwrap();
    assert!(old.summary.agent_id.is_empty());
    diagnostics.shutdown();
    let log =
        fs::read_to_string(root.path().join("diagnostic-fixture/daemon/current.jsonl")).unwrap();
    let warnings = log
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|record| record["event"]["projection_rejected"]["channel"] == "control")
        .collect::<Vec<_>>();
    assert!(!warnings.is_empty() && warnings.len() <= 2, "{log}");
    assert!(
        warnings
            .iter()
            .all(|record| record["event"]["projection_rejected"]
                == json!({"channel":"control","reason":"invalid"}))
    );
}

#[test]
fn startup_missing_provenance_is_unlinked_while_activation_stays_strict() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::grant_attribution_tests::startup_missing_provenance_is_unlinked_while_activation_stays_strict",
    ) {
        return;
    }
    let (root, runtime, historical, current, grant, plan) = configured();
    let (operation, publication) = {
        let stores = runtime.adapter.stores_lock().unwrap();
        (
            stores.control().load_operation(&current).unwrap().unwrap(),
            stores
                .control()
                .active_publication(&WorkspaceId::default())
                .unwrap()
                .unwrap()
                .verify()
                .unwrap(),
        )
    };
    assert!(runtime.adapter.register_operation_agent_grants(
        &WorkspaceId::default(), &operation, &[], &publication,
    ).is_err(), "activation must reject a missing ensure effect");
    let journal = rusqlite::Connection::open(root.path().join("live/control.db")).unwrap();
    let original: Vec<(String, String)> = [&historical, &current]
        .into_iter()
        .map(|id| {
            let encoded = journal
                .query_row(
                    "SELECT operation_json FROM operations WHERE operation_id=?1",
                    [id.as_str()],
                    |row| row.get(0),
                )
                .unwrap();
            (id.as_str().to_owned(), encoded)
        })
        .collect();
    drop(journal);
    invalidate_operation(root.path(), &historical);
    invalidate_operation(root.path(), &current);
    clear_grant_mappings(root.path());
    let scanner = runtime.adapter.scanner.clone();
    drop(runtime);
    let runtime = open_with_scanner(root.path(), scanner);
    runtime.adapter.restore_active_publication().unwrap();
    runtime.adapter.reconcile_startup_and_open().unwrap();
    runtime
        .adapter
        .finish_startup_publication_recovery()
        .unwrap();
    record_attributed_session(&runtime, &grant, "missing-current-proof", &plan);
    assert_unlinked_session(&runtime, "missing-current-proof");

    // Restore the exact original completed proof bytes in this isolated fixture.
    // This models proof becoming available, not a production journal repair.
    let journal = rusqlite::Connection::open(root.path().join("live/control.db")).unwrap();
    for (id, encoded) in original {
        journal
            .execute(
                "UPDATE operations SET operation_json=?2 WHERE operation_id=?1",
                rusqlite::params![id, encoded],
            )
            .unwrap();
    }
    drop(journal);
    runtime.adapter.restore_active_publication().unwrap();
    record_attributed_session(&runtime, &grant, "new-current-proof", &plan);
    assert_attributed_session(&runtime, "new-current-proof");
    assert_unlinked_session(&runtime, "missing-current-proof");
}

#[test]
fn parked_native_tail_requires_a_current_authenticated_service_receipt() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::grant_attribution_tests::parked_native_tail_requires_a_current_authenticated_service_receipt",
    ) {
        return;
    }
    let (root, runtime, historical, current, grant, plan) = configured();
    invalidate_operation(root.path(), &historical);
    let mut operation = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .load_operation(&current)
        .unwrap()
        .unwrap();
    // Preserve the real completed service receipt and exact grant proof, but leave the
    // native-file tail parked as production recovery does after an independent user edit.
    operation.state = hiroute_domain::OperationState::Activating;
    operation.step_mut(OperationStepKind::Activate).status =
        hiroute_domain::OperationStepStatus::Started;
    operation.safe_error_code = Some("SETTINGS_TAIL_PENDING".into());
    runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .save_operation(&mut operation)
        .unwrap();
    rusqlite::Connection::open(root.path().join("observation/activity.db"))
        .unwrap()
        .execute("DELETE FROM observation_agent_grants_v1", [])
        .unwrap();
    runtime.adapter.restore_active_publication().unwrap();
    record_attributed_session(&runtime, &grant, "parked-service-proof", &plan);
    assert_attributed_session(&runtime, "parked-service-proof");

    // This unit fixture rewinds a completed journal solely to probe attribution.
    // It does not rewind the already committed file-head transition, so it is not
    // a restartable crash image. Real pre-receipt crash/restart and post-recovery
    // attribution are exercised by publication_process's after_target/after_active
    // Agent settings cases, using the actual unfinished transaction.
    let service_receipt = operation
        .step(OperationStepKind::Activate)
        .terminal_result
        .clone();
    operation
        .step_mut(OperationStepKind::Activate)
        .terminal_result = None;
    runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .save_operation(&mut operation)
        .unwrap();
    runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .reclaim_operation_writer(&current)
        .unwrap();
    clear_grant_mappings(root.path());
    runtime.adapter.restore_active_publication().unwrap();
    record_attributed_session(&runtime, &grant, "parked-without-service-proof", &plan);
    assert_unlinked_session(&runtime, "parked-without-service-proof");
    // Restore only the exact original service receipt in this attribution fixture.
    // Availability of valid proof authorizes future writes, never retroactive identity.
    operation
        .step_mut(OperationStepKind::Activate)
        .terminal_result = service_receipt;
    runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .save_operation(&mut operation)
        .unwrap();
    runtime.adapter.restore_active_publication().unwrap();
    record_attributed_session(&runtime, &grant, "parked-proof-restored", &plan);
    assert_attributed_session(&runtime, "parked-proof-restored");
    assert_unlinked_session(&runtime, "parked-without-service-proof");
}
