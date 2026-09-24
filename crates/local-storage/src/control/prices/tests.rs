use super::*;
use crate::test_tempdir as tempdir;
use hiroute_domain::*;
use serde_json::json;
fn setup(store: &ControlStore) -> SourcePriceChangeV2 {
    let identity = SourceIdentityV1 {
        identity_revision: 1,
        provider_platform_id: "provider.test".into(),
        service_offering_id: "service.test".into(),
        entitlement_id: "entitlement.test".into(),
        usage_scope: "account".into(),
        endpoint_profile_id: "endpoint.test".into(),
        endpoint_profile_revision: 1,
        region_id: "test".into(),
        account_subject_ref: "account.test".into(),
        evidence_refs: vec![CanonicalDigest::of_bytes(b"test-evidence")],
    };
    let source = ComputeSourceV1 {
        schema: COMPUTE_STATE_SCHEMA_V1.into(),
        source_id: "source.test".into(),
        revision: 1,
        connection_option_id: "test.native.v1".into(),
        connector_id: "connector.test".into(),
        connector_revision: 1,
        identity_digest: identity.digest().unwrap(),
        identity,
        origin: SourceOrigin::NativeApi,
        billing_class: BillingClass::Paid,
        state: MaterializationState::NeedsCredential,
    };
    let binding = SourceBindingV1 {
        binding_id: "binding.test".into(),
        revision: 1,
        source_id: source.source_id.clone(),
        source_revision: 1,
        source_identity_digest: source.identity_digest.clone(),
        model_data_bundle_version: "models.test".into(),
        capability_slice_version: "capabilities.test".into(),
        offer_ref: "offer.test".into(),
        offer_evidence_digest: CanonicalDigest::of_bytes(b"offer"),
        billing_class: BillingClass::Paid,
        model_configuration_id: "model.test".into(),
        upstream_model_id: "upstream.test".into(),
        capability_id: "capability.test".into(),
        credential_pool_id: None,
    };
    source.validate_shape().unwrap();
    binding.validate_shape().unwrap();
    let c = store.connection.borrow();
    c.execute("INSERT INTO compute_sources(source_id,revision,identity_digest,source_json,updated_at) VALUES (?1,1,?2,?3,0)",params![source.source_id,source.identity_digest.as_str(),serde_json::to_string(&source).unwrap()]).unwrap();
    c.execute("INSERT INTO source_bindings(binding_id,revision,source_id,binding_json,updated_at) VALUES (?1,1,?2,?3,0)",params![binding.binding_id,source.source_id,serde_json::to_string(&binding).unwrap()]).unwrap();
    let target = PriceTargetV1 {
        workspace_id: WorkspaceId::default(),
        source_id: source.source_id,
        source_identity_digest: source.identity_digest,
        model_identity: PriceModelIdentityV1::LocalModel(binding.model_configuration_id),
        currency: "USD".into(),
        valuation_kind: PriceValuationKindV1::UsageEstimate,
    };
    SourcePriceChangeV2 {
        schema: SOURCE_PRICE_CHANGE_SCHEMA_V2.into(),
        target: target.clone(),
        binding_id: binding.binding_id,
        expected_binding_revision: 1,
        expected_source_revision: 1,
        before: None,
        after: SourcePriceOverrideV1 {
            target,
            revision: 1,
            setting: SourcePriceSettingV1::Set {
                rates: TokenRatesV1::from_legacy(1_200_000, 4_800_000),
            },
        },
    }
}
fn plan(change: &SourcePriceChangeV2) -> TransactionPlanV1 {
    TransactionPlanV1::from_source_price_planner(ChangeSpecV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        command_id: "prices.override.apply".into(),
        resource_id: Some(change.target.source_id.clone()),
        desired_state: serde_json::to_value(change).unwrap(),
    })
    .unwrap()
}

fn management_fact<T>(value: T) -> ComputeManagementFactValueV2<T> {
    ComputeManagementFactValueV2 {
        value: Some(value),
        basis: ComputeManagementFactBasisV2::UserDeclared,
    }
}

