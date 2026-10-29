//! Registered endpoint identity with real source-local model declarations and durable save.
use super::*;
use hiroute_application::compiler::CandidateFactAuthorityV1;
use hiroute_application::compute_management::{
    ComputeManagementCredentialCompilationV2, ComputeManagementEligibilityV2,
    compile_compute_management_source,
};
use hiroute_application::control::ComputeManagementControlPort;
use hiroute_application_api::{
    ComputeCandidateRefV2, ComputeConnectionApplyRequestV1, ComputeManagementChangeV2,
    ComputeManagementIntentV2, ComputeManagementSubjectV2, NativeUserCapabilityDeclarationV1,
    NativeUserFactBasisV1, NativeUserFactValueV1, NativeUserModelDeclarationV1,
};
use hiroute_domain::{
    ControlRepositoryPort, GatewayAuthenticationSemanticsV1, NativeReasoningCapabilityV1,
    ProtectedSecret, UpstreamProtocol, WorkspaceId,
};

const OPTION: &str = "bailian.token-plan.cn.v1";
const TEST: &str = "control::runtime::model_connections::registered::token_plan_tests::codex_v2_registered_user_models_save_reopen_and_keep_legacy_native_credentials";

fn user<T>(value: T) -> NativeUserFactValueV1<T> {
    NativeUserFactValueV1 {
        value: Some(value),
        basis: NativeUserFactBasisV1::UserDeclared,
    }
}

fn declaration(id: &str) -> NativeUserModelDeclarationV1 {
    NativeUserModelDeclarationV1 {
        upstream_model_id: id.into(),
        display_name: id.into(),
        catalog_configuration_id: None,
        membership: ComputeModelMembershipV2::UserDeclared,
        capabilities: NativeUserCapabilityDeclarationV1 {
            tool: user(true),
            vision: user(false),
            streaming: user(true),
            context_tokens: user(100_000),
            max_output_tokens: user(4_096),
            native_reasoning: user(NativeReasoningCapabilityV1::Fixed {
                profile: "provider-default".into(),
            }),
        },
    }
}

