use std::collections::{BTreeMap, BTreeSet};

use crate::test_tempdir as tempdir;
use hiroute_domain::{
    AuthenticationKind, BillingClass, ChangeSpecV1, ComputeCatalogProvenanceV1,
    ComputeProjectionExpectationV1, ComputeScannerEvidenceV1, ComputeSourceControlPort,
    ConnectionOptionV1, ConnectionOrigin, ConnectorDescriptorV1, ConnectorRegistryBundleV1,
    ConnectorRuntimeKind, ControlRepositoryPort, CredentialPoolControlPort,
    CredentialPoolIdentityV1, CredentialPoolMutationKind, CredentialPoolMutationV1,
    CredentialPoolV1, CredentialRefV1, EffectReconciliation, EndpointProfileV1, IdempotencyScopeV1,
    InventoryStrategyKind, MaterializationState, ObservedModelV1, OperationId, OperationV1,
    PoolCredentialV1, PortErrorCode, RevisionSetV1, SourceIdentityV1, SourceOrigin,
    TransactionPlanV1, UpstreamProtocol, WorkspaceId,
};
use rusqlite::params;

use super::*;
use crate::control::ControlStore;

pub(super) fn registry() -> ConnectorRegistryBundleV1 {
    ConnectorRegistryBundleV1 {
        schema: hiroute_domain::CONNECTOR_REGISTRY_SCHEMA_V1.into(),
        registry_version: "registry-projection-v1".into(),
        product_release: "release-projection-v1".into(),
        connectors: vec![ConnectorDescriptorV1 {
            connector_id: "connector.projection".into(),
            revision: 1,
            runtime_kind: ConnectorRuntimeKind::BuiltinNative,
            implementation_ref: "builtin/projection".into(),
            implementation_revision: 1,
            accepted_origins: BTreeSet::from([ConnectionOrigin::NativeApi]),
            authentication: AuthenticationKind::ProviderApiKey,
            required_secret_slots: vec!["provider_api_key".into()],
            endpoint_profile_refs: vec!["endpoint.projection".into()],
            catalog_adapter_ref: "catalog.projection".into(),
            catalog_adapter_revision: 1,
            error_classifier_ref: "errors.projection".into(),
            error_classifier_revision: 1,
            usage_decoder_ref: "usage.projection".into(),
            usage_decoder_revision: 1,
            cache_policy_ref: "cache.projection".into(),
            cache_policy_revision: 1,
        }],
        endpoint_profiles: vec![EndpointProfileV1 {
            endpoint_profile_id: "endpoint.projection".into(),
            revision: 1,
            connector_id: "connector.projection".into(),
            connector_revision: 1,
            provider_platform_id: "provider.projection".into(),
            service_offering_id: "service.projection".into(),
            entitlement_id: "entitlement.projection".into(),
            usage_scope: "account".into(),
            region_id: "test".into(),
            logical_endpoint_group: "projection".into(),
            protocol_endpoints: vec![hiroute_domain::ProtocolEndpointV1 {
                protocol_endpoint_id: "endpoint.projection.messages".into(),
                protocol: UpstreamProtocol::Messages,
                base_url: "https://projection.invalid".into(),
                request_path: "/v1/messages".into(),
                adapter_ref: "adapter.projection".into(),
                adapter_revision: 1,
                stable_preference: 0,
                inventory_path: None,
                authentication_semantics: None,
                required_headers: Vec::new(),
            }],
            inventory_strategy: InventoryStrategyKind::BundledCatalog,
            inventory_protocol_endpoint_id: None,
            verification_evidence: CanonicalDigest::of_bytes(b"endpoint").to_string(),
            last_verified_at: 1,
        }],
        connection_options: vec![ConnectionOptionV1 {
            connection_option_id: "projection.native.test.v1".into(),
            display_name: "Projection".into(),
            origin: ConnectionOrigin::NativeApi,
            connector_id: "connector.projection".into(),
            connector_revision: 1,
            endpoint_profile_id: "endpoint.projection".into(),
            endpoint_profile_revision: 1,
            billing_class: BillingClass::Paid,
            free_offer_ref: None,
            direct_verification_evidence: None,
        }],
    }
}

