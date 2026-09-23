#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hiroute_application::ApplicationService;
use hiroute_application_api::{
    ApplyResultV1, COMPUTE_CONNECTION_CHANGE_SCHEMA_V1, COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2,
    ClientHelloV1, ComputeCandidateFactStateV2, ComputeCandidateInputStateV2,
    ComputeCandidateProducerV2, ComputeCandidateProvenanceKindV2, ComputeCandidateViewV2,
    ComputeConnectionApplyRequestV1, ComputeConnectionChangeV1, ComputeConnectionOptionsResultV1,
    ComputeConnectionPreviewRequestV1, ComputeConnectionPreviewResultV1, ComputeManagementChangeV2,
    ComputeManagementIntentV2, ComputeManagementSubjectV2, ComputeSavePreviewRequestV2,
    ComputeSavePreviewV2, ComputeScanResultV1, ComputeSubscriptionCandidatesV2,
    ComputeSubscriptionDiscoveryStateV2, ErrorCode, LIST_COMPUTE_SUBSCRIPTIONS_OPERATION_V2,
    LOCAL_CONTROL_SCHEMA_V2, LocalControlWireRequestV2, MACHINE_ENVELOPE_SCHEMA_V2,
    MachineEnvelopeV2, MachineStatus, PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1,
    PrepareDiscoveredModelConnectionRequestV1, PrincipalKind, ProtectedClientGrantV2,
    ServerHelloV1,
};
use hiroute_cpa_bridge::{
    BorrowedCodexAuthSpec, CpaAccountKind, CpaHealth, CpaProfileBinding, CpaRuntimeSpec,
    MANAGED_CPA_ARTIFACT_VERSION, ManagedCpaRuntime, PinnedCpaArtifact, PinnedCpaBinaryLocator,
    RestartPolicy,
};
use hiroute_daemon::control::{ProductionControlRuntime, start_control};
use hiroute_domain::{
    CanonicalDigest, ConnectorRegistryBundleV1, ControlRepositoryPort, MaterializationState,
    OperationId, OperationState, ReleaseFactsManifestV2, ReleaseModelDataBundleV2, WorkspaceId,
};
use hiroute_integrations::TrustedReleaseCatalog;
use hiroute_local_storage::{ApplyCapabilityRegistrationV1, LocalStorageSet};
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use serde_json::{Value, json};

const CHILD_ROOT: &str = "HIROUTE_P25026_CHILD_ROOT";
const SECRET_SENTINEL: &str = "p25026-secret-must-never-leave-settings";
const TEST_NAME: &str =
    "production_compute_control_plane_is_transactional_and_routing_facts_fail_closed";
const DISCOVERED_PREPARE_TEST_NAME: &str =
    "registered_claude_configuration_prepares_from_group_writable_install";
const DISCOVERED_PREPARE_CHILD_ROOT: &str = "HIROUTE_DISCOVERED_PREPARE_CHILD_ROOT";

#[derive(Deserialize)]
struct CatalogFixtureV1 {
    schema: String,
    registry_json_line: String,
    model_data_json_line: String,
}

fn catalog_from_resources(
    catalog_id: &str,
    sequence: u64,
    registry: &[u8],
    model_data: &[u8],
) -> TrustedReleaseCatalog {
    let registry_value: ConnectorRegistryBundleV1 = serde_json::from_slice(registry).unwrap();
    let model_data_value: ReleaseModelDataBundleV2 = serde_json::from_slice(model_data).unwrap();
    let manifest = ReleaseFactsManifestV2 {
        schema: hiroute_domain::RELEASE_FACTS_SCHEMA_V2.into(),
        tool_version: hiroute_domain::RELEASE_FACTS_TOOL_VERSION_V2.into(),
        catalog_id: catalog_id.into(),
        product_release: registry_value.product_release.clone(),
        sequence,
        connector_registry_digest: CanonicalDigest::of_bytes(registry),
        model_data_digest: CanonicalDigest::of_bytes(model_data),
        cross_reference_digest: model_data_value
            .cross_reference_digest(&registry_value)
            .unwrap(),
    };
    TrustedReleaseCatalog::load_release_facts(
        &serde_json::to_vec(&manifest).unwrap(),
        registry,
        model_data,
    )
    .unwrap()
}

fn fixture_catalog() -> TrustedReleaseCatalog {
    let fixture: CatalogFixtureV1 = serde_json::from_str(include_str!(
        "../../../e2e/product/fixtures/control/pre-gateway-compute-routing.v1.json"
    ))
    .unwrap();
    assert_eq!(
        fixture.schema,
        "hiroute.pre-gateway-compute-routing-fixture/v1"
    );
    let registry = fixture.registry_json_line.into_bytes();
    catalog_from_resources(
        "fixture-pre-gateway-current",
        1,
        &registry,
        fixture.model_data_json_line.as_bytes(),
    )
}

