use super::*;
use hiroute_application::compute_management::ComputeSubscriptionMaintenanceScopeV1;
use hiroute_application_api::{ComputeValidationRefV2, OperationReferenceV1};
use hiroute_domain::ComputeManagementRepositoryPort;

fn subscription(candidate_ref: &str) -> ComputeCandidateFactsV2 {
    let mut facts = candidate(candidate_ref, "unused-subscription-input");
    let digest = hiroute_domain::CanonicalDigest::of_bytes(candidate_ref.as_bytes());
    let validation = ComputeValidationRefV2 {
        approval_operation: OperationReferenceV1 {
            operation_id: format!("op_{}", &digest.as_str()[7..39]),
            state: "succeeded".into(),
            sequence: 1,
            cancellable: false,
        },
        validation_ref: "validation/subscription".into(),
        validation_revision: 1,
    };
    facts.producer = ComputeCandidateProducerV2::Cpa;
    facts.lineage_ref = "lineage/subscription".into();
    facts.provenance = ComputeCandidateProvenanceV2::ConnectorOwned {
        connector_id: "connector/cpa".into(),
        account_ref: "account/subscription".into(),
    };
    facts.authentication = Some(GatewayAuthenticationSemanticsV1::Bearer);
    facts.native_recheck = None;
    facts.credential_binding = ComputeCredentialBindingV2::CpaOwned {
        account_ref: "account/subscription".into(),
        validation: validation.clone(),
    };
    facts.validation = Some(validation);
    for model in &mut facts.models {
        model.membership = ComputeModelMembershipV2::Observed;
        model.capabilities.tool.basis = ComputeCandidateFactBasisV2::ConnectorVerified;
        model.capabilities.vision.basis = ComputeCandidateFactBasisV2::ConnectorVerified;
        model.capabilities.streaming.basis = ComputeCandidateFactBasisV2::ConnectorVerified;
        model.capabilities.context_tokens.basis = ComputeCandidateFactBasisV2::ConnectorVerified;
        model.capabilities.max_output_tokens.basis = ComputeCandidateFactBasisV2::ConnectorVerified;
        model.capabilities.native_reasoning.basis = ComputeCandidateFactBasisV2::ConnectorVerified;
    }
    facts
}

fn change(
    stores: &LocalStorageSet,
    facts: &ComputeCandidateFactsV2,
    selected: &[&str],
) -> ComputeManagementChangeV2 {
    ComputeManagementChangeV2 {
        schema: "hiroute.compute-management-change/v2".into(),
        subject: ComputeManagementSubjectV2::Candidate {
            candidate: facts.candidate.clone(),
        },
        expected_revisions: stores
            .control()
            .compute_management_snapshot(&WorkspaceId::default())
            .unwrap()
            .revisions,
        selected_model_refs: selected.iter().map(|model| (*model).into()).collect(),
        intent: ComputeManagementIntentV2::SaveReady,
        key_edits: Vec::new(),
        validation: facts.validation.clone(),
    }
}

fn save(
    stores: &LocalStorageSet,
    registry: &TrustedComputeCandidateRegistry,
    facts: &ComputeCandidateFactsV2,
    selected: &[&str],
    key: &str,
) -> hiroute_domain::ComputeManagementSourceV2 {
    // This storage test starts at verified subscription facts. Admission still uses
    // the real durable handoff and transaction; the daemon tests cover the check.
    let validation = facts.validation.as_ref().unwrap();
    stores.control().with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO operations(
                    operation_id,workspace_id,principal,operation_kind,idempotency_key,
                    request_digest,accepted_change_digest,state,generation,operation_json,
                    created_at,updated_at
                 ) VALUES(?1,'personal/default','local-control','ApplySubscriptionCheck',?2,
                          'sha256:request','sha256:accepted','succeeded',1,'{}',1,1)",
                rusqlite::params![validation.approval_operation.operation_id, key],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO compute_subscription_validations(
                    operation_id,candidate_ref,candidate_revision,record_json,state,updated_at
                 ) VALUES(?1,?2,?3,?4,'verified',1)",
                rusqlite::params![
                    validation.approval_operation.operation_id,
                    facts.candidate.candidate_ref,
                    facts.candidate.candidate_revision,
                    serde_json::json!({ "validation": validation }).to_string(),
                ],
            )
            .unwrap();
    });
    registry.register_compute_candidate(facts.clone()).unwrap();
    let input = ProtectedInput;
    let planner =
        ComputeManagementPlanner::new(registry, stores.control(), stores.secrets(), &input);
    let preview = planner.preview(change(stores, facts, selected)).unwrap();
    let runtime = TransactionRuntime::default();
    let external = NoExternal;
    let coordinator = TransactionCoordinator::new(
        stores.control(),
        stores.secrets(),
        stores.runtime(),
        &external,
        &input,
        &runtime,
    );
    coordinator.reconcile_startup_and_open().unwrap();
    apply_preview(
        stores,
        &planner,
        &coordinator,
        &WorkspaceId::default(),
        preview,
        key,
    );
    let snapshot = stores
        .control()
        .compute_management_snapshot(&WorkspaceId::default())
        .unwrap();
    assert_eq!(snapshot.sources.len(), 1);
    snapshot.sources.into_iter().next().unwrap()
}

