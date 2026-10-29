//! Capability guards for the two retained parents of the history/extension merge.
//! These synthetic contract controls do not claim an old-binary upgrade run.
use super::*;
use serde_json::{Value, json};

const HISTORY: &str = "sha256:857e4f2b42278ac3877e34f2188eeecc7c1d188832316b23f577f3bc0b82b1ac";
const EXTENSION: &str = "sha256:5f42929cdb812799e4e4a10885324debb21e6489bde5e935599de57b54b8ab23";

fn retained_finished() -> Value {
    // Exact pre-admission fixture retained by the history parent. Only its
    // descriptor is advanced in these controls; membership is never invented
    // by the recovery reader or dropped to make the old contract admissible.
    serde_json::from_str::<Value>(include_str!("fixtures/pre-admission-968-finished.json")).unwrap()
        ["fact"]
        .clone()
}

fn under_contract(contract: &str, fact: Value) -> ExecutionFactEnvelopeV1 {
    let mut value: Value =
        serde_json::from_str(include_str!("fixtures/pre-admission-968-finished.json")).unwrap();
    value["schema_digest"] = json!(contract);
    value["fact"] = fact;
    serde_json::from_value(value).unwrap()
}

fn admission_fact() -> Value {
    json!({"kind":"agent_turn_admitted","version":1,"agent_turn_id":"turn/admitted",
        "branch_id":"history/branch","revision":2,"ordinal":3,
        "opening_cause":"no_comparable_history"})
}

fn assessment_fact() -> Value {
    json!({"kind":"branch_assessment_recorded","segment_id":"segment/1",
        "plan_id":"plan/coding","plan_revision":17,"model_configuration_id":"model/1",
        "profile_digest":"unknown","trigger_request_id":"request-1",
        "target_from_turn_id":"turn/admitted","target_through_turn_id":"turn/admitted",
        "target_from_ordinal":3,"target_through_ordinal":3,"assessed_at_ms":1002,
        "score":0.75,"partial":false,"reason":null})
}

fn extension_policy() -> Value {
    // Exact field shape introduced by main 80ab44102, descriptor 5f42929c.
    json!({"name":"Extension","extension":{"id":"extension/test","revision":2},
        "criteria_digest":null})
}

fn extension_route() -> Value {
    let mut value =
        serde_json::to_value(ExecutionFactV1::RouteDecision(Box::new(route()))).unwrap();
    let decision = &mut value["complexity"];
    decision["decision_protocol"] = json!("custom_extension_v1");
    decision["decision_source"] = json!("external_classifier");
    decision["complexity_score"] = Value::Null;
    decision["threshold"] = Value::Null;
    decision["classification_duration_micros"] = json!(1);
    value
}

fn extension_finished() -> Value {
    let mut value = retained_finished();
    value.as_object_mut().unwrap().remove("history_membership");
    value["attribution"] = json!("single");
    value["executed_branch_id"] = json!("smart_saving");
    value["model_configuration_id"] = json!("model/1");
    value["profile_digest"] = json!("unknown");
    value["branch_execution"] = json!({"policy":extension_policy(),
        "group":"primary","candidate_index":0});
    value
}

fn assert_recovery_only(value: ExecutionFactEnvelopeV1) {
    assert_eq!(value.validate(), Err(ExecutionFactError::UnsupportedSchema));
    value.validate_persisted_contract().unwrap();
    let bytes = serde_json::to_vec(&value).unwrap();
    let digest = CanonicalDigest::of(&value).unwrap();
    let reopened: ExecutionFactEnvelopeV1 = serde_json::from_slice(&bytes).unwrap();
    reopened.validate_persisted_contract().unwrap();
    assert_eq!(reopened, value);
    assert_eq!(serde_json::to_vec(&reopened).unwrap(), bytes);
    assert_eq!(CanonicalDigest::of(&reopened).unwrap(), digest);
}

fn assert_new_capability_rejected(contract: &str, fact: Value) {
    let old = under_contract(contract, fact.clone());
    assert_eq!(old.validate(), Err(ExecutionFactError::UnsupportedSchema));
    assert_eq!(
        old.validate_persisted_contract(),
        Err(ExecutionFactError::InvalidFact),
        "old descriptor authenticated a capability absent from its producer: {fact}"
    );
    // The negative is the descriptor boundary, not an otherwise malformed fact.
    under_contract(EXECUTION_FACT_PORT_DIGEST_V2, fact)
        .validate()
        .unwrap();
}

#[test]
fn history_parent_contract_recovers_membership_admission_and_auxiliary_without_rewriting() {
    assert_recovery_only(under_contract(HISTORY, retained_finished()));
    assert_recovery_only(under_contract(HISTORY, admission_fact()));
    let mut assessment = assessment_fact();
    assessment["history_membership"] = retained_finished()["history_membership"].clone();
    assert_recovery_only(under_contract(HISTORY, assessment));
    let mut auxiliary =
        serde_json::to_value(ExecutionFactV1::RouteDecision(Box::new(route()))).unwrap();
    auxiliary["native_auxiliary"] = json!(true);
    assert_recovery_only(under_contract(HISTORY, auxiliary));
}

#[test]
fn extension_parent_contract_recovers_direct_decision_and_policy_without_rewriting() {
    let mut decision = extension_route();
    decision["complexity"]["extension_reason"] = json!("Use primary for this input");
    decision["complexity"]["selection_reason"] = json!("extension_decision");
    decision["complexity"]["execution_group"] = json!("primary");
    decision["complexity"]["policy"] = extension_policy();
    assert_recovery_only(under_contract(EXTENSION, decision));
    assert_recovery_only(under_contract(EXTENSION, extension_finished()));
    assert_recovery_only(under_contract(EXTENSION, assessment_fact()));
}

#[test]
fn history_parent_descriptor_cannot_authenticate_extension_capabilities() {
    let mut reason = extension_route();
    reason["complexity"]["extension_reason"] = json!("Synthetic foreign capability");
    assert_new_capability_rejected(HISTORY, reason);
    for selection in ["extension_decision", "decision_unavailable"] {
        let mut decision = extension_route();
        decision["complexity"]["selection_reason"] = json!(selection);
        assert_new_capability_rejected(HISTORY, decision);
    }
    let mut policy = extension_route();
    policy["complexity"]["policy"] = extension_policy();
    assert_new_capability_rejected(HISTORY, policy);
    assert_new_capability_rejected(HISTORY, extension_finished());
}

#[test]
fn extension_parent_descriptor_cannot_authenticate_history_capabilities() {
    let mut finished = extension_finished();
    finished["history_membership"] = retained_finished()["history_membership"].clone();
    assert_new_capability_rejected(EXTENSION, finished);
    let mut assessment = assessment_fact();
    assessment["history_membership"] = retained_finished()["history_membership"].clone();
    assert_new_capability_rejected(EXTENSION, assessment);
    let mut auxiliary = extension_route();
    auxiliary["native_auxiliary"] = json!(true);
    assert_new_capability_rejected(EXTENSION, auxiliary);
    assert_new_capability_rejected(EXTENSION, admission_fact());
}