fn current_catalog() -> TrustedReleaseCatalog {
    const MANIFEST: &[u8] =
        include_bytes!("../../../assets/release-facts/current/bundle/manifest.json");
    TrustedReleaseCatalog::load_bundled_release_facts(
        MANIFEST,
        MANIFEST,
        include_bytes!("../../../assets/release-facts/current/bundle/connector-registry.json"),
        include_bytes!("../../../assets/release-facts/current/bundle/model-data.json"),
    )
    .unwrap()
}

fn fixture_current_catalog(sequence: u64) -> TrustedReleaseCatalog {
    catalog_from_resources(
        "fixture-discovery-catalog-change",
        sequence,
        include_bytes!("../../../assets/release-facts/current/bundle/connector-registry.json"),
        include_bytes!("../../../assets/release-facts/current/bundle/model-data.json"),
    )
}

fn configured_but_stopped_cpa(root: &Path, auth_source: PathBuf) -> Arc<ManagedCpaRuntime> {
    let artifact = PinnedCpaArtifact::new(
        root,
        "unused-cpa-binary",
        MANAGED_CPA_ARTIFACT_VERSION.parse().unwrap(),
        "0".repeat(64),
    );
    Arc::new(
        ManagedCpaRuntime::new(
            CpaRuntimeSpec {
                instance_id: "subscription-discovery-test".into(),
                state_root: root.join("cpa/state"),
                auth_dir: root.join("cpa/auth"),
                borrowed_codex_auth: Some(BorrowedCodexAuthSpec::new(auth_source)),
                bindings: vec![CpaProfileBinding {
                    account_kind: CpaAccountKind::Codex,
                    connector_id: "connector.cpa.codex".into(),
                    connection_option_id: "codex.subscription.global.v1".into(),
                    endpoint_profile_id: "endpoint.cpa.codex".into(),
                }],
                startup_timeout: Duration::from_secs(1),
                control_timeout: Duration::from_secs(1),
                shutdown_timeout: Duration::from_secs(1),
                restart_policy: RestartPolicy::default(),
            },
            Arc::new(current_catalog()),
            Arc::new(PinnedCpaBinaryLocator::new(artifact)),
        )
        .unwrap(),
    )
}

fn request(
    application: &ApplicationService,
    operation_id: &str,
    request_id: &str,
    payload: Value,
    protected_grant: Option<ProtectedClientGrantV2>,
) -> MachineEnvelopeV2<Value> {
    application.dispatch(
        LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: request_id.into(),
            operation_id: operation_id.into(),
            payload,
            protected_grant,
        }
        .authenticate_ambient(),
    )
}

