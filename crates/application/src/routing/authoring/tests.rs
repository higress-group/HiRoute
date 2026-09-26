use super::*;
use crate::compiler::test_fixtures::compilation_facts;

fn change() -> PlanContentChangeV2 {
    PlanContentChangeV2 {
        schema: PLAN_CONTENT_CHANGE_SCHEMA_V2.into(), target: PlanContentTargetV2::Create { creation_key: "editor-one".into() },
        editor: serde_json::from_value(serde_json::json!({
            "schema": PLAN_EDITOR_SCHEMA_V2, "display_name": "日常编码", "purpose": "辅助日常编码",
            "mode":"fixed_model", "candidates":[{"binding_id":"binding/free-b"}],
            "smart":{"economy":[],"primary":[],"primary_fallback":false,"reselect_on_user_message":false,"classifier":{"kind":"local_rules"},"complex_keywords":[]},
            "free":{"candidates":[],"primary":[],"primary_fallback":false},
            "delegation_enabled": false,
            "requirements":{},"limits":{"maximum_attempts":6,"request_timeout_ms":60000,"attempt_timeout_ms":30000}
        })).unwrap(), consumed_draft: None,
    }
}
fn state() -> PlanAuthoringSnapshotV2 {
    PlanAuthoringSnapshotV2 {
        workspace: WorkspaceId::default(),
        aliases: AliasRegistryV1::default(),
        active_publication: None,
        plan_heads: vec![],
        current_head: None,
        legacy_source: None,
        draft: None,
        facts: compilation_facts(),
        expected_revisions: RevisionSetV1 {
            target: 1,
            dependencies: Default::default(),
        },
    }
}
#[test]
fn preview_is_read_only_and_seals_proposed_alias_and_registry() {
    let change = change();
    let mut state = state();
    let before = state.aliases.clone();
    let first = preview_plan_content(&change, &state).unwrap();
    assert_eq!(
        first.plan_head.model_alias.as_str(),
        "hiroute-richangbianma"
    );
    assert_eq!(before, state.aliases);
    assert_eq!(first, preview_plan_content(&change, &state).unwrap());
    state
        .aliases
        .allocate_custom(
            AgentPlanId::parse("plan/competitor").unwrap(),
            first.plan_head.model_alias.clone(),
        )
        .unwrap();
    let second = preview_plan_content(&change, &state).unwrap();
    assert_eq!(
        second.plan_head.model_alias.as_str(),
        "hiroute-richangbianma-2"
    );
    // Ordinary Publish confirms the exact edit and database revisions. Allocating another
    // alias through a real writer advances the target revision; model the same boundary here.
    assert_eq!(first.change_digest, second.change_digest);
    assert_ne!(first.alias_registry_digest, second.alias_registry_digest);
    state.expected_revisions.target += 1;
    let after_commit = preview_plan_content(&change, &state).unwrap();
    assert_ne!(first.change_digest, after_commit.change_digest);
    assert_eq!(second.plan_head, after_commit.plan_head);
}
#[test]
fn manual_alias_survives_name_changes_and_published_alias_cannot_change() {
    let mut change = change();
    let mut state = state();
    change.editor.custom_alias = Some("my-coding".into());
    let first = preview_plan_content(&change, &state).unwrap();
    change.editor.display_name = "资料整理".into();
    assert_eq!(
        preview_plan_content(&change, &state)
            .unwrap()
            .plan_head
            .model_alias
            .as_str(),
        "my-coding"
    );
    state
        .aliases
        .allocate_custom(
            first.plan_head.reference.plan_id.clone(),
            first.plan_head.model_alias.clone(),
        )
        .unwrap();
    state.current_head = Some(first.plan_head.clone());
    change.target = PlanContentTargetV2::Update {
        plan_id: first.plan_head.reference.plan_id,
        expected_head_revision: 1,
    };
    let updated = preview_plan_content(&change, &state).unwrap();
    assert_eq!(updated.plan_head.model_alias.as_str(), "my-coding");
    assert_eq!(updated.plan_head.reference.content_revision, 2);
    change.editor.custom_alias = Some("different".into());
    assert_eq!(
        preview_plan_content(&change, &state),
        Err(PlanPreviewError::AliasImmutable)
    );
}

