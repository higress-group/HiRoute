//! Production adapter wiring with real stores/files; not CLI or native-client acceptance.
use super::super::{ManagedAgentRuntimeV1, ProductionControlRuntime};
use super::*;
use hiroute_application::agent_connection::*;
use hiroute_application_api::{
    AgentModelCheckStateV2, AgentModelSettingsStateV2, AgentModelSurfaceResultV2,
    AgentSettingsStatusRequestV2,
};
use hiroute_domain::*;
use hiroute_integrations::{
    AgentFilesystemLayoutV1, ClaudeRegistrationIndexV1, FilesystemAgentScannerV1,
};
use hiroute_local_storage::ApplyCapabilityRegistrationV1;
use serde_json::json;
use std::{collections::BTreeSet, fs, os::unix::fs::PermissionsExt};

#[path = "native_model_tests/codex_cache.rs"]
mod codex_cache;
#[path = "native_model_lifecycle_tests.rs"]
mod lifecycle;

#[test]
fn settings_status_joins_surface_checks_with_the_active_publication_revision() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_status_joins_surface_checks_with_the_active_publication_revision",
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let (runtime, cli_executable) = open_with_codex_fixture_surfaces(root.path());
    let adapter = &runtime.adapter;
    let context = adapter
        .settings_context_for_agent("agent_codex_default")
        .unwrap();
    let path = adapter.scanner.codex_user_config_target();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    let before = b"# keep me\nuser_setting = true\n";
    fs::write(&path, before).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

    let mut install =
        settings_operation(adapter, "surface-join-install", &context, before, None, 1);
    let grant_effect = adapter
        .stores_lock()
        .unwrap()
        .secrets()
        .apply_agent_access_grant(
            &install.operation_id,
            &install.plan.agent_access_grants()[0],
            None,
        )
        .unwrap();
    install.step_mut(OperationStepKind::ApplySecrets).effects = vec![grant_effect.clone()];
    adapter
        .stores_lock()
        .unwrap()
        .control()
        .save_operation(&mut install)
        .unwrap();
    let intent = install
        .plan
        .external()
        .iter()
        .find(|intent| is_settings_codex_model(intent))
        .unwrap()
        .clone();
    let file_effect = adapter.apply_external(&install, &intent).unwrap();
    record_agent_effect(adapter, &mut install, &file_effect);
    adapter
        .stores_lock()
        .unwrap()
        .secrets()
        .activate_agent_access_grant(&grant_effect)
        .unwrap();
    adapter.activate_external(&install, &file_effect).unwrap();
    let configured = fs::read_to_string(&path).unwrap();
    assert!(
        configured.contains("model_catalog_json"),
        "the activated configuration must point at the immutable catalog artifact: {configured}"
    );
    publish(adapter, &mut install);
    install.state = OperationState::Succeeded;
    adapter
        .stores_lock()
        .unwrap()
        .control()
        .finish_operation(&mut install)
        .unwrap();
    let revision = adapter
        .stores_lock()
        .unwrap()
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap()
        .publication_revision;

    let status = || {
        adapter
            .model_settings_status(&AgentSettingsStatusRequestV2 {
                schema_version: AGENT_SETTINGS_SCHEMA_V2,
                context_id: context.clone(),
            })
            .unwrap()
    };
    let result = |surface: AgentModelSurfaceV2,
                  state: AgentModelCheckStateV2,
                  reason: Option<&str>| AgentModelSurfaceResultV2 {
        surface,
        applied_revision: revision,
        state,
        reason_code: reason.map(str::to_owned),
    };

    // Configuration alone never proves a model invocation: every surface stays not_verified.
    let first = status();
    assert_eq!(first.state, AgentModelSettingsStateV2::Configured);
    assert_eq!(first.applied_revision, Some(revision));
    assert_eq!(
        first.surface_results,
        vec![
            result(
                AgentModelSurfaceV2::CodexCli,
                AgentModelCheckStateV2::NotVerified,
                None
            ),
            result(
                AgentModelSurfaceV2::CodexDesktop,
                AgentModelCheckStateV2::NotVerified,
                None
            ),
        ]
    );
    assert!(!first.model_verified);

    // Native clients may update unrelated settings after HiRoute configures the route.
    // A collaboration skill restore likewise must not turn this model selection off.
    let unrelated = configured.replace("user_setting = true", "user_setting = false");
    assert_ne!(unrelated, configured);
    fs::write(&path, &unrelated).unwrap();
    let unchanged_route = status();
    assert_eq!(unchanged_route.state, AgentModelSettingsStateV2::Configured);
    assert_eq!(unchanged_route.current_selection, first.current_selection);
    let altered_provider = unrelated.replacen(
        "model_provider = \"hiroute\"",
        "model_provider = \"elsewhere\"",
        1,
    );
    assert_ne!(altered_provider, unrelated);
    fs::write(&path, altered_provider).unwrap();
    let managed_drift = status();
    assert_eq!(managed_drift.state, AgentModelSettingsStateV2::Drift);
    assert!(managed_drift.current_selection.is_none());
    fs::write(&path, &configured).unwrap();

    let save = |record: &AgentSurfaceCheckRecordV1| {
        adapter
            .stores_lock()
            .unwrap()
            .control()
            .save_agent_surface_check(&WorkspaceId::default(), record)
            .unwrap()
    };
    assert!(save(&check_record(
        &context,
        AgentModelSurfaceV2::CodexCli,
        revision,
        AgentSurfaceCheckStateV1::Passed,
        None
    )));
    // A single passed surface is not the whole currently detected surface set.
    let partial = status();
    assert_eq!(
        partial.surface_results,
        vec![
            result(
                AgentModelSurfaceV2::CodexCli,
                AgentModelCheckStateV2::Passed,
                None
            ),
            result(
                AgentModelSurfaceV2::CodexDesktop,
                AgentModelCheckStateV2::NotVerified,
                None
            ),
        ]
    );
    assert!(!partial.model_verified);

    assert!(save(&check_record(
        &context,
        AgentModelSurfaceV2::CodexDesktop,
        revision,
        AgentSurfaceCheckStateV1::Passed,
        None
    )));
    let verified = status();
    assert_eq!(
        verified.surface_results,
        vec![
            result(
                AgentModelSurfaceV2::CodexCli,
                AgentModelCheckStateV2::Passed,
                None
            ),
            result(
                AgentModelSurfaceV2::CodexDesktop,
                AgentModelCheckStateV2::Passed,
                None
            ),
        ]
    );
    assert!(verified.model_verified);

    // Installation changes alter only discovery and per-client evidence. Removing the CLI does
    // not revoke or rewrite the shared selection; the remaining Desktop result stays current.
    let sealed_selection = verified.current_selection.clone();
    fs::remove_file(&cli_executable).unwrap();
    let desktop_only = status();
    assert_eq!(desktop_only.state, AgentModelSettingsStateV2::Configured);
    assert_eq!(desktop_only.current_selection, sealed_selection);
    assert_eq!(
        desktop_only.surface_results,
        vec![result(
            AgentModelSurfaceV2::CodexDesktop,
            AgentModelCheckStateV2::Passed,
            None
        )]
    );
    assert_eq!(desktop_only.live_check_targets.len(), 1);
    assert_eq!(
        desktop_only.live_check_targets[0].surface,
        AgentModelSurfaceV2::CodexDesktop
    );
    assert!(desktop_only.model_verified);
    write_codex_fixture(&cli_executable);

    // A failed surface carries its reason and blocks the derived verification.
    assert!(save(&check_record(
        &context,
        AgentModelSurfaceV2::CodexCli,
        revision,
        AgentSurfaceCheckStateV1::Failed,
        Some("upstream-model-mismatch")
    )));
    let failed = status();
    assert_eq!(
        failed.surface_results,
        vec![
            result(
                AgentModelSurfaceV2::CodexCli,
                AgentModelCheckStateV2::Failed,
                Some("upstream-model-mismatch")
            ),
            result(
                AgentModelSurfaceV2::CodexDesktop,
                AgentModelCheckStateV2::Passed,
                None
            ),
        ]
    );
    assert!(!failed.model_verified);

    // A grant-rotating update publishes a new revision; the old-revision records must not be
    // shown for it, and writes against the superseded revision are dropped by the store.
    let configured = fs::read(&path).unwrap();
    let mut update = settings_operation(
        adapter,
        "surface-join-update",
        &context,
        &configured,
        Some(&install.operation_id),
        2,
    );
    let update_grant = adapter
        .stores_lock()
        .unwrap()
        .secrets()
        .apply_agent_access_grant(
            &update.operation_id,
            &update.plan.agent_access_grants()[0],
            None,
        )
        .unwrap();
    update.step_mut(OperationStepKind::ApplySecrets).effects = vec![update_grant.clone()];
    adapter
        .stores_lock()
        .unwrap()
        .control()
        .save_operation(&mut update)
        .unwrap();
    let update_intent = update
        .plan
        .external()
        .iter()
        .find(|intent| is_settings_codex_model(intent))
        .unwrap()
        .clone();
    let update_file = adapter.apply_external(&update, &update_intent).unwrap();
    record_agent_effect(adapter, &mut update, &update_file);
    adapter
        .stores_lock()
        .unwrap()
        .secrets()
        .activate_agent_access_grant(&update_grant)
        .unwrap();
    adapter.activate_external(&update, &update_file).unwrap();
    publish(adapter, &mut update);
    update.state = OperationState::Succeeded;
    adapter
        .stores_lock()
        .unwrap()
        .control()
        .finish_operation(&mut update)
        .unwrap();
    let next_revision = adapter
        .stores_lock()
        .unwrap()
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap()
        .publication_revision;
    assert!(next_revision.get() > revision.get());

    assert!(!save(&check_record(
        &context,
        AgentModelSurfaceV2::CodexCli,
        revision,
        AgentSurfaceCheckStateV1::Passed,
        None
    )));
    let rotated = status();
    assert_eq!(rotated.applied_revision, Some(next_revision));
    assert_eq!(
        rotated.surface_results,
        vec![
            AgentModelSurfaceResultV2 {
                surface: AgentModelSurfaceV2::CodexCli,
                applied_revision: next_revision,
                state: AgentModelCheckStateV2::NotVerified,
                reason_code: None,
            },
            AgentModelSurfaceResultV2 {
                surface: AgentModelSurfaceV2::CodexDesktop,
                applied_revision: next_revision,
                state: AgentModelCheckStateV2::NotVerified,
                reason_code: None,
            },
        ]
    );
    assert!(!rotated.model_verified);
    // The superseded records remain stored; only the join filters them.
    let stored = adapter
        .stores_lock()
        .unwrap()
        .control()
        .agent_surface_checks(&WorkspaceId::default(), &context)
        .unwrap();
    assert_eq!(stored.len(), 2);
    assert!(
        stored
            .iter()
            .all(|record| record.applied_revision == revision)
    );

    // Verification recovers once fresh evidence is written against the new revision.
    assert!(save(&check_record(
        &context,
        AgentModelSurfaceV2::CodexCli,
        next_revision,
        AgentSurfaceCheckStateV1::Passed,
        None
    )));
    assert!(save(&check_record(
        &context,
        AgentModelSurfaceV2::CodexDesktop,
        next_revision,
        AgentSurfaceCheckStateV1::Passed,
        None
    )));
    assert!(status().model_verified);
}