fn local_control_call(
    endpoint: &Path,
    client_name: &str,
    request: &LocalControlWireRequestV2,
) -> MachineEnvelopeV2<Value> {
    let mut stream = UnixStream::connect(endpoint).unwrap();
    serde_json::to_writer(
        &mut stream,
        &ClientHelloV1 {
            api_version: LOCAL_CONTROL_SCHEMA_V2,
            machine_schema_version: MACHINE_ENVELOPE_SCHEMA_V2,
            client_name: client_name.into(),
            client_version: env!("CARGO_PKG_VERSION").into(),
        },
    )
    .unwrap();
    stream.write_all(b"\n").unwrap();
    stream.flush().unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str::<ServerHelloV1>(&line).unwrap();
    serde_json::to_writer(&mut stream, request).unwrap();
    stream.write_all(b"\n").unwrap();
    stream.flush().unwrap();
    line.clear();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

fn semantic_database_snapshot(storage_root: &Path) -> BTreeMap<String, i64> {
    let mut snapshot = BTreeMap::new();
    for database in ["control", "runtime", "secrets"] {
        let path = storage_root.join(format!("live/{database}.db"));
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        let names = {
            let mut statement = connection
                .prepare(
                    "SELECT name FROM sqlite_schema
                     WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
                )
                .unwrap();
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        for table in names {
            let quoted = table.replace('"', "\"\"");
            let count = connection
                .query_row(&format!("SELECT count(*) FROM \"{quoted}\""), [], |row| {
                    row.get(0)
                })
                .unwrap();
            snapshot.insert(format!("{database}.{table}"), count);
        }
    }
    snapshot
}

fn assert_tree_omits(path: &Path, sentinel: &[u8]) {
    for entry in fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        let file_type = entry.file_type().unwrap();
        if file_type.is_dir() {
            assert_tree_omits(&entry.path(), sentinel);
        } else if file_type.is_file() {
            let bytes = fs::read(entry.path()).unwrap();
            assert!(
                !bytes
                    .windows(sentinel.len())
                    .any(|window| window == sentinel)
            );
        }
    }
}

fn run_production_path(root: &Path) {
    let storage_root = root.join("storage");
    let runtime =
        ProductionControlRuntime::open_with_release_catalog(&storage_root, fixture_catalog())
            .unwrap();
    let application = ApplicationService::new(runtime.application_ports());
    let before_preview = semantic_database_snapshot(&storage_root);

    let scan_envelope = request(&application, "ScanCompute", "scan-1", json!({}), None);
    assert_eq!(scan_envelope.status, MachineStatus::Succeeded);
    assert!(
        !serde_json::to_vec(&scan_envelope)
            .unwrap()
            .windows(SECRET_SENTINEL.len())
            .any(|window| window == SECRET_SENTINEL.as_bytes())
    );
    let scan: ComputeScanResultV1 = serde_json::from_value(scan_envelope.data.unwrap()).unwrap();
    let item = scan
        .items
        .iter()
        .find(|item| item.inventory_eligible)
        .unwrap();
    assert!(item.supported);
    let discovery = item.discovery.as_ref().unwrap();
    assert!(discovery.valid());
    assert_eq!(
        serde_json::to_value(discovery)
            .unwrap()
            .as_object()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        item.registered_base_url.as_deref(),
        Some("https://provider.fixture.invalid")
    );
    assert_eq!(
        item.observed_model_id.as_deref(),
        Some("fixture-messages-model")
    );
    assert_eq!(
        item.model_configuration_id.as_deref(),
        Some("model.fixture.messages")
    );

    let runtime_root = root.join("runtime");
    let mut control = start_control(application.clone(), &runtime_root).unwrap();
    let equivalent = LocalControlWireRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: "compute-equivalence".into(),
        operation_id: "ScanCompute".into(),
        payload: json!({}),
        protected_grant: None,
    };
    let cli = local_control_call(control.endpoint().path(), "hiroute-cli", &equivalent);
    let desktop = local_control_call(control.endpoint().path(), "hiroute-desktop", &equivalent);
    assert_eq!(cli, desktop);
    control.shutdown();
    control.join(Duration::from_secs(5)).unwrap();

    let options_envelope = request(
        &application,
        "ListConnectionOptions",
        "options-1",
        json!({}),
        None,
    );
    assert_eq!(options_envelope.status, MachineStatus::Succeeded);
    let options: ComputeConnectionOptionsResultV1 =
        serde_json::from_value(options_envelope.data.unwrap()).unwrap();
    assert_eq!(options.options.len(), 1);
    assert_eq!(options.catalog, scan.catalog);

    let change = ComputeConnectionChangeV1 {
        schema: COMPUTE_CONNECTION_CHANGE_SCHEMA_V1.into(),
        discovered_source_ref: item.discovered_source_ref.clone().unwrap(),
        connection_option_id: item.connection_option_id.clone().unwrap(),
        model_configuration_id: item.model_configuration_id.clone().unwrap(),
        expected_source_revision: 0,
        expected_binding_revision: 0,
        expected_inventory_revision: 0,
        explicit_materialization: true,
    };
    let preview_envelope = request(
        &application,
        "PreviewComputeConnectionChange",
        "preview-1",
        serde_json::to_value(ComputeConnectionPreviewRequestV1 {
            change: change.clone(),
        })
        .unwrap(),
        None,
    );
    assert_eq!(
        preview_envelope.status,
        MachineStatus::Succeeded,
        "preview envelope: {preview_envelope:?}"
    );
    let preview: ComputeConnectionPreviewResultV1 =
        serde_json::from_value(preview_envelope.data.unwrap()).unwrap();
    assert_eq!(preview.normalized_change, change);
    assert_eq!(
        preview.projection.source.state,
        MaterializationState::NeedsCredential
    );
    assert!(preview.projection.credential_pool_identity.is_some());
    assert_eq!(semantic_database_snapshot(&storage_root), before_preview);

    let mut tampered_spec = preview.spec.clone();
    tampered_spec.desired_state["projection"]["desired"]["catalog"]["model_data_digest"] =
        serde_json::to_value(CanonicalDigest::of_bytes(b"attacker-model-data")).unwrap();
    let tampered = request(
        &application,
        "ApplyComputeConnectionChange",
        "apply-tampered-catalog",
        serde_json::to_value(ComputeConnectionApplyRequestV1 {
            spec: tampered_spec,
            accept_digest: preview.change_digest.clone(),
            expected_revisions: preview.expected_revisions.clone(),
            idempotency_key: "tampered-catalog".into(),
        })
        .unwrap(),
        None,
    );
    assert_eq!(tampered.status, MachineStatus::NotFound);
    assert_eq!(tampered.error.unwrap().code, ErrorCode::ResourceNotFound);
    assert_eq!(semantic_database_snapshot(&storage_root), before_preview);

    drop(application);
    drop(runtime);
    let stores = LocalStorageSet::open_for_daemon_startup(&storage_root).unwrap();
    assert!(
        stores
            .control()
            .desired_state(&WorkspaceId::default())
            .unwrap()
            .is_none()
    );
    assert!(
        stores
            .control()
            .compute_projection_rows()
            .unwrap()
            .is_empty()
    );
    assert!(
        stores
            .control()
            .recoverable_operations()
            .unwrap()
            .is_empty()
    );
    drop(stores);

    let runtime =
        ProductionControlRuntime::open_with_release_catalog(&storage_root, fixture_catalog())
            .unwrap();
    let application = ApplicationService::new(runtime.application_ports());
    let apply_request = ComputeConnectionApplyRequestV1 {
        spec: preview.spec.clone(),
        accept_digest: preview.change_digest.clone(),
        expected_revisions: preview.expected_revisions.clone(),
        idempotency_key: "compute-projection-one".into(),
    };
    let apply_envelope = request(
        &application,
        "ApplyComputeConnectionChange",
        "apply-1",
        serde_json::to_value(&apply_request).unwrap(),
        None,
    );
    assert_eq!(
        apply_envelope.status,
        MachineStatus::Accepted,
        "apply envelope: {apply_envelope:?}"
    );
    let applied: ApplyResultV1 = serde_json::from_value(apply_envelope.data.unwrap()).unwrap();
    assert_eq!(applied.accepted_digest, preview.change_digest);
    assert_eq!(applied.state, OperationState::Succeeded.as_str());

    let routing_port = runtime.application_ports().routing.unwrap();
    let routing_snapshot = routing_port
        .routing_compilation_snapshot(&WorkspaceId::default())
        .unwrap();
    assert_eq!(routing_snapshot.facts.candidates.len(), 1);
    assert_eq!(
        routing_snapshot.facts.candidates[0].source_state,
        MaterializationState::NeedsCredential
    );
    assert!(routing_snapshot.facts.candidates[0].inventory_model_matched);
    let before_blocked_routing = semantic_database_snapshot(&storage_root);
    let blocked_routing = request(
        &application,
        "PreviewAgentPlanChange",
        "routing-needs-credential",
        json!({
            "change": {
                "schema": "hiroute.plan-content-change/v2",
                "target": {"intent": "create", "creation_key":"needs-credential"},
                "editor": {
                    "schema": "hiroute.plan-editor/v2",
                    "display_name": "Fixture",
                    "purpose": "fixture",
                    "requirements": {
                        "tool": false,
                        "vision": false,
                        "streaming": false,
                        "minimum_context_tokens": 0,
                        "minimum_output_tokens": 0
                    },
                    "limits": {
                        "maximum_attempts": 1,
                        "request_timeout_ms": 30000,
                        "attempt_timeout_ms": 30000
                    },
                    "mode": "fixed_model", "candidates":[{"binding_id": preview.projection.binding.binding_id.clone()}],
                    "smart":{"economy":[],"primary":[],"primary_fallback":false,"classifier":{"kind":"local_rules"},"complex_keywords":[]},
                    "free":{"candidates":[],"primary":[],"primary_fallback":false},
                    "delegation_enabled": false
                }
            }
        }),
        None,
    );
    assert_eq!(blocked_routing.status, MachineStatus::ActionRequired);
    assert_eq!(
        blocked_routing.error.unwrap().code,
        ErrorCode::ActionRequired
    );
    assert_eq!(
        semantic_database_snapshot(&storage_root),
        before_blocked_routing
    );

    let replay_envelope = request(
        &application,
        "ApplyComputeConnectionChange",
        "apply-replay",
        serde_json::to_value(&apply_request).unwrap(),
        None,
    );
    assert_eq!(replay_envelope.status, MachineStatus::Accepted);
    let replayed: ApplyResultV1 = serde_json::from_value(replay_envelope.data.unwrap()).unwrap();
    assert_eq!(replayed.operation_id, applied.operation_id);

    let second_preview_envelope = request(
        &application,
        "PreviewComputeConnectionChange",
        "preview-2",
        serde_json::to_value(ComputeConnectionPreviewRequestV1 {
            change: ComputeConnectionChangeV1 {
                expected_source_revision: 1,
                expected_binding_revision: 1,
                expected_inventory_revision: 1,
                ..change
            },
        })
        .unwrap(),
        None,
    );
    assert_eq!(second_preview_envelope.status, MachineStatus::Succeeded);
    let second_preview: ComputeConnectionPreviewResultV1 =
        serde_json::from_value(second_preview_envelope.data.unwrap()).unwrap();
    let before_denied = semantic_database_snapshot(&storage_root);
    let denied = request(
        &application,
        "ApplyComputeConnectionChange",
        "apply-unexpected-grant",
        serde_json::to_value(ComputeConnectionApplyRequestV1 {
            spec: second_preview.spec,
            accept_digest: second_preview.change_digest,
            expected_revisions: second_preview.expected_revisions,
            idempotency_key: "compute-projection-two".into(),
        })
        .unwrap(),
        Some(ProtectedClientGrantV2 {
            principal_kind: PrincipalKind::InteractiveUser,
            capability: "unexpected-grant".into(),
        }),
    );
    assert_eq!(denied.status, MachineStatus::Denied);
    assert_eq!(denied.error.unwrap().code, ErrorCode::CapabilityDenied);
    assert_eq!(semantic_database_snapshot(&storage_root), before_denied);

    drop(application);
    drop(runtime);
    let stores = LocalStorageSet::open_for_daemon_startup(&storage_root).unwrap();
    let rows = stores.control().compute_projection_rows().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0.state, MaterializationState::NeedsCredential);
    assert_eq!(rows[0].0.revision, 1);
    assert_eq!(rows[0].1.revision, 1);
    assert_eq!(rows[0].2.inventory_revision, 1);
    let pool_id = rows[0].1.credential_pool_id.as_ref().unwrap();
    assert!(stores.control().credential_pool(pool_id).unwrap().is_none());
    let desired = stores
        .control()
        .desired_state(&WorkspaceId::default())
        .unwrap()
        .unwrap();
    assert!(
        desired
            .pointer("/compute_source_mutation/desired_projection/credential_pool_identity")
            .is_some(),
        "desired={desired}"
    );
    let operation = stores
        .control()
        .load_operation(&OperationId::parse(applied.operation_id).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(operation.state, OperationState::Succeeded);
    assert_eq!(operation.idempotency.principal, "interactive-user");
    drop(stores);
    assert_tree_omits(&storage_root, SECRET_SENTINEL.as_bytes());
}