#[test]
fn subscription_explicit_selection_adds_and_removes_models_on_the_same_source() {
    let directory = tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let registry = TrustedComputeCandidateRegistry::new();
    let first = save(
        &stores,
        &registry,
        &subscription("candidate/first"),
        &["model/one"],
        "first",
    );
    let mut edited = subscription("candidate/add");
    edited.existing_source_id = Some(first.source_id.clone());
    let added = save(
        &stores,
        &registry,
        &edited,
        &["model/one", "model/two"],
        "add",
    );
    assert_eq!(added.source_id, first.source_id);
    assert_eq!(added.lineage_digest, first.lineage_digest);
    assert_eq!(added.provenance, first.provenance);
    assert_eq!(added.revision, first.revision + 1);
    assert_eq!(added.models.len(), 2);
    assert_eq!(added.models[0], first.models[0]);
    assert!(added.models.iter().all(|model| model.execution_eligible));
    assert!(added.credentials.is_empty());
    let second_binding = added.models[1].binding_id.clone();

    let mut edited = subscription("candidate/remove");
    edited.existing_source_id = Some(first.source_id.clone());
    let removed = save(&stores, &registry, &edited, &["model/two"], "remove");
    assert_eq!(removed.source_id, first.source_id);
    assert_eq!(removed.revision, added.revision + 1);
    assert_eq!(removed.models.len(), 1);
    assert_eq!(removed.models[0].model_ref, "model/two");
    assert_eq!(removed.models[0].binding_id, second_binding);

    drop(stores);
    let reopened = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let persisted = reopened
        .control()
        .compute_management_source(&removed.source_id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted, removed);
}

#[test]
fn subscription_explicit_selection_rejects_ineligible_and_unknown_models_without_writes() {
    let directory = tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let registry = TrustedComputeCandidateRegistry::new();
    let first = save(
        &stores,
        &registry,
        &subscription("candidate/first"),
        &["model/one"],
        "first",
    );
    let mut edited = subscription("candidate/invalid-add");
    edited.existing_source_id = Some(first.source_id.clone());
    edited.models[1].selectable = false;
    edited.models[1].reason = Some("model_not_allowed".into());
    registry.register_compute_candidate(edited.clone()).unwrap();
    let input = ProtectedInput;
    let planner =
        ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
    let before = stores
        .control()
        .compute_management_snapshot(&WorkspaceId::default())
        .unwrap();
    for selected in [
        vec!["model/one", "model/two"],
        vec!["model/unknown"],
        vec!["model/one", "model/one"],
    ] {
        assert!(matches!(
            planner.preview(change(&stores, &edited, &selected)),
            Err(ComputeManagementPlanningErrorV2::ModelNotSelectable)
        ));
    }
    let after = stores
        .control()
        .compute_management_snapshot(&WorkspaceId::default())
        .unwrap();
    assert_eq!(after, before);
}

#[test]
fn subscription_recheck_and_background_refresh_retain_membership_when_rights_change() {
    let directory = tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let registry = TrustedComputeCandidateRegistry::new();
    let first = save(
        &stores,
        &registry,
        &subscription("candidate/first"),
        &["model/one"],
        "first",
    );
    let mut refreshed = subscription("candidate/refresh");
    refreshed.existing_source_id = Some(first.source_id.clone());
    refreshed.models[0].selectable = false;
    refreshed.models[0].reason = Some("model_not_allowed".into());
    registry
        .register_compute_candidate(refreshed.clone())
        .unwrap();
    let input = ProtectedInput;
    let planner =
        ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
    let scope = ComputeSubscriptionMaintenanceScopeV1 {
        source_id: first.source_id.clone(),
        expected_source_revision: first.revision,
    };
    assert!(
        planner
            .preview_subscription_maintenance(change(&stores, &refreshed, &["model/one"]), &scope)
            .is_ok()
    );
    for selected in [vec!["model/one", "model/two"], vec!["model/two"]] {
        assert!(matches!(
            planner
                .preview_subscription_maintenance(change(&stores, &refreshed, &selected), &scope),
            Err(ComputeManagementPlanningErrorV2::ModelNotSelectable)
        ));
    }
    let retained = save(&stores, &registry, &refreshed, &["model/one"], "recheck");
    assert_eq!(retained.source_id, first.source_id);
    assert_eq!(retained.models.len(), 1);
    assert_eq!(retained.models[0].binding_id, first.models[0].binding_id);
    assert!(!retained.models[0].execution_eligible);
    assert!(
        compile_compute_management_source(&retained)
            .unwrap()
            .is_empty()
    );
}
