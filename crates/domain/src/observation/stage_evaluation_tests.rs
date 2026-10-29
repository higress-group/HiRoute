//! Added facts are explicit capabilities, not an interpretation of old bytes.
use super::*;

fn stage() -> StageAssessmentRecordedV1 {
    let mut policy = crate::JudgmentSettingsV1::default().execution_policy("Task");
    policy.criteria_digest = Some(digest("rubric"));
    StageAssessmentRecordedV1 {
        scope: StageEvaluationScopeV1 {
            version: 1,
            executed_branch_id: "smart_saving".into(),
            policy,
        },
        history_membership: HistoryMembershipV1 {
            version: 1,
            branch_id: "history/task".into(),
            revision: 2,
            request_ids: vec!["request-a".into(), "request-b".into()],
            clipped: false,
            relation: HistoryRelationV1::Contextual,
            source_unverified: true,
            association_unverified: true,
            capture_partial: false,
        },
        segment_id: "stage/task".into(),
        plan_id: AgentPlanId::parse("plan/coding").unwrap(),
        plan_revision: 17,
        trigger_request_id: LogicalRequestId::parse("request-1").unwrap(),
        target_from_turn_id: TurnId::parse("turn-a").unwrap(),
        target_through_turn_id: TurnId::parse("turn-a").unwrap(),
        target_from_ordinal: 1,
        target_through_ordinal: 1,
        assessed_at_ms: 1002,
        score: 0.8,
        partial: false,
        reason: Some("Actual joint execution".into()),
    }
}

#[test]
fn composite_score_keeps_full_capture_and_uncertainty_without_model_identity() {
    let value = envelope(1, ExecutionFactV1::StageAssessmentRecorded(stage()), None);
    value.validate().unwrap();
    let bytes = serde_json::to_vec(&value).unwrap();
    let decoded: ExecutionFactEnvelopeV1 = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value, decoded);
    let score = decoded.fact.assessment_record().unwrap();
    assert!(score.model_configuration_id.is_none() && score.profile_digest.is_none());
    assert!(!score.partial);
    assert!(score.history_membership.unwrap().association_unverified);
    assert!(score.stage_scope.is_some());
}

#[test]
fn composite_score_requires_common_rubric_and_truthful_capture_flags() {
    let mut record = stage();
    record.history_membership.capture_partial = true;
    assert!(record.validate().is_err());
    record.partial = true;
    record.validate().unwrap();
    record.scope.policy.criteria_digest = None;
    assert!(record.validate().is_err());
    record = stage();
    record.scope.policy.extension = Some(crate::DecisionExtensionIdentityV1 {
        id: "external".into(),
        revision: 1,
    });
    assert!(record.validate().is_err());
}

#[test]
fn old_product_digest_cannot_authenticate_new_stage_or_input_facts() {
    const PRE_STAGE: &str =
        "sha256:2babb083d6f0005ded11c5712f7b8183117b6ce06f87aa1892c6ad1aa891a481";
    for fact in [
        ExecutionFactV1::StageAssessmentRecorded(stage()),
        ExecutionFactV1::RequestInputSelection(RequestInputSelectionV1 {
            version: 1,
            evidence: RequestInputSelectionEvidenceV1::NativeMatched,
            message_ordinal: Some(1),
            source_unverified: true,
        }),
    ] {
        let current = envelope(1, fact, None);
        current.validate().unwrap();
        let mut old = current;
        old.schema_digest = CanonicalDigest::parse(PRE_STAGE).unwrap();
        assert_eq!(old.validate(), Err(ExecutionFactError::UnsupportedSchema));
        assert_eq!(
            old.validate_persisted_contract(),
            Err(ExecutionFactError::InvalidFact)
        );
    }
}

#[test]
fn selected_input_is_a_bounded_position_not_proof_of_authorship() {
    let mut value = RequestInputSelectionV1 {
        version: 1,
        evidence: RequestInputSelectionEvidenceV1::NativeMatched,
        message_ordinal: Some(65_535),
        source_unverified: true,
    };
    value.validate().unwrap();
    value.message_ordinal = Some(65_536);
    assert!(value.validate().is_err());
    value.message_ordinal = None;
    assert!(value.validate().is_err());
    value.evidence = RequestInputSelectionEvidenceV1::Unavailable;
    value.validate().unwrap();
    value.message_ordinal = Some(0);
    assert!(value.validate().is_err());
}

#[test]
fn old_catalog_calls_and_responses_keep_their_exact_json_shape() {
    let call = serde_json::json!({"request_id":"request-1","limit":10,"cursor":null});
    let query: ObservationCatalogQueryV2 = serde_json::from_value(call.clone()).unwrap();
    assert!(!query.include_input_selection);
    assert_eq!(serde_json::to_value(query).unwrap(), call);
    let page = serde_json::json!({"contents":[],"next_cursor":null,"transcript_roots":[],"roots_partial":false});
    let value: ObservationCatalogPageV2 = serde_json::from_value(page.clone()).unwrap();
    assert!(value.input_selection.is_none());
    assert_eq!(serde_json::to_value(value).unwrap(), page);
}