fn management_source(source_id: &str, binding_id: &str) -> ComputeManagementSourceV2 {
    let source = ComputeManagementSourceV2 {
        schema: COMPUTE_MANAGEMENT_SOURCE_SCHEMA_V2.into(),
        source_id: source_id.into(),
        revision: 1,
        lineage_digest: CanonicalDigest::of_bytes(format!("lineage/{source_id}").as_bytes()),
        display_name: "Managed price source".into(),
        provenance: ComputeManagementProvenanceV2::UserConfigured {
            configuration_revision: 1,
            evidence_digest: CanonicalDigest::of_bytes(b"management-source-evidence"),
        },
        target: ComputeManagementTargetV2 {
            scheme: "https".into(),
            authority: "api.example.test".into(),
            port: 443,
            request_path: "/v1/responses".into(),
            upstream_protocol: UpstreamProtocol::Responses,
            protocol_profile_id: "profile.responses".into(),
            protocol_profile_revision: 1,
        },
        authentication: GatewayAuthenticationSemanticsV1::None,
        state: MaterializationState::Ready,
        models: vec![ComputeManagedModelV2 {
            model_ref: "model.management".into(),
            binding_id: binding_id.into(),
            revision: 1,
            upstream_model_id: "management-model".into(),
            display_name: "Management model".into(),
            catalog_configuration_id: None,
            membership: ComputeManagementMembershipV2::UserDeclared,
            execution_eligible: true,
            capabilities: ComputeManagedCapabilitiesV2 {
                tool: management_fact(true),
                vision: management_fact(false),
                streaming: management_fact(true),
                context_tokens: management_fact(16_384),
                max_output_tokens: management_fact(4_096),
                native_reasoning: ComputeManagementFactValueV2 {
                    value: None,
                    basis: ComputeManagementFactBasisV2::Unknown,
                },
            },
            capability_evidence_digest: CanonicalDigest::of_bytes(b"management-capabilities"),
        }],
        native_recheck: None,
        additional_native_endpoints: Vec::new(),
        credentials: Vec::new(),
        validation: None,
        last_candidate_ref: "candidate.management".into(),
        last_candidate_revision: 1,
    };
    source.validate().unwrap();
    source
}

fn insert_management_source(store: &ControlStore, source: &ComputeManagementSourceV2) {
    let owner = "op_40404040404040404040404040404040";
    let digest = CanonicalDigest::of_bytes(b"management-owner");
    let connection = store.connection.borrow();
    connection
        .execute(
            "INSERT INTO operations(
                operation_id,workspace_id,principal,operation_kind,idempotency_key,
                request_digest,accepted_change_digest,state,generation,operation_json,
                created_at,updated_at
             ) VALUES (?1,?2,'interactive-user','fixture.management','fixture-management',
                       ?3,?3,'succeeded',0,'{}',0,0)",
            params![owner, WorkspaceId::default().as_str(), digest.as_str()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO compute_management_sources(
                workspace_id,source_id,lineage_digest,revision,source_json,
                owner_operation_id,updated_at
             ) VALUES (?1,?2,?3,?4,?5,?6,0)",
            params![
                WorkspaceId::default().as_str(),
                source.source_id,
                source.lineage_digest.as_str(),
                source.revision,
                serde_json::to_string(source).unwrap(),
                owner,
            ],
        )
        .unwrap();
}

fn management_price_change(source: &ComputeManagementSourceV2) -> SourcePriceChangeV2 {
    let model = &source.models[0];
    let target = PriceTargetV1 {
        workspace_id: WorkspaceId::default(),
        source_id: source.source_id.clone(),
        source_identity_digest: source.lineage_digest.clone(),
        model_identity: PriceModelIdentityV1::LocalModel(model.model_ref.clone()),
        currency: "USD".into(),
        valuation_kind: PriceValuationKindV1::UsageEstimate,
    };
    SourcePriceChangeV2 {
        schema: SOURCE_PRICE_CHANGE_SCHEMA_V2.into(),
        target: target.clone(),
        binding_id: model.binding_id.clone(),
        expected_binding_revision: model.revision,
        expected_source_revision: source.revision,
        before: None,
        after: SourcePriceOverrideV1 {
            target,
            revision: 1,
            setting: SourcePriceSettingV1::Set {
                rates: TokenRatesV1::from_legacy(1_200_000, 4_800_000),
            },
        },
    }
}

#[test]
fn current_management_source_is_a_price_activation_cas_authority() {
    let dir = tempdir().unwrap();
    let store = ControlStore::open(
        &crate::test_storage_authority(),
        dir.path().join("control.db"),
        dir.path().join("backups"),
    )
    .unwrap();
    let source = management_source("source.management", "binding.management");
    insert_management_source(&store, &source);
    let change = management_price_change(&source);
    let effect = store
        .apply_control(
            &OperationId::parse("op_50505050505050505050505050505050").unwrap(),
            &WorkspaceId::default(),
            0,
            plan(&change).control(),
        )
        .unwrap();
    store.activate_control(&effect).unwrap();
    assert_eq!(
        store.source_price_override(&change.target).unwrap(),
        Some(change.after)
    );
}

