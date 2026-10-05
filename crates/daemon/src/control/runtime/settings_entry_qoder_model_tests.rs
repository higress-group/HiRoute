//! Additive model settings through the public transaction and real native artifact store.
use super::*;

fn model_spec(fixture: &QoderFixture) -> serde_json::Value {
    let publication = fixture
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
    let plan = publication
        .published_agent_plans()
        .unwrap()
        .into_iter()
        .find(|plan| {
            plan.active
                && plan
                    .supported_ingress
                    .contains(&AgentIngressProtocolV1::Responses)
        })
        .unwrap();
    json!({"schema_version":{"major":2,"minor":0},"context_id":fixture.context,"model":{"intent":"configure","settings":{"mode":"qoder_additional","allowed_plan_ids":[plan.agent_plan_id]}}})
}
fn model_status(fixture: &QoderFixture) -> serde_json::Value {
    let result = fixture.service.dispatch_wire(request(
        "GetAgentConnectionStatus",
        json!({"schema_version":{"major":2,"minor":0},"context_id":fixture.context}),
        None,
    ));
    assert!(result.error.is_none(), "{result:?}");
    result.data.unwrap()
}
fn restore_spec(fixture: &QoderFixture, status: &serde_json::Value) -> serde_json::Value {
    json!({"schema_version":{"major":2,"minor":0},"context_id":fixture.context,"model":{"intent":"restore","restore_point_ref":status["restore_point_ref"]}})
}

#[test]
fn pi_default_selected_after_prepare_blocks_final_removal_without_losing_models() {
    // Supply only the model-capability receipt, without a machine-wide Node dependency.
    // SDK imports/semantics are independently covered by the SDK contract and native E2E.
    let tools = crate::test_support::private_tempdir();
    let node = tools.path().join("node");
    private_file(&node, b"#!/bin/sh\nif [ \"$1\" = --version ]; then echo v22.19.0; elif [ \"$2\" = --check ] && [ \"$4\" = models ]; then echo hiroute.pi-sdk-capability/v1:ok; else exit 97; fi\n");
    fs::set_permissions(&node, fs::Permissions::from_mode(0o700)).unwrap();
    let path =
        std::env::join_paths([tools.path(), Path::new("/usr/bin"), Path::new("/bin")]).unwrap();
    if crate::test_support::isolated_agent_home_with_path(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::models::pi_default_selected_after_prepare_blocks_final_removal_without_losing_models",
        &path,
    ) {
        return;
    }
    let fixture = QoderFixture::with_kind(true, AgentKindV1::Pi);
    let mut spec = model_spec(&fixture);
    spec["model"]["settings"]["mode"] = json!("pi_additional");
    apply(&fixture.service, &fixture.runtime, spec, "pi-race-enable");
    let models_before = fs::read(&fixture.native_config).unwrap();
    let models: serde_json::Value = serde_json::from_slice(&models_before).unwrap();
    let (provider, declaration) = models["providers"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(id, _)| id.starts_with("hiroute-main-"))
        .unwrap();
    let default_file = fixture
        .native_config
        .parent()
        .unwrap()
        .join("settings.json");
    let default_before = fs::read(&default_file).unwrap();
    let status = model_status(&fixture);
    let restore = restore_spec(&fixture, &status);
    let mut ports = fixture.runtime.application_ports();
    ports.mutation = Some(Arc::new(fault::FileRace {
        adapter: fixture.runtime.adapter.clone(),
        path: default_file.clone(),
        fail_after_file: false,
        replacement: Some(
            serde_json::to_vec(&json!({"defaultProvider":provider,
            "defaultModel":declaration["models"][0]["id"],"theme":"user-edited"}))
            .unwrap(),
        ),
    }));
    let racing = LocalControlDaemon::new(ApplicationService::new(ports));
    let preview = racing
        .dispatch_wire(request(
            "PreviewAgentConnectionRestore",
            json!({"spec":restore}),
            None,
        ))
        .data
        .unwrap();
    assert_eq!(preview["applicable"], true);
    let response = racing.dispatch_wire(request(
        "ApplyAgentConnectionRestore",
        json!({"spec":restore,
        "accept_digest":preview["accept_digest"],"dependency_digest":preview["dependency_digest"],
        "expected_revisions":preview["expected_revisions"],"idempotency_key":"pi-race-restore"}),
        None,
    ));
    assert!(response.error.is_none(), "{response:?}");
    let pending = response.data.unwrap();
    assert_eq!(
        pending["state"], "rolled_back",
        "a restore rejected before its file switch must roll back safely"
    );
    assert_eq!(fs::read(&fixture.native_config).unwrap(), models_before);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&default_file).unwrap()).unwrap()["theme"],
        "user-edited"
    );
    assert_eq!(model_status(&fixture)["state"], "configured");
    fs::write(&default_file, &default_before).unwrap();
    let retry = apply(
        &fixture.service,
        &fixture.runtime,
        restore,
        "pi-race-safe-restore",
    );
    assert_eq!(retry["state"], "succeeded");
    assert_eq!(model_status(&fixture)["state"], "not_configured");
    assert_eq!(fs::read(default_file).unwrap(), default_before);
}

