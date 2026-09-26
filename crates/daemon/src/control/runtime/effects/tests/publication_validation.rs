use super::*;

use hiroute_application::control::RoutingFactsPort;
use hiroute_application::routing::preview_plan_content;

fn routing_fixture_catalog() -> hiroute_integrations::TrustedReleaseCatalog {
    crate::release_catalog::routable_discovery_fixture_catalog()
}

fn install_routable_compute_fixture(
    runtime: &super::super::super::ProductionControlRuntime,
    catalog: &hiroute_integrations::TrustedReleaseCatalog,
) {
    let candidate = catalog
        .authorize_compute_discovery(hiroute_integrations::RegisteredComputeDiscoveryFactV1 {
            agent_id: "agent.claude-code".into(),
            scanner_id: "scanner.claude.v1".into(),
            scanner_version: "1".into(),
            discovered_source_ref: "claude/settings/publication-recovery".into(),
            configuration_revision: 1,
            connection_option_id: "zhipu.coding-plan.cn.v1".into(),
            endpoint_profile_id: "endpoint.zhipu.coding-plan.cn.v1".into(),
            endpoint_profile_revision: 1,
            registered_base_url: "https://open.bigmodel.cn/api/anthropic".into(),
            observed_model_id: "glm-5.3".into(),
            model_configuration_id: "model.zhipu.glm-5.3".into(),
            protected_credential_available: true,
        })
        .unwrap();
    let projection = catalog
        .prepare_compute_projection(
            candidate,
            hiroute_domain::ComputeProjectionExpectationV1 {
                source_revision: 0,
                source_digest: None,
                binding_revision: 0,
                binding_digest: None,
                inventory_revision: 0,
                inventory_digest: None,
            },
            true,
        )
        .unwrap()
        .desired;
    let stores = runtime.adapter.stores_lock().unwrap();
    let control = stores.control();
    control
        .put_compute_source(0, &projection.source, catalog.registry(), true)
        .unwrap();
    control
        .put_source_binding(
            0,
            &projection.binding,
            catalog.registry(),
            catalog.model_data(),
        )
        .unwrap();
    control
        .put_inventory_snapshot(&projection.inventory)
        .unwrap();
    let pool_identity = projection.credential_pool_identity.as_ref().unwrap();
    let credential = hiroute_domain::CredentialRefV1::new(
        "credential/publication-recovery",
        format!("source/{}", projection.source.source_id),
        "hirouted",
        "provider-auth",
        [format!(
            "connection-option/{}",
            projection.source.connection_option_id
        )],
        1,
    )
    .unwrap();
    let pool = pool_identity
        .materialize_first(
            credential,
            CanonicalDigest::of_bytes(b"publication-recovery-credential"),
        )
        .unwrap();
    control
        .put_credential_pool(0, &pool, catalog.registry(), catalog.model_data())
        .unwrap();
}

