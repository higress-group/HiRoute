use super::lifecycle::subscription_source_evidence_from_plan;
use super::*;
use hiroute_domain::{ChangeSpecV1, SubscriptionCheckIntentV2, TransactionPlanV1};
use serde_json::json;

#[test]
fn subscription_login_lease_conflict_is_unavailable_with_its_original_reason() {
    let code = "SUBSCRIPTION_LOGIN_IN_USE";
    assert_eq!(
        subscription_error_status(Some(code)),
        ComputeSubscriptionCheckStatusV2::Unavailable
    );
}

#[test]
fn rehydration_uses_approved_source_evidence_not_verified_account_evidence() {
    let source_evidence = CanonicalDigest::of_bytes(b"native-source-metadata");
    let verified_account_evidence = CanonicalDigest::of_bytes(b"checked-account-and-token");
    let original = ComputeCandidateRefV2 {
        candidate_ref: "candidate/cpa/codex/test".into(),
        candidate_revision: 7,
    };
    let checked = ComputeCandidateRefV2 {
        candidate_ref: original.candidate_ref.clone(),
        candidate_revision: 8,
    };
    let intent = SubscriptionCheckIntentV2::from_application(
        original.candidate_ref.clone(),
        original.candidate_revision,
        source_evidence.clone(),
        None,
    )
    .unwrap();
    let spec = ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: hiroute_domain::COMPUTE_SUBSCRIPTION_CHECK_COMMAND_ID_V2.into(),
        resource_id: Some(original.candidate_ref.clone()),
        desired_state: json!({
            "schema": "hiroute.compute-subscription-check/v2",
            "candidate": original.clone(),
            "expected_evidence_digest": source_evidence.clone(),
        }),
    };
    let plan = TransactionPlanV1::from_compute_subscription_check_planner(spec, intent).unwrap();

    let recovered = subscription_source_evidence_from_plan(&plan, &original, &checked).unwrap();
    assert_eq!(recovered, source_evidence);
    assert_ne!(recovered, verified_account_evidence);
}
