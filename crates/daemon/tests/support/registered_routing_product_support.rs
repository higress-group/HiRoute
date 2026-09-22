use super::*;
use hiroute_application_api::{
    CodexClientCapabilityPreviewV1, PlanContentChangeV2, PlanContentPreviewV2, PlanContentTargetV2,
    PlanEditorOptionsV1,
};
use hiroute_domain::{BillingClass, ComputeManagementRepositoryPort, WorkspaceId};
use hiroute_local_storage::LocalStorageSet;

pub async fn assert_candidate(daemon: &ProductDaemon, binding_id: &str) {
    let options: PlanEditorOptionsV1 = succeeded(
        daemon
            .client
            .query(
                "GetPlanEditorOptions",
                "registered-routing-options",
                &json!({}),
            )
            .await
            .unwrap(),
    );
    assert_eq!(options.candidates.len(), 1);
    let candidate = &options.candidates[0];
    assert_eq!(candidate.binding_id, binding_id);
    assert_eq!(candidate.model_configuration_id, "model.zhipu.glm-5.3");
    assert_eq!(candidate.billing_class, BillingClass::Subscription);
    assert!(candidate.routable);
    assert!(
        candidate
            .ingress_protocols
            .contains(&UpstreamProtocol::Messages)
    );
    assert!(
        candidate
            .ingress_protocols
            .contains(&UpstreamProtocol::Responses)
    );
}