pub(super) fn prepared(
    registry: &ConnectorRegistryBundleV1,
) -> hiroute_domain::PreparedComputeProjectionV1 {
    let evidence = CanonicalDigest::of_bytes(b"scanner-evidence");
    let identity = SourceIdentityV1 {
        identity_revision: 1,
        provider_platform_id: "provider.projection".into(),
        service_offering_id: "service.projection".into(),
        entitlement_id: "entitlement.projection".into(),
        usage_scope: "account".into(),
        endpoint_profile_id: "endpoint.projection".into(),
        endpoint_profile_revision: 1,
        region_id: "test".into(),
        account_subject_ref: "account.projection".into(),
        evidence_refs: vec![evidence.clone()],
    };
    let source = ComputeSourceV1 {
        schema: hiroute_domain::COMPUTE_STATE_SCHEMA_V1.into(),
        source_id: "source.projection".into(),
        revision: 1,
        connection_option_id: "projection.native.test.v1".into(),
        connector_id: "connector.projection".into(),
        connector_revision: 1,
        origin: SourceOrigin::NativeApi,
        identity_digest: identity.digest().unwrap(),
        identity,
        billing_class: BillingClass::Paid,
        state: MaterializationState::NeedsCredential,
    };
    let binding = SourceBindingV1 {
        binding_id: "binding.projection".into(),
        revision: 1,
        source_id: source.source_id.clone(),
        source_revision: 1,
        source_identity_digest: source.identity_digest.clone(),
        model_data_bundle_version: "models-projection-v1".into(),
        capability_slice_version: "capabilities-projection-v1".into(),
        offer_ref: "offer.projection".into(),
        offer_evidence_digest: CanonicalDigest::of_bytes(b"offer"),
        billing_class: BillingClass::Paid,
        model_configuration_id: "model.projection".into(),
        upstream_model_id: "upstream-projection".into(),
        capability_id: "capability.projection".into(),
        credential_pool_id: Some("pool.projection".into()),
    };
    let credential_pool_identity = CredentialPoolIdentityV1 {
        pool_id: "pool.projection".into(),
        binding_id: binding.binding_id.clone(),
        binding_revision: binding.revision,
        binding_digest: CanonicalDigest::of(&binding).unwrap(),
        source_id: source.source_id.clone(),
        source_revision: source.revision,
        connection_option_id: source.connection_option_id.clone(),
        source_identity_digest: source.identity_digest.clone(),
        offer_ref: binding.offer_ref.clone(),
        offer_revision: 1,
        offer_evidence_digest: binding.offer_evidence_digest.clone(),
        billing_class: binding.billing_class,
        model_configuration_id: binding.model_configuration_id.clone(),
        authentication: AuthenticationKind::ProviderApiKey,
    };
    let observed_models = vec![ObservedModelV1 {
        upstream_model_id: binding.upstream_model_id.clone(),
        metadata: BTreeMap::new(),
    }];
    hiroute_domain::PreparedComputeProjectionV1 {
        expected: ComputeProjectionExpectationV1 {
            source_revision: 0,
            source_digest: None,
            binding_revision: 0,
            binding_digest: None,
            inventory_revision: 0,
            inventory_digest: None,
        },
        desired: ComputeControlProjectionV1 {
            schema: hiroute_domain::COMPUTE_CONTROL_PROJECTION_SCHEMA_V1.into(),
            source,
            binding,
            inventory: ComputeInventorySnapshotV1 {
                source_id: "source.projection".into(),
                endpoint_profile_id: "endpoint.projection".into(),
                inventory_revision: 1,
                inventory_digest: CanonicalDigest::of(&observed_models).unwrap(),
                observed_models,
                captured_at: 1,
            },
            credential_pool_identity: Some(credential_pool_identity),
            catalog: ComputeCatalogProvenanceV1 {
                product_release: registry.product_release.clone(),
                catalog_binding_id: "fixture-catalog".into(),
                release_sequence: 1,
                connector_registry_version: registry.registry_version.clone(),
                connector_registry_digest: CanonicalDigest::of(registry).unwrap(),
                model_data_bundle_version: "models-projection-v1".into(),
                model_data_digest: CanonicalDigest::of_bytes(b"model-data"),
                cross_reference_digest: CanonicalDigest::of_bytes(b"cross-reference"),
            },
            scanner: ComputeScannerEvidenceV1 {
                scanner_id: "scanner.projection".into(),
                scanner_version: "1".into(),
                discovered_source_ref: "discovered.projection".into(),
                configuration_revision: 1,
                evidence_digest: evidence,
            },
        },
    }
}

pub(super) fn projection_mutation(
    prepared: &hiroute_domain::PreparedComputeProjectionV1,
    current: Option<&ComputeSourceV1>,
    registry: &ConnectorRegistryBundleV1,
) -> hiroute_domain::ComputeSourceMutationV1 {
    let spec = ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: "compute.connection.apply".into(),
        resource_id: Some(prepared.desired.source.source_id.clone()),
        desired_state: serde_json::json!({
            "connection_option_id": prepared.desired.source.connection_option_id,
            "source_id": prepared.desired.source.source_id,
            "explicit_materialization": true,
            "expected_source_revision": prepared.expected.source_revision,
            "projection": prepared,
        }),
    };
    TransactionPlanV1::from_compute_projection_planner(
        spec,
        current,
        prepared.clone(),
        registry,
        true,
    )
    .unwrap()
    .compute_source()
    .unwrap()
}