#[test]
fn settings_codex_catalog_dispatch_does_not_require_a_verified_consumer() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_codex_catalog_dispatch_does_not_require_a_verified_consumer",
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let runtime = open_without_fixture_grants(root.path());
    let adapter = &runtime.adapter;
    let context = adapter
        .settings_context_for_agent("agent_codex_default")
        .unwrap();
    let path = adapter.scanner.codex_user_config_target();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    let before = b"# keep me\nuser_setting = true\n";
    fs::write(&path, before).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

    let mut install = settings_operation(adapter, "catalog-closed", &context, before, None, 1);
    let grant_effect = adapter
        .stores_lock()
        .unwrap()
        .secrets()
        .apply_agent_access_grant(
            &install.operation_id,
            &install.plan.agent_access_grants()[0],
            None,
        )
        .unwrap();
    install.step_mut(OperationStepKind::ApplySecrets).effects = vec![grant_effect];
    adapter
        .stores_lock()
        .unwrap()
        .control()
        .save_operation(&mut install)
        .unwrap();
    let catalog_intent = install
        .plan
        .external()
        .iter()
        .find(|intent| is_settings_codex_catalog(intent))
        .unwrap()
        .clone();
    // Catalog production is structural. A missing or unknown executable cannot become a hidden
    // product admission gate; actual client behavior is verified separately per surface.
    let staged = adapter.apply_external(&install, &catalog_intent).unwrap();
    assert_eq!(staged.effect_id, catalog_intent.effect_id());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(
        adapter
            .artifacts
            .load_native_restore(&install.operation_id, &catalog_intent)
            .unwrap()
            .is_some()
    );
}