#[test]
fn current_unversioned_publication_source_can_be_adopted_on_first_update() {
    let mut change = change();
    let mut state = state();
    let existing = preview_plan_content(&change, &state).unwrap();
    state
        .aliases
        .allocate_custom(
            existing.plan_head.reference.plan_id.clone(),
            existing.plan_head.model_alias.clone(),
        )
        .unwrap();
    state.current_head = Some(existing.plan_head.clone());
    state.plan_heads = vec![existing.plan_head.clone()];
    state.legacy_source = Some(existing.plan_version.clone());
    change.target = PlanContentTargetV2::Update {
        plan_id: existing.plan_head.reference.plan_id.clone(),
        expected_head_revision: existing.plan_head.head_revision,
    };

    let updated = preview_plan_content(&change, &state).unwrap();
    assert_eq!(updated.legacy_source, Some(existing.plan_version));
    assert_eq!(updated.plan_head.reference.content_revision, 2);
    assert_eq!(updated.plan_head.head_revision, 2);
}

#[test]
fn consumed_draft_binds_identity_and_revision_but_publishes_the_submitted_editor() {
    let mut change = change();
    let mut state = state();
    change.consumed_draft = Some(PlanDraftRefV1 {
        draft_id: "draft/one".into(),
        revision: 1,
    });
    assert_eq!(
        preview_plan_content(&change, &state),
        Err(PlanPreviewError::Stale)
    );
    state.draft = Some(PlanDraftV1 {
        schema: PLAN_DRAFT_SCHEMA_V1.into(),
        workspace_id: state.workspace.clone(),
        draft_id: "draft/one".into(),
        revision: 1,
        plan_id: None,
        base_head_revision: None,
        editor: change.editor.clone(),
    });
    let before = preview_plan_content(&change, &state).unwrap();
    change.editor.purpose = "Edited after saving the draft".into();
    let edited = preview_plan_content(&change, &state).unwrap();
    assert_eq!(
        edited.plan_version.configuration.purpose.as_str(),
        change.editor.purpose
    );
    assert_ne!(before.change_digest, edited.change_digest);
    assert_eq!(edited.consumed_draft, change.consumed_draft);
    let original = state.draft.as_ref().unwrap().clone();
    for mismatch in ["identity", "plan"] {
        let draft = state.draft.as_mut().unwrap();
        *draft = original.clone();
        match mismatch {
            "identity" => draft.draft_id = "draft/other".into(),
            "plan" => {
                draft.plan_id = Some(AgentPlanId::parse("plan/other").unwrap());
                draft.base_head_revision = Some(1);
            }
            _ => unreachable!(),
        }
        assert_eq!(
            preview_plan_content(&change, &state),
            Err(PlanPreviewError::Stale)
        );
    }
    state.draft = Some(original);
    state.draft.as_mut().unwrap().revision = 2;
    assert_eq!(
        preview_plan_content(&change, &state),
        Err(PlanPreviewError::Stale)
    );

    state
        .aliases
        .allocate_custom(
            before.plan_head.reference.plan_id.clone(),
            before.plan_head.model_alias.clone(),
        )
        .unwrap();
    state.current_head = Some(before.plan_head.clone());
    change.target = PlanContentTargetV2::Update {
        plan_id: before.plan_head.reference.plan_id.clone(),
        expected_head_revision: before.plan_head.head_revision,
    };
    let draft = state.draft.as_mut().unwrap();
    draft.revision = 1;
    draft.plan_id = Some(before.plan_head.reference.plan_id.clone());
    draft.base_head_revision = Some(before.plan_head.head_revision);
    preview_plan_content(&change, &state).unwrap();
    state.draft.as_mut().unwrap().base_head_revision = Some(before.plan_head.head_revision + 1);
    assert_eq!(
        preview_plan_content(&change, &state),
        Err(PlanPreviewError::Stale)
    );
}