#[test]
fn compute_projection_stages_recovers_activates_and_compensates_as_one_boundary() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("data");
    let database = root.join("control.db");
    let backups = root.join("backups");
    let registry = registry();
    let prepared = prepared(&registry);
    registry.validate().unwrap();
    prepared.validate().unwrap();
    prepared.desired.source.validate(&registry, true).unwrap();
    let spec = ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: "compute.connection.apply".into(),
        resource_id: Some(prepared.desired.source.source_id.clone()),
        desired_state: serde_json::json!({
            "connection_option_id": prepared.desired.source.connection_option_id,
            "source_id": prepared.desired.source.source_id,
            "explicit_materialization": true,
            "expected_source_revision": 0,
            "projection": prepared,
        }),
    };
    let plan = TransactionPlanV1::from_compute_projection_planner(
        spec,
        None,
        prepared.clone(),
        &registry,
        true,
    )
    .unwrap();
    let mutation = plan.compute_source().unwrap();
    let operation = OperationId::parse("op_26262626262626262626262626262626").unwrap();
    let workspace = WorkspaceId::default();

    let store = ControlStore::open(&crate::test_storage_authority(), &database, &backups).unwrap();
    let effect = store
        .apply_compute_source(&operation, &workspace, 0, &mutation)
        .unwrap();
    assert!(store.compute_projection_rows().unwrap().is_empty());
    assert!(store.desired_state(&workspace).unwrap().is_none());
    assert!(matches!(
        store.observe_control(&operation, &workspace).unwrap(),
        EffectReconciliation::Staged(_)
    ));
    drop(store);

    let store = ControlStore::open(&crate::test_storage_authority(), &database, &backups).unwrap();
    assert!(matches!(
        store.observe_control(&operation, &workspace).unwrap(),
        EffectReconciliation::Staged(_)
    ));
    store.activate_control(&effect).unwrap();
    let rows = store.compute_projection_rows().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, prepared.desired.source);
    assert_eq!(rows[0].1, prepared.desired.binding);
    assert_eq!(rows[0].2, prepared.desired.inventory);
    assert_eq!(store.current_revisions(&workspace).unwrap().target, 1);
    drop(store);

    let store = ControlStore::open(&crate::test_storage_authority(), &database, &backups).unwrap();
    assert!(matches!(
        store.observe_control(&operation, &workspace).unwrap(),
        EffectReconciliation::Applied(_)
    ));
    assert_eq!(
        store.compensate_control(&effect).unwrap(),
        hiroute_domain::CompensationOutcome::Compensated
    );
    assert!(store.compute_projection_rows().unwrap().is_empty());
    assert!(store.desired_state(&workspace).unwrap().is_none());
}