fn check_record(
    context: &str,
    surface: AgentModelSurfaceV2,
    revision: GatewayPublicationRevision,
    state: AgentSurfaceCheckStateV1,
    reason: Option<&str>,
) -> AgentSurfaceCheckRecordV1 {
    AgentSurfaceCheckRecordV1 {
        schema: AGENT_SURFACE_CHECK_SCHEMA.into(),
        context_id: context.to_owned(),
        surface,
        applied_revision: revision,
        state,
        checked_model_ids: vec!["hiroute/model.v1".into()],
        capability_scope_digest: CanonicalDigest::of_bytes(b"capability-scope"),
        check_request_digest: CanonicalDigest::of_bytes(b"check-request"),
        reason_code: reason.map(str::to_owned),
    }
}

/// The status join only resolves contexts the settings entry itself derives, so this fixture
/// builds the same journal shape as `operation` against the real codex context, and can encode
/// the update case (previous binding, rotated grant) that moves the publication revision.
fn settings_operation(
    adapter: &LocalControlAdapter,
    key: &str,
    context: &str,
    before: &[u8],
    previous: Option<&OperationId>,
    plans: usize,
) -> OperationV1 {
    ensure_target_cache(adapter);
    let publication = golden_publication_plans_only();
    let selected: Vec<_> = publication
        .published_agent_plans()
        .unwrap()
        .iter()
        .filter(|plan| {
            plan.active
                && plan
                    .supported_ingress
                    .contains(&AgentIngressProtocolV1::Responses)
        })
        .take(plans)
        .cloned()
        .collect();
    assert_eq!(selected.len(), plans, "the golden publication lacks plans");
    let plan_ids: BTreeSet<AgentPlanId> = selected
        .iter()
        .map(|plan| plan.agent_plan_id.clone())
        .collect();
    let grant = AgentModelGrantV2::from_plan_ids(
        AgentIngressProtocolV1::Responses,
        plan_ids.clone(),
        &publication,
    )
    .unwrap();
    let spec = ChangeSpecV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        command_id: "agents.settings.apply".into(),
        resource_id: Some(context.to_owned()),
        desired_state: json!({"schema_version":{"major":2,"minor":0},"context_id":context,
            "model":{"intent":"configure","settings":{"mode":"codex_default",
                "native_model_mode":"hiroute_only",
                "fixed_models":[],
                "allowed_plan_ids":plan_ids,
                "default_selection":{"kind":"plan","plan_id":selected[0].agent_plan_id}}}}),
    };
    let accept = CanonicalDigest::of_bytes(key.as_bytes());
    let control = AgentConnectionControlIntentV1::from_settings_planner(
        AgentConnectionTransactionSubjectV1::from_registered_profile(
            "agent_codex_default",
            "codex-responses-v1",
            "builtin/codex-responses/v1",
        )
        .unwrap(),
        &spec,
        false,
        &json!({"accept_digest":accept,"model_grant":grant}),
    )
    .unwrap();
    let target = AgentConnectionEffectRoleV1::ManagedConfiguration
        .target_for(control.subject())
        .unwrap();
    let mutation = AgentAccessGrantMutationV1::ensure(
        WorkspaceId::DEFAULT,
        AgentAccessGrantScopeV1::new(format!("agent-connection/{context}"), grant.clone()).unwrap(),
        previous.map_or(0, |_| 1),
    )
    .unwrap();
    let settings_spec: AgentSettingsSpecV2 =
        serde_json::from_value(spec.desired_state.clone()).unwrap();
    let AgentFacetIntent::Configure { settings } = settings_spec.model else {
        panic!("fixture must configure Codex model settings");
    };
    let baseline = {
        let stores = adapter.stores_lock().unwrap();
        codex_catalog_baseline(stores.control(), &adapter.artifacts, previous, &target).unwrap()
    };
    let catalog_plan = codex_catalog_plan_for(
        &adapter.scanner.codex_user_config_target(),
        &settings,
        &publication,
        &baseline,
    )
    .unwrap();
    let catalog_facts = catalog_facts(&catalog_plan);
    let catalog_digest = catalog_plan.content_digest;
    let change = CodexModelFileAction::Configure {
        previous_operation: previous.cloned(),
        provider_id: "hiroute".into(),
        endpoint: "http://127.0.0.1:5837/v1".into(),
        model: Some(selected[0].model_alias.as_str().to_owned()),
        model_catalog: Some(catalog_digest.clone()),
    };
    let catalog = settings_codex_catalog_intent(&control, context, &catalog_facts).unwrap();
    let native = settings_codex_model_file_intent(
        &control,
        context,
        CanonicalDigest::of_bytes(before),
        adapter.current_external_fingerprint(&target).unwrap(),
        change,
    )
    .unwrap();
    let stores = adapter.stores_lock().unwrap();
    let base = stores
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap();
    drop(stores);
    let publication_intent =
        settings_model_publication_intent(&control, context, base.digest, None).unwrap();
    let transaction = TransactionPlanV1::from_agent_connection_planner_with_agent_access_grants(
        spec,
        control,
        vec![mutation],
        vec![catalog, native, publication_intent],
    )
    .unwrap();
    let stores = adapter.stores_lock().unwrap();
    let workspace = WorkspaceId::default();
    let scope =
        IdempotencyScopeV1::new("interactive-user", "ApplyAgentConnectionChange", key).unwrap();
    let mut operation = OperationV1::new(
        OperationId::derive(&workspace, &scope, &accept),
        workspace.clone(),
        scope,
        accept.clone(),
        accept,
        stores.control().current_revisions(&workspace).unwrap(),
        transaction,
    )
    .unwrap();
    let capability = format!("settings-surface-join-capability-{key}");
    stores
        .apply_capability_registrar()
        .register(
            ApplyCapabilityRegistrationV1::from_protected_launcher(
                capability.clone(),
                operation.idempotency.principal.clone(),
                workspace.clone(),
                operation.idempotency.operation_kind.clone(),
                operation.accepted_digest.clone(),
                operation.expected_revisions.clone(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64
                    + 60,
            )
            .unwrap(),
        )
        .unwrap();
    let auth = stores
        .control()
        .verify_apply_authorization(
            &ProtectedApplyCapability::new(capability).unwrap(),
            &workspace,
            &operation.idempotency.principal,
            &operation.idempotency.operation_kind,
            &operation.accepted_digest,
            &operation.expected_revisions,
        )
        .unwrap();
    assert_eq!(
        stores.control().begin_operation(&operation, &auth).unwrap(),
        BeginOperationOutcome::Created
    );
    operation.state = OperationState::ApplyingAgentArtifacts;
    stores.control().save_operation(&mut operation).unwrap();
    operation
}