#[test]
fn legacy_price_binding_survives_a_same_id_management_source_without_that_binding() {
    let dir = tempdir().unwrap();
    let store = ControlStore::open(
        &crate::test_storage_authority(),
        dir.path().join("control.db"),
        dir.path().join("backups"),
    )
    .unwrap();
    let legacy = setup(&store);
    let management = management_source(&legacy.target.source_id, "binding.management");
    insert_management_source(&store, &management);
    let effect = store
        .apply_control(
            &OperationId::parse("op_60606060606060606060606060606060").unwrap(),
            &WorkspaceId::default(),
            0,
            plan(&legacy).control(),
        )
        .unwrap();
    store.activate_control(&effect).unwrap();
    assert_eq!(
        store.source_price_override(&legacy.target).unwrap(),
        Some(legacy.after)
    );
}

#[test]
fn source_prices_control_activation_is_atomic_cas_recoverable_and_compensatable() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("control.db");
    let backup = dir.path().join("backups");
    let store = ControlStore::open(&crate::test_storage_authority(), &db, &backup).unwrap();
    let change = setup(&store);
    let p = plan(&change);
    let op = OperationId::parse("op_10101010101010101010101010101010").unwrap();
    let effect = store
        .apply_control(&op, &WorkspaceId::default(), 0, p.control())
        .unwrap();
    assert!(
        store
            .source_price_override(&change.target)
            .unwrap()
            .is_none()
    );
    drop(store);
    let store = ControlStore::open(&crate::test_storage_authority(), &db, &backup).unwrap();
    store.activate_control(&effect).unwrap();
    assert_eq!(
        store.source_price_override(&change.target).unwrap(),
        Some(change.after.clone())
    );
    store.activate_control(&effect).unwrap();
    assert_eq!(
        store
            .current_revisions(&WorkspaceId::default())
            .unwrap()
            .target,
        1
    );
    let op2 = OperationId::parse("op_20202020202020202020202020202020").unwrap();
    let stale = store
        .apply_control(&op2, &WorkspaceId::default(), 1, p.control())
        .unwrap();
    assert_eq!(
        store.activate_control(&stale).unwrap_err().code,
        PortErrorCode::Conflict
    );
    assert_eq!(
        store.source_price_override(&change.target).unwrap(),
        Some(change.after.clone())
    );
    assert_eq!(
        store
            .current_revisions(&WorkspaceId::default())
            .unwrap()
            .target,
        1
    );
    assert_eq!(
        store.compensate_control(&effect).unwrap(),
        CompensationOutcome::Compensated
    );
    assert!(
        store
            .source_price_override(&change.target)
            .unwrap()
            .is_none()
    );
}
#[test]
fn source_prices_identity_race_never_partially_writes() {
    let dir = tempdir().unwrap();
    let store = ControlStore::open(
        &crate::test_storage_authority(),
        dir.path().join("control.db"),
        dir.path().join("backups"),
    )
    .unwrap();
    let change = setup(&store);
    let p = plan(&change);
    let op = OperationId::parse("op_30303030303030303030303030303030").unwrap();
    let effect = store
        .apply_control(&op, &WorkspaceId::default(), 0, p.control())
        .unwrap();
    let mut source = store
        .compute_source(&change.target.source_id)
        .unwrap()
        .unwrap();
    source.revision += 1;
    store
        .connection
        .borrow()
        .execute(
            "UPDATE compute_sources SET revision=2,source_json=?1 WHERE source_id=?2",
            params![serde_json::to_string(&source).unwrap(), source.source_id],
        )
        .unwrap();
    assert_eq!(
        store.activate_control(&effect).unwrap_err().code,
        PortErrorCode::Conflict
    );
    assert!(
        store
            .source_price_override(&change.target)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .current_revisions(&WorkspaceId::default())
            .unwrap()
            .target,
        0
    );
    assert!(
        store
            .desired_state(&WorkspaceId::default())
            .unwrap()
            .is_none()
    );
    // Strict operation projection rejects a client-forged extra control effect.
    let mut forged = p.spec().clone();
    forged.desired_state["arbitrary"] = json!(true);
    assert!(TransactionPlanV1::from_source_price_planner(forged).is_err());
}
