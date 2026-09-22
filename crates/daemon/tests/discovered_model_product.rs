#![cfg(all(unix, feature = "integration-test-hooks"))]

use std::net::TcpListener;

use hiroute_application_api::{
    APPLY_COMPUTE_SAVE_OPERATION_V2, ApplyResultV1, COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2,
    ComputeCandidateFactStateV2, ComputeCandidateInputStateV2, ComputeCandidateProducerV2,
    ComputeCandidateProvenanceKindV2, ComputeConnectionAccessKindV1,
    ComputeConnectionApplyRequestV1, ComputeDiscoveryRefV1, ComputeManagementChangeV2,
    ComputeManagementIntentV2, ComputeManagementQueryV2, ComputeManagementSnapshotV2,
    ComputeManagementSubjectV2, ComputeModelMembershipV2, ComputeSaveDispositionV2,
    ComputeSavePreviewV2, ComputeSaveResultV2, ComputeScanRequestV1, ComputeScanResultV1,
    ErrorCode, LOCAL_CONTROL_SCHEMA_V2, LocalControlWireRequestV2, MachineEnvelopeV2,
    MachineStatus, PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1, PrincipalKind,
    ProtectedClientGrantV2,
};
use hiroute_domain::{
    CanonicalDigest, GatewayAuthenticationSemanticsV1, MaterializationState, RevisionSetV1,
    UpstreamProtocol,
};
use serde_json::{Value, json};

#[path = "support/discovered_model_product_support.rs"]
mod product_support;
#[path = "support/registered_routing_product_support.rs"]
mod routing_support;
use product_support::{
    CHANGED_PROJECT_SETTINGS, CORRECT_PROJECT_SETTINGS, ProductDaemon, SECRET_SENTINEL,
    assert_tree_omits, configure_product_root, write_project_settings,
};

async fn scan(daemon: &ProductDaemon, request_id: &str) -> ComputeScanResultV1 {
    succeeded(
        daemon
            .client
            .query("ScanCompute", request_id, &ComputeScanRequestV1 {})
            .await
            .unwrap(),
    )
}

async fn prepare(
    daemon: &mut ProductDaemon,
    request_id: &str,
    request: hiroute_application_api::PrepareDiscoveredModelConnectionRequestV1,
) -> MachineEnvelopeV2<hiroute_application_api::ComputeCandidateViewV2> {
    let revisions = daemon.revisions(&format!("{request_id}-status")).await;
    let grant = daemon.register(
        PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1,
        CanonicalDigest::of(&request).unwrap(),
        revisions,
    );
    daemon
        .client
        .prepare_discovered_model_connection(request_id, request, grant)
        .await
        .unwrap()
}

fn change(
    candidate: &hiroute_application_api::ComputeCandidateViewV2,
    revisions: RevisionSetV1,
) -> ComputeManagementChangeV2 {
    ComputeManagementChangeV2 {
        schema: COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.into(),
        subject: ComputeManagementSubjectV2::Candidate {
            candidate: candidate.candidate.clone(),
        },
        expected_revisions: revisions,
        selected_model_refs: vec![candidate.models[0].model_ref.clone()],
        intent: ComputeManagementIntentV2::SaveReady,
        key_edits: Vec::new(),
        validation: None,
    }
}

fn succeeded<T: std::fmt::Debug>(envelope: MachineEnvelopeV2<T>) -> T {
    assert_eq!(envelope.status, MachineStatus::Succeeded, "{envelope:?}");
    assert!(envelope.error.is_none(), "{envelope:?}");
    envelope.data.expect("successful response has data")
}

fn assert_error<T: std::fmt::Debug>(envelope: &MachineEnvelopeV2<T>, code: ErrorCode) {
    assert_eq!(envelope.status, code.status(), "{envelope:?}");
    assert_eq!(
        envelope.error.as_ref().map(|error| error.code),
        Some(code),
        "{envelope:?}"
    );
    assert!(envelope.data.is_none(), "{envelope:?}");
}

fn discovered(scan: &ComputeScanResultV1) -> ComputeDiscoveryRefV1 {
    let item = scan
        .items
        .iter()
        .find(|item| item.agent_id == "agent_claude_default")
        .unwrap();
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
    let discovery = item.discovery.clone().unwrap();
    assert!(discovery.valid());
    let public = serde_json::to_vec(item).unwrap();
    for forbidden in [
        SECRET_SENTINEL.as_bytes(),
        b"field_selector",
        b"input_slot",
        b"source_ref",
        b"open.bigmodel.cn",
    ] {
        assert!(
            !public
                .windows(forbidden.len())
                .any(|bytes| bytes == forbidden)
        );
    }
    assert_eq!(
        serde_json::to_value(&discovery)
            .unwrap()
            .as_object()
            .unwrap()
            .len(),
        2,
        "the UI correlation handle stays closed and opaque"
    );
    discovery
}