fn open(root: &std::path::Path) -> ProductionControlRuntime {
    configure_runtime(
        root,
        ProductionControlRuntime::prepare_for_role_all(
            root,
            crate::release_catalog::fixture_catalog(),
            None,
        )
        .unwrap(),
    )
}

fn open_with_scanner(
    root: &std::path::Path,
    scanner: hiroute_integrations::FilesystemAgentScannerV1,
) -> ProductionControlRuntime {
    configure_runtime(
        root,
        ProductionControlRuntime::prepare_for_role_all_with_scanner(
            root,
            crate::release_catalog::fixture_catalog(),
            None,
            scanner,
        )
        .unwrap(),
    )
}

fn write_codex_fixture(path: &std::path::Path) {
    fs::write(path, b"#!/bin/sh\nprintf 'codex-cli fixture\\n'\n").unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn open_with_codex_fixture_surfaces(
    root: &std::path::Path,
) -> (ProductionControlRuntime, std::path::PathBuf) {
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
    let cli = root.join("codex-cli-fixture");
    let desktop = root.join("codex-desktop-fixture");
    write_codex_fixture(&cli);
    write_codex_fixture(&desktop);
    let home = root.join("home");
    fs::create_dir(&home).unwrap();
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
    let mut layout = AgentFilesystemLayoutV1::from_process(&home, root);
    layout.codex_executable = cli.clone();
    layout.codex_desktop_executable = Some(desktop);
    layout.claude_executable = root.join("missing-claude");
    let registry = serde_json::from_slice(include_bytes!(
        "../../../../../assets/connector-registry/current/registry-seed.json"
    ))
    .unwrap();
    let models: ReleaseModelDataBundleV2 = serde_json::from_slice(include_bytes!(
        "../../../../../assets/release-facts/current/bundle/model-data.json"
    ))
    .unwrap();
    let scanner = FilesystemAgentScannerV1::new(
        layout,
        ClaudeRegistrationIndexV1::from_verified_model_data(&registry, &models.data).unwrap(),
    );
    let runtime = ProductionControlRuntime::prepare_for_role_all_with_scanner(
        root,
        crate::release_catalog::fixture_catalog(),
        None,
        scanner,
    )
    .unwrap();
    (
        configure_runtime_with_initial(root, runtime, golden_publication_plans_only()),
        cli,
    )
}

fn open_without_fixture_grants(root: &std::path::Path) -> ProductionControlRuntime {
    configure_runtime_with_initial(
        root,
        ProductionControlRuntime::prepare_for_role_all(
            root,
            crate::release_catalog::fixture_catalog(),
            None,
        )
        .unwrap(),
        golden_publication_plans_only(),
    )
}

/// The golden still carries its pre-migration executable sections and legacy Plan contract;
/// their fixture/decoder convergence is a pending adjudication and this test needs only the
/// compiled plans, so the disputed sections are emptied and the domain's own deterministic
/// current-contract upgrade is applied in this in-test view instead of adjudicating here.
fn golden_publication_plans_only() -> GatewayPublicationV1 {
    let mut value: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../../e2e/product/golden/routing/compiled-publication.v2.json"
    ))
    .unwrap();
    let object = value.as_object_mut().unwrap();
    object.insert("grants".into(), serde_json::json!([]));
    object.insert("aliases".into(), serde_json::json!([]));
    serde_json::from_value::<GatewayPublicationV1>(value)
        .unwrap()
        .into_current()
        .unwrap()
}

