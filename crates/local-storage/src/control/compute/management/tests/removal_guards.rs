//! Exercise the real Control effect boundary, including a reference arriving after preview.
use super::lifecycle::{edit, saved};
use super::*;
use hiroute_application_api::ComputeManagementEditV1;
use hiroute_domain::{
    ComputeManagementRepositoryPort, OperationId, PlanDraftV1, TransactionPlanV1,
};

fn referencing_draft(binding: &str) -> PlanDraftV1 {
    serde_json::from_value(serde_json::json!({
        "schema":"hiroute.plan-draft/v1", "workspace_id":WorkspaceId::DEFAULT,
        "draft_id":"draft/model-removal", "revision":1,
        "editor":{"schema":"hiroute.plan-editor/v2", "display_name":"Using a model", "purpose":"",
          "mode":"fixed_model", "candidates":[{"binding_id":binding}],
          "smart":{"economy":[],"primary":[],"judgment":hiroute_domain::JudgmentSettingsV1::default(),"reselect_on_user_message":false,"classifier":{"kind":"local_rules"},"complex_keywords":[]},
          "free":{"candidates":[],"primary":[],"primary_fallback":false},"delegation_enabled":false,
          "requirements":{},"limits":{"maximum_attempts":6,"request_timeout_ms":60000,"attempt_timeout_ms":30000}}
    })).unwrap()
}

#[test]
fn reference_added_after_preview_blocks_stage_and_added_after_stage_blocks_activation() {
    let root = tempdir().unwrap();
    populate_saved_source(root.path(), &WorkspaceId::default());
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let source = saved(&stores);
    let registry = TrustedComputeCandidateRegistry::new();
    let input = ProtectedInput;
    let planner =
        ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
    let preview = planner
        .preview(edit(&stores, &source, ComputeManagementEditV1::Delete, &[]))
        .unwrap();
    let workspace = WorkspaceId::default();
    let operation = OperationId::parse("op_55555555555555555555555555555555").unwrap();
    seed_operation(&stores, &operation, preview.plan());
    let draft = referencing_draft(&source.models[0].binding_id);
    stores.control().save_plan_draft(&draft, None).unwrap();
    let referenced = planner
        .preview(edit(&stores, &source, ComputeManagementEditV1::Delete, &[]))
        .unwrap();
    assert_eq!(
        referenced.result.affected_plan_refs,
        vec![draft.draft_id.clone()]
    );
    assert!(
        planner
            .prepare_apply(ComputeConnectionApplyRequestV1 {
                spec: preview.result.spec.clone(),
                accept_digest: preview.result.accept_digest.clone(),
                expected_revisions: preview.result.expected_revisions.clone(),
                idempotency_key: "delete-stale".into()
            })
            .is_err()
    );
    assert!(
        stores
            .control()
            .apply_control(
                &operation,
                &workspace,
                preview.result.expected_revisions.target,
                preview.plan().control()
            )
            .is_err()
    );
    stores
        .control()
        .discard_plan_draft(&workspace, &draft.draft_id, 1)
        .unwrap();
    let staged = stores
        .control()
        .apply_control(
            &operation,
            &workspace,
            preview.result.expected_revisions.target,
            preview.plan().control(),
        )
        .unwrap();
    stores.control().save_plan_draft(&draft, None).unwrap();
    assert!(stores.control().activate_control(&staged).is_err());
    assert_eq!(saved(&stores), source);
}