#[test]
fn qoder_additional_models_preserve_native_default_and_restore_independently_of_skill() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::models::qoder_additional_models_preserve_native_default_and_restore_independently_of_skill",
    ) {
        return;
    }
    let fixture = QoderFixture::with_models(true);
    let before = fs::read(&fixture.native_config).unwrap();
    apply(
        &fixture.service,
        &fixture.runtime,
        fixture.configure("explicit"),
        "qoder-model-skill",
    );
    let skill = fs::read(&fixture.skill).unwrap();
    let spec = model_spec(&fixture);
    let installed = apply(
        &fixture.service,
        &fixture.runtime,
        spec.clone(),
        "qoder-model-install",
    );
    assert_eq!(installed["state"], "succeeded");
    let status = model_status(&fixture);
    assert_eq!(status["state"], "configured");
    assert_eq!(status["current_selection"], spec["model"]["settings"]);
    assert_eq!(status["model_verified"], false);
    assert_eq!(status["live_check_targets"][0]["surface"], "qoder_cli");
    let configured: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.native_config).unwrap()).unwrap();
    assert_eq!(configured["model"]["name"], "native/default");
    assert_eq!(
        configured["providers"]["native"],
        json!({"user":"preserved"})
    );
    assert_eq!(configured["unknown"], json!({"keep":true}));
    assert_eq!(
        fs::metadata(&fixture.native_config)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(fs::read(&fixture.skill).unwrap(), skill);
    let provider =
        hiroute_application::agent_connection::additional_model_provider_id(&fixture.context);
    assert_eq!(
        configured["providers"][&provider]["baseUrl"],
        "http://127.0.0.1:5837/_hiroute/qoder/v1"
    );
    let bearer = configured["providers"][&provider]["apiKey"]
        .as_str()
        .unwrap();
    let stores = fixture.runtime.adapter.stores_lock().unwrap();
    for operation in stores
        .control()
        .succeeded_agent_operations_for_kind(&WorkspaceId::default(), "ApplyAgentConnectionChange")
        .unwrap()
    {
        let operation = stores
            .control()
            .load_operation(&operation.operation_id)
            .unwrap()
            .unwrap();
        let serialized = serde_json::to_string(&operation).unwrap();
        assert!(
            !serialized.contains(bearer),
            "connection bearer must stay outside journal"
        );
        assert!(
            !serialized.contains("native/default"),
            "original configuration stays in encrypted restore material"
        );
    }
    drop(stores);
    let restore = restore_spec(&fixture, &status);
    apply(
        &fixture.service,
        &fixture.runtime,
        restore,
        "qoder-model-restore",
    );
    assert_eq!(model_status(&fixture)["state"], "not_configured");
    assert_eq!(fs::read(&fixture.skill).unwrap(), skill);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&fixture.native_config).unwrap())
            .unwrap(),
        serde_json::from_slice::<serde_json::Value>(&before).unwrap()
    );
    // Re-enabling uses the durable post-revocation generation, never a fresh generation zero.
    apply(
        &fixture.service,
        &fixture.runtime,
        spec,
        "qoder-model-reenable",
    );
    assert_eq!(model_status(&fixture)["state"], "configured");
    assert_eq!(fs::read(&fixture.skill).unwrap(), skill);
}