fn configure_runtime(
    root: &std::path::Path,
    runtime: ProductionControlRuntime,
) -> ProductionControlRuntime {
    let initial = GatewayPublicationV1::decode_persisted(include_bytes!(
        "../../../../../e2e/product/golden/routing/compiled-publication.v2.json"
    ))
    .unwrap();
    configure_runtime_with_initial(root, runtime, initial)
}

fn configure_runtime_with_initial(
    root: &std::path::Path,
    runtime: ProductionControlRuntime,
    initial: GatewayPublicationV1,
) -> ProductionControlRuntime {
    *runtime.adapter.managed_agent_runtime.lock().unwrap() = Some(ManagedAgentRuntimeV1 {
        gateway_base_url: "http://127.0.0.1:5837/v1".into(),
        trusted_hiroute_executable: "/test/hiroute".into(),
        worker_executor_availability: std::sync::Arc::new(
            crate::delegation::installation::WorkerExecutorAvailabilityRegistry::unconfigured(),
        ),
        resident_service_ready: false,
    });
    use hiroute_application::publication::PublicationTargetPort;
    let target = std::sync::Arc::new(crate::gateway_ports::GatewayPublicationAdapter::new(
        std::sync::Arc::new(
            hiroute_gateway::server::publication::GatewayPublicationInstaller::open(
                root.join("gateway-lkg.json"),
            )
            .unwrap(),
        ),
    ));
    let stores = runtime.adapter.stores_lock().unwrap();
    if stores
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .is_none()
    {
        let record =
            PublicationRecordV1::from_publication(WorkspaceId::default(), &initial).unwrap();
        stores.control().prepare_publication(&record, None).unwrap();
        target.activate_verified(&record).unwrap();
        stores
            .control()
            .mark_publication_active(
                &WorkspaceId::default(),
                record.publication_revision,
                &record.digest,
            )
            .unwrap();
    }
    drop(stores);
    *runtime.adapter.publication_target.lock().unwrap() = Some(target);
    runtime
}