fn run_discovered_prepare_path(root: &Path) {
    let storage_root = root.join("storage");
    let runtime =
        ProductionControlRuntime::open_with_release_catalog(&storage_root, current_catalog())
            .unwrap();
    let ports = runtime.application_ports();
    let revisions = ports
        .control
        .snapshot(&WorkspaceId::default())
        .unwrap()
        .revisions;
    let application = ApplicationService::new(ports);
    let unavailable_subscriptions = request(
        &application,
        LIST_COMPUTE_SUBSCRIPTIONS_OPERATION_V2,
        "subscription-runtime-unavailable",
        json!({}),
        None,
    );
    assert_eq!(unavailable_subscriptions.status, MachineStatus::Succeeded);
    let unavailable_subscriptions: ComputeSubscriptionCandidatesV2 =
        serde_json::from_value(unavailable_subscriptions.data.unwrap()).unwrap();
    assert_eq!(
        unavailable_subscriptions.discovery_state,
        ComputeSubscriptionDiscoveryStateV2::RuntimeUnavailable
    );
    assert_eq!(
        unavailable_subscriptions.reason_code.as_deref(),
        Some("subscription_runtime_unavailable")
    );
    assert!(unavailable_subscriptions.candidates.is_empty());
    let scan_envelope = request(
        &application,
        "ScanCompute",
        "discovery-scan",
        json!({}),
        None,
    );
    assert_eq!(scan_envelope.status, MachineStatus::Succeeded);
    let scan: ComputeScanResultV1 = serde_json::from_value(scan_envelope.data.unwrap()).unwrap();
    let item = scan
        .items
        .iter()
        .find(|item| item.agent_id == "agent_claude_default")
        .unwrap();
    assert!(item.supported, "group-writable installation was rejected");
    assert_eq!(item.configuration_state, "registered_with_protected_input");
    assert!(item.inventory_eligible);
    assert_eq!(
        item.connection_option_id.as_deref(),
        Some("zhipu.coding-plan.cn.v1")
    );
    assert_eq!(
        item.model_configuration_id.as_deref(),
        Some("model.zhipu.glm-5.3")
    );
    assert!(item.discovered_source_ref.is_none());
    assert!(item.registered_base_url.is_none());
    assert!(item.credential_import.is_none());
    let prepare = PrepareDiscoveredModelConnectionRequestV1 {
        discovery: item.discovery.clone().unwrap(),
        prepare_id: "prepare/zhipu-coding-plan/1".into(),
    };
    let accepted_digest = CanonicalDigest::of(&prepare).unwrap();
    drop(application);
    drop(runtime);

    let capability = "discovered-prepare-desktop-capability-00000001".to_owned();
    let stores = LocalStorageSet::open_for_daemon_startup(&storage_root).unwrap();
    stores
        .apply_capability_registrar()
        .register(
            ApplyCapabilityRegistrationV1::from_protected_launcher(
                capability.clone(),
                "desktop",
                WorkspaceId::default(),
                PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1,
                accepted_digest,
                revisions,
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64
                    + 60,
            )
            .unwrap(),
        )
        .unwrap();
    drop(stores);

    let runtime =
        ProductionControlRuntime::open_with_release_catalog(&storage_root, current_catalog())
            .unwrap();
    let ports = runtime.application_ports();
    let current_revisions = ports
        .control
        .snapshot(&WorkspaceId::default())
        .unwrap()
        .revisions;
    let application = ApplicationService::new(ports);
    let before_prepare = semantic_database_snapshot(&storage_root);
    let grant = ProtectedClientGrantV2 {
        principal_kind: PrincipalKind::Desktop,
        capability,
    };
    let prepared_envelope = request(
        &application,
        PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1,
        "discovery-prepare",
        serde_json::to_value(&prepare).unwrap(),
        Some(grant.clone()),
    );
    assert_eq!(
        prepared_envelope.status,
        MachineStatus::Succeeded,
        "prepared envelope: {prepared_envelope:?}"
    );
    let candidate: ComputeCandidateViewV2 =
        serde_json::from_value(prepared_envelope.data.unwrap()).unwrap();
    assert_eq!(candidate.producer, ComputeCandidateProducerV2::Native);
    assert_eq!(
        candidate.provenance,
        ComputeCandidateProvenanceKindV2::Registered
    );
    assert_eq!(
        candidate.input_state,
        ComputeCandidateInputStateV2::Provided
    );
    assert_eq!(candidate.fact_state, ComputeCandidateFactStateV2::Complete);
    assert!(candidate.validation.is_none());
    assert!(candidate.issues.is_empty());
    assert_eq!(candidate.models.len(), 1);
    assert_eq!(candidate.models[0].upstream_model_id, "glm-5.3");
    assert!(candidate.models[0].selectable);
    let public_candidate = serde_json::to_vec(&candidate).unwrap();
    for forbidden in [
        SECRET_SENTINEL.as_bytes(),
        b"discovered_source_ref",
        b"field_selector",
        b"input_slot",
        b"scanner_id",
        b"authority",
    ] {
        assert!(
            !public_candidate
                .windows(forbidden.len())
                .any(|window| window == forbidden),
            "public candidate leaked {}",
            String::from_utf8_lossy(forbidden)
        );
    }
    let replay = request(
        &application,
        PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1,
        "discovery-prepare-replay",
        serde_json::to_value(&prepare).unwrap(),
        Some(grant),
    );
    assert_eq!(replay.status, MachineStatus::Succeeded);
    let replayed: ComputeCandidateViewV2 = serde_json::from_value(replay.data.unwrap()).unwrap();
    assert_eq!(replayed, candidate);

    let change = ComputeManagementChangeV2 {
        schema: COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.into(),
        subject: ComputeManagementSubjectV2::Candidate {
            candidate: candidate.candidate.clone(),
        },
        expected_revisions: current_revisions,
        selected_model_refs: vec![candidate.models[0].model_ref.clone()],
        intent: ComputeManagementIntentV2::SaveReady,
        key_edits: Vec::new(),
        validation: None,
    };
    let preview_envelope = request(
        &application,
        "PreviewComputeSave",
        "discovery-save-preview",
        serde_json::to_value(ComputeSavePreviewRequestV2 { change }).unwrap(),
        None,
    );
    assert_eq!(
        preview_envelope.status,
        MachineStatus::Succeeded,
        "save preview: {preview_envelope:?}"
    );
    let preview: ComputeSavePreviewV2 =
        serde_json::from_value(preview_envelope.data.unwrap()).unwrap();
    assert_eq!(preview.candidate, Some(candidate.candidate));
    assert_eq!(semantic_database_snapshot(&storage_root), before_prepare);
    assert!(!root.join("claude-or-helper-executed").exists());
    assert_tree_omits(&storage_root, SECRET_SENTINEL.as_bytes());

    drop(application);
    drop(runtime);
    let cpa = configured_but_stopped_cpa(root, root.join("home/.codex/auth.json"));
    assert!(matches!(
        cpa.health().unwrap(),
        CpaHealth::Stopped { last_exit: None }
    ));
    let runtime = ProductionControlRuntime::open_with_release_catalog_and_cpa(
        &storage_root,
        current_catalog(),
        Arc::clone(&cpa),
    )
    .unwrap();
    let application = ApplicationService::new(runtime.application_ports());
    let available_subscriptions = request(
        &application,
        LIST_COMPUTE_SUBSCRIPTIONS_OPERATION_V2,
        "subscription-runtime-configured",
        json!({}),
        None,
    );
    assert_eq!(available_subscriptions.status, MachineStatus::Succeeded);
    let available_subscriptions: ComputeSubscriptionCandidatesV2 =
        serde_json::from_value(available_subscriptions.data.unwrap()).unwrap();
    assert_eq!(
        available_subscriptions.discovery_state,
        ComputeSubscriptionDiscoveryStateV2::Complete
    );
    assert!(available_subscriptions.reason_code.is_none());
    assert_eq!(available_subscriptions.candidates.len(), 1);
    assert_eq!(
        available_subscriptions.candidates[0].producer,
        ComputeCandidateProducerV2::Cpa
    );
    assert!(available_subscriptions.candidates[0].models.is_empty());
    assert!(matches!(
        cpa.health().unwrap(),
        CpaHealth::Stopped { last_exit: None }
    ));
    assert_tree_omits(&storage_root, SECRET_SENTINEL.as_bytes());
}

