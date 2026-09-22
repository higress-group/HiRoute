use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};

use hiroute_application::TransactionRuntime;
use hiroute_application_api::{
    CHANGE_SPEC_SCHEMA_V1, ChangeSpecV1, PLAN_CONTENT_CHANGE_SCHEMA_V2, PlanContentChangeV2,
    PlanContentTargetV2,
};
use hiroute_domain::{
    AgentConfigPermissionIntentV1, BeginOperationOutcome, CanonicalDigest,
    ConnectorRegistryBundleV1, GatewayPublicationRevision, GatewayPublicationV1,
    IdempotencyScopeV1, OperationV1, PlanHeadV1, PlanLifecycleV1, PlanVersionV1,
    ProtectedApplyCapability, PublicationRecordV1, PublicationRepositoryPort,
    ReleaseModelDataBundleV2, TransactionPlanV1, WorkspaceId,
};
use hiroute_integrations::{
    AgentFilesystemLayoutV1, ClaudeRegistrationIndexV1, FilesystemAgentScannerV1,
};
use hiroute_local_storage::ApplyCapabilityRegistrationV1;
use hiroute_local_storage::LocalStorageSet;
use serde_json::json;

use super::*;

mod publication_validation;
mod service_status;
mod skill_activation;

#[test]
fn publication_restart_reconciles_every_persisted_install_window() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::effects::tests::publication_restart_reconciles_every_persisted_install_window",
    ) {
        return;
    }
    use crate::gateway_ports::GatewayPublicationAdapter;
    use hiroute_application::publication::PublicationTargetPort;
    use hiroute_gateway::server::composition::RuntimePublicationFeed;
    use hiroute_gateway::server::publication::{
        GatewayPrepareOutcome, GatewayPublicationInstaller, PublicationFailpoint,
    };
    use std::sync::Arc;

    for window in 0..5 {
        eprintln!("restart-window {window}: open");
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = super::super::ProductionControlRuntime::open_with_release_catalog(
            directory.path(),
            crate::release_catalog::fixture_catalog(),
        )
        .unwrap();
        let lkg = directory.path().join("gateway-lkg.json");
        let target = Arc::new(GatewayPublicationAdapter::new(Arc::new(
            GatewayPublicationInstaller::open(&lkg).unwrap(),
        )));
        *runtime.adapter.publication_target.lock().unwrap() = Some(target.clone());
        let desired = GatewayPublicationV1::decode_persisted(include_bytes!(
            "../../../../../../e2e/product/golden/routing/compiled-publication.v2.json"
        ))
        .unwrap();
        let (mut operation, intent, record) = routing_operation(
            &runtime.adapter,
            desired,
            None,
            &format!("restart-{window}"),
        );
        let effect = runtime.adapter.apply_external(&operation, &intent).unwrap();
        eprintln!("restart-window {window}: initial effect");
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
        if window > 0 {
            let checkpoint = runtime
                .adapter
                .prepare_publication_activation(&operation, &effect)
                .unwrap();
            operation
                .step_mut(OperationStepKind::CompilePublication)
                .effects = vec![checkpoint.clone()];
            runtime
                .adapter
                .stores_lock()
                .unwrap()
                .control()
                .save_operation(&mut operation)
                .unwrap();
            if window == 2 {
                let snapshot = hiroute_integrations::gateway::project_publication(
                    &record.verify().unwrap().gateway_snapshot().unwrap(),
                )
                .unwrap();
                let GatewayPrepareOutcome::Prepared(prepared) =
                    target.installer().prepare(snapshot).unwrap()
                else {
                    panic!("not prepared")
                };
                assert!(
                    target
                        .installer()
                        .publish_with_failpoint(prepared, PublicationFailpoint::AfterDurableLkg)
                        .is_err()
                );
                assert!(target.installer().durability_uncertain());
                assert!(
                    runtime
                        .adapter
                        .prepare_publication_rollback(&operation, &checkpoint)
                        .is_err()
                );
            } else if window == 3 {
                target.activate_verified(&record).unwrap();
                assert!(
                    runtime
                        .adapter
                        .prepare_publication_rollback(&operation, &checkpoint)
                        .is_err()
                );
            } else if window == 4 {
                runtime
                    .adapter
                    .activate_external(&operation, &checkpoint)
                    .unwrap();
                assert!(
                    runtime
                        .adapter
                        .prepare_publication_rollback(&operation, &checkpoint)
                        .is_err()
                );
            }
        }
        assert!(RuntimePublicationFeed::pin(target.as_ref()).is_none());
        eprintln!("restart-window {window}: reopen");
        let operation_id = operation.operation_id.clone();
        drop(runtime);
        drop(target);

        let recovered = super::super::ProductionControlRuntime::prepare_for_role_all(
            directory.path(),
            crate::release_catalog::fixture_catalog(),
            None,
        )
        .unwrap();
        let target = Arc::new(GatewayPublicationAdapter::new(Arc::new(
            GatewayPublicationInstaller::open(&lkg).unwrap(),
        )));
        assert!(RuntimePublicationFeed::pin(target.as_ref()).is_none());
        recovered
            .configure_managed_agent_runtime(
                "http://127.0.0.1:5837/v1".into(),
                "/test/hiroute".into(),
                Some(target.clone()),
            )
            .unwrap();
        eprintln!("restart-window {window}: recovered");
        let stores = recovered.adapter.stores_lock().unwrap();
        let terminal = stores
            .control()
            .load_operation(&operation_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            terminal.state,
            hiroute_domain::OperationState::Succeeded,
            "window {window}"
        );
        assert_eq!(
            stores
                .control()
                .active_publication(&WorkspaceId::default())
                .unwrap(),
            Some(record.clone())
        );
        assert!(!stores.control().writer_recovery_required().unwrap());
        assert!(target.verify_installed(&record).unwrap());
        assert!(RuntimePublicationFeed::pin(target.as_ref()).is_some());
        assert_eq!(
            terminal
                .step(OperationStepKind::CompilePublication)
                .effects
                .len(),
            1
        );
    }
}
use crate::control::runtime::LocalControlAdapter;