#[test]
fn complete_content_operation_binds_the_version_head_and_publication() {
    let change = change();
    let state = state();
    let preview = preview_plan_content(&change, &state).unwrap();
    let mut aliases = state.aliases.clone();
    aliases
        .allocate_custom(
            preview.plan_head.reference.plan_id.clone(),
            preview.plan_head.model_alias.clone(),
        )
        .unwrap();
    let revision = GatewayPublicationRevision::new(1).unwrap();
    let publication = GatewayPublicationV1::new(
        state.workspace.clone(),
        revision,
        AliasRegistryV1::default(),
        vec![],
    )
    .unwrap()
    .next_with_plan_content(
        revision,
        aliases,
        preview.plan_version.compiled.clone(),
        vec![preview.plan_head.clone()],
    )
    .unwrap();
    let record =
        PublicationRecordV1::from_publication(state.workspace.clone(), &publication).unwrap();
    let spec = ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: "routing.apply".into(),
        resource_id: Some(format!(
            "agent-plan/{}",
            preview.plan_head.reference.plan_id.as_str()
        )),
        desired_state: serde_json::to_value(&change).unwrap(),
    };
    let plan = TransactionPlanV1::from_plan_content_planner(
        spec,
        preview.plan_version.clone(),
        preview.plan_head.clone(),
        None,
        None,
        record.clone(),
        None,
    )
    .unwrap();
    let content = plan.plan_content_control().unwrap().unwrap();
    assert_eq!(content.plan_version, preview.plan_version);
    assert_eq!(
        routing_publication_record(&plan.external()[0]).unwrap(),
        Some(record)
    );
    let roundtrip = TransactionPlanV1::from_registered_typed_planner(
        plan.spec().clone(),
        plan.control().clone(),
        None,
        vec![],
        vec![],
        plan.external().to_vec(),
    )
    .unwrap();
    assert_eq!(roundtrip.plan_content_control().unwrap().unwrap(), content);
    let mut changed = plan.control().clone();
    changed["plan_head"]["head_revision"] = serde_json::json!(99);
    assert!(
        TransactionPlanV1::from_registered_typed_planner(
            plan.spec().clone(),
            changed,
            None,
            vec![],
            vec![],
            plan.external().to_vec(),
        )
        .is_err()
    );
}

#[test]
fn ordinary_publication_confirmation_binds_edit_target_versions_and_draft_intent() {
    let original = change();
    let revisions = state().expected_revisions;
    let digest = plan_content_confirmation_digest(&original, &revisions).unwrap();
    let mut edited = original.clone();
    edited.editor.purpose = "another purpose".into();
    assert_ne!(
        digest,
        plan_content_confirmation_digest(&edited, &revisions).unwrap()
    );
    edited = original.clone();
    edited.target = PlanContentTargetV2::Create {
        creation_key: "another-plan".into(),
    };
    assert_ne!(
        digest,
        plan_content_confirmation_digest(&edited, &revisions).unwrap()
    );
    let mut new_revisions = revisions.clone();
    new_revisions.target += 1;
    assert_ne!(
        digest,
        plan_content_confirmation_digest(&original, &new_revisions).unwrap()
    );
    // Every wire field is part of the confirmation, including optional draft ownership.
    let mut json = serde_json::to_value(&original).unwrap();
    json["consumed_draft"] = serde_json::json!({"draft_id":"draft/confirmation", "revision":3});
    let with_draft: PlanContentChangeV2 = serde_json::from_value(json).unwrap();
    assert_ne!(
        digest,
        plan_content_confirmation_digest(&with_draft, &revisions).unwrap()
    );
}