fn publish(adapter: &LocalControlAdapter, operation: &mut OperationV1) {
    let intent = operation
        .plan
        .external()
        .iter()
        .find(|intent| intent.kind() == OwnedEffectKind::Publication)
        .unwrap()
        .clone();
    adapter.validate_external_admission(&intent).unwrap();
    let stores = adapter.stores_lock().unwrap();
    let base = stores
        .control()
        .active_publication(&operation.workspace_id)
        .unwrap()
        .unwrap();
    let grant_effect = match stores
        .secrets()
        .observe_agent_access_grant(
            &operation.operation_id,
            &operation.plan.agent_access_grants()[0],
        )
        .unwrap()
    {
        EffectReconciliation::Staged(effect) | EffectReconciliation::Applied(effect) => effect,
        _ => panic!("original grant effect is missing"),
    };
    drop(stores);
    let mut tampered = operation.clone();
    tampered.accepted_digest = CanonicalDigest::of_bytes(b"unconfirmed-publication");
    assert!(settings_model_publication_record(&tampered, &intent, &base, &grant_effect).is_err());
    let mut stale = base.clone();
    stale.digest = CanonicalDigest::of_bytes(b"different-publication-base");
    assert!(settings_model_publication_record(operation, &intent, &stale, &grant_effect).is_err());
    let staged = adapter.apply_external(operation, &intent).unwrap();
    operation
        .step_mut(OperationStepKind::CompilePublication)
        .effects = vec![staged.clone()];
    adapter
        .stores_lock()
        .unwrap()
        .control()
        .save_operation(operation)
        .unwrap();
    let checkpoint = adapter
        .prepare_publication_activation(operation, &staged)
        .unwrap();
    operation
        .step_mut(OperationStepKind::CompilePublication)
        .effects = vec![checkpoint.clone()];
    adapter
        .stores_lock()
        .unwrap()
        .control()
        .save_operation(operation)
        .unwrap();
    adapter.begin_publication_activation(operation).unwrap();
    adapter.activate_external(operation, &checkpoint).unwrap();
    adapter.finish_publication_activation(operation).unwrap();
    assert!(matches!(
        adapter.observe_external(operation, &intent).unwrap(),
        EffectReconciliation::Applied(_)
    ));
}