#[test]
fn qoder_restore_rejects_native_default_reference_then_preserves_unrelated_edits() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::models::qoder_restore_rejects_native_default_reference_then_preserves_unrelated_edits",
    ) {
        return;
    }
    let fixture = QoderFixture::with_models(true);
    apply(
        &fixture.service,
        &fixture.runtime,
        model_spec(&fixture),
        "qoder-default-install",
    );
    let status = model_status(&fixture);
    let restore = restore_spec(&fixture, &status);
    let provider =
        hiroute_application::agent_connection::additional_model_provider_id(&fixture.context);
    let alias = status["live_check_targets"][0]["client_model_ids"][0]
        .as_str()
        .unwrap();
    let mut native: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.native_config).unwrap()).unwrap();
    native["model"]["name"] = json!(format!("{provider}/{alias}"));
    native["unknown"]["new_user_value"] = json!(9);
    private_file(
        &fixture.native_config,
        &serde_json::to_vec(&native).unwrap(),
    );
    let prior = fs::read(&fixture.native_config).unwrap();
    let preview = fixture.service.dispatch_wire(request(
        "PreviewAgentConnectionRestore",
        json!({"spec":restore}),
        None,
    ));
    assert!(preview.error.is_none(), "{preview:?}");
    let preview = preview.data.unwrap();
    assert_eq!(preview["applicable"], false);
    assert!(
        preview["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|block| block["reason"] == "qoder_default_in_use")
    );
    assert_eq!(fs::read(&fixture.native_config).unwrap(), prior);
    assert_eq!(model_status(&fixture)["state"], "configured");
    native["model"]["name"] = json!("native/default");
    private_file(
        &fixture.native_config,
        &serde_json::to_vec(&native).unwrap(),
    );
    apply(
        &fixture.service,
        &fixture.runtime,
        restore,
        "qoder-default-restore",
    );
    let restored: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.native_config).unwrap()).unwrap();
    assert!(restored["providers"].get(&provider).is_none());
    assert_eq!(restored["unknown"]["new_user_value"], 9);
    assert_eq!(restored["model"]["name"], "native/default");
}