#[test]
fn codex_v2_registered_user_models_save_reopen_and_keep_legacy_native_credentials() {
    if crate::test_support::isolated_agent_home(TEST) {
        return;
    }
    let catalog = crate::release_catalog::current_fixture_catalog();
    let root = crate::test_support::private_tempdir();
    let provenance = catalog.compute_catalog_provenance().unwrap();
    let request = RegisteredModelConnectionCheckRequestV1 {
        inference_model_id: None,
        models: vec![declaration("qwen3.6-flash"), declaration("qwen3.7-plus")],
        connection_option_id: OPTION.into(),
        display_name: Some("Token Plan source-local models".into()),
        expected_catalog: ComputeCatalogProvenanceViewV1 {
            product_release: provenance.product_release,
            catalog_binding_id: provenance.catalog_binding_id,
            release_sequence: provenance.release_sequence,
            connector_registry_digest: provenance.connector_registry_digest,
            model_data_digest: provenance.model_data_digest,
            cross_reference_digest: provenance.cross_reference_digest,
        },
        candidate_ref: None,
        lineage_ref: "lineage/codex-v2-token-plan".into(),
        edit_revision: 1,
        check_id: "check/codex-v2-token-plan".into(),
        input_candidate: ComputeCandidateRefV2 {
            candidate_ref: "candidate/native/codex-v2-token-plan".into(),
            candidate_revision: 1,
        },
        existing_source_id: None,
        expected_source_revision: None,
    };
    let saved = {
        let runtime = crate::control::runtime::ProductionControlRuntime::open_with_release_catalog(
            root.path(),
            catalog.clone(),
        )
        .unwrap();
        let draft = runtime.adapter.trusted_registered_draft(&request).unwrap();
        // No inference or directory transport. Endpoints come from the authenticated registry;
        // model declarations remain UserDeclared even though the source is Registered.
        assert!(draft.inference_model_id.is_none());
        assert!(draft.inventory_path_override.is_none());
        assert_eq!(
            draft.authentication,
            GatewayAuthenticationSemanticsV1::Bearer
        );
        assert!(
            draft
                .models
                .iter()
                .all(|m| m.catalog_configuration_id.is_none())
        );
        runtime
            .register_manual_protected_input(
                request.input_candidate.clone(),
                ProtectedSecret::new(b"isolated-token-plan-not-a-real-key".to_vec()).unwrap(),
            )
            .unwrap();
        let checked = runtime
            .adapter
            .check_registered_model_connection(request.clone())
            .unwrap();
        assert_eq!(checked.candidate.models.len(), 2);
        assert!(
            checked
                .candidate
                .models
                .iter()
                .all(|m| m.selectable && m.membership == ComputeModelMembershipV2::UserDeclared)
        );
        let snapshot = runtime
            .adapter
            .stores_lock()
            .unwrap()
            .control()
            .compute_management_snapshot(&WorkspaceId::default())
            .unwrap();
        let preview = runtime
            .adapter
            .preview_compute_save(ComputeManagementChangeV2 {
                schema: hiroute_application_api::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.into(),
                subject: ComputeManagementSubjectV2::Candidate {
                    candidate: checked.candidate.candidate.clone(),
                },
                expected_revisions: snapshot.revisions,
                selected_model_refs: checked
                    .candidate
                    .models
                    .iter()
                    .map(|m| m.model_ref.clone())
                    .collect(),
                intent: ComputeManagementIntentV2::SaveReady,
                key_edits: Vec::new(),
                validation: None,
                edit: None,
            })
            .unwrap();
        let apply = ComputeConnectionApplyRequestV1 {
            spec: preview.spec,
            accept_digest: preview.accept_digest,
            expected_revisions: preview.expected_revisions,
            idempotency_key: "codex-v2-token-plan-save".into(),
        };
        let service = crate::control::LocalControlDaemon::new(
            hiroute_application::ApplicationService::new(runtime.application_ports()),
        );
        let dispatch = |apply: &ComputeConnectionApplyRequestV1| {
            service.dispatch_wire(hiroute_application_api::LocalControlWireRequestV2 {
                schema_version: hiroute_application_api::LOCAL_CONTROL_SCHEMA_V2,
                request_id: "codex-v2-token-plan-save".into(),
                operation_id: "ApplyComputeConnectionChange".into(),
                payload: serde_json::to_value(apply).unwrap(),
                protected_grant: None,
            })
        };
        // Candidate references have a different grammar from operation keys. Invalid
        // caller keys must not become DAEMON_UNAVAILABLE or admit any transaction.
        for key in [
            String::new(),
            request.input_candidate.candidate_ref.clone(),
            "key with spaces".into(),
            "key-\u{00e9}".into(),
            "x".repeat(201),
        ] {
            let mut invalid = apply.clone();
            invalid.idempotency_key = key;
            let rejected = dispatch(&invalid);
            assert_eq!(
                rejected.error.unwrap().code,
                hiroute_application_api::ErrorCode::InvalidArguments
            );
            assert!(rejected.operation.is_none());
            let stores = runtime.adapter.stores_lock().unwrap();
            assert!(
                stores
                    .control()
                    .recoverable_operations()
                    .unwrap()
                    .is_empty()
            );
            assert!(
                stores
                    .control()
                    .compute_management_snapshot(&WorkspaceId::default())
                    .unwrap()
                    .sources
                    .is_empty()
            );
        }
        // The same live preview and protected input remain usable after rejection.
        let applied = dispatch(&apply);
        assert!(applied.error.is_none(), "{:?}", applied.error);
        assert_eq!(applied.operation.as_ref().unwrap().state, "succeeded");
        let replay = dispatch(&apply);
        assert!(replay.error.is_none(), "{:?}", replay.error);
        assert_eq!(replay.operation, applied.operation);
        runtime
            .release_manual_protected_input(&request.input_candidate.candidate_ref)
            .unwrap();
        let snapshot = runtime
            .adapter
            .stores_lock()
            .unwrap()
            .control()
            .compute_management_snapshot(&WorkspaceId::default())
            .unwrap();
        assert_eq!(snapshot.sources.len(), 1);
        snapshot.sources.into_iter().next().unwrap()
    };
    let runtime = crate::control::runtime::ProductionControlRuntime::open_with_release_catalog(
        root.path(),
        catalog.clone(),
    )
    .unwrap();
    let reopened = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .compute_management_source(&saved.source_id)
        .unwrap()
        .unwrap();
    assert_eq!(reopened, saved);
    let facts = compile_compute_management_source(&reopened).unwrap();
    assert_eq!(facts.len(), 2);
    for fact in &facts {
        assert_eq!(
            fact.eligibility,
            ComputeManagementEligibilityV2::RuntimeQualified
        );
        assert!(fact.catalog_configuration_id.is_none());
        let candidate =
            crate::control::runtime::candidate_execution::materialize_management_candidate(
                fact,
                Some(&catalog),
            )
            .unwrap();
        assert_eq!(
            candidate.authority,
            CandidateFactAuthorityV1::RuntimeFallback
        );
        assert_eq!(candidate.connection_option_id, OPTION);
        let profile = candidate
            .protocol_profiles
            .iter()
            .find(|p| p.ingress_protocol == UpstreamProtocol::Responses)
            .unwrap();
        assert_eq!(
            profile.adapter_revision,
            hiroute_domain::STANDARD_RESPONSES_JSON_ADAPTER_V1
        );
        let target = profile.native_target.as_ref().unwrap();
        assert_eq!(
            target.operational_target.uri(),
            "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/responses"
        );
        assert!(target.validate_for(profile));
    }

    // Retained old Registered native destinations remain separate. This is an explicit old
    // source fixture; never rewrite a user's source or claim catalog model provenance.
    let mut legacy = facts[0].clone();
    let responses = legacy
        .additional_native_endpoints
        .iter_mut()
        .find(|e| e.target.upstream_protocol == UpstreamProtocol::Responses)
        .unwrap();
    responses.target.protocol_profile_id = "adapter.openai-responses.v1".into();
    let legacy_destination = responses.target.credential_destination().unwrap();
    let destinations = std::iter::once(legacy.target.credential_destination().unwrap())
        .chain(
            legacy
                .additional_native_endpoints
                .iter()
                .map(|e| e.target.credential_destination().unwrap()),
        )
        .collect::<Vec<_>>();
    legacy.credential = ComputeManagementCredentialCompilationV2::Native {
        ordered: vec![hiroute_domain::ComputeCredentialSelectionV2::Credential {
            credential_ref: hiroute_domain::CredentialRefV1::new(
                "credential/retained-native",
                format!("source/{}", legacy.source_id),
                "hirouted",
                "provider-auth",
                destinations,
                1,
            )
            .unwrap(),
        }],
    };
    let candidate = crate::control::runtime::candidate_execution::materialize_management_candidate(
        &legacy,
        Some(&catalog),
    )
    .unwrap();
    let profile = candidate
        .protocol_profiles
        .iter()
        .find(|p| p.ingress_protocol == UpstreamProtocol::Responses)
        .unwrap();
    assert_eq!(profile.adapter_revision, "adapter.openai-responses.v1@1");
    assert_eq!(
        profile
            .native_target
            .as_ref()
            .unwrap()
            .credential_destination_ref,
        legacy_destination
    );
    assert!(
        profile
            .native_target
            .as_ref()
            .unwrap()
            .validate_for(profile)
    );
    legacy
        .additional_native_endpoints
        .iter_mut()
        .find(|e| e.target.upstream_protocol == UpstreamProtocol::Responses)
        .unwrap()
        .target
        .authority = "127.0.0.1".into();
    assert!(
        crate::control::runtime::candidate_execution::materialize_management_candidate(
            &legacy,
            Some(&catalog)
        )
        .is_none()
    );
    let subscription = catalog
        .resolve_connection_option("codex.subscription.global.v1")
        .unwrap();
    assert!(
        subscription
            .endpoint_profile
            .protocol_endpoints
            .iter()
            .all(|e| e.adapter_ref != "adapter.standard-responses-json.v1")
    );
}