fn write_executable(path: &Path, version: &str) {
    fs::write(path, format!("#!/bin/sh\nprintf '%s\\n' '{version}'\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn current_publication(
    publication: GatewayPublicationV1,
) -> (GatewayPublicationV1, Vec<PlanVersionV1>) {
    let workspace = publication.workspace_id.clone();
    let versions = publication
        .plans
        .iter()
        .cloned()
        .map(|compiled| {
            let legacy = PlanVersionV1::from_legacy_compiled(workspace.clone(), compiled).unwrap();
            PlanVersionV1::new(
                workspace.clone(),
                legacy.configuration,
                legacy.compiled.into_current().unwrap(),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let mut publication = publication.into_current().unwrap();
    publication.plan_heads = versions
        .iter()
        .map(|version| PlanHeadV1 {
            reference: version.reference.clone(),
            head_revision: version.reference.content_revision,
            model_alias: version.compiled.model_alias().clone(),
            status: PlanLifecycleV1::Enabled,
        })
        .collect();
    publication.validate_current_contract().unwrap();
    (publication, versions)
}

fn routing_operation(
    adapter: &LocalControlAdapter,
    desired: GatewayPublicationV1,
    prior: Option<GatewayPublicationV1>,
    key: &str,
) -> (OperationV1, ExternalEffectIntentV1, PublicationRecordV1) {
    let (mut desired, versions) = current_publication(desired);
    desired.publication_revision =
        GatewayPublicationRevision::new(if prior.is_some() { 11 } else { 1 }).unwrap();
    let desired_record =
        PublicationRecordV1::from_publication(WorkspaceId::default(), &desired).unwrap();
    let before_publication_digest = prior.map(|prior| {
        let (mut prior, _) = current_publication(prior);
        prior.publication_revision = GatewayPublicationRevision::new(10).unwrap();
        let prior_record =
            PublicationRecordV1::from_publication(WorkspaceId::default(), &prior).unwrap();
        let stores = adapter.stores.lock().unwrap();
        stores
            .control()
            .prepare_publication(&prior_record, None)
            .unwrap();
        stores
            .control()
            .mark_publication_active(
                &WorkspaceId::default(),
                prior_record.publication_revision,
                &prior_record.digest,
            )
            .unwrap();
        prior_record.digest
    });
    let version = versions
        .iter()
        .find(|version| version.compiled.body.agent_plan_revision == 1)
        .unwrap()
        .clone();
    let head = desired
        .plan_heads
        .iter()
        .find(|head| head.reference == version.reference)
        .unwrap()
        .clone();
    let change = PlanContentChangeV2 {
        schema: PLAN_CONTENT_CHANGE_SCHEMA_V2.into(),
        target: PlanContentTargetV2::Create {
            creation_key: key.to_owned(),
        },
        editor: version
            .configuration
            .editor(Some(head.model_alias.as_str().to_owned()))
            .unwrap(),
        consumed_draft: None,
    };
    let spec = ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: "routing.apply".to_owned(),
        resource_id: Some(format!("agent-plan/{}", version.reference.plan_id.as_str())),
        desired_state: serde_json::to_value(change).unwrap(),
    };
    let plan = TransactionPlanV1::from_plan_content_planner(
        spec,
        version,
        head,
        None,
        None,
        desired_record.clone(),
        before_publication_digest,
    )
    .unwrap();
    let workspace = WorkspaceId::default();
    let request_digest = CanonicalDigest::of_bytes(key.as_bytes());
    let accepted_digest = CanonicalDigest::of_bytes(b"publication-change");
    let scope = IdempotencyScopeV1::new("interactive-user", "ApplyAgentPlanChange", key).unwrap();
    let expected_revisions = adapter
        .stores
        .lock()
        .unwrap()
        .control()
        .current_revisions(&workspace)
        .unwrap();
    let operation = OperationV1::new(
        OperationId::derive(&workspace, &scope, &request_digest),
        workspace,
        scope,
        request_digest,
        accepted_digest,
        expected_revisions,
        plan,
    )
    .unwrap();
    begin_operation_for_test(adapter, &operation);
    let effect = operation.plan.external()[0].clone();
    (operation, effect, desired_record)
}

fn begin_operation_for_test(adapter: &LocalControlAdapter, operation: &OperationV1) {
    let stores = adapter.stores.lock().unwrap();
    let capability = format!("publication-capability-{}", operation.operation_id);
    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        + 60;
    stores
        .apply_capability_registrar()
        .register(
            ApplyCapabilityRegistrationV1::from_protected_launcher(
                capability.clone(),
                operation.idempotency.principal.clone(),
                operation.workspace_id.clone(),
                operation.idempotency.operation_kind.clone(),
                operation.accepted_digest.clone(),
                operation.expected_revisions.clone(),
                expires,
            )
            .unwrap(),
        )
        .unwrap();
    let authorization = stores
        .control()
        .verify_apply_authorization(
            &ProtectedApplyCapability::new(capability).unwrap(),
            &operation.workspace_id,
            &operation.idempotency.principal,
            &operation.idempotency.operation_kind,
            &operation.accepted_digest,
            &operation.expected_revisions,
        )
        .unwrap();
    assert_eq!(
        stores
            .control()
            .begin_operation(operation, &authorization)
            .unwrap(),
        BeginOperationOutcome::Created
    );
}

fn permission_finding_fixture(
    settings: &Path,
) -> hiroute_integrations::PermissionHardeningRequiredV1 {
    use std::os::unix::fs::MetadataExt;

    let metadata = fs::metadata(settings).unwrap();
    let source_digest = CanonicalDigest::of_bytes(
        format!("claude-settings\0User\0{}", settings.to_string_lossy()).as_bytes(),
    );
    let identity = CanonicalDigest::of_bytes(
        format!(
            "agent-config-identity/v2\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
            metadata.dev(),
            metadata.ino(),
            metadata.uid(),
            metadata.gid(),
            metadata.nlink(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec(),
        )
        .as_bytes(),
    );
    let revision_digest = CanonicalDigest::of_bytes(
        format!(
            "agent-config-permission/v1\0{identity}\0{}",
            metadata.permissions().mode() & 0o777
        )
        .as_bytes(),
    );
    hiroute_integrations::PermissionHardeningRequiredV1 {
        scanner_id: "builtin.agent-filesystem".into(),
        scanner_version: "1".into(),
        discovered_source_ref: format!("claude/settings/{}", &source_digest.as_str()[7..39]),
        observed_identity: identity,
        observed_revision: u64::from_str_radix(&revision_digest.as_str()[7..23], 16)
            .unwrap()
            .max(1),
        display_path: "~/.claude/settings.json".into(),
        required_mode: 0o600,
    }
}

#[test]
fn permission_effect_hardens_only_pinned_identity_and_never_reverts_mode() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    let project = directory.path().join("project");
    let bin = directory.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let claude = bin.join("claude");
    write_executable(&claude, "2.1.231 (Claude Code)");
    let settings = home.join(".claude/settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, br#"{"env":{}}"#).unwrap();
    fs::set_permissions(&settings, fs::Permissions::from_mode(0o644)).unwrap();

    let registry: ConnectorRegistryBundleV1 = serde_json::from_slice(include_bytes!(
        "../../../../../../assets/connector-registry/current/registry-seed.json"
    ))
    .unwrap();
    let model_data: ReleaseModelDataBundleV2 = serde_json::from_slice(include_bytes!(
        "../../../../../../assets/release-facts/current/bundle/model-data.json"
    ))
    .unwrap();
    let mut layout = AgentFilesystemLayoutV1::from_process(&home, &project);
    layout.codex_executable = bin.join("missing-codex");
    layout.claude_executable = claude;
    layout.claude_launch_settings = None;
    layout.claude_project_settings.clear();
    layout.claude_managed_settings.clear();
    let scanner = FilesystemAgentScannerV1::new(
        layout,
        ClaudeRegistrationIndexV1::from_verified_model_data(&registry, &model_data.data).unwrap(),
    );
    // Ordinary discovery no longer offers a chmod action for a readable 0644 file.
    assert!(
        scanner
            .scan()
            .iter()
            .all(|discovery| discovery.permission_hardening.is_none())
    );
    // Reconstruct a historical scanner-issued finding to keep persisted permission
    // operations and their compensation behavior covered without restoring that UI path.
    let finding = permission_finding_fixture(&settings);
    let storage = directory.path().join("storage");
    let stores = LocalStorageSet::open_for_daemon_startup(&storage).unwrap();
    let artifacts = stores
        .open_managed_artifacts(storage.join("artifacts"), storage.join("restores"))
        .unwrap();
    let plan_admission =
        Arc::new(hiroute_application::publication::admission::SharedAdmissionGate::new());
    let delegation_safety = Arc::new(
        hiroute_application::delegation::safety::RunSafetyProjection::new(
            plan_admission.clone(),
            "test-delegation-epoch".to_owned(),
        )
        .unwrap(),
    );
    delegation_safety.finish_startup_recovery();
    let adapter = LocalControlAdapter {
        price_snapshot: std::sync::Arc::new(
            hiroute_application::prices::PriceSnapshotSlot::default(),
        ),
        publication_diagnostics: Mutex::new(Default::default()),
        stores: Mutex::new(stores),
        delegation_digest_authority: hiroute_observation::DigestAuthority::new([8; 32]),
        delegation_observation: Arc::new(
            hiroute_observation::LocalObservationStore::open(
                storage.join("delegation-observation"),
                hiroute_observation::DigestAuthority::new([8; 32]),
            )
            .unwrap(),
        ),
        observation_workspace_key: zeroize::Zeroizing::new([8; 32]),
        delegation_epoch: "test-delegation-epoch".to_owned(),
        delegation_safety,
        delegation_run_authority: Arc::new(
            crate::delegation::run_authority::DelegationRunAuthority::default(),
        ),
        delegation_finalization: Arc::new(
            crate::delegation::finalization::DelegationFinalization::default(),
        ),
        scanner,
        artifacts,
        release_catalog: None,
        protected_inputs: Mutex::new(BTreeMap::new()),
        manual_protected_inputs: Mutex::new(BTreeMap::new()),
        agent_token_inputs: Mutex::new(BTreeMap::new()),
        model_connections: hiroute_integrations::NativeModelConnectionServiceV1::new(
            hiroute_application::compute_management::TrustedComputeCandidateRegistry::new(),
            Arc::new(hiroute_integrations::ReqwestModelDirectoryTransportV1),
        ),
        model_connection_cancellations: Mutex::new(BTreeMap::new()),
        prepared_discoveries: Mutex::new(BTreeMap::new()),
        permission_findings: Mutex::new(BTreeMap::from([(
            finding.discovered_source_ref.clone(),
            finding.clone(),
        )])),
        admission: TransactionRuntime::default(),
        plan_admission,
        observation_activity_path: storage.join("observation/activity.db"),
        cpa_sources: None,
        cpa_runtime: None,
        subscription_sources: Mutex::new(BTreeMap::new()),
        subscription_targets: Mutex::new(BTreeMap::new()),
        subscription_maintenance: Mutex::new(
            crate::control::runtime::subscriptions::SubscriptionMaintenance::new().unwrap(),
        ),
        managed_agent_runtime: Mutex::new(None),
        publication_target: Mutex::new(None),
        delegation_native_cleanup_cursor: Mutex::new(None),
        delegation_task_maintenance_cursor: Mutex::new(None),
    };
    let typed = AgentConfigPermissionIntentV1::from_filesystem_scanner(
        finding.discovered_source_ref.clone(),
        finding.observed_identity.clone(),
        finding.observed_revision,
    )
    .unwrap();
    let spec = ChangeSpecV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        command_id: "agents.config-permissions.apply".to_owned(),
        resource_id: Some(format!("agent-config/{}", finding.discovered_source_ref)),
        desired_state: json!({
            "scanner_id": finding.scanner_id,
            "scanner_version": finding.scanner_version,
            "source_ref": finding.discovered_source_ref,
            "observed_identity": finding.observed_identity,
            "observed_revision": finding.observed_revision,
            "required_mode": 384,
        }),
    };
    let plan = TransactionPlanV1::from_agent_config_permission_planner(spec, typed).unwrap();
    let intent = plan.external()[0].clone();
    assert_eq!(
        adapter
            .current_external_fingerprint(intent.target())
            .unwrap(),
        intent.before_fingerprint().cloned()
    );
    let workspace = WorkspaceId::default();
    let scope =
        IdempotencyScopeV1::new("interactive-user", "ApplySetup", "permission-test").unwrap();
    let request_digest = CanonicalDigest::of_bytes(b"permission-operation");
    let operation = OperationV1::new(
        OperationId::derive(&workspace, &scope, &request_digest),
        workspace.clone(),
        scope,
        request_digest.clone(),
        request_digest,
        adapter
            .stores_lock()
            .unwrap()
            .control()
            .current_revisions(&workspace)
            .unwrap(),
        plan,
    )
    .unwrap();
    begin_operation_for_test(&adapter, &operation);
    let effect = adapter.apply_external(&operation, &intent).unwrap();
    assert_eq!(
        fs::metadata(&settings).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(matches!(
        adapter.observe_external(&operation, &intent).unwrap(),
        EffectReconciliation::Staged(_)
    ));
    let mut operation = operation;
    operation
        .step_mut(OperationStepKind::ApplyAgentArtifacts)
        .effects = vec![effect.clone()];
    adapter
        .stores_lock()
        .unwrap()
        .control()
        .save_operation(&mut operation)
        .unwrap();
    assert_eq!(
        adapter.compensate_external(&operation, &effect).unwrap(),
        CompensationOutcome::Compensated
    );
    assert_eq!(
        fs::metadata(&settings).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn product_plan_only_publication_installs_identified_no_new_calls_state() {
    use crate::gateway_ports::GatewayPublicationAdapter;
    use hiroute_application::publication::PublicationTargetPort;
    use hiroute_gateway::server::composition::RuntimePublicationFeed;
    use hiroute_gateway::server::publication::GatewayPublicationInstaller;

    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::effects::tests::product_plan_only_publication_installs_identified_no_new_calls_state",
    ) {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = super::super::ProductionControlRuntime::open_with_release_catalog(
        directory.path(),
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    let mut desired = GatewayPublicationV1::decode_persisted(include_bytes!(
        "../../../../../../e2e/product/golden/routing/compiled-publication.v2.json"
    ))
    .unwrap();
    desired.aliases.clear();
    desired.grants.clear();
    let (operation, intent, desired_record) =
        routing_operation(&runtime.adapter, desired, None, "plan-only-publication");
    assert_eq!(
        runtime
            .adapter
            .validate_external_admission(&intent)
            .unwrap_err()
            .code,
        PortErrorCode::Unavailable
    );
    let target = Arc::new(GatewayPublicationAdapter::new(Arc::new(
        GatewayPublicationInstaller::open(directory.path().join("gateway-lkg.json")).unwrap(),
    )));
    *runtime.adapter.publication_target.lock().unwrap() = Some(target.clone());
    runtime
        .adapter
        .validate_external_admission(&intent)
        .unwrap();

    let effect = runtime.adapter.apply_external(&operation, &intent).unwrap();
    assert!(matches!(
        runtime
            .adapter
            .observe_external(&operation, &intent)
            .unwrap(),
        EffectReconciliation::Staged(_)
    ));
    assert_eq!(
        runtime
            .adapter
            .activate_external(&operation, &effect)
            .unwrap_err()
            .code,
        PortErrorCode::InvalidData
    );
    let effect = runtime
        .adapter
        .prepare_publication_activation(&operation, &effect)
        .unwrap();
    let mut operation = operation;
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
    assert_eq!(
        runtime
            .adapter
            .current_external_fingerprint(PUBLICATION_TARGET)
            .unwrap(),
        Some(desired_record.digest.clone())
    );
    assert!(target.verify_installed(&desired_record).unwrap());
    runtime.adapter.reconcile_active_publication().unwrap();
    assert!(RuntimePublicationFeed::pin(target.as_ref()).is_some());
    assert!(
        !directory
            .path()
            .join("managed-artifacts/publication/current")
            .exists()
    );
}

#[test]
fn executable_publication_without_gateway_target_rejects_admission_and_install_decision() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::effects::tests::executable_publication_without_gateway_target_rejects_admission_and_install_decision",
    ) {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = super::super::ProductionControlRuntime::open_with_release_catalog(
        directory.path(),
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    let desired = GatewayPublicationV1::decode_persisted(include_bytes!(
        "../../../../../../e2e/product/golden/routing/compiled-publication.v2.json"
    ))
    .unwrap();
    let mut prior = desired.clone();
    prior.aliases.clear();
    prior.grants.clear();
    let (operation, intent, desired_record) = routing_operation(
        &runtime.adapter,
        desired,
        Some(prior),
        "executable-publication",
    );

    let effect = runtime.adapter.apply_external(&operation, &intent).unwrap();
    assert_eq!(
        runtime
            .adapter
            .validate_external_admission(&intent)
            .unwrap_err()
            .code,
        PortErrorCode::Unavailable
    );
    let error = runtime
        .adapter
        .prepare_publication_activation(&operation, &effect)
        .unwrap_err();
    assert_eq!(error.code, PortErrorCode::Unavailable);
    let stores = runtime.adapter.stores.lock().unwrap();
    assert_eq!(
        stores
            .control()
            .prepared_publication(&WorkspaceId::default())
            .unwrap(),
        Some(desired_record)
    );
    assert_eq!(
        stores
            .control()
            .active_publication(&WorkspaceId::default())
            .unwrap()
            .unwrap()
            .publication_revision
            .get(),
        10
    );
}
