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
        ("openai.platform.global.v1", "gpt-5.6-luna", None),
        ("openai.platform.global.v1", "gpt-6-astra", None),
        ("openai.platform.global.v1", "gpt-6.1-terra", None),
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
        if model == "gpt-6-astra" {
            for (key, expected) in [
                ("tool", serde_json::json!(true)),
                ("vision", serde_json::json!(true)),
                ("streaming", serde_json::json!(true)),
                ("context_tokens", serde_json::json!(1_050_000)),
                ("max_output_tokens", serde_json::json!(128_000)),
            ] {
                let view = &defaults["models"][0]["fields"][key];
                assert!(view["saved_value"].is_null());
                assert_eq!(view["effective_value"], expected);
                assert!(matches!(
                    view["origin"].as_str(),
                    Some("embedded" | "website_cache" | "inferred")
                ));
            }
        }
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
        drop(stores);
        if model == "gpt-6-astra" {
            let custom: std::collections::BTreeMap<hiroute_domain::ModelDefaultFieldV1, hiroute_domain::ModelDefaultPreferenceV1> = serde_json::from_value(serde_json::json!({
                "tool":{"intent":"custom","value":false},"vision":{"intent":"custom","value":null},
                "context_tokens":{"intent":"custom","value":64000},"max_output_tokens":{"intent":"custom","value":4096},
                "native_reasoning":{"intent":"custom","value":{"kind":"discrete","parameter":"reasoning_effort","profiles":["low","high"],"default_profile":"low"}}
            })).unwrap();
            save_preferences(
                &runtime,
                &service,
                &saved.source_id,
                binding,
                custom.clone(),
                "custom-model-facts",
            );
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
            let read = dispatch(
                &service,
                "GetComputeDefaults",
                serde_json::json!({"source_id":saved.source_id}),
            );
            for (key, expected) in [
                ("tool", serde_json::json!(false)),
                ("vision", serde_json::Value::Null),
                ("context_tokens", serde_json::json!(64000)),
                ("max_output_tokens", serde_json::json!(4096)),
            ] {
                let field = &read["models"][0]["fields"][key];
                assert_eq!(field["saved_value"], expected);
                assert_eq!(field["effective_value"], expected);
                assert_eq!(field["intent"], "custom");
                assert_eq!(field["origin"], "custom");
            }
            let options = dispatch(
                &service,
                "GetPlanEditorOptions",
                serde_json::json!({"authoring":{"mode":"fixed_model","display_name":"custom-default-check","candidates":[{"binding_id":binding,"reasoning":{"kind":"profile","profile":"high"}}]},"include_unavailable":true}),
            );
            let context = dispatch(
                &service,
                "GetPlanEditorOptions",
                serde_json::json!({"editor":options["authoring"]["editor"],"include_unavailable":true}),
            );
            assert_eq!(
                context["context_window"]["maximum_tokens"], 64000,
                "{context}"
            );
            let candidate = options["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["binding_id"] == binding.as_str())
                .unwrap();
            assert_eq!(
                candidate["reasoning"]["profiles"],
                serde_json::json!(["low", "high"])
            );
            let raw = runtime
                .adapter
                .stores_lock()
                .unwrap()
                .control()
                .compute_management_source(&saved.source_id)
                .unwrap()
                .unwrap();
            assert_eq!(raw.models, saved.models);
            assert_eq!(raw.credentials, saved.credentials);
            let follow = custom
                .keys()
                .map(|field| (*field, ModelDefaultPreferenceV1::FollowMetadata))
                .collect();
            save_preferences(
                &runtime,
                &service,
                &saved.source_id,
                binding,
                follow,
                "follow-model-metadata",
            );
            let followed = dispatch(
                &service,
                "GetComputeDefaults",
                serde_json::json!({"source_id":saved.source_id}),
            );
            assert_eq!(
                followed["models"][0]["fields"]["context_tokens"]["effective_value"],
                1_050_000
            );
            assert_eq!(
                followed["models"][0]["fields"]["vision"]["effective_value"],
                true
            );
            assert_eq!(
                followed["models"][0]["fields"]["tool"]["intent"],
                "follow_metadata"
            );
        }
    }
}

fn save_preferences(
    runtime: &super::super::ProductionControlRuntime,
    service: &crate::control::LocalControlDaemon,
    source_id: &str,
    binding: &str,
    fields: std::collections::BTreeMap<ModelDefaultFieldV1, ModelDefaultPreferenceV1>,
    identity: &str,
) {
    let defaults = dispatch(
        service,
        "GetComputeDefaults",
        serde_json::json!({"source_id":source_id}),
    );
    let change:ComputeManagementChangeV3=serde_json::from_value(serde_json::json!({
        "schema":"hiroute.compute-management-change/v3", "change":{
            "schema":"hiroute.compute-management-change/v2","subject":{"kind":"saved_source","source_id":source_id},
            "expected_revisions":defaults["revisions"],"selected_model_refs":[],"intent":"save_ready","key_edits":[],"edit":{"action":"rename","display_name":"API reasoning regression"}
        },"default_preferences":{(binding):fields}
    })).unwrap();
    let p = runtime.adapter.preview_compute_save_v3(change).unwrap();
    let response = service.dispatch_wire(hiroute_application_api::LocalControlWireRequestV2 {
        schema_version: hiroute_application_api::LOCAL_CONTROL_SCHEMA_V2,
        request_id: identity.into(),
        operation_id: "ApplyComputeConnectionChange".into(),
        payload: serde_json::to_value(ComputeConnectionApplyRequestV1 {
            spec: p.spec,
            accept_digest: p.accept_digest,
            expected_revisions: p.expected_revisions,
            idempotency_key: identity.into(),
        })
        .unwrap(),
        protected_grant: None,
    });
    assert!(response.error.is_none(), "{:?}", response.error);
    assert_eq!(response.operation.unwrap().state, "succeeded");
}