#[test]
fn deleted_source_compensation_restores_exact_bytes_but_cannot_win_after_later_create_delete() {
    let root = tempdir().unwrap();
    populate_saved_source(root.path(), &WorkspaceId::default());
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let source = saved(&stores);
    let registry = TrustedComputeCandidateRegistry::new();
    let input = ProtectedInput;
    let planner =
        ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
    let workspace = WorkspaceId::default();
    let stage = |plan: &TransactionPlanV1, id: &str| {
        let revision = stores
            .control()
            .current_revisions(&workspace)
            .unwrap()
            .target;
        let operation = OperationId::parse(id).unwrap();
        seed_operation(&stores, &operation, plan);
        let effect = stores
            .control()
            .apply_control(&operation, &workspace, revision, plan.control())
            .unwrap();
        stores.control().activate_control(&effect).unwrap();
        effect
    };
    let deletion = planner
        .preview(edit(&stores, &source, ComputeManagementEditV1::Delete, &[]))
        .unwrap();
    let effect = stage(deletion.plan(), "op_11111111111111111111111111111111");
    assert!(
        stores
            .control()
            .compute_management_source(&source.source_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        stores.control().compensate_control(&effect).unwrap(),
        CompensationOutcome::Compensated
    );
    assert_eq!(saved(&stores), source);
    let deletion = planner
        .preview(edit(&stores, &source, ComputeManagementEditV1::Delete, &[]))
        .unwrap();
    let old_effect = stage(deletion.plan(), "op_22222222222222222222222222222222");
    // Same lineage can be imported again. Its later delete must never give the old delete
    // ownership merely because both operations leave the source row absent.
    let mut recreated = source.clone();
    recreated.revision = 1;
    recreated.credentials.clear();
    recreated.state = MaterializationState::NeedsCredential;
    let spec = hiroute_domain::ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: "compute.connection.apply".into(),
        resource_id: Some(source.source_id.clone()),
        desired_state: serde_json::to_value(ComputeManagementChangeV2 {
            schema: "hiroute.compute-management-change/v2".into(),
            subject: ComputeManagementSubjectV2::Candidate {
                candidate: candidate("candidate/new-owner", "slot/primary").candidate,
            },
            expected_revisions: stores.control().current_revisions(&workspace).unwrap(),
            selected_model_refs: recreated.models.iter().map(|model| model.model_ref.clone()).collect(),
            intent: ComputeManagementIntentV2::SaveReady,
            key_edits: Vec::new(), validation: None, edit: None,
        }).unwrap(),
    };
    let creation = TransactionPlanV1::from_compute_management_change(
        spec,
        None,
        Some(recreated.clone()),
        Vec::new(),
    )
    .unwrap();
    stage(&creation, "op_33333333333333333333333333333333");
    let deletion = planner
        .preview(edit(
            &stores,
            &recreated,
            ComputeManagementEditV1::Delete,
            &[],
        ))
        .unwrap();
    stage(deletion.plan(), "op_44444444444444444444444444444444");
    assert_eq!(
        stores.control().compensate_control(&old_effect).unwrap(),
        CompensationOutcome::OwnershipLost
    );
    assert!(
        stores
            .control()
            .compute_management_source(&source.source_id)
            .unwrap()
            .is_none()
    );
}

fn seed_operation(stores: &LocalStorageSet, id: &OperationId, plan: &TransactionPlanV1) {
    // This is a storage effect interleaving test, not coordinator admission evidence.
    // Keep a well-formed parent journal so the management effect's FK is exercised.
    let digest = CanonicalDigest::of_bytes(id.as_str().as_bytes());
    let operation = OperationV1::new(
        id.clone(),
        WorkspaceId::default(),
        hiroute_domain::IdempotencyScopeV1::new("local-control", "ApplyComputeSave", id.as_str())
            .unwrap(),
        digest.clone(),
        digest.clone(),
        stores
            .control()
            .current_revisions(&WorkspaceId::default())
            .unwrap(),
        plan.clone(),
    )
    .unwrap();
    stores.control().with_connection(|connection| connection.execute(
        "INSERT INTO operations(operation_id,workspace_id,principal,operation_kind,idempotency_key,request_digest,accepted_change_digest,state,generation,operation_json,created_at,updated_at) VALUES(?1,?2,'local-control','ApplyComputeSave',?1,?3,?3,'accepted',1,?4,1,1)",
        rusqlite::params![id.as_str(),WorkspaceId::DEFAULT,digest.as_str(),serde_json::to_string(&operation).unwrap()]).unwrap());
}
