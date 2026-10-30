use super::*;

#[test]
fn baseline_c_default_publication_remains_valid_without_replacing_frozen_questions() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/baseline-c-default-publication.json")).unwrap();
    let judgment: JudgmentSettingsV1 = serde_json::from_value(fixture["judgment"].clone()).unwrap();
    let frozen: FrozenNativeProtocolV1 = serde_json::from_value(fixture["frozen"].clone()).unwrap();
    frozen.validate_smart(&judgment).unwrap();
    assert_eq!(serde_json::to_value(&frozen).unwrap(), fixture["frozen"]);
    assert_eq!(
        frozen.scopes[super::super::SMART_SAVING_SCOPE_ID].assessment,
        stock_c_questions()["q1"]
    );
    let mut changed = judgment.clone();
    changed.competence.instructions.push_str(" changed");
    assert_eq!(
        frozen.validate_smart(&changed),
        Err(NativeProtocolError::ContentMismatch)
    );
}

#[test]
fn stock_questions_and_composition_are_the_public_frozen_c_full() {
    let p = NativeProtocolV1::default();
    let mut j = JudgmentSettingsV1::default();
    j.competence.floor_millis = 317;
    j.degree.simple_threshold_millis = 819;
    p.stock_assessment(&mut j);
    j.competence.standard = Some(super::super::AssessmentStandardV1::Default);
    assert_eq!(j.competence.floor_millis, 317);
    assert_eq!(j.degree.simple_threshold_millis, 819);
    assert_eq!(composed_assessment(Some(&p), &j), stock_c_questions()["q1"]);
    assert_eq!(
        CanonicalDigest::of(&stock_c_questions()).unwrap().as_str(),
        format!("sha256:{C_FULL_CHECKSUM}")
    );
}

#[test]
fn supported_c_versions_bind_exact_bundles_and_separate_policy_identity() {
    let old_questions = stock_c_questions();
    let new_questions = stock_c_v2_questions();
    assert_eq!(old_questions["q0"], new_questions["q0"]);
    assert_eq!(old_questions["q1"], new_questions["q1"]);
    assert_ne!(old_questions["q2"], new_questions["q2"]);
    assert_eq!(
        CanonicalDigest::of(&old_questions).unwrap().as_str(),
        format!("sha256:{C_FULL_CHECKSUM}")
    );
    assert_eq!(
        CanonicalDigest::of(&new_questions).unwrap().as_str(),
        format!("sha256:{C_V2_FULL_CHECKSUM}")
    );
    let old = NativeProtocolV1::c_v1();
    let new = NativeProtocolV1::default();
    assert_eq!(new.schema, SYSTEM_ONE_C_V2);
    for protocol in [&old, &new] {
        protocol.validate().unwrap();
        let selection = super::super::NativeProtocolSelectionV1 {
            schema: protocol.schema.clone(),
            prior_context_max_turns: protocol.prior_context_max_turns,
        };
        assert_eq!(selection.resolve().unwrap(), *protocol);
    }
    for (schema, fixed) in [
        (SYSTEM_ONE_C_V1, new.fixed.clone()),
        (SYSTEM_ONE_C_V2, old.fixed.clone()),
    ] {
        let mismatched = NativeProtocolV1 {
            schema: schema.into(),
            fixed,
            prior_context_max_turns: 8,
        };
        assert_eq!(
            mismatched.validate(),
            Err(NativeProtocolError::FixedContent)
        );
    }
    let mut unknown = new.clone();
    unknown.schema = "hiroute.system-one-c/v99".into();
    assert_eq!(unknown.validate(), Err(NativeProtocolError::Unsupported));
    let selection = super::super::NativeProtocolSelectionV1 {
        prior_context_max_turns: 7,
        ..Default::default()
    };
    assert_eq!(selection.resolve(), Err(NativeProtocolError::HistoryCap));

    let judgment = JudgmentSettingsV1::default();
    let freeze = |protocol: &NativeProtocolV1| {
        FrozenNativeProtocolV1::freeze(
            protocol,
            judgment.degree.definition().into(),
            [(super::super::SMART_SAVING_SCOPE_ID.into(), judgment.clone())],
        )
        .unwrap()
    };
    let old_frozen = freeze(&old);
    let new_frozen = freeze(&new);
    assert_ne!(old_frozen.digest, new_frozen.digest);
    assert_ne!(
        old_frozen.execution_policy(super::super::SMART_SAVING_SCOPE_ID, &judgment, "same"),
        new_frozen.execution_policy(super::super::SMART_SAVING_SCOPE_ID, &judgment, "same")
    );
    let mut stock = judgment.clone();
    old.stock_assessment(&mut stock);
    stock.competence.standard = Some(super::super::AssessmentStandardV1::Default);
    let direct_new = FrozenNativeProtocolV1::freeze(
        &new,
        stock.degree.definition().into(),
        [(super::super::SMART_SAVING_SCOPE_ID.into(), stock)],
    )
    .unwrap();
    assert_ne!(
        direct_new.scopes[super::super::SMART_SAVING_SCOPE_ID].assessment,
        old_questions["q1"]
    );
    let mut tampered = new_frozen;
    tampered.protocol.fixed.applicability = old.fixed.applicability;
    assert_eq!(tampered.validate(), Err(NativeProtocolError::FixedContent));
}

