use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use hiroute_application::control::ComputeManagementControlPort;
use hiroute_application::{ConnectionOptionAuthorizationPort, TransactionRuntime};
use hiroute_domain::{
    CanonicalDigest, ComputeProjectionExpectationV1, CredentialRefV1, PortErrorCode, WorkspaceId,
};
use hiroute_integrations::{
    CpaRegisteredSourceV1, RegisteredComputeDiscoveryFactV1, TrustedReleaseCatalog,
};
use hiroute_local_storage::LocalStorageSet;

use super::*;

struct UnavailableCpaSources;

impl hiroute_cpa_bridge::CpaRegisteredSourcePort for UnavailableCpaSources {
    fn begin_routing_batch(
        &self,
    ) -> Result<hiroute_cpa_bridge::CpaRoutingBatch<'_>, hiroute_cpa_bridge::CpaLifecycleError>
    {
        Err(hiroute_cpa_bridge::CpaLifecycleError::NotStarted)
    }

    fn discover_registered_sources(
        &self,
    ) -> Result<Vec<CpaRegisteredSourceV1>, hiroute_cpa_bridge::CpaLifecycleError> {
        Err(hiroute_cpa_bridge::CpaLifecycleError::NotStarted)
    }
}

fn routable_fixture_catalog() -> TrustedReleaseCatalog {
    crate::release_catalog::routable_discovery_fixture_catalog()
}

fn discovery() -> RegisteredComputeDiscoveryFactV1 {
    RegisteredComputeDiscoveryFactV1 {
        agent_id: "agent.claude-code".into(),
        scanner_id: "scanner.claude.v1".into(),
        scanner_version: "1".into(),
        discovered_source_ref: "claude/settings/fixture".into(),
        configuration_revision: 1,
        connection_option_id: "zhipu.coding-plan.cn.v1".into(),
        endpoint_profile_id: "endpoint.zhipu.coding-plan.cn.v1".into(),
        endpoint_profile_revision: 1,
        registered_base_url: "https://open.bigmodel.cn/api/anthropic".into(),
        observed_model_id: "glm-5.3".into(),
        model_configuration_id: "model.zhipu.glm-5.3".into(),
        protected_credential_available: true,
    }
}