#[test]
fn projection_refresh_rebinds_pool_and_model_change_tombstones_old_binding() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("data");
    let store = ControlStore::open(
        &crate::test_storage_authority(),
        root.join("control.db"),
        root.join("backups"),
    )
    .unwrap();
    let registry = registry();
    let workspace = WorkspaceId::default();
    let initial = prepared(&registry);
    let initial_mutation = projection_mutation(&initial, None, &registry);
    let initial_operation = OperationId::parse("op_31313131313131313131313131313131").unwrap();
    let initial_effect = store
        .apply_compute_source(&initial_operation, &workspace, 0, &initial_mutation)
        .unwrap();
    store.activate_control(&initial_effect).unwrap();

    let credential = CredentialRefV1::new(
        "credential.projection",
        "source/source.projection",
        "hirouted",
        "provider-auth",
        ["connection-option/projection.native.test.v1".into()],
        1,
    )
    .unwrap();
    let pool = initial
        .desired
        .credential_pool_identity
        .as_ref()
        .unwrap()
        .materialize_first(credential, CanonicalDigest::of_bytes(b"credential"))
        .unwrap();
    let pool_mutation = CredentialPoolMutationV1::from_registered_planner(
        CredentialPoolMutationKind::Add,
        None,
        pool.clone(),
    )
    .unwrap();
    let pool_operation = OperationId::parse("op_32323232323232323232323232323232").unwrap();
    let pool_effect = store
        .apply_credential_pool(&pool_operation, &workspace, 1, &pool_mutation)
        .unwrap();
    store.activate_control(&pool_effect).unwrap();

    let current_source = initial.desired.source.clone();
    let mut refreshed = initial.clone();
    refreshed.expected = store
        .compute_projection_expectation(
            &initial.desired.source.source_id,
            &initial.desired.binding.binding_id,
            &initial.desired.inventory.endpoint_profile_id,
        )
        .unwrap();
    let refreshed_evidence = CanonicalDigest::of_bytes(b"scanner-evidence-v2");
    refreshed.desired.source.revision = 2;
    refreshed.desired.source.identity.identity_revision = 2;
    refreshed.desired.source.identity.evidence_refs = vec![refreshed_evidence.clone()];
    refreshed.desired.source.identity_digest = refreshed.desired.source.identity.digest().unwrap();
    refreshed.desired.binding.revision = 2;
    refreshed.desired.binding.source_revision = 2;
    refreshed.desired.binding.source_identity_digest =
        refreshed.desired.source.identity_digest.clone();
    refreshed.desired.inventory.inventory_revision = 2;
    refreshed.desired.scanner.scanner_version = "2".into();
    refreshed.desired.scanner.discovered_source_ref = "discovered.projection.v2".into();
    refreshed.desired.scanner.configuration_revision = 2;
    refreshed.desired.scanner.evidence_digest = refreshed_evidence;
    let refreshed_pool = refreshed.desired.credential_pool_identity.as_mut().unwrap();
    refreshed_pool.binding_revision = 2;
    refreshed_pool.binding_digest = CanonicalDigest::of(&refreshed.desired.binding).unwrap();
    refreshed_pool.source_revision = 2;
    refreshed_pool.source_identity_digest = refreshed.desired.source.identity_digest.clone();
    refreshed.validate().unwrap();
    let refresh_mutation = projection_mutation(&refreshed, Some(&current_source), &registry);
    let refresh_operation = OperationId::parse("op_33333333333333333333333333333333").unwrap();
    let refresh_effect = store
        .apply_compute_source(&refresh_operation, &workspace, 2, &refresh_mutation)
        .unwrap();
    store.activate_control(&refresh_effect).unwrap();
    let rebound = store.credential_pool("pool.projection").unwrap().unwrap();
    assert_eq!(rebound.revision, 2);
    assert_eq!(rebound.binding_revision, 2);
    assert_eq!(rebound.source_revision, 2);
    assert_eq!(rebound.credentials, pool.credentials);

    let refreshed_source = refreshed.desired.source.clone();
    let mut changed_model = refreshed.clone();
    changed_model.desired.binding.binding_id = "binding.projection.next".into();
    changed_model.desired.binding.revision = 1;
    changed_model.desired.binding.model_configuration_id = "model.projection.next".into();
    changed_model.desired.binding.upstream_model_id = "upstream-projection-next".into();
    changed_model.desired.binding.capability_id = "capability.projection.next".into();
    changed_model.desired.binding.credential_pool_id = Some("pool.projection.next".into());
    changed_model.expected = store
        .compute_projection_expectation(
            &changed_model.desired.source.source_id,
            &changed_model.desired.binding.binding_id,
            &changed_model.desired.inventory.endpoint_profile_id,
        )
        .unwrap();
    let changed_evidence = CanonicalDigest::of_bytes(b"scanner-evidence-v3");
    changed_model.desired.source.revision = 3;
    changed_model.desired.source.identity.identity_revision = 3;
    changed_model.desired.source.identity.evidence_refs = vec![changed_evidence.clone()];
    changed_model.desired.source.identity_digest =
        changed_model.desired.source.identity.digest().unwrap();
    changed_model.desired.binding.source_revision = 3;
    changed_model.desired.binding.source_identity_digest =
        changed_model.desired.source.identity_digest.clone();
    changed_model.desired.inventory.inventory_revision = 3;
    changed_model.desired.inventory.observed_models = vec![ObservedModelV1 {
        upstream_model_id: "upstream-projection-next".into(),
        metadata: BTreeMap::new(),
    }];
    changed_model.desired.inventory.inventory_digest =
        CanonicalDigest::of(&changed_model.desired.inventory.observed_models).unwrap();
    changed_model.desired.scanner.scanner_version = "3".into();
    changed_model.desired.scanner.discovered_source_ref = "discovered.projection.v3".into();
    changed_model.desired.scanner.configuration_revision = 3;
    changed_model.desired.scanner.evidence_digest = changed_evidence;
    let changed_pool = changed_model
        .desired
        .credential_pool_identity
        .as_mut()
        .unwrap();
    changed_pool.pool_id = "pool.projection.next".into();
    changed_pool.binding_id = "binding.projection.next".into();
    changed_pool.binding_revision = 1;
    changed_pool.binding_digest = CanonicalDigest::of(&changed_model.desired.binding).unwrap();
    changed_pool.source_revision = 3;
    changed_pool.source_identity_digest = changed_model.desired.source.identity_digest.clone();
    changed_pool.model_configuration_id = "model.projection.next".into();
    changed_model.validate().unwrap();
    let changed_mutation = projection_mutation(&changed_model, Some(&refreshed_source), &registry);
    let changed_operation = OperationId::parse("op_34343434343434343434343434343434").unwrap();
    let changed_effect = store
        .apply_compute_source(&changed_operation, &workspace, 3, &changed_mutation)
        .unwrap();
    store.activate_control(&changed_effect).unwrap();

    assert!(
        store
            .source_binding("binding.projection")
            .unwrap()
            .is_none()
    );
    assert!(store.credential_pool("pool.projection").unwrap().is_none());
    assert_eq!(
        store
            .source_binding("binding.projection.next")
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    assert_eq!(store.compute_projection_rows().unwrap().len(), 1);
    store.with_connection(|connection| {
        let old_binding_active: bool = connection
            .query_row(
                "SELECT active FROM source_bindings WHERE binding_id='binding.projection'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let old_pool_active: bool = connection
            .query_row(
                "SELECT active FROM credential_pools WHERE pool_id='pool.projection'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!old_binding_active);
        assert!(!old_pool_active);
    });

    assert_eq!(
        store.compensate_control(&changed_effect).unwrap(),
        hiroute_domain::CompensationOutcome::Compensated
    );
    assert!(
        store
            .source_binding("binding.projection")
            .unwrap()
            .is_some()
    );
    assert!(store.credential_pool("pool.projection").unwrap().is_some());
    assert!(
        store
            .source_binding("binding.projection.next")
            .unwrap()
            .is_none()
    );
}
#[test]
fn legacy_v7_graph_continues_through_canonical_alias_without_losing_pool_credentials() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempdir().unwrap();
    let root = directory.path().join("data");
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let database = root.join("control.db");
    let registry = registry();
    let mut current_projection = prepared(&registry).desired;
    current_projection.source.identity.account_subject_ref = "account/agent/agent.claude".into();
    let physical_source_id = legacy_v7_source_id(&current_projection).unwrap();
    let suffix = physical_source_id.strip_prefix("source/agent-").unwrap();
    current_projection.source.identity_digest =
        current_projection.source.identity.digest().unwrap();
    let canonical_suffix = current_projection
        .source
        .identity_digest
        .as_str()
        .strip_prefix("sha256:")
        .unwrap()
        .get(..24)
        .unwrap();
    let canonical_source_id = format!("source/agent-{canonical_suffix}");
    let legacy_binding_id = format!("binding/agent-{suffix}");
    let legacy_pool_id = format!("pool/agent-{suffix}");
    let evidence = CanonicalDigest::of_bytes(b"legacy-scanner");
    let identity = SourceIdentityV1 {
        identity_revision: 7,
        provider_platform_id: "provider.projection".into(),
        service_offering_id: "service.projection".into(),
        entitlement_id: "entitlement.projection".into(),
        usage_scope: "account".into(),
        endpoint_profile_id: "endpoint.projection".into(),
        endpoint_profile_revision: 1,
        region_id: "test".into(),
        account_subject_ref: format!("account/agent-{suffix}"),
        evidence_refs: vec![evidence],
    };
    let source = ComputeSourceV1 {
        schema: hiroute_domain::COMPUTE_STATE_SCHEMA_V1.into(),
        source_id: physical_source_id.clone(),
        revision: 7,
        connection_option_id: "projection.native.test.v1".into(),
        connector_id: "connector.projection".into(),
        connector_revision: 1,
        origin: SourceOrigin::NativeApi,
        identity_digest: CanonicalDigest::of(&identity).unwrap(),
        identity,
        billing_class: BillingClass::Paid,
        state: MaterializationState::NeedsCredential,
    };
    let binding = SourceBindingV1 {
        binding_id: legacy_binding_id.clone(),
        revision: 4,
        source_id: physical_source_id.clone(),
        source_revision: source.revision,
        source_identity_digest: source.identity_digest.clone(),
        model_data_bundle_version: "models-projection-v1".into(),
        capability_slice_version: "capabilities-projection-v1".into(),
        offer_ref: "offer.projection".into(),
        offer_evidence_digest: CanonicalDigest::of_bytes(b"offer"),
        billing_class: BillingClass::Paid,
        model_configuration_id: "model.projection".into(),
        upstream_model_id: "upstream-projection".into(),
        capability_id: "capability.projection".into(),
        credential_pool_id: Some(legacy_pool_id.clone()),
    };
    let credential = CredentialRefV1::new(
        "credential.legacy",
        format!("source/{physical_source_id}"),
        "hirouted",
        "provider-auth",
        ["connection-option/projection.native.test.v1".into()],
        2,
    )
    .unwrap();
    let pool = CredentialPoolV1 {
        pool_id: legacy_pool_id.clone(),
        binding_id: legacy_binding_id.clone(),
        binding_revision: binding.revision,
        binding_digest: CanonicalDigest::of(&binding).unwrap(),
        source_id: physical_source_id.clone(),
        source_revision: source.revision,
        connection_option_id: source.connection_option_id.clone(),
        source_identity_digest: source.identity_digest.clone(),
        offer_ref: binding.offer_ref.clone(),
        offer_revision: 1,
        offer_evidence_digest: binding.offer_evidence_digest.clone(),
        billing_class: binding.billing_class,
        model_configuration_id: binding.model_configuration_id.clone(),
        authentication: AuthenticationKind::ProviderApiKey,
        revision: 3,
        credentials: vec![PoolCredentialV1 {
            credential,
            fingerprint: CanonicalDigest::of_bytes(b"legacy-credential"),
            ordinal: 0,
            enabled: true,
        }],
    };
    let observed_models = vec![ObservedModelV1 {
        upstream_model_id: binding.upstream_model_id.clone(),
        metadata: BTreeMap::new(),
    }];
    let inventory = ComputeInventorySnapshotV1 {
        source_id: physical_source_id.clone(),
        endpoint_profile_id: "endpoint.projection".into(),
        inventory_revision: 5,
        inventory_digest: CanonicalDigest::of(&observed_models).unwrap(),
        observed_models,
        captured_at: 1,
    };
    let connection = rusqlite::Connection::open(&database).unwrap();
    crate::migrations::initialize_control_v7_fixture(&connection).unwrap();
    {
        connection
            .execute(
                "INSERT INTO compute_sources VALUES (?1,?2,?3,?4,1)",
                params![
                    source.source_id,
                    source.revision,
                    source.identity_digest.as_str(),
                    serde_json::to_string(&source).unwrap()
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO source_bindings VALUES (?1,?2,?3,?4,1)",
                params![
                    binding.binding_id,
                    binding.revision,
                    binding.source_id,
                    serde_json::to_string(&binding).unwrap()
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO source_inventory_snapshots VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    inventory.source_id,
                    inventory.endpoint_profile_id,
                    inventory.inventory_revision,
                    inventory.inventory_digest.as_str(),
                    serde_json::to_string(&inventory.observed_models).unwrap(),
                    inventory.captured_at
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO credential_pools(
                    pool_id,binding_id,binding_revision,binding_digest,source_id,
                    source_revision,connection_option_id,offer_ref,offer_revision,
                    offer_evidence_digest,billing_class,model_configuration_id,revision,
                    homogeneous_identity_digest,authentication_kind,pool_json,updated_at
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'paid',?11,?12,?13,
                           'provider_api_key',?14,1)",
                params![
                    pool.pool_id,
                    pool.binding_id,
                    pool.binding_revision,
                    pool.binding_digest.as_str(),
                    pool.source_id,
                    pool.source_revision,
                    pool.connection_option_id,
                    pool.offer_ref,
                    pool.offer_revision,
                    pool.offer_evidence_digest.as_str(),
                    pool.model_configuration_id,
                    pool.revision,
                    CanonicalDigest::of(&pool.identity()).unwrap().as_str(),
                    serde_json::to_string(&pool).unwrap()
                ],
            )
            .unwrap();
    }
    drop(connection);
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600)).unwrap();
    let store = ControlStore::open(
        &crate::test_storage_authority(),
        &database,
        root.join("backups"),
    )
    .unwrap();
    let canonical_binding_id = derived_binding_id(&canonical_source_id, &binding).unwrap();
    let canonical_pool_id = canonical_binding_id.replacen("binding/", "pool/", 1);
    assert_eq!(
        store
            .compute_projection_expectation(
                &canonical_source_id,
                &canonical_binding_id,
                "endpoint.projection",
            )
            .unwrap_err()
            .code,
        PortErrorCode::Conflict,
    );
    let expected = store
        .compute_projection_expectation_with_legacy_lineage(
            &canonical_source_id,
            &canonical_binding_id,
            "endpoint.projection",
            &physical_source_id,
        )
        .unwrap();
    assert_eq!(
        (
            expected.source_revision,
            expected.binding_revision,
            expected.inventory_revision
        ),
        (7, 4, 5)
    );
    store.with_connection(|connection| {
        let aliases: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM compute_source_identity_aliases",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(aliases, 0, "Preview must not persist a legacy alias");
    });

    let mut next = prepared(&registry);
    next.expected = expected;
    next.desired.source.source_id = canonical_source_id.clone();
    next.desired.source.revision = 8;
    next.desired.source.identity.identity_revision = 8;
    next.desired.source.identity.account_subject_ref = "account/agent/agent.claude".into();
    let current_evidence = CanonicalDigest::of_bytes(b"current-scan");
    next.desired.source.identity.evidence_refs = vec![current_evidence.clone()];
    next.desired.source.identity_digest = next.desired.source.identity.digest().unwrap();
    next.desired.binding.binding_id = canonical_binding_id.clone();
    next.desired.binding.revision = 5;
    next.desired.binding.source_id = canonical_source_id.clone();
    next.desired.binding.source_revision = 8;
    next.desired.binding.source_identity_digest = next.desired.source.identity_digest.clone();
    next.desired.binding.credential_pool_id = Some(canonical_pool_id.clone());
    next.desired.inventory.source_id = canonical_source_id.clone();
    next.desired.inventory.inventory_revision = 6;
    let pool_identity = next.desired.credential_pool_identity.as_mut().unwrap();
    pool_identity.pool_id = canonical_pool_id.clone();
    pool_identity.binding_id = canonical_binding_id.clone();
    pool_identity.binding_revision = 5;
    pool_identity.binding_digest = CanonicalDigest::of(&next.desired.binding).unwrap();
    pool_identity.source_id = canonical_source_id.clone();
    pool_identity.source_revision = 8;
    pool_identity.source_identity_digest = next.desired.source.identity_digest.clone();
    next.desired.scanner.evidence_digest = current_evidence;
    next.validate().unwrap();
    let mutation = projection_mutation(&next, None, &registry);
    let operation = OperationId::parse("op_35353535353535353535353535353535").unwrap();
    let effect = store
        .apply_compute_source(&operation, &WorkspaceId::default(), 0, &mutation)
        .unwrap();
    assert!(matches!(
        store
            .observe_control(&operation, &WorkspaceId::default())
            .unwrap(),
        EffectReconciliation::Staged(_)
    ));
    store.activate_control(&effect).unwrap();

    let rows = store.compute_projection_rows().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0.source_id, physical_source_id);
    assert_eq!(rows[0].0.revision, 8);
    assert_eq!(rows[0].1.binding_id, canonical_binding_id);
    let continued_pool = store.credential_pool(&canonical_pool_id).unwrap().unwrap();
    assert_eq!(continued_pool.revision, 4);
    assert_eq!(continued_pool.credentials, pool.credentials);
    assert_eq!(continued_pool.source_id, rows[0].0.source_id);
    assert!(store.source_binding(&legacy_binding_id).unwrap().is_none());
    assert!(store.credential_pool(&legacy_pool_id).unwrap().is_none());
    assert_eq!(
        store
            .compute_projection_expectation(
                &canonical_source_id,
                &rows[0].1.binding_id,
                "endpoint.projection"
            )
            .unwrap()
            .source_revision,
        8
    );

    drop(store);
    let store = ControlStore::open(
        &crate::test_storage_authority(),
        &database,
        root.join("backups-reopened"),
    )
    .unwrap();
    let current = store.compute_source(&canonical_source_id).unwrap().unwrap();
    assert_eq!(current.source_id, physical_source_id);
    let mut refreshed = next;
    refreshed.expected = store
        .compute_projection_expectation(
            &canonical_source_id,
            &canonical_binding_id,
            "endpoint.projection",
        )
        .unwrap();
    let refreshed_evidence = CanonicalDigest::of_bytes(b"current-scan-restarted");
    refreshed.desired.source.revision = 9;
    refreshed.desired.source.identity.identity_revision = 9;
    refreshed.desired.source.identity.evidence_refs = vec![refreshed_evidence.clone()];
    refreshed.desired.source.identity_digest = refreshed.desired.source.identity.digest().unwrap();
    refreshed.desired.binding.revision = 6;
    refreshed.desired.binding.source_revision = 9;
    refreshed.desired.binding.source_identity_digest =
        refreshed.desired.source.identity_digest.clone();
    refreshed.desired.inventory.inventory_revision = 7;
    refreshed.desired.scanner.scanner_version = "9".into();
    refreshed.desired.scanner.configuration_revision = 9;
    refreshed.desired.scanner.evidence_digest = refreshed_evidence;
    let refreshed_pool = refreshed.desired.credential_pool_identity.as_mut().unwrap();
    refreshed_pool.binding_revision = 6;
    refreshed_pool.binding_digest = CanonicalDigest::of(&refreshed.desired.binding).unwrap();
    refreshed_pool.source_revision = 9;
    refreshed_pool.source_identity_digest = refreshed.desired.source.identity_digest.clone();
    refreshed.validate().unwrap();
    let mutation = projection_mutation(&refreshed, Some(&current), &registry);
    let refresh_operation = OperationId::parse("op_36363636363636363636363636363636").unwrap();
    let refresh_effect = store
        .apply_compute_source(&refresh_operation, &WorkspaceId::default(), 1, &mutation)
        .unwrap();
    store.activate_control(&refresh_effect).unwrap();
    let rows = store.compute_projection_rows().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0.source_id, physical_source_id);
    assert_eq!(rows[0].0.revision, 9);
    assert_eq!(rows[0].1.binding_id, canonical_binding_id);
    assert_eq!(rows[0].1.revision, 6);
    let continued_pool = store.credential_pool(&canonical_pool_id).unwrap().unwrap();
    assert_eq!(continued_pool.revision, 5);
    assert_eq!(continued_pool.credentials, pool.credentials);
    assert_eq!(continued_pool.source_id, physical_source_id);
    store.with_connection(|connection| {
        let count: u64 = connection
            .query_row("SELECT COUNT(*) FROM compute_sources", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    });
}
pub(super) fn v7_operation_json() -> (
    String,
    OperationId,
    WorkspaceId,
    CanonicalDigest,
    CanonicalDigest,
) {
    let registry = registry();
    let mut prepared = prepared(&registry);
    let suffix = "abababababababababababab";
    let source_id = format!("source/agent-{suffix}");
    prepared.desired.source.source_id = source_id.clone();
    prepared.desired.source.identity.account_subject_ref = format!("account/agent-{suffix}");
    prepared.desired.source.identity_digest = prepared.desired.source.identity.digest().unwrap();
    prepared.desired.binding.binding_id = format!("binding/agent-{suffix}");
    prepared.desired.binding.source_id = source_id.clone();
    prepared.desired.binding.source_identity_digest =
        prepared.desired.source.identity_digest.clone();
    prepared.desired.binding.credential_pool_id = Some(format!("pool/agent-{suffix}"));
    prepared.desired.inventory.source_id = source_id.clone();
    let identity = prepared.desired.credential_pool_identity.as_mut().unwrap();
    identity.pool_id = format!("pool/agent-{suffix}");
    identity.binding_id = format!("binding/agent-{suffix}");
    identity.binding_digest = CanonicalDigest::of(&prepared.desired.binding).unwrap();
    identity.source_id = source_id.clone();
    identity.source_identity_digest = prepared.desired.source.identity_digest.clone();
    prepared.validate().unwrap();
    let spec = ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: "compute.connection.apply".into(),
        resource_id: Some(source_id.clone()),
        desired_state: serde_json::json!({
            "connection_option_id": prepared.desired.source.connection_option_id,
            "source_id": source_id,
            "explicit_materialization": true,
            "expected_source_revision": 0,
            "projection": prepared,
        }),
    };
    let plan =
        TransactionPlanV1::from_compute_projection_planner(spec, None, prepared, &registry, true)
            .unwrap();
    let workspace = WorkspaceId::default();
    let request_digest = CanonicalDigest::of_bytes(b"v7-operation-request");
    let accepted_digest = CanonicalDigest::of_bytes(b"v7-accepted-change");
    let scope = IdempotencyScopeV1::new(
        "interactive-user",
        "ApplyComputeConnection",
        "v7-upgrade-fixture",
    )
    .unwrap();
    let operation_id = OperationId::derive(&workspace, &scope, &request_digest);
    let operation = OperationV1::new(
        operation_id.clone(),
        workspace.clone(),
        scope,
        request_digest.clone(),
        accepted_digest.clone(),
        RevisionSetV1 {
            target: 0,
            dependencies: BTreeMap::new(),
        },
        plan,
    )
    .unwrap();
    let mut durable = serde_json::to_value(operation).unwrap();
    let accepted = durable["accepted_digest"].clone();
    let expected_revisions = durable["expected_revisions"].clone();
    let plan = durable["plan"].as_object_mut().unwrap();
    let mut raw_projection = plan["spec"]["desired_state"]["projection"].clone();
    rewrite_projection_as_v7(&mut raw_projection);
    plan.get_mut("spec").unwrap()["desired_state"]["projection"] = raw_projection.clone();
    let spec_digest = CanonicalDigest::of(&plan["spec"]).unwrap();
    let mutation = plan.get_mut("control").unwrap()["compute_source_mutation"]
        .as_object_mut()
        .unwrap();
    mutation.insert(
        "desired".into(),
        raw_projection["desired"]["source"].clone(),
    );
    mutation.insert(
        "desired_projection".into(),
        raw_projection["desired"].clone(),
    );
    mutation.insert(
        "desired_source_digest".into(),
        serde_json::json!(CanonicalDigest::of(&mutation["desired"]).unwrap()),
    );
    mutation.insert(
        "desired_identity_digest".into(),
        mutation["desired"]["identity_digest"].clone(),
    );
    mutation.insert(
        "desired_projection_digest".into(),
        serde_json::json!(CanonicalDigest::of(&mutation["desired_projection"]).unwrap()),
    );
    mutation.insert("change_spec_digest".into(), serde_json::json!(spec_digest));
    let step_zero = serde_json::json!({
        "spec": plan["spec"].clone(),
        "accepted_digest": accepted,
        "expected_revisions": expected_revisions,
    });
    // This fixture isolates the schema-7 projection/CAS migration inside the current journal.
    // Historical journal proofs are covered by the independently frozen schema-22 sources.
    let step_two = serde_json::json!([
        plan["control"].clone(),
        serde_json::Value::Null,
        serde_json::Value::Null
    ]);
    let _ = plan;
    durable["steps"][0]["deterministic_input_digest"] = serde_json::json!(
        CanonicalDigest::of(&(
            "hiroute.operation-step-input/v1",
            hiroute_domain::OperationStepKind::Prepare,
            step_zero
        ))
        .unwrap()
    );
    durable["steps"][2]["deterministic_input_digest"] = serde_json::json!(
        CanonicalDigest::of(&(
            "hiroute.operation-step-input/v1",
            hiroute_domain::OperationStepKind::MaterializeSources,
            step_two
        ))
        .unwrap()
    );
    (
        serde_json::to_string(&durable).unwrap(),
        operation_id,
        workspace,
        request_digest,
        accepted_digest,
    )
}