fn record_agent_effect(
    adapter: &LocalControlAdapter,
    operation: &mut OperationV1,
    effect: &OwnedEffectV1,
) {
    operation
        .step_mut(OperationStepKind::ApplyAgentArtifacts)
        .effects
        .push(effect.clone());
    adapter
        .stores_lock()
        .unwrap()
        .control()
        .save_operation(operation)
        .unwrap();
}

fn operation(
    adapter: &LocalControlAdapter,
    key: &str,
    before: &[u8],
    original: Option<&OperationId>,
) -> OperationV1 {
    ensure_target_cache(adapter);
    let publication = GatewayPublicationV1::decode_persisted(include_bytes!(
        "../../../../../e2e/product/golden/routing/compiled-publication.v2.json"
    ))
    .unwrap();
    let plans = publication.published_agent_plans().unwrap();
    let plan = plans
        .iter()
        .find(|p| {
            p.active
                && p.supported_ingress
                    .contains(&AgentIngressProtocolV1::Responses)
        })
        .unwrap();
    let grant = AgentModelGrantV2::from_plan_ids(
        AgentIngressProtocolV1::Responses,
        BTreeSet::from([plan.agent_plan_id.clone()]),
        &publication,
    )
    .unwrap();
    let spec = ChangeSpecV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        command_id: "agents.settings.apply".into(),
        resource_id: Some("context/one".into()),
        desired_state: json!({"schema_version":{"major":2,"minor":0},"context_id":"context/one",
            "model": original.map_or_else(|| json!({"intent":"configure","settings":{"mode":"codex_default","native_model_mode":"hiroute_only","fixed_models":[],"allowed_plan_ids":[plan.agent_plan_id],"default_selection":{"kind":"plan","plan_id":plan.agent_plan_id}}}),
                |op| json!({"intent":"restore","restore_point_ref":codex_model_restore_point_ref(op)}))}),
    };
    let accept = CanonicalDigest::of_bytes(key.as_bytes());
    let control = AgentConnectionControlIntentV1::from_settings_planner(
        AgentConnectionTransactionSubjectV1::from_registered_profile(
            "agent_codex_default",
            "codex-responses-v1",
            "builtin/codex-responses/v1",
        )
        .unwrap(),
        &spec,
        false,
        &json!({"accept_digest":accept,"model_grant":grant}),
    )
    .unwrap();
    let target = AgentConnectionEffectRoleV1::ManagedConfiguration
        .target_for(control.subject())
        .unwrap();
    let mutation = if original.is_some() {
        AgentAccessGrantMutationV1::revoke(WorkspaceId::DEFAULT, "agent-connection/context/one", 1)
            .unwrap()
    } else {
        AgentAccessGrantMutationV1::ensure(
            WorkspaceId::DEFAULT,
            AgentAccessGrantScopeV1::new("agent-connection/context/one", grant.clone()).unwrap(),
            0,
        )
        .unwrap()
    };
    let catalog_plan = if original.is_none() {
        let baseline = CodexCatalogBaseline::Scope;
        Some(
            codex_catalog_plan_for(
                &adapter.scanner.codex_user_config_target(),
                match &serde_json::from_value::<AgentSettingsSpecV2>(spec.desired_state.clone())
                    .unwrap()
                    .model
                {
                    AgentFacetIntent::Configure { settings } => settings,
                    _ => unreachable!(),
                },
                &publication,
                &baseline,
            )
            .unwrap(),
        )
    } else {
        None
    };
    let catalog = catalog_plan.as_ref().map(|plan| {
        settings_codex_catalog_intent(&control, "context/one", &catalog_facts(plan)).unwrap()
    });
    let change = original.map_or_else(
        || CodexModelFileAction::Configure {
            previous_operation: None,
            provider_id: "hiroute".into(),
            endpoint: "http://127.0.0.1:5837/v1".into(),
            model: Some(plan.model_alias.as_str().to_owned()),
            model_catalog: Some(catalog_plan.as_ref().unwrap().content_digest.clone()),
        },
        |op| CodexModelFileAction::Restore {
            original_operation: op.clone(),
            native_model: None,
        },
    );
    let native = settings_codex_model_file_intent(
        &control,
        "context/one",
        CanonicalDigest::of_bytes(before),
        adapter.current_external_fingerprint(&target).unwrap(),
        change,
    )
    .unwrap();
    let stores = adapter.stores_lock().unwrap();
    let base = stores
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap();
    let restore_grant = if original.is_some() {
        stores
            .secrets()
            .inspect_agent_access_grant(WorkspaceId::DEFAULT, "agent-connection/context/one")
            .unwrap()
    } else {
        None
    };
    drop(stores);
    let publication =
        settings_model_publication_intent(&control, "context/one", base.digest, restore_grant)
            .unwrap();
    let mut external = Vec::new();
    if let Some(catalog) = catalog {
        external.push(catalog);
    }
    external.push(native);
    external.push(publication);
    let transaction = TransactionPlanV1::from_agent_connection_planner_with_agent_access_grants(
        spec,
        control,
        vec![mutation],
        external,
    )
    .unwrap();
    let stores = adapter.stores_lock().unwrap();
    let workspace = WorkspaceId::default();
    let scope =
        IdempotencyScopeV1::new("interactive-user", "ApplyAgentSettingsChange", key).unwrap();
    let mut operation = OperationV1::new(
        OperationId::derive(&workspace, &scope, &accept),
        workspace.clone(),
        scope,
        accept.clone(),
        accept,
        stores.control().current_revisions(&workspace).unwrap(),
        transaction,
    )
    .unwrap();
    let capability = format!("settings-codex-test-capability-{key}");
    stores
        .apply_capability_registrar()
        .register(
            ApplyCapabilityRegistrationV1::from_protected_launcher(
                capability.clone(),
                operation.idempotency.principal.clone(),
                workspace.clone(),
                operation.idempotency.operation_kind.clone(),
                operation.accepted_digest.clone(),
                operation.expected_revisions.clone(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64
                    + 60,
            )
            .unwrap(),
        )
        .unwrap();
    let auth = stores
        .control()
        .verify_apply_authorization(
            &ProtectedApplyCapability::new(capability).unwrap(),
            &workspace,
            &operation.idempotency.principal,
            &operation.idempotency.operation_kind,
            &operation.accepted_digest,
            &operation.expected_revisions,
        )
        .unwrap();
    assert_eq!(
        stores.control().begin_operation(&operation, &auth).unwrap(),
        BeginOperationOutcome::Created
    );
    operation.state = OperationState::ApplyingAgentArtifacts;
    stores.control().save_operation(&mut operation).unwrap();
    operation
}