#[test]
fn publication_recovery_rejects_corrupt_markers_and_upgrades_verified_legacy() {
    use crate::gateway_ports::GatewayPublicationAdapter;
    use hiroute_gateway::server::publication::GatewayPublicationInstaller;
    use std::sync::Arc;

    if crate::test_support::isolated_agent_home(
        "control::runtime::effects::tests::publication_validation::publication_recovery_rejects_corrupt_markers_and_upgrades_verified_legacy",
    ) {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let runtime = super::super::super::ProductionControlRuntime::open_with_release_catalog(
        directory.path(),
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    let target = Arc::new(GatewayPublicationAdapter::new(Arc::new(
        GatewayPublicationInstaller::open(directory.path().join("gateway-lkg.json")).unwrap(),
    )));
    *runtime.adapter.publication_target.lock().unwrap() = Some(target);
    let mut desired = GatewayPublicationV1::decode_persisted(include_bytes!(
        "../../../../../../../e2e/product/golden/routing/compiled-publication.v2.json"
    ))
    .unwrap();
    desired.aliases.clear();
    desired.grants.clear();
    let (mut operation, intent, record) =
        routing_operation(&runtime.adapter, desired, None, "marker-validation");
    let effect = runtime.adapter.apply_external(&operation, &intent).unwrap();
    operation
        .step_mut(OperationStepKind::CompilePublication)
        .effects = vec![effect.clone()];
    runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .save_operation(&mut operation)
        .unwrap();
    let invalid_values = [
        ("schema", json!("hiroute.publication-effect/v999")),
        (
            "after_digest",
            json!(CanonicalDigest::of_bytes(b"different-record")),
        ),
        ("operation_id", json!("operation/not-the-owner")),
        ("publication_revision", json!(999)),
        ("record", serde_json::Value::Null),
    ];
    for (field, value) in invalid_values {
        let mut corrupt = effect.clone();
        std::sync::Arc::make_mut(&mut corrupt.compensation)[field] = value;
        assert!(
            runtime
                .adapter
                .activate_external(&operation, &corrupt)
                .is_err(),
            "{field}"
        );
        assert!(
            runtime
                .adapter
                .prepare_publication_rollback(&operation, &corrupt)
                .is_err(),
            "{field}"
        );
        let stores = runtime.adapter.stores_lock().unwrap();
        assert!(
            stores
                .control()
                .active_publication(&WorkspaceId::default())
                .unwrap()
                .is_none()
        );
        assert_eq!(
            stores
                .control()
                .prepared_publication(&WorkspaceId::default())
                .unwrap(),
            Some(record.clone())
        );
    }
    let mut legacy = effect;
    std::sync::Arc::make_mut(&mut legacy.compensation)["schema"] =
        json!("hiroute.product-publication-marker/v1");
    std::sync::Arc::make_mut(&mut legacy.compensation)
        .as_object_mut()
        .unwrap()
        .remove("record");
    std::sync::Arc::make_mut(&mut legacy.compensation)
        .as_object_mut()
        .unwrap()
        .remove("decision");
    operation
        .step_mut(OperationStepKind::CompilePublication)
        .effects = vec![legacy.clone()];
    runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .save_operation(&mut operation)
        .unwrap();
    let normalized = match runtime
        .adapter
        .observe_external(&operation, &intent)
        .unwrap()
    {
        EffectReconciliation::Staged(effect) => effect,
        _ => panic!("expected staged normalized marker"),
    };
    assert_eq!(
        normalized.compensation["schema"],
        "hiroute.product-publication-marker/v2"
    );
    assert_eq!(normalized.compensation["decision"], "prepared");
    assert_eq!(
        normalized.compensation["record"],
        serde_json::to_value(&record).unwrap()
    );
    let upgraded = runtime
        .adapter
        .prepare_publication_activation(&operation, &legacy)
        .unwrap();
    assert_eq!(
        upgraded.compensation["schema"],
        "hiroute.product-publication-marker/v2"
    );
    assert_eq!(upgraded.compensation["decision"], "install");
    operation
        .step_mut(OperationStepKind::CompilePublication)
        .effects = vec![upgraded.clone()];
    runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .save_operation(&mut operation)
        .unwrap();
    runtime
        .adapter
        .activate_external(&operation, &upgraded)
        .unwrap();
    assert_eq!(
        runtime
            .adapter
            .stores_lock()
            .unwrap()
            .control()
            .active_publication(&WorkspaceId::default())
            .unwrap(),
        Some(record)
    );
}

#[test]
fn master_authored_v2_plan_is_recovered_from_durable_compiler_v1_publication() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::effects::tests::publication_validation::master_authored_v2_plan_is_recovered_from_durable_compiler_v1_publication",
    ) {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let workspace = WorkspaceId::default();
    let catalog = routing_fixture_catalog();
    let runtime = super::super::super::ProductionControlRuntime::open_with_release_catalog(
        directory.path(),
        catalog.clone(),
    )
    .unwrap();
    install_routable_compute_fixture(&runtime, &catalog);
    let facts =
        RoutingFactsPort::routing_compilation_snapshot(runtime.adapter.as_ref(), &workspace)
            .unwrap();
    let binding_id = facts
        .facts
        .candidates
        .iter()
        .find(|candidate| candidate.is_routable())
        .unwrap()
        .binding
        .binding_id
        .clone();
    let create: PlanContentChangeV2 = serde_json::from_value(json!({
        "schema": PLAN_CONTENT_CHANGE_SCHEMA_V2,
        "target": {"intent": "create", "creation_key": "master-v2-plan-recovery"},
        "editor": {
            "schema": hiroute_domain::PLAN_EDITOR_SCHEMA_V2,
            "display_name": "Master recovery plan",
            "purpose": "Persist the actual authoring compiler output before aggregate convergence",
            "mode": "fixed_model",
            "candidates": [{"binding_id": binding_id}],
            "smart": {"economy": [], "primary": [], "primary_fallback": false, "reselect_on_user_message": false, "classifier": {"kind":"local_rules"}, "complex_keywords": []},
            "free": {"candidates": [], "primary": [], "primary_fallback": false},
            "delegation_enabled": false,
            "requirements": {},
            "limits": {"maximum_attempts": 2, "request_timeout_ms": 60000, "attempt_timeout_ms": 30000}
        },
        "consumed_draft": null
    }))
    .unwrap();
    let authoring =
        RoutingFactsPort::plan_authoring_snapshot(runtime.adapter.as_ref(), &workspace, &create)
            .unwrap();
    let authored = preview_plan_content(&create, &authoring).unwrap();
    assert_eq!(
        authored.plan_version.compiled.body.schema,
        hiroute_domain::AGENT_PLAN_COMPILED_SCHEMA_V2
    );
    assert_eq!(
        authored.plan_version.compiled.body.compiler_revision,
        hiroute_domain::AGENT_PLAN_COMPILER_REVISION_V2
    );

    // Reproduce master's bootstrap writer: its empty V2/compiler-V1 seed was promoted to V3,
    // but `preview_plan_content` supplied the already-current compiled Plan V2 unchanged.
    let legacy_schema = GatewayPublicationV1::decode_persisted(include_bytes!(
        "../../../../../../../e2e/product/golden/routing/compiled-publication.v2.json"
    ))
    .unwrap()
    .schema;
    let revision_one = GatewayPublicationRevision::new(1).unwrap();
    let mut seed = GatewayPublicationV1::new(
        workspace.clone(),
        revision_one,
        hiroute_domain::AliasRegistryV1::default(),
        vec![],
    )
    .unwrap();
    seed.schema = legacy_schema;
    seed.compiler_revision = hiroute_domain::AGENT_PLAN_COMPILER_REVISION_V1.into();
    seed.validate().unwrap();
    let mut aliases = seed.alias_registry.clone();
    assert_eq!(
        aliases
            .allocate_named(
                authored.plan_head.reference.plan_id.clone(),
                authored.plan_version.configuration.display_name.as_str(),
            )
            .unwrap(),
        authored.plan_head.model_alias
    );
    let mut historical = seed;
    historical.schema = hiroute_domain::GATEWAY_PUBLICATION_SCHEMA_V3.into();
    historical.alias_registry = aliases;
    historical.plans = vec![authored.plan_version.compiled.clone()];
    historical.plan_heads = vec![authored.plan_head.clone()];
    historical.aliases.clear();
    historical.validate().unwrap();
    let historical_record =
        PublicationRecordV1::from_publication(workspace.clone(), &historical).unwrap();
    let historical_bytes = historical_record.bytes.clone();
    {
        let stores = runtime.adapter.stores_lock().unwrap();
        stores
            .control()
            .prepare_publication(&historical_record, None)
            .unwrap();
        stores
            .control()
            .mark_publication_active(
                &workspace,
                historical_record.publication_revision,
                &historical_record.digest,
            )
            .unwrap();
    }
    drop(runtime);

    let gateway_lkg = directory.path().join("gateway-lkg.json");
    let recovered = super::super::super::ProductionControlRuntime::prepare_for_role_all(
        directory.path(),
        catalog.clone(),
        None,
    )
    .unwrap();
    let target = std::sync::Arc::new(crate::gateway_ports::GatewayPublicationAdapter::new(
        std::sync::Arc::new(
            hiroute_gateway::server::publication::GatewayPublicationInstaller::open(&gateway_lkg)
                .unwrap(),
        ),
    ));
    recovered
        .configure_managed_agent_runtime(
            "http://127.0.0.1:5837/v1".into(),
            "/test/hiroute".into(),
            Some(target),
        )
        .unwrap();
    assert_eq!(
        recovered
            .adapter
            .stores_lock()
            .unwrap()
            .control()
            .active_publication(&workspace)
            .unwrap(),
        Some(historical_record.clone())
    );
    assert_eq!(historical_record.bytes, historical_bytes);

    let update = PlanContentChangeV2 {
        schema: PLAN_CONTENT_CHANGE_SCHEMA_V2.into(),
        target: PlanContentTargetV2::Update {
            plan_id: authored.plan_head.reference.plan_id.clone(),
            expected_head_revision: authored.plan_head.head_revision,
        },
        editor: create.editor.clone(),
        consumed_draft: None,
    };
    let recovered_authoring =
        RoutingFactsPort::plan_authoring_snapshot(recovered.adapter.as_ref(), &workspace, &update)
            .unwrap();
    assert_eq!(recovered_authoring.active_publication, Some(historical));
    assert_eq!(
        recovered_authoring
            .legacy_source
            .as_ref()
            .map(|version| &version.compiled),
        Some(&authored.plan_version.compiled)
    );
    let updated = preview_plan_content(&update, &recovered_authoring).unwrap();
    let mut heads = recovered_authoring.plan_heads.clone();
    heads.retain(|head| head.reference.plan_id != updated.plan_head.reference.plan_id);
    heads.push(updated.plan_head.clone());
    let current = recovered_authoring
        .active_publication
        .as_ref()
        .unwrap()
        .next_with_plan_content(
            GatewayPublicationRevision::new(2).unwrap(),
            recovered_authoring.aliases.clone(),
            updated.plan_version.compiled,
            heads,
        )
        .unwrap();
    current.validate_current_contract().unwrap();
    let current_record =
        PublicationRecordV1::from_publication(workspace.clone(), &current).unwrap();
    {
        let stores = recovered.adapter.stores_lock().unwrap();
        stores
            .control()
            .prepare_publication(&current_record, Some(revision_one))
            .unwrap();
        stores
            .control()
            .mark_publication_active(
                &workspace,
                current_record.publication_revision,
                &current_record.digest,
            )
            .unwrap();
    }
    drop(recovered);

    let reopened = super::super::super::ProductionControlRuntime::prepare_for_role_all(
        directory.path(),
        catalog,
        None,
    )
    .unwrap();
    let target = std::sync::Arc::new(crate::gateway_ports::GatewayPublicationAdapter::new(
        std::sync::Arc::new(
            hiroute_gateway::server::publication::GatewayPublicationInstaller::open(&gateway_lkg)
                .unwrap(),
        ),
    ));
    reopened
        .configure_managed_agent_runtime(
            "http://127.0.0.1:5837/v1".into(),
            "/test/hiroute".into(),
            Some(target),
        )
        .unwrap();
    let stores = reopened.adapter.stores_lock().unwrap();
    assert_eq!(
        stores.control().active_publication(&workspace).unwrap(),
        Some(current_record.clone())
    );
    assert_eq!(
        stores
            .control()
            .last_known_good_publication(&workspace)
            .unwrap(),
        Some(historical_record)
    );
    current_record
        .verify()
        .unwrap()
        .validate_current_contract()
        .unwrap();
}

#[test]
fn publication_no_new_calls_replaces_servable_before_with_identified_denial() {
    use crate::gateway_ports::GatewayPublicationAdapter;
    use hiroute_application::publication::PublicationTargetPort;
    use hiroute_gateway::server::composition::RuntimePublicationFeed;
    use hiroute_gateway::server::dispatch::GatewayRequestAuthority;
    use hiroute_gateway::server::publication::GatewayPublicationInstaller;
    use hiroute_gateway::server::request_plan::IngressProtocol;
    use std::sync::Arc;
    if crate::test_support::isolated_agent_home(
        "control::runtime::effects::tests::publication_validation::publication_no_new_calls_replaces_servable_before_with_identified_denial",
    ) {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let runtime = super::super::super::ProductionControlRuntime::open_with_release_catalog(
        directory.path(),
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    let prior = GatewayPublicationV1::decode_persisted(include_bytes!(
        "../../../../../../../e2e/product/golden/routing/compiled-publication.v2.json"
    ))
    .unwrap();
    let mut desired = prior.clone();
    desired.aliases.clear();
    desired.grants.clear();
    let (mut operation, intent, denied) = routing_operation(
        &runtime.adapter,
        desired,
        Some(prior),
        "identified-no-new-calls",
    );
    let before = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap();
    let target = Arc::new(GatewayPublicationAdapter::new(Arc::new(
        GatewayPublicationInstaller::open(directory.path().join("gateway-lkg.json")).unwrap(),
    )));
    *runtime.adapter.publication_target.lock().unwrap() = Some(target.clone());
    target.activate_verified(&before).unwrap();
    target.resume_requests().unwrap();
    runtime
        .adapter
        .validate_external_admission(&intent)
        .unwrap();
    assert!(target.verify_installed(&before).unwrap());
    assert_eq!(
        RuntimePublicationFeed::pin(target.as_ref())
            .unwrap()
            .publication_revision(),
        before.publication_revision.get()
    );

    let effect = runtime.adapter.apply_external(&operation, &intent).unwrap();
    let effect = runtime
        .adapter
        .prepare_publication_activation(&operation, &effect)
        .unwrap();
    operation
        .step_mut(OperationStepKind::CompilePublication)
        .effects = vec![effect.clone()];
    runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .save_operation(&mut operation)
        .unwrap();
    runtime
        .adapter
        .activate_external(&operation, &effect)
        .unwrap();

    assert!(target.verify_installed(&denied).unwrap());
    assert_eq!(
        RuntimePublicationFeed::pin(target.as_ref())
            .unwrap()
            .publication_revision(),
        denied.publication_revision.get()
    );
    assert!(
        GatewayRequestAuthority::new(target)
            .begin(IngressProtocol::Messages, Some("Bearer scope-test-claude"))
            .is_err()
    );
}