fn rewrite_projection_as_v7(projection: &mut serde_json::Value) {
    let desired = projection.get_mut("desired").unwrap();
    let identity: SourceIdentityV1 =
        serde_json::from_value(desired["source"]["identity"].clone()).unwrap();
    let legacy_digest = CanonicalDigest::of(&identity).unwrap();
    desired["source"]["identity_digest"] = serde_json::json!(legacy_digest);
    desired["binding"]["source_identity_digest"] = serde_json::json!(legacy_digest);
    let binding_digest = CanonicalDigest::of(&desired["binding"]).unwrap();
    desired["credential_pool_identity"]["source_identity_digest"] =
        serde_json::json!(legacy_digest);
    desired["credential_pool_identity"]["binding_digest"] = serde_json::json!(binding_digest);
}

#[test]
fn real_v7_operation_fixture_decodes_stages_and_activates_after_v8_upgrade() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().unwrap();
    let root = directory.path().join("data");
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let database = root.join("control.db");
    let (encoded, operation_id, workspace, request_digest, accepted_digest) = v7_operation_json();
    let connection = rusqlite::Connection::open(&database).unwrap();
    crate::migrations::initialize_control_v7_fixture(&connection).unwrap();
    let operation: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    connection
        .execute(
            "INSERT INTO operations(
                operation_id,workspace_id,principal,operation_kind,idempotency_key,
                request_digest,accepted_change_digest,state,generation,operation_json,
                created_at,updated_at
             ) VALUES (?1,?2,'interactive-user','ApplyComputeConnection',
                       'v7-upgrade-fixture',?3,?4,'accepted',0,?5,1,1)",
            params![
                operation_id.as_str(),
                workspace.as_str(),
                request_digest.as_str(),
                accepted_digest.as_str(),
                encoded
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO writer_claim VALUES (1,?1,1)",
            params![operation_id.as_str()],
        )
        .unwrap();
    for step in operation["steps"].as_array().unwrap() {
        connection
            .execute(
                "INSERT INTO operation_steps VALUES (?1,?2,?3,?4,?5)",
                params![
                    operation_id.as_str(),
                    step["sequence"].as_u64().unwrap(),
                    step["kind"].as_str().unwrap(),
                    step["status"].as_str().unwrap(),
                    serde_json::to_string(&serde_json::json!({
                        "schema": "hiroute.operation-step/v1",
                        "step": step,
                    }))
                    .unwrap()
                ],
            )
            .unwrap();
    }
    drop(connection);
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600)).unwrap();

    let store = ControlStore::open(
        &crate::test_storage_authority(),
        &database,
        root.join("backups"),
    )
    .unwrap();
    let recovered = store.recoverable_operations().unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].request_digest, request_digest);
    assert_eq!(recovered[0].accepted_digest, accepted_digest);
    let mutation = recovered[0].plan.compute_source().unwrap();
    let effect = store
        .apply_compute_source(&operation_id, &workspace, 0, &mutation)
        .unwrap();
    assert!(matches!(
        store.observe_control(&operation_id, &workspace).unwrap(),
        EffectReconciliation::Staged(_)
    ));
    drop(store);

    let store = ControlStore::open(
        &crate::test_storage_authority(),
        &database,
        root.join("backups-2"),
    )
    .unwrap();
    let recovered = store.recoverable_operations().unwrap();
    assert_eq!(recovered.len(), 1);
    store.activate_control(&effect).unwrap();
    assert!(matches!(
        store.observe_control(&operation_id, &workspace).unwrap(),
        EffectReconciliation::Applied(_)
    ));
    assert_eq!(
        store
            .apply_compute_source(&operation_id, &workspace, 0, &mutation)
            .unwrap(),
        effect
    );
    store.with_connection(|connection| {
        let mut completed: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        completed["state"] = serde_json::json!("succeeded");
        connection
            .execute(
                "UPDATE operations SET state='succeeded',operation_json=?2
                 WHERE operation_id=?1",
                params![
                    operation_id.as_str(),
                    serde_json::to_string(&completed).unwrap()
                ],
            )
            .unwrap();
        connection.execute("DELETE FROM writer_claim", []).unwrap();
    });
    drop(store);
    let store = ControlStore::open(
        &crate::test_storage_authority(),
        &database,
        root.join("backups-completed"),
    )
    .unwrap();
    let completed = store.load_operation(&operation_id).unwrap().unwrap();
    assert_eq!(completed.state, hiroute_domain::OperationState::Succeeded);
    assert_eq!(completed.request_digest, request_digest);
    assert_eq!(completed.accepted_digest, accepted_digest);
    assert!(store.recoverable_operations().unwrap().is_empty());
}