#[test]
fn frozen_content_and_effective_authoring_must_both_agree() {
    let p = NativeProtocolV1::default();
    let mut j = JudgmentSettingsV1::default();
    p.current_assessment(&mut j);
    j.competence.standard = Some(super::super::AssessmentStandardV1::Default);
    let frozen = FrozenNativeProtocolV1::freeze(
        &p,
        j.degree.definition().into(),
        [(super::super::SMART_SAVING_SCOPE_ID.into(), j.clone())],
    )
    .unwrap();
    frozen.validate_smart(&j).unwrap();
    assert_eq!(
        serde_json::from_value::<FrozenNativeProtocolV1>(serde_json::to_value(&frozen).unwrap())
            .unwrap(),
        frozen
    );
    let mut corrupt = frozen.clone();
    corrupt.protocol.prior_context_max_turns = 2;
    assert_eq!(corrupt.validate(), Err(NativeProtocolError::HistoryCap));
    let mut corrupt = frozen.clone();
    corrupt.digest = CanonicalDigest::of(&"altered content identity").unwrap();
    assert_eq!(corrupt.validate(), Err(NativeProtocolError::DigestMismatch));
    let mut changed = j.clone();
    changed.degree.simple_threshold_millis = 900;
    assert_eq!(
        frozen.validate_smart(&changed),
        Err(NativeProtocolError::ContentMismatch)
    );
    let mut changed = p.clone();
    changed.fixed.applicability["instructions"] = json!("substituted default");
    assert_eq!(changed.validate(), Err(NativeProtocolError::FixedContent));
    let mut changed = p.clone();
    changed.schema = "hiroute.system-one-c/v99".into();
    assert_eq!(changed.validate(), Err(NativeProtocolError::Unsupported));
}

#[test]
fn frozen_c_serialization_survives_key_normalization_between_consumers() {
    let protocol = NativeProtocolV1::default();
    let mut judgment = JudgmentSettingsV1::default();
    protocol.stock_assessment(&mut judgment);
    let frozen = FrozenNativeProtocolV1::freeze(
        &protocol,
        judgment.degree.definition().into(),
        [(super::super::SMART_SAVING_SCOPE_ID.into(), judgment)],
    )
    .unwrap();
    let before = serde_json::to_vec(&frozen).unwrap();
    // Model a receiver that sorts generic objects, including a binary built
    // without serde_json/preserve_order. Typed outer field order stays intact.
    let wire = crate::canonicalize_json(serde_json::to_value(&frozen).unwrap());
    let recovered: FrozenNativeProtocolV1 = serde_json::from_value(wire).unwrap();
    recovered.validate().unwrap();
    assert_eq!(serde_json::to_vec(&recovered).unwrap(), before);
}

#[test]
fn legacy_omission_and_rubric_digest_remain_byte_stable() {
    let bytes = include_str!("authoring/fixtures/released-plan-editor.v2.json");
    let fixture: Value = serde_json::from_str(bytes).unwrap();
    let editor: super::super::PlanEditorStateV2 =
        serde_json::from_value(fixture["editor"].clone()).unwrap();
    let bytes = serde_json::to_vec(&editor).unwrap();
    let reopened: super::super::PlanEditorStateV2 = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(serde_json::to_vec(&reopened).unwrap(), bytes);
    assert!(
        serde_json::to_value(&editor.smart)
            .unwrap()
            .get("native_protocol")
            .is_none()
    );
    assert!(
        serde_json::to_value(&editor.smart)
            .unwrap()
            .get("assessment_review")
            .is_none()
    );
    assert!(
        serde_json::to_value(&editor.smart)
            .unwrap()
            .get("historical_applicability")
            .is_none()
    );
    let j = JudgmentSettingsV1::default();
    assert_eq!(
        j.execution_policy("old").criteria_digest,
        Some(CanonicalDigest::of(&(&j.competence.instructions, &j.competence.criteria)).unwrap())
    );
    assert_eq!(j.degree.simple_threshold_millis, 800);
    assert_eq!(j.competence.floor_millis, 500);
}