fn run_discovered_catalog_change_rejection(root: &Path) {
    let storage_root = root.join("catalog-change-storage");
    let runtime = ProductionControlRuntime::open_with_release_catalog(
        &storage_root,
        fixture_current_catalog(1),
    )
    .unwrap();
    let application = ApplicationService::new(runtime.application_ports());
    let scanned = request(
        &application,
        "ScanCompute",
        "catalog-change-scan",
        json!({}),
        None,
    );
    let scanned: ComputeScanResultV1 = serde_json::from_value(scanned.data.unwrap()).unwrap();
    let discovery = scanned
        .items
        .iter()
        .find(|item| item.agent_id == "agent_claude_default")
        .and_then(|item| item.discovery.clone())
        .unwrap();
    let prepare = PrepareDiscoveredModelConnectionRequestV1 {
        discovery,
        prepare_id: "prepare/catalog-change".into(),
    };
    drop(application);
    drop(runtime);

    // The client-bundled catalog is immutable for one client revision. Use a fresh durable root to
    // model a later client revision while carrying only the old opaque discovery reference across
    // the daemon boundary.
    let updated_storage_root = root.join("catalog-change-storage-updated");
    let runtime = ProductionControlRuntime::open_with_release_catalog(
        &updated_storage_root,
        fixture_current_catalog(2),
    )
    .unwrap();
    let revisions = runtime
        .application_ports()
        .control
        .snapshot(&WorkspaceId::default())
        .unwrap()
        .revisions;
    drop(runtime);
    let capability = "catalog-change-prepare-capability-00000001".to_owned();
    let stores = LocalStorageSet::open_for_daemon_startup(&updated_storage_root).unwrap();
    stores
        .apply_capability_registrar()
        .register(
            ApplyCapabilityRegistrationV1::from_protected_launcher(
                capability.clone(),
                "desktop",
                WorkspaceId::default(),
                PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1,
                CanonicalDigest::of(&prepare).unwrap(),
                revisions,
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64
                    + 60,
            )
            .unwrap(),
        )
        .unwrap();
    drop(stores);

    let runtime = ProductionControlRuntime::open_with_release_catalog(
        &updated_storage_root,
        fixture_current_catalog(2),
    )
    .unwrap();
    let application = ApplicationService::new(runtime.application_ports());
    let rejected = request(
        &application,
        PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1,
        "catalog-change-prepare",
        serde_json::to_value(prepare).unwrap(),
        Some(ProtectedClientGrantV2 {
            principal_kind: PrincipalKind::Desktop,
            capability,
        }),
    );
    assert_eq!(rejected.status, MachineStatus::Conflict, "{rejected:?}");
    assert_eq!(
        rejected.error.unwrap().code,
        ErrorCode::RevisionConflict,
        "catalog provenance must be part of the opaque discovery evidence"
    );
}