fn smaller_budget_operation(fixture: &QoderFixture) -> (OperationV1, PublicationRecordV1) {
    let stores = fixture.runtime.adapter.stores_lock().unwrap();
    let current = stores
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap();
    let mut next = current.verify().unwrap();
    let alias = next
        .grants
        .iter()
        .find(|grant| grant.model_grant.protocol == AgentIngressProtocolV1::Responses)
        .unwrap()
        .model_grant
        .routes
        .keys()
        .next()
        .unwrap()
        .clone();
    let index = next
        .plans
        .iter()
        .position(|plan| plan.model_alias().as_str() == alias)
        .unwrap();
    // The shared publication fixture predates durable Plan heads. Adopt its authenticated
    // current compiled content through the same existing first-edit compatibility path.
    let original_version = hiroute_domain::PlanVersionV1::from_unversioned_compiled_recovery(
        WorkspaceId::default(),
        next.plans[index].clone(),
    )
    .unwrap();
    if next.plan_heads.is_empty() {
        next.plan_heads = next
            .plans
            .iter()
            .cloned()
            .map(|plan| {
                let version = hiroute_domain::PlanVersionV1::from_unversioned_compiled_recovery(
                    WorkspaceId::default(),
                    plan,
                )
                .unwrap();
                hiroute_domain::PlanHeadV1 {
                    head_revision: version.reference.content_revision,
                    model_alias: version.compiled.model_alias().clone(),
                    reference: version.reference,
                    status: hiroute_domain::PlanLifecycleV1::Enabled,
                }
            })
            .collect();
        next.plan_heads
            .sort_by(|a, b| a.reference.plan_id.cmp(&b.reference.plan_id));
    }
    let before = next
        .plan_heads
        .iter()
        .find(|head| head.reference.plan_id == *next.plans[index].agent_plan_id())
        .unwrap()
        .clone();
    let mut body = (*next.plans[index].body).clone();
    let budget = hiroute_integrations::qoder_plan_token_budget(&body.materialized).unwrap();
    body.materialized.attempt_owned.limits.context_window_tokens =
        Some(budget.context_window_tokens - 1);
    body.agent_plan_revision += 1;
    body.materialized_route_digest = body.materialized.route_digest().unwrap();
    let compiled = hiroute_domain::CompiledAgentPlanV1::seal_current(body).unwrap();
    let legacy = hiroute_domain::PlanVersionV1::from_unversioned_compiled_recovery(
        WorkspaceId::default(),
        compiled.clone(),
    )
    .unwrap();
    let version =
        hiroute_domain::PlanVersionV1::new(WorkspaceId::default(), legacy.configuration, compiled)
            .unwrap();
    let head = hiroute_domain::PlanHeadV1 {
        reference: version.reference.clone(),
        head_revision: before.head_revision + 1,
        model_alias: version.compiled.model_alias().clone(),
        status: hiroute_domain::PlanLifecycleV1::Enabled,
    };
    let mut heads = next.plan_heads.clone();
    *heads
        .iter_mut()
        .find(|item| item.reference.plan_id == head.reference.plan_id)
        .unwrap() = head.clone();
    let next = next
        .next_with_plan_content(
            hiroute_domain::GatewayPublicationRevision::new(current.publication_revision.get() + 1)
                .unwrap(),
            next.alias_registry.clone(),
            version.compiled.clone(),
            heads,
        )
        .unwrap();
    let record = PublicationRecordV1::from_publication(WorkspaceId::default(), &next).unwrap();
    let change = api::PlanContentChangeV2 {
        schema: api::PLAN_CONTENT_CHANGE_SCHEMA_V2.into(),
        target: api::PlanContentTargetV2::Update {
            plan_id: head.reference.plan_id.clone(),
            expected_head_revision: before.head_revision,
        },
        editor: version
            .configuration
            .editor(Some(head.model_alias.as_str().into()))
            .unwrap(),
        consumed_draft: None,
    };
    let spec = hiroute_domain::ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: "routing.apply".into(),
        resource_id: Some(format!("agent-plan/{}", head.reference.plan_id.as_str())),
        desired_state: serde_json::to_value(change).unwrap(),
    };
    let plan = hiroute_domain::TransactionPlanV1::from_plan_content_planner(
        spec,
        version,
        head,
        Some(before),
        None,
        record.clone(),
        Some(current.digest),
    )
    .unwrap()
    .with_legacy_plan_source(original_version)
    .unwrap();
    let scope = hiroute_domain::IdempotencyScopeV1::new(
        "interactive-user",
        "ApplyAgentPlanChange",
        "qoder-smaller-budget",
    )
    .unwrap();
    let digest = CanonicalDigest::of_bytes(b"qoder-smaller-budget");
    let operation = OperationV1::new(
        OperationId::derive(&WorkspaceId::default(), &scope, &digest),
        WorkspaceId::default(),
        scope,
        digest.clone(),
        digest,
        stores
            .control()
            .current_revisions(&WorkspaceId::default())
            .unwrap(),
        plan,
    )
    .unwrap();
    (operation, record)
}

