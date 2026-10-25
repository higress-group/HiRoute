//! Real stores and transaction entrypoints, with explicit native-capability fixtures.
use super::*;
use hiroute_domain::{AgentModelBudgetConflictV1, AgentModelBudgetReasonV1};

fn configure(fixture: &QoderFixture, agent: &str, mode: &str, plan: &str, key: &str) {
    let context = fixture
        .runtime
        .adapter
        .settings_context_for_agent(agent)
        .unwrap();
    apply(
        &fixture.service,
        &fixture.runtime,
        json!({
            "schema_version":{"major":2,"minor":0},"context_id":context,
            "model":{"intent":"configure","settings":{"mode":mode,"allowed_plan_ids":[plan]}}
        }),
        key,
    );
}

fn status(fixture: &QoderFixture, agent: &str) -> serde_json::Value {
    let context = fixture
        .runtime
        .adapter
        .settings_context_for_agent(agent)
        .unwrap();
    fixture
        .service
        .dispatch_wire(request(
            "GetAgentConnectionStatus",
            json!({
                "schema_version":{"major":2,"minor":0},"context_id":context
            }),
            None,
        ))
        .data
        .unwrap()
}

fn restore(fixture: &QoderFixture, agent: &str, key: &str) {
    let state = status(fixture, agent);
    let context = fixture
        .runtime
        .adapter
        .settings_context_for_agent(agent)
        .unwrap();
    apply(
        &fixture.service,
        &fixture.runtime,
        json!({
            "schema_version":{"major":2,"minor":0},"context_id":context,
            "model":{"intent":"restore","restore_point_ref":state["restore_point_ref"]}
        }),
        key,
    );
    assert_eq!(status(fixture, agent)["state"], "not_configured");
}

fn rejected(
    fixture: &QoderFixture,
    alias: &str,
) -> (OperationV1, PublicationRecordV1, AgentModelBudgetConflictV1) {
    let (operation, record) = models::budget_change_operation(fixture, Some(alias.into()), true);
    let error = fixture
        .runtime
        .adapter
        .validate_external_admission(&operation.plan.external()[0])
        .unwrap_err();
    assert_eq!(error.context, "agent.model.budget.conflict");
    let conflict = *error.model_budget_conflict.unwrap();
    assert!(conflict.is_valid());
    for blocker in &conflict.blockers {
        assert_ne!(blocker.agent_kind, AgentKindV1::Qoder);
        assert_eq!(blocker.model_alias.as_str(), alias);
        assert_eq!(blocker.reason, AgentModelBudgetReasonV1::CapacityReduced);
        let next = blocker.proposed.unwrap();
        assert_eq!(
            next.context_window_tokens,
            blocker.declared.context_window_tokens
        );
        assert_eq!(
            next.max_output_tokens + 1,
            blocker.declared.max_output_tokens
        );
    }
    (operation, record, conflict)
}

