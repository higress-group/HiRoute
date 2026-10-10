use super::*;

fn wire(
    name: &str,
    payload: serde_json::Value,
) -> hiroute_application_api::LocalControlWireRequestV2 {
    hiroute_application_api::LocalControlWireRequestV2 {
        schema_version: hiroute_application_api::LOCAL_CONTROL_SCHEMA_V2,
        request_id: format!("checkpoint-{name}"),
        operation_id: name.into(),
        payload,
        protected_grant: None,
    }
}

#[test]
fn publication_checkpoint_wire_is_installed_cas_bound_replayable_and_recovery_gated() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::effects::tests::checkpoint::publication_checkpoint_wire_is_installed_cas_bound_replayable_and_recovery_gated",
    ) {
        return;
    }
    use crate::control::LocalControlDaemon;
    use crate::gateway_ports::GatewayPublicationAdapter;
    use hiroute_application::ApplicationService;
    use hiroute_gateway::server::publication::GatewayPublicationInstaller;
    let directory = tempfile::tempdir().unwrap();
    let runtime = super::super::super::ProductionControlRuntime::open_with_release_catalog(
        directory.path(),
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    let workspace = WorkspaceId::default();
    let publication = GatewayPublicationV1::new(
        workspace.clone(),
        GatewayPublicationRevision::new(1).unwrap(),
        hiroute_domain::AliasRegistryV1::default(),
        vec![],
    )
    .unwrap();
    let before = PublicationRecordV1::from_publication(workspace.clone(), &publication).unwrap();
    {
        let stores = runtime.adapter.stores_lock().unwrap();
        stores.control().prepare_publication(&before, None).unwrap();
        stores
            .control()
            .mark_publication_active(&workspace, before.publication_revision, &before.digest)
            .unwrap();
    }
    let service = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()));
    let change = json!({"schema": hiroute_domain::PUBLICATION_CHECKPOINT_CHANGE_SCHEMA_V1});
    let preview =
        || service.dispatch_wire(wire("PreviewAgentPlanChange", json!({"change":change})));
    assert!(
        preview().error.is_some(),
        "A stored publication without verified installation is insufficient"
    );
    let target = Arc::new(GatewayPublicationAdapter::new(Arc::new(
        GatewayPublicationInstaller::open(directory.path().join("gateway-lkg.json")).unwrap(),
    )));
    runtime
        .configure_managed_agent_runtime(
            "http://127.0.0.1:5837/v1".into(),
            "/test/hiroute".into(),
            Some(target),
        )
        .unwrap();
    let ready = preview();
    assert!(ready.error.is_none(), "{ready:?}");
    let ready = ready.data.unwrap();
    let apply = json!({"change":change, "accept_digest":ready["change_digest"], "expected_revisions":ready["expected_revisions"], "idempotency_key":"empty-checkpoint"});
    let accepted = service.dispatch_wire(wire("ApplyAgentPlanChange", apply.clone()));
    assert!(accepted.error.is_none(), "{accepted:?}");
    let accepted = accepted.data.unwrap();
    assert_eq!(accepted["state"], "succeeded", "{accepted}");
    let after = {
        let stores = runtime.adapter.stores_lock().unwrap();
        let control = stores.control();
        let after = control.active_publication(&workspace).unwrap().unwrap();
        assert_eq!(
            control.last_known_good_publication(&workspace).unwrap(),
            Some(before.clone())
        );
        assert_eq!(after.publication_revision.get(), 2);
        assert!(after.verify_current().unwrap().plans.is_empty());
        assert!(after.verify_current().unwrap().grants.is_empty());
        assert!(control.prepared_publication(&workspace).unwrap().is_none());
        after
    };
    let replay = service.dispatch_wire(wire("ApplyAgentPlanChange", apply.clone()));
    assert!(replay.error.is_none(), "{replay:?}");
    assert_eq!(
        replay.data.unwrap()["operation_id"],
        accepted["operation_id"]
    );
    let mut stale = apply;
    stale["idempotency_key"] = json!("new-key-stale-preview");
    assert!(
        service
            .dispatch_wire(wire("ApplyAgentPlanChange", stale))
            .error
            .is_some()
    );
    assert_eq!(
        runtime
            .adapter
            .stores_lock()
            .unwrap()
            .control()
            .active_publication(&workspace)
            .unwrap(),
        Some(after.clone())
    );
    // Recovery and another prepared publication continue to block a new checkpoint.
    runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .begin_plan_version_recovery()
        .unwrap();
    assert!(preview().error.is_some());
    runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .reconcile_plan_versions(&workspace, &[], 1)
        .unwrap();
    assert!(preview().error.is_none());
    let competing = hiroute_domain::checkpoint_publication(&after).unwrap();
    runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .prepare_publication(&competing, Some(after.publication_revision))
        .unwrap();
    assert!(preview().error.is_some());
}

pub(super) fn operation(
    adapter: &LocalControlAdapter,
    prior: GatewayPublicationV1,
    key: &str,
) -> (OperationV1, ExternalEffectIntentV1, PublicationRecordV1) {
    let (mut prior, _) = current_publication(prior);
    prior.publication_revision = GatewayPublicationRevision::new(10).unwrap();
    let workspace = WorkspaceId::default();
    let before = PublicationRecordV1::from_publication(workspace.clone(), &prior).unwrap();
    let revisions = {
        let stores = adapter.stores_lock().unwrap();
        stores.control().prepare_publication(&before, None).unwrap();
        stores
            .control()
            .mark_publication_active(&workspace, before.publication_revision, &before.digest)
            .unwrap();
        stores.control().current_revisions(&workspace).unwrap()
    };
    let plan = TransactionPlanV1::from_publication_checkpoint(ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: "routing.apply".into(), resource_id: Some("publication/current".into()),
        desired_state: json!({"schema":hiroute_domain::PUBLICATION_CHECKPOINT_CHANGE_SCHEMA_V1}),
    }, before).unwrap();
    let scope = IdempotencyScopeV1::new("interactive-user", "ApplyAgentPlanChange", key).unwrap();
    let request_digest = CanonicalDigest::of_bytes(key.as_bytes());
    let operation = OperationV1::new(
        OperationId::derive(&workspace, &scope, &request_digest),
        workspace,
        scope,
        request_digest,
        CanonicalDigest::of_bytes(b"checkpoint-accepted"),
        revisions,
        plan,
    )
    .unwrap();
    begin_operation_for_test(adapter, &operation);
    let intent = operation.plan.external()[0].clone();
    let record = hiroute_domain::routing_publication_record(&intent)
        .unwrap()
        .unwrap();
    (operation, intent, record)
}
