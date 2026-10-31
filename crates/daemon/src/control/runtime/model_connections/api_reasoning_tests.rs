//! Real registered check/preview/save/reopen/read/routing; only the upstream directory is synthetic.
use super::*;
use hiroute_application::control::ComputeManagementControlPort;
use hiroute_application_api::{
    ComputeCandidateRefV2, ComputeCatalogProvenanceViewV1, ComputeConnectionApplyRequestV1,
    ComputeManagementChangeV3, ComputeManagementIntentV2, NativeUserCapabilityDeclarationV1,
    NativeUserFactBasisV1, NativeUserFactValueV1, NativeUserModelDeclarationV1,
};
use hiroute_domain::{
    ModelDefaultFieldV1, ModelDefaultPreferenceV1, NativeReasoningCapabilityV1, ProtectedSecret,
};
use hiroute_integrations::{
    ModelConnectionProbeCredentialV1, ModelDirectoryHttpResponseV1, ModelDirectoryTransportErrorV1,
    ModelDirectoryTransportV1, NormalizedModelConnectionTargetV1,
};
use std::sync::Arc;
use std::time::Duration;

struct Directory(&'static str);
impl ModelDirectoryTransportV1 for Directory {
    fn get(
        &self,
        _: &NormalizedModelConnectionTargetV1,
        _: Option<&str>,
        _: Option<ModelConnectionProbeCredentialV1<'_>>,
        _: Duration,
        _: usize,
    ) -> Result<ModelDirectoryHttpResponseV1, ModelDirectoryTransportErrorV1> {
        Ok(ModelDirectoryHttpResponseV1 {
            status: 200,
            body: serde_json::to_vec(&serde_json::json!({"data":[{"id":self.0}]})).unwrap(),
            truncated: false,
        })
    }
}
fn user<T>(value: T) -> NativeUserFactValueV1<T> {
    NativeUserFactValueV1 {
        value: Some(value),
        basis: NativeUserFactBasisV1::UserDeclared,
    }
}
fn dispatch(
    service: &crate::control::LocalControlDaemon,
    operation: &str,
    payload: serde_json::Value,
) -> serde_json::Value {
    let response = service.dispatch_wire(hiroute_application_api::LocalControlWireRequestV2 {
        schema_version: hiroute_application_api::LOCAL_CONTROL_SCHEMA_V2,
        request_id: "api-reasoning-regression".into(),
        operation_id: operation.into(),
        payload,
        protected_grant: None,
    });
    assert!(response.error.is_none(), "{:?}", response.error);
    response.data.unwrap()
}
#[test]
fn api_reasoning_survives_normal_v3_save_reopen_and_preserves_manual_values() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::model_connections::api_reasoning_tests::api_reasoning_survives_normal_v3_save_reopen_and_preserves_manual_values",
    ) {
        return;
    }
    let catalog = crate::release_catalog::current_fixture_catalog();
    let fixed = NativeReasoningCapabilityV1::Fixed {
        profile: "non-thinking".into(),
    };
    let subset = NativeReasoningCapabilityV1::Discrete {
        parameter: "reasoning_effort".into(),
        profiles: vec!["low".into(), "high".into()],
        default_profile: Some("low".into()),
    };
    for (option, model, manual) in [
        ("openai.platform.global.v1", "gpt-6.1-sol", None),
        ("anthropic.platform.global.v1", "claude-opus-5-5", None),
        ("openai.platform.global.v1", "gpt-6.1-sol", Some(fixed)),
        ("openai.platform.global.v1", "gpt-6.1-sol", Some(subset)),
    ] {
        let root = crate::test_support::private_tempdir();
        let runtime =
            super::super::ProductionControlRuntime::open_with_release_catalog_and_model_transport(
                root.path(),
                catalog.clone(),
                Arc::new(Directory(model)),
            )
            .unwrap();
        let p = catalog.compute_catalog_provenance().unwrap();
        let input = ComputeCandidateRefV2 {
            candidate_ref: "candidate/native/api-reasoning".into(),
            candidate_revision: 1,
        };
        let request = RegisteredModelConnectionCheckRequestV1 {
            inference_model_id: None,
            models: manual
                .clone()
                .map(|reasoning| {
                    vec![NativeUserModelDeclarationV1 {
                        upstream_model_id: model.into(),
                        display_name: model.into(),
                        catalog_configuration_id: None,
                        membership: ComputeModelMembershipV2::UserDeclared,
                        capabilities: NativeUserCapabilityDeclarationV1 {
                            tool: user(true),
                            vision: user(false),
                            streaming: user(true),
                            context_tokens: user(100_000),
                            max_output_tokens: user(4_096),
                            native_reasoning: user(reasoning),
                        },
                    }]
                })
                .unwrap_or_default(),
            connection_option_id: option.into(),
            display_name: Some("API reasoning regression".into()),
            expected_catalog: ComputeCatalogProvenanceViewV1 {
                product_release: p.product_release,
                catalog_binding_id: p.catalog_binding_id,
                release_sequence: p.release_sequence,
                connector_registry_digest: p.connector_registry_digest,
                model_data_digest: p.model_data_digest,
                cross_reference_digest: p.cross_reference_digest,
            },
            candidate_ref: None,
            lineage_ref: "lineage/api-reasoning".into(),
            edit_revision: 1,
            check_id: "check/api-reasoning".into(),
            input_candidate: input.clone(),
            existing_source_id: None,
            expected_source_revision: None,
        };
        runtime
            .register_manual_protected_input(
                input.clone(),
                ProtectedSecret::new(b"test-only-api-key".to_vec()).unwrap(),
            )
            .unwrap();
        let checked = runtime
            .adapter
            .check_registered_model_connection(request)
            .unwrap();
        let selected = checked
            .candidate
            .models
            .iter()
            .find(|m| m.upstream_model_id == model)
            .unwrap();
        assert!(selected.selectable);
        let revisions = runtime
            .adapter
            .stores_lock()
            .unwrap()
            .control()
            .current_revisions(&WorkspaceId::default())
            .unwrap();
        let preview = runtime
            .adapter
            .preview_compute_save_v3(ComputeManagementChangeV3 {
                schema: hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3.into(),
                change: ComputeManagementChangeV2 {
                    schema: hiroute_application_api::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.into(),
                    subject: ComputeManagementSubjectV2::Candidate {
                        candidate: checked.candidate.candidate.clone(),
                    },
                    expected_revisions: revisions,
                    selected_model_refs: vec![selected.model_ref.clone()],
                    intent: ComputeManagementIntentV2::SaveReady,
                    key_edits: vec![],
                    validation: None,
                    edit: None,
                },
                default_preferences: Default::default(),
                api_link: None,
            })
            .unwrap();
        let apply = ComputeConnectionApplyRequestV1 {
            spec: preview.spec,
            accept_digest: preview.accept_digest,
            expected_revisions: preview.expected_revisions,
            idempotency_key: "api-reasoning-save".into(),
        };
        let service = crate::control::LocalControlDaemon::new(
            hiroute_application::ApplicationService::new(runtime.application_ports()),
        );
        let response = service.dispatch_wire(hiroute_application_api::LocalControlWireRequestV2 {
            schema_version: hiroute_application_api::LOCAL_CONTROL_SCHEMA_V2,
            request_id: "api-reasoning-save".into(),
            operation_id: "ApplyComputeConnectionChange".into(),
            payload: serde_json::to_value(apply).unwrap(),
            protected_grant: None,
        });
        assert!(response.error.is_none(), "{:?}", response.error);
        assert_eq!(response.operation.unwrap().state, "succeeded");
        let saved = runtime
            .adapter
            .stores_lock()
            .unwrap()
            .control()
            .compute_management_snapshot(&WorkspaceId::default())
            .unwrap()
            .sources
            .remove(0);
        let binding = &saved.models[0].binding_id;
        let preferences = runtime
            .adapter
            .stores_lock()
            .unwrap()
            .control()
            .source_default_preferences(&saved.source_id)
            .unwrap();
        assert_eq!(
            preferences[binding][&ModelDefaultFieldV1::NativeReasoning],
            ModelDefaultPreferenceV1::PreserveSaved
        );
        assert_eq!(saved.models[0].capabilities.native_reasoning.value, manual);
        drop(service);
        drop(runtime);
        let runtime = super::super::ProductionControlRuntime::open_with_release_catalog(
            root.path(),
            catalog.clone(),
        )
        .unwrap();
        let service = crate::control::LocalControlDaemon::new(
            hiroute_application::ApplicationService::new(runtime.application_ports()),
        );
        let defaults = dispatch(
            &service,
            "GetComputeDefaults",
            serde_json::json!({"source_id":saved.source_id}),
        );
        let field = &defaults["models"][0]["fields"]["native_reasoning"];
        assert_eq!(field["saved_value"], serde_json::to_value(&manual).unwrap());
        assert_eq!(field["intent"], "preserve_saved");
        let expected = if let Some(manual) = manual {
            assert_eq!(field["origin"], "preserved");
            manual
        } else {
            assert!(matches!(
                field["origin"].as_str(),
                Some("embedded" | "website_cache")
            ));
            let effective: NativeReasoningCapabilityV1 =
                serde_json::from_value(field["effective_value"].clone()).unwrap();
            assert!(
                matches!(&effective, NativeReasoningCapabilityV1::Discrete {profiles, ..} if profiles.contains(&"high".to_owned()))
            );
            effective
        };
        assert_eq!(
            field["effective_value"],
            serde_json::to_value(&expected).unwrap()
        );
        let models = dispatch(&service, "ListComputeV3", serde_json::json!({}));
        assert_eq!(
            models["sources"][0]["models"][0]["native_reasoning"],
            serde_json::to_value(&expected).unwrap()
        );
        let options = dispatch(
            &service,
            "GetPlanEditorOptions",
            serde_json::json!({"authoring":{},"include_unavailable":true}),
        );
        let candidate = options["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["binding_id"] == binding.as_str())
            .expect("saved API binding must stay routable");
        assert_eq!(
            candidate["reasoning"],
            serde_json::to_value(&expected).unwrap()
        );
        let stores = runtime.adapter.stores_lock().unwrap();
        assert_eq!(
            stores
                .control()
                .compute_management_source(&saved.source_id)
                .unwrap()
                .unwrap(),
            saved
        );
        assert_eq!(
            stores
                .control()
                .source_default_preferences(&saved.source_id)
                .unwrap(),
            preferences
        );
    }
}