fn ensure_target_cache(adapter: &LocalControlAdapter) {
    let config = adapter.scanner.codex_user_config_target();
    let home = config.parent().unwrap();
    fs::create_dir_all(home).unwrap();
    fs::set_permissions(home, fs::Permissions::from_mode(0o700)).unwrap();
    let cache = home.join("models_cache.json");
    if !cache.exists() {
        fs::write(
            &cache,
            include_bytes!(
                "../../../../../crates/integrations/src/agents/codex_bundled_catalog.json"
            ),
        )
        .unwrap();
        fs::set_permissions(cache, fs::Permissions::from_mode(0o644)).unwrap();
    }
}

fn catalog_facts(plan: &hiroute_integrations::CodexCatalogPlan) -> SettingsModelCatalogFacts {
    let producer_kind = match plan.producer.metadata_source {
        hiroute_integrations::CodexCatalogMetadataSourceV1::UserConfigured => {
            CodexCatalogProducerKindV1::UserConfigured
        }
        hiroute_integrations::CodexCatalogMetadataSourceV1::TargetCache => {
            CodexCatalogProducerKindV1::TargetCache
        }
        hiroute_integrations::CodexCatalogMetadataSourceV1::TargetBundled => {
            CodexCatalogProducerKindV1::TargetBundled
        }
        hiroute_integrations::CodexCatalogMetadataSourceV1::HirouteGenerated => {
            CodexCatalogProducerKindV1::HirouteGenerated
        }
    };
    SettingsModelCatalogFacts {
        source_revision: hiroute_integrations::CODEX_CATALOG_SOURCE_REVISION.into(),
        content_digest: plan.content_digest.clone(),
        before_fingerprint: None,
        producer_kind,
        producer_path: plan.producer.path.to_str().unwrap().into(),
        producer_content_digest: plan.producer.content_digest.clone(),
        producer_context_digest: plan.producer.context_digest.clone(),
        producer_dependency_digest: plan.producer.dependency_digest.clone(),
    }
}

#[path = "settings_entry_tests.rs"]
mod settings_entry_tests;