#[test]
fn qoder_installed_budget_rejects_publication_admission_and_install_checkpoint_without_cutover() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::models::qoder_installed_budget_rejects_publication_admission_and_install_checkpoint_without_cutover",
    ) {
        return;
    }
    let fixture = QoderFixture::with_models(true);
    apply(
        &fixture.service,
        &fixture.runtime,
        model_spec(&fixture),
        "qoder-budget-install",
    );
    let bytes = fs::read(&fixture.native_config).unwrap();
    let before_status = model_status(&fixture);
    let (operation, _) = smaller_budget_operation(&fixture);
    let intent = operation.plan.external()[0].clone();
    let error = fixture
        .runtime
        .adapter
        .validate_external_admission(&intent)
        .unwrap_err();
    assert_eq!(
        hiroute_application::TransactionError::Port(error.clone()).error_code(),
        api::ErrorCode::QoderModelBudgetConflict
    );
    assert_eq!(error.context, "qoder.model.budget.shrink");
    assert!(
        fixture
            .runtime
            .adapter
            .stores_lock()
            .unwrap()
            .control()
            .load_operation(&operation.operation_id)
            .unwrap()
            .is_none()
    );
    // Inject an already-admitted checkpoint to exercise the independent recovery-time guard.
    // Public admission above rejected this same record before journal mutation.
    let stores = fixture.runtime.adapter.stores_lock().unwrap();
    assert_eq!(
        stores.control().begin_local_operation(&operation).unwrap(),
        hiroute_domain::BeginOperationOutcome::Created
    );
    drop(stores);
    // Recovery must reach the independent Install guard through the production
    // coordinator, compensate the staged publication, and retain the public error.
    let adapter = fixture.runtime.adapter.as_ref();
    let recovered = hiroute_application::TransactionCoordinator::new(
        adapter,
        adapter,
        adapter,
        adapter,
        adapter,
        &adapter.admission,
    )
    .run(&operation.operation_id)
    .unwrap();
    assert_eq!(
        recovered.state,
        OperationState::RolledBack,
        "{recovered:#?}"
    );
    assert_eq!(
        recovered.safe_error_code.as_deref(),
        Some("QODER_MODEL_BUDGET_CONFLICT")
    );
    assert_eq!(fs::read(&fixture.native_config).unwrap(), bytes);
    assert_eq!(model_status(&fixture), before_status);
}

#[test]
fn qoder_pending_file_tail_keeps_collaboration_preview_independent_and_retries_exact_operation() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::models::qoder_pending_file_tail_keeps_collaboration_preview_independent_and_retries_exact_operation",
    ) {
        return;
    }
    let fixture = QoderFixture::with_models(true);
    let before = fs::read(&fixture.native_config).unwrap();
    let spec = model_spec(&fixture);
    let mut ports = fixture
        .runtime
        .application_ports()
        .with_agent_connection(Arc::new(FixtureFacts::collaboration(
            fixture.runtime.adapter.clone(),
        )));
    ports.mutation = Some(Arc::new(super::super::fault::FileRace {
        adapter: fixture.runtime.adapter.clone(),
        path: fixture.native_config.clone(),
        fail_after_file: false,
        replacement: None,
    }));
    let racing = LocalControlDaemon::new(ApplicationService::new(ports));
    let preview = racing.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert!(preview.error.is_none(), "{preview:?}");
    let preview = preview.data.unwrap();
    assert_eq!(preview["applicable"], true);
    let parked = racing.dispatch_wire(request(
        "ApplyAgentConnectionChange",
        json!({"spec":spec,
        "accept_digest":preview["accept_digest"],"dependency_digest":preview["dependency_digest"],
        "expected_revisions":preview["expected_revisions"],"idempotency_key":"qoder-parked-model"}),
        None,
    ));
    assert!(parked.error.is_none(), "{parked:?}");
    let parked = parked.data.unwrap();
    assert_eq!(parked["state"], "activating");
    assert_eq!(model_status(&fixture)["state"], "pending");
    let id: OperationId = serde_json::from_value(parked["operation_id"].clone()).unwrap();
    assert!(
        fixture
            .runtime
            .adapter
            .guard_additional_pending_model_change(None)
            .is_err()
    );
    assert!(
        fixture
            .runtime
            .adapter
            .guard_additional_pending_model_change(Some(&id))
            .is_ok()
    );
    let blocked = fixture.service.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert!(blocked.error.is_some());
    // A collaboration preview still does not read the damaged native model file. Its Apply
    // must wait for the old tail because the existing coordinator owns one Control revision.
    let skill_preview = fixture.service.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":fixture.configure("explicit")}),
        None,
    ));
    assert!(skill_preview.error.is_none(), "{skill_preview:?}");
    let skill_preview = skill_preview.data.unwrap();
    assert_eq!(skill_preview["applicable"], true);
    let revisions = fixture
        .runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .current_revisions(&WorkspaceId::default())
        .unwrap();
    let deferred = fixture.service.dispatch_wire(request("ApplyAgentConnectionChange", json!({
        "spec":fixture.configure("explicit"), "accept_digest":skill_preview["accept_digest"],
        "dependency_digest":skill_preview["dependency_digest"], "expected_revisions":skill_preview["expected_revisions"],
        "idempotency_key":"qoder-skill-before-tail-recovery"}), None));
    assert!(deferred.error.is_some());
    assert!(deferred.operation.is_none());
    assert!(!fixture.skill.exists());
    let stores = fixture.runtime.adapter.stores_lock().unwrap();
    assert_eq!(
        stores
            .control()
            .current_revisions(&WorkspaceId::default())
            .unwrap(),
        revisions
    );
    let scope = hiroute_domain::IdempotencyScopeV1::new(
        "interactive-user",
        "ApplyAgentConnectionChange",
        "qoder-skill-before-tail-recovery",
    )
    .unwrap();
    assert!(
        stores
            .control()
            .operation_for_idempotency(&WorkspaceId::default(), &scope)
            .unwrap()
            .is_none()
    );
    drop(stores);
    let wrong = fixture.service.dispatch_wire(request("ApplyAgentConnectionChange", json!({
        "schema":"hiroute.agent-settings-retry/v1","context_id":fixture.runtime.adapter.settings_context(SettingsAgentClass::Codex),"operation_id":id}),None));
    assert!(wrong.error.is_some());
    assert_eq!(model_status(&fixture)["state"], "pending");
    private_file(&fixture.native_config, &before);
    let request_body = json!({"schema":"hiroute.agent-settings-retry/v1","context_id":fixture.context,"operation_id":id});
    for _ in 0..2 {
        let resumed = fixture.service.dispatch_wire(request(
            "ApplyAgentConnectionChange",
            request_body.clone(),
            None,
        ));
        assert!(resumed.error.is_none(), "{resumed:?}");
        let resumed = resumed.data.unwrap();
        assert_eq!(resumed["state"], "succeeded");
        assert_eq!(resumed["operation_id"], parked["operation_id"]);
    }
    assert_eq!(model_status(&fixture)["state"], "configured");
    assert!(
        fixture
            .runtime
            .adapter
            .guard_additional_pending_model_change(None)
            .is_ok()
    );
    apply(
        &fixture.service,
        &fixture.runtime,
        fixture.configure("explicit"),
        "qoder-skill-after-tail",
    );
    assert!(fixture.skill.exists());
}