pub async fn publish(
    daemon: &mut ProductDaemon,
    source: &ComputeManagementSnapshotV2,
) -> PlanContentPreviewV2 {
    let saved = &source.sources[0];
    let model = &saved.models[0];
    assert_candidate(daemon, &model.binding_id).await;
    let change: PlanContentChangeV2 = serde_json::from_value(json!({
        "schema": "hiroute.plan-content-change/v2",
        "target": {"intent": "create", "creation_key": "registered-product-plan"},
        "editor": {
            "schema": "hiroute.plan-editor/v2",
            "display_name": "Registered GLM plan",
            "purpose": "Use a saved registered model without legacy projection",
            "mode": "fixed_model",
            "candidates": [{"binding_id": model.binding_id}],
            "smart": {"economy": [], "primary": [], "primary_fallback": false, "classifier": {"kind":"local_rules"}, "complex_keywords": []},
            "free": {"candidates": [], "primary": [], "primary_fallback": false},
            "delegation_enabled": false,
            "requirements": {"tool": true, "streaming": true, "minimum_context_tokens": 4096, "minimum_output_tokens": 1024},
            "limits": {"maximum_attempts": 2, "request_timeout_ms": 30000, "attempt_timeout_ms": 30000}
        },
        "consumed_draft": null
    })).unwrap();
    let options: PlanEditorOptionsV1 = succeeded(
        daemon
            .client
            .query(
                "GetPlanEditorOptions",
                "registered-codex-capabilities",
                &json!({"editor": change.editor}),
            )
            .await
            .unwrap(),
    );
    let CodexClientCapabilityPreviewV1::Available { limitations, .. } = options
        .codex_capabilities
        .expect("compiled GLM Responses preview")
    else {
        panic!("registered GLM native Responses facts should support Codex routing");
    };
    assert!(
        limitations.is_empty(),
        "native Responses must not carry Messages limits"
    );
    let preview: PlanContentPreviewV2 = succeeded(
        daemon
            .client
            .query(
                "PreviewAgentPlanChange",
                "registered-plan-preview",
                &json!({"change": change}),
            )
            .await
            .unwrap(),
    );
    let groups = &preview
        .plan_version
        .compiled
        .body
        .materialized
        .attempt_owned
        .groups;
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].candidates.len(), 1);
    let candidate = &groups[0].candidates[0];
    assert!(matches!(
        hiroute_integrations::codex_plan_capability_preview(&preview.plan_version.compiled),
        CodexClientCapabilityPreviewV1::Available { .. }
    ));
    let bundled: Value = serde_json::from_slice(include_bytes!(
        "../../../../crates/integrations/src/agents/codex_bundled_catalog.json"
    ))
    .unwrap();
    let codex_catalog =
        hiroute_integrations::CodexCatalogSelection::for_current_adapter(bundled).unwrap();
    let merged = codex_catalog
        .append_plans(
            std::slice::from_ref(&preview.plan_version.compiled),
            hiroute_integrations::CodexDefaultPolicy {
                explicit_model: None,
                uses_codex_backend: true,
                allow_provider_model_fallback: false,
            },
            hiroute_integrations::CodexCatalogMetadataSourceV1::UserConfigured,
            None,
        )
        .expect("native GLM Responses plan enters Codex catalog");
    let codex_entry = merged["models"].as_array().unwrap().last().unwrap();
    assert_eq!(
        codex_entry["slug"],
        preview.plan_version.compiled.model_alias().as_str()
    );
    assert!(
        !codex_entry["description"]
            .as_str()
            .unwrap()
            .contains("Messages 候选")
    );
    assert_eq!(candidate.binding_id, model.binding_id);
    assert_eq!(candidate.binding_revision, model.revision);
    assert_eq!(candidate.source_id, saved.source_id);
    assert_eq!(candidate.source_revision, saved.revision);
    assert_eq!(candidate.connection_option_id, "zhipu.coding-plan.cn.v1");
    assert_eq!(candidate.offer_ref, "offer.zhipu.coding-plan");
    assert_eq!(candidate.billing_class, BillingClass::Subscription);
    assert_eq!(candidate.model_configuration_id, "model.zhipu.glm-5.3");
    assert_eq!(
        candidate.capability_id,
        "cap.zhipu.glm-5.3.coding-plan.responses"
    );
    assert_eq!(candidate.upstream_protocol, UpstreamProtocol::Responses);
    assert_eq!(
        candidate.endpoint,
        "https://open.bigmodel.cn/api/v1/responses"
    );
    assert_eq!(candidate.credential_refs.len(), 1);
    let target = hiroute_domain::ComputeManagementTargetV2 {
        scheme: saved.target.scheme.clone(),
        authority: saved.target.authority.clone(),
        port: saved.target.port,
        request_path: saved.target.request_path.clone(),
        upstream_protocol: saved.target.upstream_protocol,
        protocol_profile_id: saved.target.protocol_profile_id.clone(),
        protocol_profile_revision: saved.target.protocol_profile_revision,
    };
    assert_eq!(
        candidate.credential_destination_ref,
        Some(target.credential_destination().unwrap())
    );
    for (ingress, path, capability_id, required_headers) in [
        (
            UpstreamProtocol::Responses,
            "/api/v1/responses",
            "cap.zhipu.glm-5.3.coding-plan.responses",
            Vec::new(),
        ),
        (
            UpstreamProtocol::Messages,
            "/api/anthropic/v1/messages",
            "cap.zhipu.glm-5.3.coding-plan.messages",
            vec![("anthropic-version".into(), "2023-06-01".into())],
        ),
    ] {
        let profile = candidate
            .protocol_profiles
            .iter()
            .find(|profile| profile.ingress_protocol == ingress)
            .unwrap();
        assert_eq!(profile.capability.upstream_protocol, ingress);
        assert_eq!(profile.capability.capability_id, capability_id);
        assert_eq!(profile.capability.native_model, "glm-5.3");
        assert_eq!(profile.connector.request_path, path);
        assert_eq!(
            profile.connector.authentication.exact(),
            Some(&GatewayAuthenticationSemanticsV1::Bearer)
        );
        assert_eq!(
            profile.connector.headers.exact().unwrap().required_headers,
            required_headers
        );
        assert_eq!(
            profile.capability.request.function_tools,
            hiroute_domain::GatewayFidelityV1::Exact
        );
    }
    let mut unsupported = change.clone();
    unsupported.editor.requirements.vision = true;
    let rejected: MachineEnvelopeV2<Value> = daemon
        .client
        .query(
            "PreviewAgentPlanChange",
            "registered-vision-rejected",
            &json!({"change": unsupported}),
        )
        .await
        .unwrap();
    assert_ne!(
        rejected.status,
        MachineStatus::Succeeded,
        "unknown/unsupported vision must not qualify"
    );
    let applied: MachineEnvelopeV2<ApplyResultV1> = daemon.client.call_typed(LocalControlWireRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: "registered-plan-apply".into(), operation_id: "ApplyAgentPlanChange".into(),
        payload: json!({"change": change, "accept_digest": preview.change_digest,
            "expected_revisions": preview.expected_revisions, "idempotency_key": "registered-product-publish"}),
        protected_grant: None,
    }).await.unwrap();
    assert_eq!(applied.status, MachineStatus::Accepted, "{applied:?}");
    assert_eq!(applied.data.unwrap().state, "succeeded");
    let status: hiroute_application_api::AgentPlanStatusV2 = succeeded(
        daemon
            .client
            .query(
                "GetAgentPlanStatus",
                "registered-published-status",
                &json!({"agent_plan_id": preview.plan_head.reference.plan_id}),
            )
            .await
            .unwrap(),
    );
    assert_eq!(status.head, preview.plan_head);

    let mut update = change;
    update.target = PlanContentTargetV2::Update {
        plan_id: preview.plan_head.reference.plan_id.clone(),
        expected_head_revision: preview.plan_head.head_revision,
    };
    update.editor.display_name = "Registered GLM plan after restart".into();
    let updated: PlanContentPreviewV2 = succeeded(
        daemon
            .client
            .query(
                "PreviewAgentPlanChange",
                "registered-plan-update-preview",
                &json!({"change": update}),
            )
            .await
            .unwrap(),
    );
    assert_eq!(updated.plan_head.head_revision, 2);
    assert_eq!(updated.plan_head.reference.content_revision, 2);
    assert_eq!(
        updated
            .plan_version
            .compiled
            .body
            .materialized
            .attempt_owned
            .groups[0]
            .candidates[0],
        *candidate,
        "editing and republishing must retain the exact registered upstream"
    );
    let applied: MachineEnvelopeV2<ApplyResultV1> = daemon
        .client
        .call_typed(LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "registered-plan-update-apply".into(),
            operation_id: "ApplyAgentPlanChange".into(),
            payload: json!({"change": update, "accept_digest": updated.change_digest,
                "expected_revisions": updated.expected_revisions, "idempotency_key": "registered-product-republish"}),
            protected_grant: None,
        })
        .await
        .unwrap();
    assert_eq!(applied.status, MachineStatus::Accepted, "{applied:?}");
    assert_eq!(applied.data.unwrap().state, "succeeded");
    let status: hiroute_application_api::AgentPlanStatusV2 = succeeded(
        daemon
            .client
            .query(
                "GetAgentPlanStatus",
                "registered-republished-status",
                &json!({"agent_plan_id": updated.plan_head.reference.plan_id}),
            )
            .await
            .unwrap(),
    );
    assert_eq!(status.head, updated.plan_head);
    updated
}

