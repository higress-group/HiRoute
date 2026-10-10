use super::*;

fn pending_candidate(candidate_ref: &str, revision: u64) -> ComputeCandidateFactsV2 {
    let mut facts = candidate(candidate_ref, "unused-pending-input");
    facts.candidate.candidate_revision = revision;
    facts.correlation.edit_revision = revision;
    facts.correlation.check_id = format!("check/{candidate_ref}/{revision}");
    facts.correlation.input_digest =
        CanonicalDigest::of_bytes(format!("pending-input/{revision}").as_bytes());
    facts.display_name = format!("Pending endpoint revision {revision}");
    facts.evidence_digest =
        CanonicalDigest::of_bytes(format!("pending-evidence/{revision}").as_bytes());
    facts.provenance = ComputeCandidateProvenanceV2::UserConfigured {
        configuration_revision: revision,
        evidence_digest: CanonicalDigest::of_bytes(
            format!("pending-configuration/{revision}").as_bytes(),
        ),
    };
    facts.credential_binding = ComputeCredentialBindingV2::NativePendingInput;
    mark_connection_check_required(&mut facts);
    facts
}

fn mark_connection_check_required(facts: &mut ComputeCandidateFactsV2) {
    for model in &mut facts.models {
        model.selectable = false;
        model.reason = Some("model_connections.connection_check_required".into());
    }
}

#[test]
fn pending_credential_disabled_save_uses_only_the_current_candidate_revision() {
    let directory = tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let registry = TrustedComputeCandidateRegistry::new();
    let first = pending_candidate("candidate/pending", 1);
    registry.register_compute_candidate(first.clone()).unwrap();
    let current = pending_candidate("candidate/pending", 2);
    registry
        .register_compute_candidate(current.clone())
        .unwrap();

    let stale_get = registry
        .get_compute_candidate(&first.candidate)
        .unwrap_err();
    assert_eq!(stale_get.code, PortErrorCode::Conflict);

    let input = ProtectedInput;
    let workspace = WorkspaceId::default();
    let planner =
        ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
    let revisions = hiroute_domain::ComputeManagementRepositoryPort::compute_management_snapshot(
        stores.control(),
        &workspace,
    )
    .unwrap()
    .revisions;
    let change = |candidate: ComputeCandidateRefV2, intent| ComputeManagementChangeV2 {
        edit: None,
        schema: "hiroute.compute-management-change/v2".into(),
        subject: ComputeManagementSubjectV2::Candidate { candidate },
        expected_revisions: revisions.clone(),
        selected_model_refs: vec!["model/one".into(), "model/two".into()],
        intent,
        key_edits: Vec::new(),
        validation: None,
    };
    let Err(stale_preview) = planner.preview(change(
        first.candidate,
        ComputeManagementIntentV2::SaveDisabled,
    )) else {
        panic!("a superseded candidate revision must not preview");
    };
    assert!(matches!(
        stale_preview,
        ComputeManagementPlanningErrorV2::RevisionConflict
    ));

    let Err(ready) = planner.preview(change(
        current.candidate.clone(),
        ComputeManagementIntentV2::SaveReady,
    )) else {
        panic!("an unavailable candidate model must not become ready");
    };
    assert!(matches!(
        ready,
        ComputeManagementPlanningErrorV2::ModelNotSelectable
    ));

    let preview = planner
        .preview(change(
            current.candidate.clone(),
            ComputeManagementIntentV2::SaveDisabled,
        ))
        .unwrap();
    assert!(preview.plan().secrets().is_empty());
    let transaction_runtime = TransactionRuntime::default();
    let external = NoExternal;
    let coordinator = TransactionCoordinator::new(
        stores.control(),
        stores.secrets(),
        stores.runtime(),
        &external,
        &input,
        &transaction_runtime,
    );
    coordinator.reconcile_startup_and_open().unwrap();
    apply_preview(
        &stores,
        &planner,
        &coordinator,
        &workspace,
        preview,
        "mvp-11-pending-disabled-save",
    );

    let snapshot = hiroute_domain::ComputeManagementRepositoryPort::compute_management_snapshot(
        stores.control(),
        &workspace,
    )
    .unwrap();
    assert_eq!(snapshot.sources.len(), 1);
    let source = &snapshot.sources[0];
    assert_eq!(source.state, MaterializationState::NeedsCredential);
    assert_eq!(source.last_candidate_revision, 2);
    assert_eq!(source.models.len(), 2);
    assert!(source.credentials.is_empty());
    assert_eq!(
        source.models[0].capabilities.context_tokens.value,
        Some(32_000)
    );
    assert!(matches!(
        compile_compute_management_source(source),
        Err(ComputeManagementCompilationErrorV2::NotReady)
    ));
}

