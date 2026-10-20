//! Current startup authority is strict; unrelated historical attribution is best effort.
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
        .to_string();
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
    // Simulate an old successful journal that the current subject/proof reader cannot
    // authenticate. No production record is rewritten by the repair.
    rusqlite::Connection::open(root.join("live/control.db"))
        .unwrap()
        .execute(
            "UPDATE operations SET operation_json=json_set(operation_json,
             '$.plan.control',json('{}')) WHERE operation_id=?1",
            [id.as_str()],
        )
        .unwrap();
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
    runtime.adapter.reconcile_startup_and_open().unwrap();
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
fn current_grant_and_activation_stay_strict_when_provenance_is_missing() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::grant_attribution_tests::current_grant_and_activation_stay_strict_when_provenance_is_missing",
    ) {
        return;
    }
    let (root, runtime, historical, current, _, _) = configured();
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
    invalidate_operation(root.path(), &historical);
    invalidate_operation(root.path(), &current);
    assert_eq!(
        runtime
            .adapter
            .restore_active_publication()
            .unwrap_err()
            .code,
        PortErrorCode::InvalidData,
        "a persisted mapping is not a substitute for current publication proof",
    );
}