pub fn assert_durable_routing(root: &std::path::Path, preview: &PlanContentPreviewV2) {
    let stores = LocalStorageSet::open_for_daemon_startup(root.join("storage")).unwrap();
    assert!(
        stores
            .control()
            .compute_projection_rows()
            .unwrap()
            .is_empty()
    );
    let source = &stores
        .control()
        .compute_management_snapshot(&WorkspaceId::default())
        .unwrap()
        .sources[0];
    let recheck = source
        .native_recheck
        .as_ref()
        .expect("a saved Registered source retains its exact endpoint/header descriptor");
    assert_eq!(recheck.inventory_path, None);
    assert!(
        recheck
            .protocol_header_semantics
            .required_headers
            .is_empty()
    );
    let candidate = &preview
        .plan_version
        .compiled
        .body
        .materialized
        .attempt_owned
        .groups[0]
        .candidates[0];
    let profile = &candidate.protocol_profiles[0];
    let request = hiroute_domain::NativeCredentialLeaseRequestV1 {
        schema_version: hiroute_domain::NATIVE_CREDENTIAL_LEASE_REQUEST_SCHEMA_V1.into(),
        stable_binding_id: candidate.binding_id.clone(),
        credential_id: candidate.credential_refs[0].clone(),
        credential_destination_ref: candidate.credential_destination_ref.clone().unwrap(),
        excluded_key_ids: Vec::new(),
        connector_runtime: candidate.connector_runtime,
        connector_id: candidate.connector_id.clone(),
        upstream_protocol: candidate.upstream_protocol,
        upstream_model_id: candidate.upstream_model_id.clone(),
        native_transport_model: candidate.native_transport_model.clone(),
        logical_endpoint: candidate.endpoint.clone(),
        operational_target: candidate.operational_target.clone(),
        operational_target_digest: candidate.operational_target_digest.clone(),
        request_path: profile.connector.request_path.clone(),
        runtime_epoch: None,
        target_epoch: None,
        protocol_profile_digest: candidate.protocol_profile_digest.clone(),
        authentication: profile.connector.authentication.exact().unwrap().clone(),
    };
    assert_eq!(source.credentials.len(), 1);
    assert_eq!(
        source.credentials[0].credential.credential_id(),
        request.credential_id
    );
    assert!(
        stores
            .secrets()
            .lease_native_credential_exact(&request)
            .unwrap()
            .is_some()
    );
    let messages = candidate
        .protocol_profiles
        .iter()
        .find(|profile| profile.ingress_protocol == UpstreamProtocol::Messages)
        .unwrap();
    let mut messages_request = request.clone();
    messages_request.upstream_protocol = UpstreamProtocol::Messages;
    messages_request.request_path = messages.connector.request_path.clone();
    messages_request.operational_target = candidate
        .operational_target
        .for_protocol_path(&messages.connector.request_path)
        .unwrap();
    messages_request.logical_endpoint = messages_request.operational_target.uri().into();
    messages_request.operational_target_digest =
        CanonicalDigest::of(&messages_request.operational_target).unwrap();
    messages_request.protocol_profile_digest = CanonicalDigest::of(messages).unwrap();
    assert!(
        stores
            .secrets()
            .lease_native_credential_exact(&messages_request)
            .unwrap()
            .is_some(),
        "the same saved key must authorize the catalog-bound Messages path"
    );
    let mut legacy_destination = request;
    legacy_destination.credential_destination_ref =
        format!("connection-option/{}", candidate.connection_option_id);
    assert!(
        stores
            .secrets()
            .lease_native_credential_exact(&legacy_destination)
            .is_err()
    );
    let db = rusqlite::Connection::open_with_flags(
        root.join("storage/live/control.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut statement = db
        .prepare(
            "SELECT version_json FROM plan_versions
             WHERE plan_id=? AND state='published' ORDER BY content_revision",
        )
        .unwrap();
    let persisted = statement
        .query_map([preview.plan_head.reference.plan_id.as_str()], |row| {
            row.get::<_, String>(0)
        })
        .unwrap()
        .map(|row| serde_json::from_str::<hiroute_domain::PlanVersionV1>(&row.unwrap()).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(persisted.len(), 2);
    assert_eq!(persisted[1], preview.plan_version);
    assert_eq!(
        persisted[0].compiled.body.materialized.attempt_owned.groups[0].candidates[0],
        persisted[1].compiled.body.materialized.attempt_owned.groups[0].candidates[0],
        "both immutable publications retain the same exact registered upstream"
    );
}