#[test]
fn categorical_default_assessment_is_labeled_from_authoring_intent() {
    let p = NativeProtocolV1::default();
    let mut j = JudgmentSettingsV1::default();
    p.current_assessment(&mut j);
    j.competence.standard = Some(super::super::AssessmentStandardV1::Default);
    let branch = super::super::MaterializedBranchV1 {
        id: "writing".into(),
        name: "Writing".into(),
        condition: "Write an article".into(),
        group: super::super::MaterializedGroupId::Branch(0),
        primary_group: Some(super::super::MaterializedGroupId::BranchPrimary(0)),
        judgment: Some(j.clone()),
    };
    let mut other = branch.clone();
    other.id = "general".into();
    other.name = "General".into();
    other.condition = "Other work".into();
    other.group = super::super::MaterializedGroupId::Branch(1);
    other.primary_group = None;
    let branches = [branch, other];
    let frozen = FrozenNativeProtocolV1::freeze(
        &p,
        branch_decision_definition(&branches, "writing"),
        branches
            .iter()
            .map(|b| (b.id.clone(), b.judgment.clone().unwrap())),
    )
    .unwrap();
    let before = serde_json::to_vec(&frozen).unwrap();
    // The frozen value is the actual shape; no runtime relabeling is needed.
    assert_eq!(frozen.scopes["writing"].standard, "default");
    assert_eq!(
        crate::RoutingObservationV1::not_dispatched(&frozen, "writing").standard,
        "default"
    );
    frozen.validate_branches(&branches, "writing").unwrap();
    assert_eq!(serde_json::to_vec(&frozen).unwrap(), before);
    let mut invalid = frozen.clone();
    invalid.scopes.get_mut("writing").unwrap().standard = "stock".into();
    invalid.digest = CanonicalDigest::of(&(
        &invalid.protocol,
        &invalid.context_layout,
        &invalid.decision,
        &invalid.scopes,
    ))
    .unwrap();
    assert_eq!(
        invalid.validate(),
        Err(NativeProtocolError::ContentMismatch)
    );
}

#[test]
fn native_category_v1_text_is_immutable_supported_publication_content() {
    let template: Value =
        serde_json::from_str(include_str!("fixtures/native_category_v1.json")).unwrap();
    assert_eq!(
        CanonicalDigest::of(&template).unwrap().as_str(),
        "sha256:b3bdc8c141db782c4e405557bcde84945c3401f1443e35dac023daf57a29086d"
    );
}

#[test]
fn direct_freezing_resolves_default_intent_and_preserves_explicit_custom_bytes() {
    let protocol = NativeProtocolV1::default();
    let original = JudgmentSettingsV1::default();
    let freeze = |judgment: JudgmentSettingsV1| {
        FrozenNativeProtocolV1::freeze(
            &protocol,
            judgment.degree.definition().into(),
            [(super::super::SMART_SAVING_SCOPE_ID.into(), judgment)],
        )
        .unwrap()
    };
    let default = freeze(original.clone());
    let mut current = original.clone();
    protocol.current_assessment(&mut current);
    assert_eq!(
        default.scopes[super::super::SMART_SAVING_SCOPE_ID].assessment,
        composed_assessment(Some(&protocol), &current)
    );
    assert_ne!(
        default.scopes[super::super::SMART_SAVING_SCOPE_ID].assessment,
        stock_c_questions()["q1"]
    );
    assert_eq!(
        default.scopes[super::super::SMART_SAVING_SCOPE_ID].standard,
        "default"
    );
    let mut custom = original.clone();
    custom.competence.standard = Some(super::super::AssessmentStandardV1::Custom);
    let frozen = freeze(custom.clone());
    assert_eq!(
        frozen.scopes[super::super::SMART_SAVING_SCOPE_ID].assessment,
        composed_assessment(Some(&protocol), &custom)
    );
    assert_eq!(
        frozen.scopes[super::super::SMART_SAVING_SCOPE_ID].standard,
        "custom"
    );
    assert_eq!(
        original,
        JudgmentSettingsV1::default(),
        "freezing mutated authored input"
    );
}
