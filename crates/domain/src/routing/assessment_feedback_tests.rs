use super::*;
use crate::CanonicalDigest;

fn prior_default() -> (JudgmentSettingsV1, FrozenNativeProtocolV1) {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/baseline-c-v2-default-publication.json"
    ))
    .unwrap();
    (
        serde_json::from_value(fixture["judgment"].clone()).unwrap(),
        serde_json::from_value(fixture["frozen"].clone()).unwrap(),
    )
}

#[test]
fn prior_c_v2_default_reopens_exactly_and_rejects_authored_drift() {
    let (old, frozen) = prior_default();
    let bytes = serde_json::to_vec(&frozen).unwrap();
    let reopened: FrozenNativeProtocolV1 = serde_json::from_slice(&bytes).unwrap();
    reopened.validate_smart(&old).unwrap();
    assert_eq!(serde_json::to_vec(&reopened).unwrap(), bytes);
    for field in ["instructions", "criterion", "floor", "degree", "standard"] {
        let mut changed = old.clone();
        match field {
            "instructions" => changed.competence.instructions.push(' '),
            "criterion" => changed.competence.criteria[2].push(' '),
            "floor" => changed.competence.floor_millis += 1,
            "degree" => changed.degree.simple_threshold_millis += 1,
            "standard" => changed.competence.standard = Some(AssessmentStandardV1::Custom),
            _ => unreachable!(),
        }
        assert!(reopened.validate_smart(&changed).is_err(), "{field}");
    }
    let mut corrupted: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    corrupted["scopes"][SMART_SAVING_SCOPE_ID]["assessment"]["instructions"] =
        serde_json::json!("changed without authenticating the digest");
    let corrupted: FrozenNativeProtocolV1 = serde_json::from_value(corrupted).unwrap();
    assert_eq!(
        corrupted.validate(),
        Err(NativeProtocolError::DigestMismatch)
    );
}

#[test]
fn prior_c_v2_branch_defaults_validate_against_their_original_scope_text() {
    let (old, mut frozen) = prior_default();
    let branches: Vec<_> = ["code", "general"]
        .into_iter()
        .enumerate()
        .map(|(i, id)| MaterializedBranchV1 {
            id: id.into(),
            name: id.into(),
            condition: format!("Handle {id}"),
            group: MaterializedGroupId::Branch(i as u16),
            primary_group: None,
            judgment: Some(old.clone()),
        })
        .collect();
    let scope = frozen.scopes.remove(SMART_SAVING_SCOPE_ID).unwrap();
    frozen.decision = c_v1_branch_decision_definition(&branches, "general");
    frozen.scopes = branches
        .iter()
        .map(|b| (b.id.clone(), scope.clone()))
        .collect();
    frozen.digest = CanonicalDigest::of(&(
        &frozen.protocol,
        &frozen.context_layout,
        &frozen.decision,
        &frozen.scopes,
    ))
    .unwrap();
    let before = serde_json::to_vec(&frozen).unwrap();
    frozen.validate_branches(&branches, "general").unwrap();
    assert_eq!(serde_json::to_vec(&frozen).unwrap(), before);
    for field in ["criterion", "floor", "condition"] {
        let mut changed = branches.clone();
        match field {
            "criterion" => changed[0].judgment.as_mut().unwrap().competence.criteria[2].push(' '),
            "floor" => {
                changed[0]
                    .judgment
                    .as_mut()
                    .unwrap()
                    .competence
                    .floor_millis += 1
            }
            "condition" => changed[0].condition.push(' '),
            _ => unreachable!(),
        }
        assert_eq!(
            frozen.validate_branches(&changed, "general"),
            Err(NativeProtocolError::ContentMismatch),
            "{field}"
        );
    }
}

#[test]
fn current_default_seals_positive_feedback_and_changes_only_its_assessment_identity() {
    let (old, frozen) = prior_default();
    let selection = NativeProtocolSelectionV1::default();
    let current = old.resolve_assessment(Some(&selection)).unwrap();
    assert!(
        current
            .competence
            .instructions
            .contains("Generic courtesy, tentative or partial approval")
    );
    assert!(current.competence.criteria[2].contains("Clear user acceptance"));
    assert!(current.competence.criteria[2].contains("specific unresolved contradiction"));
    assert_eq!(
        current.competence.criteria[..2],
        old.competence.criteria[..2]
    );
    assert_eq!(current.degree, old.degree);
    assert_eq!(current.competence.floor_millis, old.competence.floor_millis);
    let next = FrozenNativeProtocolV1::freeze(
        &frozen.protocol,
        frozen.decision.clone(),
        [(SMART_SAVING_SCOPE_ID.into(), old.clone())],
    )
    .unwrap();
    next.validate_smart(&current).unwrap();
    assert_eq!(next.protocol, frozen.protocol);
    assert_ne!(next.digest, frozen.digest);
    assert_ne!(
        next.execution_policy(SMART_SAVING_SCOPE_ID, &current, "same"),
        frozen.execution_policy(SMART_SAVING_SCOPE_ID, &old, "same")
    );
    assert!(next.validate_smart(&old).is_err());
    let mut custom = old.clone();
    custom.competence.standard = Some(AssessmentStandardV1::Custom);
    assert_eq!(custom.resolve_assessment(Some(&selection)).unwrap(), custom);
}

#[test]
fn frozen_default_resolution_preserves_old_text_but_not_changed_thresholds_or_custom_input() {
    let (old, _) = prior_default();
    let selection = NativeProtocolSelectionV1::default();
    let input = JudgmentSettingsV1::default();
    let resolved = input
        .resolve_assessment_for_frozen(Some(&selection), Some(&old))
        .unwrap();
    assert_eq!(resolved, old);
    let mut changed = input.clone();
    changed.competence.floor_millis = 723;
    changed.degree.simple_threshold_millis = 619;
    let changed = changed
        .resolve_assessment_for_frozen(Some(&selection), Some(&old))
        .unwrap();
    assert_eq!(changed.competence.instructions, old.competence.instructions);
    assert_eq!(changed.competence.floor_millis, 723);
    assert_eq!(changed.degree.simple_threshold_millis, 619);
    let mut custom = input;
    custom.competence.standard = Some(AssessmentStandardV1::Custom);
    assert_eq!(
        custom
            .resolve_assessment_for_frozen(Some(&selection), Some(&old))
            .unwrap(),
        custom
    );
}