#[test]
fn failed_connection_can_save_disabled_user_declared_models_but_not_ready() {
    let directory = tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let registry = TrustedComputeCandidateRegistry::new();
    let mut facts = candidate("candidate/connection-failed", "slot/primary");
    mark_connection_check_required(&mut facts);
    facts.models[1].membership = ComputeModelMembershipV2::Observed;
    facts.models[1].reason = Some("model_connections.capability_required".into());
    registry.register_compute_candidate(facts.clone()).unwrap();

    let input = ProtectedInput;
    let workspace = WorkspaceId::default();
    let planner =
        ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
    let revisions = hiroute_domain::ComputeManagementRepositoryPort::compute_management_snapshot(
        stores.control(),
        &workspace,
    )
    .unwrap()
    .revisions;
    let change = |intent| ComputeManagementChangeV2 {
        edit: None,
        schema: "hiroute.compute-management-change/v2".into(),
        subject: ComputeManagementSubjectV2::Candidate {
            candidate: facts.candidate.clone(),
        },
        expected_revisions: revisions.clone(),
        selected_model_refs: vec!["model/one".into()],
        intent,
        key_edits: Vec::new(),
        validation: None,
    };
    let Err(ready) = planner.preview(change(ComputeManagementIntentV2::SaveReady)) else {
        panic!("a failed connection must not become ready");
    };
    assert!(matches!(
        ready,
        ComputeManagementPlanningErrorV2::ModelNotSelectable
    ));
    let mut inventory_change = change(ComputeManagementIntentV2::SaveDisabled);
    inventory_change.selected_model_refs = vec!["model/two".into()];
    let Err(inventory) = planner.preview(inventory_change) else {
        panic!("an observed model with unknown capabilities must remain unavailable");
    };
    assert!(matches!(
        inventory,
        ComputeManagementPlanningErrorV2::ModelNotSelectable
    ));

    let preview = planner
        .preview(change(ComputeManagementIntentV2::SaveDisabled))
        .unwrap();
    assert_eq!(preview.plan().secrets().len(), 1);
    let transaction_runtime = TransactionRuntime::default();
    let external = NoExternal;
    let coordinator = TransactionCoordinator::new(
        stores.control(),
        stores.secrets(),
        stores.runtime(),
        &external,
        &input,
        &transaction_runtime,
    );
    coordinator.reconcile_startup_and_open().unwrap();
    apply_preview(
        &stores,
        &planner,
        &coordinator,
        &workspace,
        preview,
        "mvp-11-failed-connection-disabled-save",
    );

    let snapshot = hiroute_domain::ComputeManagementRepositoryPort::compute_management_snapshot(
        stores.control(),
        &workspace,
    )
    .unwrap();
    assert_eq!(snapshot.sources.len(), 1);
    let source = &snapshot.sources[0];
    assert_eq!(source.state, MaterializationState::Disabled);
    assert_eq!(source.models.len(), 1);
    assert_eq!(source.credentials.len(), 1);
    assert_eq!(
        source.models[0].capabilities.max_output_tokens.value,
        Some(4_096)
    );
    assert!(matches!(
        compile_compute_management_source(source),
        Err(ComputeManagementCompilationErrorV2::NotReady)
    ));
}