#[test]
fn trusted_catalog_lineage_proof_selects_only_its_exact_legacy_source() {
    let catalog = routable_fixture_catalog();
    let candidate = catalog.authorize_compute_discovery(discovery()).unwrap();
    let ids = catalog.compute_projection_ids(&candidate).unwrap();
    let mut legacy = catalog
        .prepare_compute_projection(
            candidate,
            ComputeProjectionExpectationV1 {
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
    let suffix = ids
        .legacy_v7_source_id()
        .strip_prefix("source/agent-")
        .unwrap();
    legacy.source.source_id = ids.legacy_v7_source_id().to_owned();
    legacy.source.revision = 1;
    legacy.source.identity.identity_revision = 1;
    legacy.source.identity.account_subject_ref = format!("account/agent-{suffix}");
    legacy.source.identity_digest = legacy.source.identity.digest().unwrap();
    legacy.binding.binding_id = format!("binding/agent-{suffix}");
    legacy.binding.revision = 1;
    legacy.binding.source_id = legacy.source.source_id.clone();
    legacy.binding.source_revision = legacy.source.revision;
    legacy.binding.source_identity_digest = legacy.source.identity_digest.clone();
    legacy.binding.credential_pool_id = Some(format!("pool/agent-{suffix}"));
    legacy.inventory.source_id = legacy.source.source_id.clone();
    legacy.inventory.inventory_revision = 1;
    legacy.credential_pool_identity = None;

    let directory = tempfile::tempdir().unwrap();
    let stores =
        LocalStorageSet::open_for_daemon_startup(directory.path().join("storage")).unwrap();
    stores
        .control()
        .put_compute_source(0, &legacy.source, catalog.registry(), true)
        .unwrap();
    stores
        .control()
        .put_source_binding(0, &legacy.binding, catalog.registry(), catalog.model_data())
        .unwrap();
    stores
        .control()
        .put_inventory_snapshot(&legacy.inventory)
        .unwrap();
    let expected = projection_expectation_for_ids(stores.control(), &ids).unwrap();
    assert_eq!(
        (
            expected.source_revision,
            expected.binding_revision,
            expected.inventory_revision,
        ),
        (1, 1, 1),
    );
    assert!(
        stores
            .control()
            .compute_source(ids.source_id())
            .unwrap()
            .is_none()
    );
    let mut changed_scanner = discovery();
    changed_scanner.scanner_version = "2".into();
    let changed_scanner = catalog
        .compute_projection_ids(
            &catalog
                .authorize_compute_discovery(changed_scanner)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(changed_scanner.source_id(), ids.source_id());
    assert_eq!(changed_scanner.binding_id(), ids.binding_id());
    assert_eq!(
        projection_expectation_for_ids(stores.control(), &changed_scanner)
            .unwrap_err()
            .code,
        PortErrorCode::Conflict,
    );
    assert_eq!(
        stores
            .control()
            .compute_projection_expectation(
                ids.source_id(),
                ids.binding_id(),
                ids.endpoint_profile_id(),
            )
            .unwrap_err()
            .code,
        PortErrorCode::Conflict,
    );

    assert_eq!(
        stores
            .control()
            .compute_projection_expectation_with_legacy_lineage(
                ids.source_id(),
                ids.binding_id(),
                ids.endpoint_profile_id(),
                "source/agent-ffffffffffffffffffffffff",
            )
            .unwrap_err()
            .code,
        PortErrorCode::Conflict,
    );
    assert!(
        stores
            .control()
            .compute_source(ids.source_id())
            .unwrap()
            .is_none()
    );
}

#[test]
fn current_catalog_drift_excludes_only_stale_projection_from_routing_snapshot() {
    let catalog = routable_fixture_catalog();
    let release_sequence = catalog
        .compute_catalog_provenance()
        .unwrap()
        .release_sequence;
    let directory = tempfile::tempdir().unwrap();
    let storage_root = directory.path().join("storage");
    let stores = LocalStorageSet::open_for_daemon_startup(&storage_root).unwrap();
    super::super::release_install::activate_release_catalog_revisions(&stores, &catalog).unwrap();
    let current = catalog
        .authorize_compute_discovery(discovery())
        .and_then(|candidate| {
            catalog.prepare_compute_projection(
                candidate,
                ComputeProjectionExpectationV1 {
                    source_revision: 0,
                    source_digest: None,
                    binding_revision: 0,
                    binding_digest: None,
                    inventory_revision: 0,
                    inventory_digest: None,
                },
                true,
            )
        })
        .unwrap()
        .desired;
    stores
        .control()
        .put_compute_source(0, &current.source, catalog.registry(), true)
        .unwrap();
    stores
        .control()
        .put_source_binding(
            0,
            &current.binding,
            catalog.registry(),
            catalog.model_data(),
        )
        .unwrap();
    stores
        .control()
        .put_inventory_snapshot(&current.inventory)
        .unwrap();
    let pool_identity = current.credential_pool_identity.as_ref().unwrap();
    let credential = CredentialRefV1::new(
        "credential/current",
        format!("source/{}", current.source.source_id),
        "hirouted",
        "provider-auth",
        [format!(
            "connection-option/{}",
            current.source.connection_option_id
        )],
        1,
    )
    .unwrap();
    let pool = pool_identity
        .materialize_first(credential, CanonicalDigest::of_bytes(b"credential-current"))
        .unwrap();
    stores
        .control()
        .put_credential_pool(0, &pool, catalog.registry(), catalog.model_data())
        .unwrap();

    let mut stale_registry = catalog.registry().clone();
    stale_registry.registry_version = "registry-pre-gateway-stale-v1".into();
    let mut stale_option = stale_registry
        .connection_options
        .iter()
        .find(|option| option.connection_option_id == current.source.connection_option_id)
        .unwrap()
        .clone();
    stale_option.connection_option_id = "provider.stale.test.v1".into();
    stale_registry.connection_options.push(stale_option);
    stale_registry.validate().unwrap();
    let mut stale_model_data = catalog.model_data().clone();
    stale_model_data.connector_registry_version = stale_registry.registry_version.clone();
    stale_model_data.validate_against(&stale_registry).unwrap();
    let mut stale_source = current.source.clone();
    stale_source.source_id = "source/stale".into();
    stale_source.connection_option_id = "provider.stale.test.v1".into();
    stale_source.identity.account_subject_ref = "account/stale".into();
    stale_source.identity_digest = stale_source.identity.digest().unwrap();
    let mut stale_binding = current.binding.clone();
    stale_binding.binding_id = "binding/stale".into();
    stale_binding.source_id = stale_source.source_id.clone();
    stale_binding.source_identity_digest = stale_source.identity_digest.clone();
    stale_binding.credential_pool_id = Some("pool/stale".into());
    let mut stale_inventory = current.inventory.clone();
    stale_inventory.source_id = stale_source.source_id.clone();
    stores
        .control()
        .put_compute_source(0, &stale_source, &stale_registry, true)
        .unwrap();
    stores
        .control()
        .put_source_binding(0, &stale_binding, &stale_registry, &stale_model_data)
        .unwrap();
    stores
        .control()
        .put_inventory_snapshot(&stale_inventory)
        .unwrap();

    let mut previous_model_data = catalog.model_data().clone();
    previous_model_data.bundle_version = "model-data-pre-gateway-stale-v1".into();
    previous_model_data
        .validate_against(catalog.registry())
        .unwrap();
    let mut stale_model_binding = current.binding.clone();
    stale_model_binding.binding_id = "binding/stale-model-revision".into();
    stale_model_binding.model_data_bundle_version = previous_model_data.bundle_version.clone();
    stale_model_binding.credential_pool_id = Some("pool/stale-model-revision".into());
    stores
        .control()
        .put_source_binding(
            0,
            &stale_model_binding,
            catalog.registry(),
            &previous_model_data,
        )
        .unwrap();
    assert_eq!(stores.control().compute_projection_rows().unwrap().len(), 3);

    let home = directory.path().join("home");
    let project = directory.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let scanner =
        super::super::release_agent_scanner(&home, &project, &catalog, None, None).unwrap();
    assert_eq!(
        scanner.claude_executable_target(),
        std::path::PathBuf::from("claude")
    );
    let selected_claude = directory.path().join("selected/bin/claude");
    let selected_scanner = super::super::release_agent_scanner(
        &home,
        &project,
        &catalog,
        None,
        Some(selected_claude.clone()),
    )
    .unwrap();
    assert_eq!(selected_scanner.claude_executable_target(), selected_claude);
    let artifacts = stores
        .open_managed_artifacts(
            storage_root.join("managed-artifacts"),
            storage_root.join("artifact-restores"),
        )
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
        delegation_digest_authority: hiroute_observation::DigestAuthority::new([9; 32]),
        delegation_observation: Arc::new(
            hiroute_observation::LocalObservationStore::open(
                storage_root.join("delegation-observation"),
                hiroute_observation::DigestAuthority::new([9; 32]),
            )
            .unwrap(),
        ),
        observation_workspace_key: zeroize::Zeroizing::new([9; 32]),
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
        release_catalog: Some(catalog),
        protected_inputs: Mutex::new(BTreeMap::new()),
        manual_protected_inputs: Mutex::new(BTreeMap::new()),
        agent_token_inputs: Mutex::new(BTreeMap::new()),
        model_connections: hiroute_integrations::NativeModelConnectionServiceV1::new(
            hiroute_application::compute_management::TrustedComputeCandidateRegistry::new(),
            Arc::new(hiroute_integrations::ReqwestModelDirectoryTransportV1),
        ),
        model_connection_cancellations: Mutex::new(BTreeMap::new()),
        prepared_discoveries: Mutex::new(BTreeMap::new()),
        permission_findings: Mutex::new(BTreeMap::new()),
        admission: TransactionRuntime::default(),
        plan_admission,
        observation_activity_path: storage_root.join("observation/activity.db"),
        cpa_sources: Some(Arc::new(UnavailableCpaSources)),
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
    let pool_id = current.binding.credential_pool_id.as_deref().unwrap();
    let subscriptions = adapter.compute_subscriptions().unwrap();
    assert_eq!(
        subscriptions.discovery_state,
        hiroute_application_api::ComputeSubscriptionDiscoveryStateV2::RuntimeUnavailable
    );
    assert!(subscriptions.candidates.is_empty());
    assert_eq!(
        subscriptions.reason_code.as_deref(),
        Some("subscription_runtime_unavailable")
    );
    assert!(adapter.registered_cpa_candidates().is_err());
    assert!(adapter.routing_cpa_candidates(true).is_empty());
    let pool_identity = adapter
        .credential_pool_identity(pool_id, &current.binding.binding_id)
        .unwrap()
        .unwrap();
    assert_eq!(pool_identity.source_revision, current.source.revision);
    assert_eq!(pool_identity.binding_revision, current.binding.revision);
    assert_eq!(
        pool_identity.binding_digest,
        CanonicalDigest::of(&current.binding).unwrap()
    );
    let snapshot = hiroute_application::control::RoutingFactsPort::routing_compilation_snapshot(
        &adapter,
        &WorkspaceId::default(),
    )
    .unwrap();

    assert_eq!(snapshot.facts.candidates.len(), 1);
    assert_eq!(
        snapshot.facts.candidates[0].binding.binding_id,
        current.binding.binding_id
    );
    assert_eq!(
        snapshot.facts.candidates[0].credential_refs,
        ["credential/current"]
    );
    assert_eq!(
        snapshot.facts.candidates[0].connector_runtime,
        hiroute_domain::ConnectorRuntimeKind::BuiltinNative
    );
    assert_eq!(snapshot.facts.candidates[0].protocol_profiles.len(), 3);
    let responses = snapshot.facts.candidates[0]
        .protocol_profiles
        .iter()
        .find(|profile| profile.ingress_protocol == hiroute_domain::UpstreamProtocol::Responses)
        .expect("the existing Responses-to-Messages conversion remains routable");
    assert_eq!(
        responses.capability.upstream_protocol,
        hiroute_domain::UpstreamProtocol::Messages
    );
    assert_eq!(
        snapshot
            .expected_revisions
            .dependencies
            .get("release.registry"),
        Some(&release_sequence)
    );
    assert_eq!(
        snapshot
            .expected_revisions
            .dependencies
            .get("release.model_data"),
        Some(&release_sequence)
    );
    assert_ne!(
        CanonicalDigest::of(&snapshot.facts.candidates).unwrap(),
        CanonicalDigest::of_bytes(b"empty")
    );
}
