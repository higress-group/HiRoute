use super::*;

fn additional_grant_operation(
    key: &str,
    regenerate: bool,
) -> (
    OperationV1,
    hiroute_domain::AgentAccessGrantRefV1,
    hiroute_domain::OwnedEffectV1,
) {
    use hiroute_domain::{
        AgentAccessGrantMaterialActionV1, AgentAccessGrantMutationV1, AgentAccessGrantRefV1,
        AgentAccessGrantScopeV1, AgentIngressProtocolV1, AgentModelGrantV2, AgentModelRouteV2,
        AgentPlanId, ModelAlias, OwnedEffectKind,
    };
    let context = "agent-context/qoder-test";
    let connection = format!("agent-connection/{context}");
    let plan_id = AgentPlanId::parse("plan/locator-test").unwrap();
    let alias = ModelAlias::parse("hiroute/0123456789abcdef").unwrap();
    let route = AgentModelRouteV2::Plan {
        plan_id: plan_id.clone(),
        alias: alias.clone(),
        revision: 1,
        semantic_digest: CanonicalDigest::of_bytes(b"locator-route"),
    };
    let model_grant = AgentModelGrantV2::seal(
        AgentIngressProtocolV1::Responses,
        [(alias.as_str().to_owned(), route)].into_iter().collect(),
    )
    .unwrap();
    let scope = AgentAccessGrantScopeV1::new(&connection, model_grant).unwrap();
    let mutation = AgentAccessGrantMutationV1::ensure(WorkspaceId::DEFAULT, scope.clone(), 7)
        .unwrap()
        .with_material_action(if regenerate {
            AgentAccessGrantMaterialActionV1::Regenerate
        } else {
            AgentAccessGrantMaterialActionV1::Preserve
        })
        .unwrap();
    let spec = ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: "agents.settings.apply".into(),
        resource_id: Some(context.into()),
        desired_state: json!({"schema_version":{"major":2,"minor":0},"context_id":context,
            "model":{"intent":"configure","settings":{"mode":"qoder_additional", "allowed_plan_ids":[plan_id]}},
            "access_token":{"intent":if regenerate {"regenerate"} else {"keep"}}}),
    };
    let control = AgentConnectionControlIntentV1::from_settings_planner(
        AgentConnectionTransactionSubjectV1::from_registered_profile(
            "agent_qoder_default",
            "qoder-collaboration-v1",
            "builtin/qoder-collaboration/v1",
        )
        .unwrap(),
        &spec,
        false,
        &json!({"revision":1}),
    )
    .unwrap();
    let external = [
        AgentConnectionEffectRoleV1::GrantScopedPublication,
        AgentConnectionEffectRoleV1::ManagedConfiguration,
    ]
    .into_iter()
    .map(|role| {
        ExternalEffectIntentV1::from_agent_connection_planner(
            &control,
            role,
            None,
            &json!({"context_id":context,"fixture":"locator"}),
            0o644,
        )
        .unwrap()
    })
    .collect();
    let plan = TransactionPlanV1::from_agent_connection_planner_with_agent_access_grants(
        spec,
        control,
        vec![mutation],
        external,
    )
    .unwrap();
    let operation = agent_operation(plan, key);
    let material = CanonicalDigest::of_bytes(if regenerate {
        &b"rotated"[..]
    } else {
        &b"preserved"[..]
    });
    let generation = if regenerate { 8 } else { 7 };
    let grant_id = if regenerate {
        "grant_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    } else {
        "grant_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    };
    let reference = AgentAccessGrantRefV1::new(
        grant_id,
        WorkspaceId::DEFAULT,
        scope.clone(),
        generation,
        material.clone(),
    )
    .unwrap();
    let effect = hiroute_domain::OwnedEffectV1 {
        effect_id: format!("agent-access-grant:{connection}"),
        kind: OwnedEffectKind::Secret,
        target: connection.clone(),
        before_fingerprint: None,
        after_fingerprint: Some(material.clone()),
        compensation: json!({"schema":hiroute_domain::AGENT_ACCESS_GRANT_EFFECT_SCHEMA_V1,
            "operation_id":operation.operation_id,"connection_id":connection,"owner_scope":WorkspaceId::DEFAULT,
            "grant_id":grant_id,"generation":generation,"scope_hash":scope.digest().unwrap(),
            "material_sha256":material}).into(),
    };
    (operation, reference, effect)
}

fn persist_with_grant_effect(
    control: &ControlStore,
    operation: &mut OperationV1,
    effect: hiroute_domain::OwnedEffectV1,
) {
    let token = format!("capability-{}", operation.idempotency.key);
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
    operation.steps[1].effects.push(effect);
    add_component_file_receipts(operation);
    control.finish_operation(operation).unwrap();
    control.complete_operation_finalization(operation).unwrap();
}

#[test]
fn locator_grant_receipt_binds_identity_material_and_operation() {
    use super::super::super::{agent_history_cache::AgentHistoryRow, agent_locator_input};
    for regenerate in [false, true] {
        let directory = tempdir().unwrap();
        let control = open(directory.path());
        let (mut operation, expected, effect) =
            additional_grant_operation("grant-locator", regenerate);
        persist_with_grant_effect(&control, &mut operation, effect);
        let db = control.connection.borrow();
        let row = || {
            db.query_row(
            "SELECT operation_id,workspace_id,principal,operation_kind,idempotency_key,request_digest,accepted_change_digest,state,generation,operation_json FROM operations WHERE operation_id=?1",
            [operation.operation_id.as_str()], AgentHistoryRow::read,
        ).unwrap()
        };
        let input = agent_locator_input::classify(&*db, &row())
            .unwrap()
            .unwrap();
        assert_eq!(
            agent_locator_input::grant_reference(&db, &row(), &input).unwrap(),
            expected
        );
        for (field, bad) in [
            (
                "operation_id",
                "op_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            ),
            (
                "scope_hash",
                CanonicalDigest::of_bytes(b"wrong-scope").to_string(),
            ),
            (
                "material_sha256",
                CanonicalDigest::of_bytes(b"wrong-material").to_string(),
            ),
        ] {
            db.execute_batch("BEGIN").unwrap();
            let journal_path = format!("$.steps[1].effects[0].compensation.{field}");
            let indexed_path = format!("$.step.effects[0].compensation.{field}");
            db.execute("UPDATE operations SET operation_json=json_set(operation_json,?2,?3) WHERE operation_id=?1",
                rusqlite::params![operation.operation_id.as_str(), journal_path, bad]).unwrap();
            db.execute("UPDATE operation_steps SET step_json=json_set(step_json,?2,?3) WHERE operation_id=?1 AND step_no=1",
                rusqlite::params![operation.operation_id.as_str(), indexed_path, bad]).unwrap();
            let changed = row();
            let current = agent_locator_input::classify(&*db, &changed)
                .unwrap()
                .unwrap();
            assert!(
                agent_locator_input::grant_reference(&db, &changed, &current).is_err(),
                "{field}"
            );
            db.execute_batch("ROLLBACK").unwrap();
        }
    }
}
