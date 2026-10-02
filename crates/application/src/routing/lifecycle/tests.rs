use super::*;

fn state() -> PlanLifecycleSnapshotV1 {
    let legacy = serde_json::from_slice::<hiroute_domain::GatewayPublicationV1>(include_bytes!(
        "../../../../../e2e/product/fixtures/routing/current-publication.v3.json"
    ))
    .unwrap();
    let versions = legacy
        .plans
        .iter()
        .map(|compiled| {
            let recovered = PlanVersionV1::from_unversioned_compiled_recovery(
                legacy.workspace_id.clone(),
                compiled.clone(),
            )
            .unwrap();
            PlanVersionV1::new(
                legacy.workspace_id.clone(),
                recovered.configuration,
                recovered.compiled.into_current().unwrap(),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let heads = versions
        .iter()
        .map(|v| PlanHeadV1 {
            reference: v.reference.clone(),
            model_alias: v.compiled.model_alias().clone(),
            head_revision: v.reference.content_revision,
            status: PlanLifecycleV1::Enabled,
        })
        .collect::<Vec<_>>();
    let mut publication = legacy.into_current().unwrap();
    publication.grants.clear();
    publication.aliases.clear();
    publication.plan_heads = heads.clone();
    publication.validate_current_contract().unwrap();
    PlanLifecycleSnapshotV1 {
        head: heads[0].clone(),
        version: versions[0].clone(),
        publication,
        expected_revisions: RevisionSetV1 {
            target: 1,
            dependencies: Default::default(),
        },
        references_digest: CanonicalDigest::of_bytes(b"verified-empty-references"),
        has_agent_references: false,
        has_default_model_reference: false,
        has_version_holds: false,
    }
}
fn change(state: &PlanLifecycleSnapshotV1, status: PlanLifecycleV1) -> PlanLifecycleChangeV1 {
    PlanLifecycleChangeV1 {
        schema: PLAN_LIFECYCLE_CHANGE_SCHEMA_V1.into(),
        plan_id: state.head.reference.plan_id.clone(),
        expected_head_revision: state.head.head_revision,
        status,
    }
}
#[test]
fn disabling_referenced_plan_preserves_version_and_reenable_preserves_alias() {
    let mut state = state();
    state.has_agent_references = true;
    state.has_version_holds = true;
    let saved_version = state.version.clone();
    let off = preview_plan_lifecycle(&change(&state, PlanLifecycleV1::Disabled), &state).unwrap();
    assert_eq!(off.plan_head.reference, state.head.reference);
    assert!(off.has_agent_references && off.has_version_holds);
    state.publication = lifecycle_publication(&state, off.plan_head.clone()).unwrap();
    state.head = off.plan_head;
    let on = preview_plan_lifecycle(&change(&state, PlanLifecycleV1::Enabled), &state).unwrap();
    assert_eq!(on.plan_head.reference, saved_version.reference);
    assert_eq!(
        on.plan_head.model_alias,
        *saved_version.compiled.model_alias()
    );
    assert_eq!(state.version, saved_version);
}
#[test]
fn delete_requires_all_references_and_holds_removed_and_seals_tombstone() {
    let mut state = state();
    let change = change(&state, PlanLifecycleV1::Deleted);
    state.has_agent_references = true;
    assert_eq!(
        preview_plan_lifecycle(&change, &state),
        Err(PlanPreviewError::Referenced)
    );
    state.has_agent_references = false;
    state.has_version_holds = true;
    assert_eq!(
        preview_plan_lifecycle(&change, &state),
        Err(PlanPreviewError::Referenced)
    );
    state.has_version_holds = false;
    let preview = preview_plan_lifecycle(&change, &state).unwrap();
    let publication = lifecycle_publication(&state, preview.plan_head.clone()).unwrap();
    assert!(
        publication
            .alias_registry
            .tombstones
            .contains(&state.head.model_alias)
    );
    let spec = ChangeSpecV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        command_id: "routing.apply".into(),
        resource_id: Some(format!("agent-plan/{}", change.plan_id.as_str())),
        desired_state: serde_json::to_value(&change).unwrap(),
    };
    let plan = TransactionPlanV1::from_plan_content_planner(
        spec.clone(),
        state.version.clone(),
        preview.plan_head.clone(),
        Some(state.head.clone()),
        None,
        PublicationRecordV1::from_publication(
            state.head.reference.workspace_id.clone(),
            &publication,
        )
        .unwrap(),
        Some(state.publication.digest().unwrap()),
    )
    .unwrap();
    assert_eq!(
        plan.plan_content_control().unwrap().unwrap().plan_head,
        preview.plan_head
    );
    let mut forged = spec;
    forged.desired_state["status"] = serde_json::json!("enabled");
    assert!(
        TransactionPlanV1::from_plan_content_planner(
            forged,
            state.version.clone(),
            preview.plan_head,
            Some(state.head.clone()),
            None,
            PublicationRecordV1::from_publication(
                state.head.reference.workspace_id.clone(),
                &publication
            )
            .unwrap(),
            Some(state.publication.digest().unwrap())
        )
        .is_err()
    );
}
#[test]
fn lifecycle_preview_binds_reference_generation_and_rejects_stale_head() {
    let mut state = state();
    let mut change = change(&state, PlanLifecycleV1::Disabled);
    let before = preview_plan_lifecycle(&change, &state).unwrap();
    state.references_digest = CanonicalDigest::of_bytes(b"another-reference-generation");
    assert_ne!(
        before.change_digest,
        preview_plan_lifecycle(&change, &state)
            .unwrap()
            .change_digest
    );
    change.expected_head_revision += 1;
    assert_eq!(
        preview_plan_lifecycle(&change, &state),
        Err(PlanPreviewError::Stale)
    );
}

#[test]
fn default_model_reference_blocks_disable_but_allowed_list_reference_does_not() {
    let mut state = state();
    state.has_agent_references = true;
    state.has_default_model_reference = true;
    let disable = change(&state, PlanLifecycleV1::Disabled);
    assert_eq!(
        preview_plan_lifecycle(&disable, &state),
        Err(PlanPreviewError::Referenced)
    );
    state.has_default_model_reference = false;
    assert!(preview_plan_lifecycle(&disable, &state).is_ok());
}
