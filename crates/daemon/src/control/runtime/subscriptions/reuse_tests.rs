//! Saved owner lookup survives normal connection lifecycle changes without new approval.
use super::*;
use hiroute_application_api::{ComputeCandidateViewV2, ComputeManagementEditV1};

fn reuse_candidate(fixture: &Fixture) -> ComputeCandidateViewV2 {
    let before = fixture.snapshot();
    let source = &before.sources[0];
    assert!(
        fixture
            .adapter()
            .retained_subscription_candidate_for_source(source)
            .unwrap()
            .is_none(),
        "the old complete-presentation lookup must reject this legitimately edited source"
    );
    let spawns = fixture.spawns();
    let mut result = fixture
        .adapter()
        .saved_subscription_candidate(hiroute_application_api::ComputeSavedSubscriptionSourceV1 {
            source_id: source.source_id.clone(),
            expected_source_revision: source.revision,
        })
        .unwrap();
    assert_eq!(result.candidates.len(), 1);
    let candidate = result.candidates.remove(0);
    assert_eq!(candidate.candidate.candidate_ref, source.last_candidate_ref);
    assert_eq!(
        candidate.existing_source_id.as_deref(),
        Some(source.source_id.as_str())
    );
    assert_eq!(
        fixture.snapshot(),
        before,
        "lookup must not save credentials or models"
    );
    assert_eq!(
        fixture.spawns(),
        spawns,
        "lookup must not start CPA or check a login"
    );
    candidate
}

fn saved_edit(fixture: &Fixture, edit: ComputeManagementEditV1, refs: Vec<String>, key: &str) {
    let snapshot = fixture.snapshot();
    fixture.save(fixture.save_request(
        ComputeManagementChangeV2 {
            schema: COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.into(),
            subject: ComputeManagementSubjectV2::SavedSource {
                source_id: snapshot.sources[0].source_id.clone(),
            },
            expected_revisions: snapshot.revisions,
            selected_model_refs: refs,
            intent: ComputeManagementIntentV2::SaveReady,
            key_edits: Vec::new(),
            validation: None,
            edit: Some(edit),
        },
        key,
    ));
}

#[test]
fn saved_owner_survives_rename_remove_and_consecutive_targeted_appends() {
    if crate::test_support::isolated_agent_home(concat!(
        "control::runtime::subscriptions::login_admission_tests::reuse_tests::",
        "saved_owner_survives_rename_remove_and_consecutive_targeted_appends"
    )) {
        return;
    }
    let fixture = Fixture::new();
    let login = fixture.authorize(SubscriptionLoginProviderV1::Codex);
    let checked = fixture.check(&login);
    fixture.save(fixture.candidate_request(&checked, "first-save"));
    assert_eq!(fixture.snapshot().sources[0].models.len(), 1);
    saved_edit(
        &fixture,
        ComputeManagementEditV1::Rename {
            display_name: "My subscription".into(),
        },
        Vec::new(),
        "rename",
    );
    reuse_candidate(&fixture);

    // Each addition uses a new genuine Check and targets only the new model;
    // earlier members stay in the saved source across both successful saves.
    for (index, model_id) in ["gpt-5.5", "gpt-5.4"].into_iter().enumerate() {
        let inventory = if index == 0 {
            format!("gpt-5.3-codex-spark,{model_id}")
        } else {
            format!("gpt-5.3-codex-spark,gpt-5.5,{model_id}")
        };
        fs::write(fixture.root.path().join("catalog-model"), inventory).unwrap();
        let pending = reuse_candidate(&fixture);
        let checked = fixture.check_candidate(pending.candidate, &format!("check-add-{index}"));
        let candidate = checked.checked_candidate.as_ref().unwrap();
        let member = candidate
            .models
            .iter()
            .find(|model| model.upstream_model_id == model_id)
            .unwrap();
        fixture.save(fixture.save_request(
            ComputeManagementChangeV2 {
                schema: COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.into(),
                subject: ComputeManagementSubjectV2::Candidate {
                    candidate: candidate.candidate.clone(),
                },
                expected_revisions: fixture.snapshot().revisions,
                selected_model_refs: vec![member.model_ref.clone()],
                intent: ComputeManagementIntentV2::SaveReady,
                key_edits: Vec::new(),
                validation: checked.validation.clone(),
                edit: Some(ComputeManagementEditV1::AppendModels),
            },
            &format!("save-add-{index}"),
        ));
        let source = &fixture.snapshot().sources[0];
        assert_eq!(source.models.len(), index + 2);
        assert_eq!(source.display_name, "My subscription");
        reuse_candidate(&fixture);
    }
    let removed_ref = fixture.snapshot().sources[0].models[0].model_ref.clone();
    saved_edit(
        &fixture,
        ComputeManagementEditV1::RemoveModels,
        vec![removed_ref.clone()],
        "remove",
    );
    let source = &fixture.snapshot().sources[0];
    assert_eq!(source.models.len(), 2);
    assert!(
        source
            .models
            .iter()
            .all(|model| model.model_ref != removed_ref)
    );
    reuse_candidate(&fixture);
}
