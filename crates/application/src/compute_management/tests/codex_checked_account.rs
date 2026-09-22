use super::*;

#[test]
fn codex_connection_only_model_uses_checked_same_account_without_expanding_saved_source() {
    let mut source = complete_management_source();
    source.provenance = hiroute_domain::ComputeManagementProvenanceV2::ConnectorOwned {
        connector_id: "connector/cpa".into(),
        account_ref: "account/current".into(),
    };
    source.authentication = GatewayAuthenticationSemanticsV1::Bearer;
    source.credentials.clear();
    source.validation = Some(hiroute_domain::ComputeManagementValidationV2 {
        approval_operation_id: "operation/check-a".into(),
        validation_ref: "validation/cpa".into(),
        validation_revision: 1,
    });
    let mut checked = cpa_verified_facts();
    checked.models = vec![
        connector_verified_model("model/luna", "gpt-5.6-luna"),
        connector_verified_model("model/sol", "gpt-5.6-sol"),
    ];
    let saved = &checked.models[0];
    source.models[0].model_ref = saved.model_ref.clone();
    source.models[0].upstream_model_id = saved.upstream_model_id.clone();
    source.models[0].display_name = saved.display_name.clone();
    source.models[0].catalog_configuration_id = saved.catalog_configuration_id.clone();
    source.models[0].membership = hiroute_domain::ComputeManagementMembershipV2::Catalog;
    source.models[0].capabilities = super::mutation_support::map_capabilities(&saved.capabilities);
    source.models[0].capability_evidence_digest = saved.capability_evidence_digest.clone();
    source.validate().unwrap();

    let fact = compile_connection_only_codex_model(&source, &checked, &checked.models[1]).unwrap();
    assert_eq!(fact.source_id, source.source_id);
    assert_eq!(fact.upstream_model_id, "gpt-5.6-sol");
    assert_eq!(fact.binding_revision, 1);
    assert_eq!(source.models.len(), 1);
    assert_eq!(source.models[0].upstream_model_id, "gpt-5.6-luna");

    let mut other_account = checked.clone();
    other_account.provenance = ComputeCandidateProvenanceV2::ConnectorOwned {
        connector_id: "connector/cpa".into(),
        account_ref: "account/other".into(),
    };
    assert!(
        compile_connection_only_codex_model(&source, &other_account, &other_account.models[1])
            .is_err()
    );
}

#[test]
fn protected_binding_debug_output_is_redacted() {
    let binding = ComputeCredentialBindingV2::NativeProtected {
        descriptor: ProtectedInputSourceDescriptorV1::ManualInput,
        input_slot: "protected-slot-canary".to_owned(),
    };
    let rendered = format!("{binding:?}");

    assert!(!rendered.contains("protected-slot-canary"));
    assert!(rendered.contains("<redacted>"));
}