fn configure_discovered_prepare_child(root: &Path) {
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
    let home = root.join("home");
    let binary_root = root.join("bin");
    let settings = home.join(".claude/settings.json");
    let execution_marker = root.join("claude-or-helper-executed");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::create_dir_all(&binary_root).unwrap();
    fs::create_dir_all(root.join("workspace")).unwrap();
    for storage in [
        root.join("storage"),
        root.join("catalog-change-storage"),
        root.join("catalog-change-storage-updated"),
    ] {
        fs::create_dir_all(&storage).unwrap();
        fs::set_permissions(storage, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(
        settings.parent().unwrap(),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    fs::set_permissions(root.join("workspace"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        &settings,
        serde_json::to_vec(&json!({
            "apiKeyHelper": format!("touch {}", execution_marker.display()),
            "env": {
                "ANTHROPIC_BASE_URL": "https://open.bigmodel.cn/api/anthropic",
                "ANTHROPIC_MODEL": "glm-5.3",
                "ANTHROPIC_AUTH_TOKEN": SECRET_SENTINEL
            }
        }))
        .unwrap(),
    )
    .unwrap();
    fs::set_permissions(&settings, fs::Permissions::from_mode(0o600)).unwrap();
    let claude = binary_root.join("claude");
    fs::write(
        &claude,
        "#!/bin/sh\nprintf '%s\\n' '2.1.231 (Claude Code)'\n",
    )
    .unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&binary_root, fs::Permissions::from_mode(0o775)).unwrap();
    let codex_auth = home.join(".codex/auth.json");
    fs::create_dir_all(codex_auth.parent().unwrap()).unwrap();
    fs::set_permissions(
        codex_auth.parent().unwrap(),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    fs::write(
        &codex_auth,
        serde_json::to_vec(&json!({
            "tokens": {"access_token": SECRET_SENTINEL},
            "account": {"id": "metadata-only-fixture"}
        }))
        .unwrap(),
    )
    .unwrap();
    fs::set_permissions(&codex_auth, fs::Permissions::from_mode(0o600)).unwrap();
}

fn configure_child(root: &Path) {
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
    let home = root.join("home");
    let binary_root = root.join("bin");
    let settings = home.join(".claude/settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::create_dir_all(&binary_root).unwrap();
    fs::create_dir_all(root.join("workspace")).unwrap();
    fs::create_dir_all(root.join("storage")).unwrap();
    fs::set_permissions(root.join("storage"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(
        settings.parent().unwrap(),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    fs::set_permissions(root.join("workspace"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        &settings,
        serde_json::to_vec(&json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://provider.fixture.invalid",
                "ANTHROPIC_MODEL": "fixture-messages-model",
                "ANTHROPIC_AUTH_TOKEN": SECRET_SENTINEL
            }
        }))
        .unwrap(),
    )
    .unwrap();
    fs::set_permissions(&settings, fs::Permissions::from_mode(0o600)).unwrap();
    let claude = binary_root.join("claude");
    fs::write(
        &claude,
        b"#!/bin/sh\nprintf '%s\\n' '2.1.231 (Claude Code)'\n",
    )
    .unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn production_compute_control_plane_is_transactional_and_routing_facts_fail_closed() {
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        run_production_path(&PathBuf::from(root));
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    configure_child(directory.path());
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST_NAME, "--nocapture"])
        .env(CHILD_ROOT, directory.path())
        .env("HOME", directory.path().join("home"))
        .env_remove("CODEX_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env("PATH", directory.path().join("bin"))
        .env_remove("ANTHROPIC_BASE_URL")
        .env_remove("ANTHROPIC_MODEL")
        .env_remove("ANTHROPIC_DEFAULT_OPUS_MODEL")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .current_dir(directory.path().join("workspace"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn registered_claude_configuration_prepares_from_group_writable_install() {
    if let Some(root) = std::env::var_os(DISCOVERED_PREPARE_CHILD_ROOT) {
        let root = PathBuf::from(root);
        run_discovered_prepare_path(&root);
        run_discovered_catalog_change_rejection(&root);
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    configure_discovered_prepare_child(directory.path());
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", DISCOVERED_PREPARE_TEST_NAME, "--nocapture"])
        .env(DISCOVERED_PREPARE_CHILD_ROOT, directory.path())
        .env("HOME", directory.path().join("home"))
        .env_remove("CODEX_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env("PATH", directory.path().join("bin"))
        .env_remove("ANTHROPIC_BASE_URL")
        .env_remove("ANTHROPIC_MODEL")
        .env_remove("ANTHROPIC_DEFAULT_OPUS_MODEL")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .current_dir(directory.path().join("workspace"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