#[test]
fn output_shrink_names_all_live_consumers_and_preserves_checkpoint_details_after_reopen() {
    let tools = crate::test_support::private_tempdir();
    let node = tools.path().join("node");
    private_file(&node, b"#!/bin/sh\nif [ \"$1\" = --version ]; then echo v22.19.0; elif [ \"$2\" = --check ] && [ \"$4\" = models ]; then echo hiroute.pi-sdk-capability/v1:ok; else exit 97; fi\n");
    fs::set_permissions(&node, fs::Permissions::from_mode(0o700)).unwrap();
    let path =
        std::env::join_paths([tools.path(), Path::new("/usr/bin"), Path::new("/bin")]).unwrap();
    if crate::test_support::isolated_agent_home_with_path(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::budget_tests::output_shrink_names_all_live_consumers_and_preserves_checkpoint_details_after_reopen",
        &path,
    ) {
        return;
    }
    let fixture = QoderFixture::with_all_models();
    let published = fixture
        .runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap()
        .verify()
        .unwrap();
    let plans: Vec<_> = published
        .published_agent_plans()
        .unwrap()
        .into_iter()
        .filter(|plan| {
            plan.active
                && plan
                    .supported_ingress
                    .contains(&AgentIngressProtocolV1::Responses)
        })
        .collect();
    assert!(plans.len() >= 2);
    let free = plans[0].agent_plan_id.as_str();
    let other = plans[1].agent_plan_id.as_str();
    let alias = published
        .plans
        .iter()
        .find(|plan| plan.agent_plan_id().as_str() == free)
        .unwrap()
        .model_alias()
        .as_str()
        .to_owned();
    configure(
        &fixture,
        "agent_qoder_default",
        "qoder_additional",
        other,
        "budget-unrelated-qoder",
    );
    configure(
        &fixture,
        "agent_pi_default",
        "pi_additional",
        free,
        "budget-pi",
    );
    configure(
        &fixture,
        "agent_dsh_default",
        "dsh_additional",
        free,
        "budget-dsh",
    );
    // A new generation supersedes its historical declaration, without deleting history.
    configure(
        &fixture,
        "agent_pi_default",
        "pi_additional",
        free,
        "budget-pi-next-generation",
    );
    // Publication admission depends on the live declarations, not a replaced journal.
    // Preserve the exact old bytes for later explicit Restore/history assertions.
    let journal =
        rusqlite::Connection::open(fixture._root.path().join("storage/live/control.db")).unwrap();
    let (old_id, old_json): (String, String) = journal
        .query_row(
            "SELECT operation_id,operation_json FROM operations WHERE idempotency_key='budget-pi'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    journal
        .execute(
            "UPDATE operations SET operation_json='unrelated old damage' WHERE operation_id=?1",
            [&old_id],
        )
        .unwrap();
    let (candidate, _) = models::budget_change_operation(&fixture, Some(alias.clone()), true);
    let admission = fixture
        .runtime
        .adapter
        .validate_external_admission(&candidate.plan.external()[0]);
    journal
        .execute(
            "UPDATE operations SET operation_json=?2 WHERE operation_id=?1",
            rusqlite::params![old_id, old_json],
        )
        .unwrap();
    assert_eq!(
        admission.unwrap_err().context,
        "agent.model.budget.conflict",
        "superseded history must not replace current budget checks"
    );
    // Current file and grant proofs remain intact even when optional completed
    // service provenance is absent. Restore the exact fixture rows afterwards.
    journal.execute_batch("CREATE TEMP TABLE saved_optional_steps AS SELECT * FROM operation_steps
        WHERE operation_id=(SELECT operation_id FROM operations WHERE idempotency_key='budget-pi-next-generation')
        AND step_kind='activate';
        DELETE FROM operation_steps WHERE operation_id IN (SELECT operation_id FROM saved_optional_steps)
        AND step_kind='activate';").unwrap();
    let admission = fixture
        .runtime
        .adapter
        .validate_external_admission(&candidate.plan.external()[0]);
    journal
        .execute_batch(
            "INSERT INTO operation_steps SELECT * FROM saved_optional_steps;
        DROP TABLE saved_optional_steps;",
        )
        .unwrap();
    assert_eq!(
        admission.unwrap_err().context,
        "agent.model.budget.conflict",
        "optional service provenance must not disable current budget checks"
    );
    let (_, _, before) = rejected(&fixture, &alias);
    assert_eq!(
        before
            .blockers
            .iter()
            .map(|b| b.agent_kind)
            .collect::<Vec<_>>(),
        vec![AgentKindV1::Pi, AgentKindV1::DeepseekHarness]
    );
    restore(
        &fixture,
        "agent_qoder_default",
        "budget-stop-unrelated-qoder",
    );
    let (operation, _, after) = rejected(&fixture, &alias);
    assert_eq!(before, after);
    let native_before =
        ["agent_pi_default", "agent_dsh_default"].map(|agent| status(&fixture, agent));
    let published_before = fixture
        .runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap();

    // Inject only the accepted journal to independently exercise the Install checkpoint.
    fixture
        .runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .begin_local_operation(&operation)
        .unwrap();
    let adapter = fixture.runtime.adapter.as_ref();
    let rolled_back = hiroute_application::TransactionCoordinator::new(
        adapter,
        adapter,
        adapter,
        adapter,
        adapter,
        &adapter.admission,
    )
    .run(&operation.operation_id)
    .unwrap();
    assert_eq!(rolled_back.state, OperationState::RolledBack);
    assert_eq!(
        rolled_back.safe_error_code.as_deref(),
        Some("AGENT_MODEL_BUDGET_CONFLICT")
    );
    assert_eq!(rolled_back.model_budget_conflict(), Some(after.clone()));
    assert_eq!(
        ["agent_pi_default", "agent_dsh_default"].map(|agent| status(&fixture, agent)),
        native_before
    );
    assert_eq!(
        fixture
            .runtime
            .adapter
            .stores_lock()
            .unwrap()
            .control()
            .active_publication(&WorkspaceId::default())
            .unwrap(),
        published_before
    );
    // Cold decoding retains the frozen diagnostic and does not create new authorization.
    let reopened = fixture
        .runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .load_operation(&operation.operation_id)
        .unwrap()
        .unwrap();
    assert_eq!(reopened.model_budget_conflict(), Some(after.clone()));
    let lookup = json!({"principal_kind":"interactive_user","operation_kind":"ApplyAgentPlanChange",
        "idempotency_key":operation.idempotency.key,"accepted_digest":operation.accepted_digest});
    let legacy =
        fixture
            .service
            .dispatch_wire(request("FindOperationByIdempotency", lookup.clone(), None));
    assert!(legacy.error.is_none(), "{legacy:?}");
    assert!(
        legacy.data.as_ref().unwrap()["operation"]
            .get("model_budget_conflict")
            .is_none()
    );
    let mut detailed = lookup;
    detailed["include_model_budget_conflict"] = json!(true);
    let response =
        fixture
            .service
            .dispatch_wire(request("FindOperationByIdempotency", detailed, None));
    assert_eq!(
        response.data.unwrap()["operation"]["model_budget_conflict"],
        serde_json::to_value(&after).unwrap()
    );

    restore(&fixture, "agent_pi_default", "budget-stop-pi");
    let (_, _, remaining) = rejected(&fixture, &alias);
    assert_eq!(remaining.blockers.len(), 1);
    assert_eq!(
        remaining.blockers[0].agent_kind,
        AgentKindV1::DeepseekHarness
    );
    restore(&fixture, "agent_dsh_default", "budget-stop-dsh");
    let (operation, _) = models::budget_change_operation(&fixture, Some(alias), true);
    fixture
        .runtime
        .adapter
        .validate_external_admission(&operation.plan.external()[0])
        .unwrap();
    let scope = hiroute_domain::IdempotencyScopeV1::new(
        "interactive-user",
        "ApplyAgentPlanChange",
        "budget-after-all-consumers-removed",
    )
    .unwrap();
    let digest = CanonicalDigest::of_bytes(b"budget-after-all-consumers-removed");
    let final_operation = OperationV1::new(
        OperationId::derive(&WorkspaceId::default(), &scope, &digest),
        WorkspaceId::default(),
        scope,
        digest.clone(),
        digest,
        operation.expected_revisions.clone(),
        (*operation.plan).clone(),
    )
    .unwrap();
    fixture
        .runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .begin_local_operation(&final_operation)
        .unwrap();
    let installed = hiroute_application::TransactionCoordinator::new(
        adapter,
        adapter,
        adapter,
        adapter,
        adapter,
        &adapter.admission,
    )
    .run(&final_operation.operation_id)
    .unwrap();
    assert_eq!(installed.state, OperationState::Succeeded, "{installed:#?}");
    assert!(
        !fixture
            .runtime
            .adapter
            .stores_lock()
            .unwrap()
            .control()
            .succeeded_agent_operations_for_kind(
                &WorkspaceId::default(),
                "ApplyAgentConnectionChange"
            )
            .unwrap()
            .is_empty()
    );
}