#[test]
fn qoder_managed_bearer_permission_drift_revokes_live_eligibility_but_allows_restore() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::models::qoder_managed_bearer_permission_drift_revokes_live_eligibility_but_allows_restore",
    ) {
        return;
    }
    let fixture = QoderFixture::with_models(true);
    // An unmanaged user file may be readable by others; the confirmed managed write seals it.
    fs::set_permissions(&fixture.native_config, fs::Permissions::from_mode(0o644)).unwrap();
    apply(
        &fixture.service,
        &fixture.runtime,
        model_spec(&fixture),
        "qoder-permission-install",
    );
    let status = model_status(&fixture);
    assert_eq!(status["state"], "configured");
    assert_eq!(
        fs::metadata(&fixture.native_config)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    fs::set_permissions(&fixture.native_config, fs::Permissions::from_mode(0o644)).unwrap();
    let drift = model_status(&fixture);
    assert_eq!(drift["state"], "drift");
    assert_eq!(drift["model_verified"], false);
    assert!(drift["live_check_targets"].is_null() || drift["live_check_targets"] == json!([]));
    apply(
        &fixture.service,
        &fixture.runtime,
        restore_spec(&fixture, &status),
        "qoder-permission-restore",
    );
    assert_eq!(model_status(&fixture)["state"], "not_configured");
    let restored: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.native_config).unwrap()).unwrap();
    assert_eq!(restored["model"]["name"], "native/default");
    let provider =
        hiroute_application::agent_connection::additional_model_provider_id(&fixture.context);
    assert!(restored["providers"].get(&provider).is_none());
}