fn assert_saved_source(snapshot: &ComputeManagementSnapshotV2, source_id: &str) {
    assert_eq!(snapshot.sources.len(), 1);
    let source = &snapshot.sources[0];
    assert_eq!(source.source_id, source_id);
    assert_eq!(
        source.provenance,
        ComputeCandidateProvenanceKindV2::Registered
    );
    assert_eq!(
        source.connection_identity.access_kind,
        ComputeConnectionAccessKindV1::Api
    );
    assert_eq!(
        source.connection_identity.connection_option_id.as_deref(),
        Some("zhipu.coding-plan.cn.v1")
    );
    assert_eq!(source.target.scheme, "https");
    assert_eq!(source.target.authority, "open.bigmodel.cn");
    assert_eq!(source.target.port, 443);
    assert_eq!(source.target.request_path, "/api/v1/responses");
    assert_eq!(source.target.upstream_protocol, UpstreamProtocol::Responses);
    assert_eq!(
        source.target.protocol_profile_id,
        "adapter.openai-responses.v1"
    );
    assert_eq!(
        source.authentication,
        GatewayAuthenticationSemanticsV1::Bearer
    );
    assert_eq!(source.state, MaterializationState::Ready);
    assert_eq!(source.models.len(), 1);
    assert_eq!(source.models[0].upstream_model_id, "glm-5.3");
    assert_eq!(
        source.models[0].catalog_configuration_id.as_deref(),
        Some("model.zhipu.glm-5.3")
    );
    assert_eq!(
        source.models[0].membership,
        ComputeModelMembershipV2::Catalog
    );
    assert_eq!(
        source.keys.len(),
        1,
        "one source-bound credential is required"
    );
    assert!(source.keys[0].enabled);
    assert_eq!(source.ready_model_count, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hirouted_client_core_discovery_save_and_restart_are_closed_and_network_free() {
    let directory = tempfile::tempdir().unwrap();
    configure_product_root(directory.path());
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    proxy.set_nonblocking(true).unwrap();
    let proxy_address = proxy.local_addr().unwrap();
    let mut daemon = ProductDaemon::start(directory.path(), proxy_address);

    let initial_scan = scan(&daemon, "initial-scan").await;
    let initial_discovery = discovered(&initial_scan);
    let baseline_prepare = hiroute_application_api::PrepareDiscoveredModelConnectionRequestV1 {
        discovery: initial_discovery.clone(),
        prepare_id: "prepare/product/baseline".into(),
    };

    let unknown = daemon
        .client
        .call_wire(LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "prepare-unknown-field".into(),
            operation_id: PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1.into(),
            payload: json!({
                "discovery": initial_discovery,
                "prepare_id": "prepare/product/unknown-field",
                "unexpected": true,
            }),
            protected_grant: Some(ProtectedClientGrantV2 {
                principal_kind: PrincipalKind::Desktop,
                capability: "unknown-field-is-rejected-before-authority".into(),
            }),
        })
        .await
        .unwrap();
    assert_error(&unknown, ErrorCode::InvalidArguments);

    let non_desktop = daemon
        .client
        .call_wire(LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "prepare-non-desktop".into(),
            operation_id: PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1.into(),
            payload: serde_json::to_value(&baseline_prepare).unwrap(),
            protected_grant: Some(ProtectedClientGrantV2 {
                principal_kind: PrincipalKind::InteractiveUser,
                capability: "interactive-user-cannot-prepare-discovery".into(),
            }),
        })
        .await
        .unwrap();
    assert_error(&non_desktop, ErrorCode::CapabilityDenied);
    let unsupported_cancel: MachineEnvelopeV2<Value> = daemon
        .client
        .query(
            "CancelDiscoveredModelConnectionPrepare",
            "prepare-has-no-cancel-endpoint",
            &json!({}),
        )
        .await
        .unwrap();
    assert_error(&unsupported_cancel, ErrorCode::UnknownCommand);

    let wrong_revisions = daemon.revisions("wrong-digest-status").await;
    let wrong_grant = daemon.register(
        PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1,
        CanonicalDigest::of_bytes(b"wrong-operation-digest"),
        wrong_revisions,
    );
    let wrong_digest = daemon
        .client
        .prepare_discovered_model_connection(
            "prepare-wrong-digest",
            baseline_prepare.clone(),
            wrong_grant,
        )
        .await
        .unwrap();
    assert_error(&wrong_digest, ErrorCode::CapabilityDenied);

    let forged = hiroute_application_api::PrepareDiscoveredModelConnectionRequestV1 {
        discovery: ComputeDiscoveryRefV1 {
            discovery_ref: format!("discovery/{}", "0".repeat(64)),
            discovery_revision: initial_discovery.discovery_revision,
        },
        prepare_id: "prepare/product/forged".into(),
    };
    let forged_result = prepare(&mut daemon, "prepare-forged", forged).await;
    assert_error(&forged_result, ErrorCode::RevisionConflict);

    write_project_settings(directory.path(), CHANGED_PROJECT_SETTINGS);
    let scan_to_prepare = prepare(
        &mut daemon,
        "prepare-after-config-change",
        hiroute_application_api::PrepareDiscoveredModelConnectionRequestV1 {
            discovery: initial_discovery,
            prepare_id: "prepare/product/scan-stale".into(),
        },
    )
    .await;
    assert_error(&scan_to_prepare, ErrorCode::ActionRequired);
    write_project_settings(directory.path(), CORRECT_PROJECT_SETTINGS);

    let preview_scan = scan(&daemon, "preview-stale-rescan").await;
    let preview_candidate = succeeded(
        prepare(
            &mut daemon,
            "preview-stale-prepare",
            hiroute_application_api::PrepareDiscoveredModelConnectionRequestV1 {
                discovery: discovered(&preview_scan),
                prepare_id: "prepare/product/preview-stale".into(),
            },
        )
        .await,
    );
    let preview_revisions = daemon.revisions("preview-stale-status").await;
    write_project_settings(directory.path(), CHANGED_PROJECT_SETTINGS);
    let stale_preview = daemon
        .client
        .preview_compute_save(
            "preview-after-config-change",
            change(&preview_candidate, preview_revisions),
        )
        .await
        .unwrap();
    assert_error(&stale_preview, ErrorCode::ChangePreviewStale);
    write_project_settings(directory.path(), CORRECT_PROJECT_SETTINGS);

    let apply_scan = scan(&daemon, "apply-stale-rescan").await;
    let apply_candidate = succeeded(
        prepare(
            &mut daemon,
            "apply-stale-prepare",
            hiroute_application_api::PrepareDiscoveredModelConnectionRequestV1 {
                discovery: discovered(&apply_scan),
                prepare_id: "prepare/product/apply-stale".into(),
            },
        )
        .await,
    );
    let apply_snapshot = succeeded(
        daemon
            .client
            .compute_management_snapshot(
                "apply-stale-snapshot",
                ComputeManagementQueryV2::default(),
            )
            .await
            .unwrap(),
    );
    let apply_preview = succeeded(
        daemon
            .client
            .preview_compute_save(
                "apply-stale-preview",
                change(&apply_candidate, apply_snapshot.revisions),
            )
            .await
            .unwrap(),
    );
    write_project_settings(directory.path(), CHANGED_PROJECT_SETTINGS);
    let stale_apply = daemon
        .client
        .apply_compute_save(
            "apply-after-config-change",
            apply_request(&apply_preview, "product-stale-apply"),
        )
        .await
        .unwrap();
    assert_error(&stale_apply, ErrorCode::ChangePreviewStale);
    let empty = succeeded(
        daemon
            .client
            .compute_management_snapshot("empty-after-stale", ComputeManagementQueryV2::default())
            .await
            .unwrap(),
    );
    assert!(empty.sources.is_empty());
    write_project_settings(directory.path(), CORRECT_PROJECT_SETTINGS);

    let mut endpoint_sequence = vec!["ScanCompute"];
    let happy_scan = scan(&daemon, "happy-scan").await;
    let happy_request = hiroute_application_api::PrepareDiscoveredModelConnectionRequestV1 {
        discovery: discovered(&happy_scan),
        prepare_id: "prepare/product/happy".into(),
    };
    endpoint_sequence.push(PREPARE_DISCOVERED_MODEL_CONNECTION_OPERATION_V1);
    let candidate = succeeded(prepare(&mut daemon, "happy-prepare", happy_request.clone()).await);
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
    assert_eq!(candidate.models.len(), 1);
    assert_eq!(candidate.models[0].upstream_model_id, "glm-5.3");
    assert_eq!(
        candidate.models[0].membership,
        ComputeModelMembershipV2::Catalog
    );
    assert!(candidate.models[0].selectable);
    assert_eq!(candidate.correlation.check_id, happy_request.prepare_id);
    assert_eq!(
        candidate.correlation.edit_revision, happy_request.discovery.discovery_revision,
        "the UI can discard a late result by its exact prepare/edit correlation"
    );
    let replay =
        succeeded(prepare(&mut daemon, "happy-prepare-retry", happy_request.clone()).await);
    assert_eq!(
        replay, candidate,
        "same prepare_id and request must be retry-safe"
    );
    let collision = prepare(
        &mut daemon,
        "happy-prepare-id-collision",
        hiroute_application_api::PrepareDiscoveredModelConnectionRequestV1 {
            discovery: ComputeDiscoveryRefV1 {
                discovery_ref: format!("discovery/{}", "f".repeat(64)),
                discovery_revision: happy_request.discovery.discovery_revision,
            },
            prepare_id: happy_request.prepare_id.clone(),
        },
    )
    .await;
    assert_error(&collision, ErrorCode::RevisionConflict);

    let snapshot = succeeded(
        daemon
            .client
            .compute_management_snapshot("happy-snapshot", ComputeManagementQueryV2::default())
            .await
            .unwrap(),
    );
    endpoint_sequence.push("PreviewComputeSave");
    let preview = succeeded(
        daemon
            .client
            .preview_compute_save("happy-preview", change(&candidate, snapshot.revisions))
            .await
            .unwrap(),
    );
    endpoint_sequence.push(APPLY_COMPUTE_SAVE_OPERATION_V2);
    let applied_envelope = daemon
        .client
        .apply_compute_save(
            "happy-apply",
            apply_request(&preview, "product-happy-apply"),
        )
        .await
        .unwrap();
    assert_eq!(
        applied_envelope.status,
        MachineStatus::Accepted,
        "{applied_envelope:?}"
    );
    let applied: ApplyResultV1 = applied_envelope.data.clone().unwrap();
    assert_eq!(applied.state, "succeeded");
    let operation = applied_envelope.operation.clone().unwrap();
    endpoint_sequence.push("GetComputeSaveResult");
    let result: ComputeSaveResultV2 = succeeded(
        daemon
            .client
            .get_compute_save_result("happy-result", operation.clone())
            .await
            .unwrap(),
    );
    assert_eq!(result.disposition, ComputeSaveDispositionV2::Saved);
    assert_eq!(result.management_state, Some(MaterializationState::Ready));
    assert_eq!(result.bindings.len(), 1);
    let source_id = result.source_id.clone().unwrap();
    let saved = succeeded(
        daemon
            .client
            .compute_management_snapshot(
                "happy-saved-source",
                ComputeManagementQueryV2 {
                    source_id: Some(source_id.clone()),
                },
            )
            .await
            .unwrap(),
    );
    assert_saved_source(&saved, &source_id);
    routing_support::assert_candidate(&daemon, &result.bindings[0].binding_id).await;
    daemon.stop();

    let mut restarted = ProductDaemon::start(directory.path(), proxy_address);
    let restarted_result = succeeded(
        restarted
            .client
            .get_compute_save_result("restart-result", operation)
            .await
            .unwrap(),
    );
    assert_eq!(
        restarted_result.disposition,
        ComputeSaveDispositionV2::Saved
    );
    let expired_candidate = restarted
        .client
        .get_compute_candidate("restart-expired-candidate", candidate.candidate)
        .await
        .unwrap();
    assert_error(&expired_candidate, ErrorCode::ResourceNotFound);
    endpoint_sequence.push("GetCompute");
    let restarted_source = succeeded(
        restarted
            .client
            .compute_management_snapshot(
                "restart-source",
                ComputeManagementQueryV2 {
                    source_id: Some(source_id.clone()),
                },
            )
            .await
            .unwrap(),
    );
    assert_saved_source(&restarted_source, &source_id);
    let published = routing_support::publish(&mut restarted, &restarted_source).await;
    restarted.stop();
    routing_support::assert_durable_routing(directory.path(), &published);

    assert_eq!(
        endpoint_sequence,
        [
            "ScanCompute",
            "PrepareDiscoveredModelConnection",
            "PreviewComputeSave",
            "ApplyComputeSave",
            "GetComputeSaveResult",
            "GetCompute",
        ]
    );
    assert!(directory.path().join("claude-version-probed").exists());
    assert!(
        !directory
            .path()
            .join("claude-unexpected-invocation")
            .exists()
    );
    assert_tree_omits(
        &directory.path().join("storage"),
        SECRET_SENTINEL.as_bytes(),
    );
    assert!(matches!(
        proxy.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));
}

fn apply_request(
    preview: &ComputeSavePreviewV2,
    idempotency_key: &str,
) -> ComputeConnectionApplyRequestV1 {
    ComputeConnectionApplyRequestV1 {
        spec: preview.spec.clone(),
        accept_digest: preview.accept_digest.clone(),
        expected_revisions: preview.expected_revisions.clone(),
        idempotency_key: idempotency_key.into(),
    }
}
