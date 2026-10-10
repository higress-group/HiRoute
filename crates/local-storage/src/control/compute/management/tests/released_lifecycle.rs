//! Actual released CLI -> daemon bytes, captured without the current serializer.
//! Producer: public v0.2.0, d625e653048b2306c88da07826d52027e9891675.
use super::*;

#[test]
fn compute_released_save_omits_edit_and_preserves_journal_control_and_accept_digest() {
    let raw = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/v0.2.0-compute-operation.json"));
    assert_eq!(CanonicalDigest::of_bytes(raw.as_bytes()).as_str(),
        "sha256:36a7ba3deb41a15a88cc6af76bba399b132b8cce07a4648d2168cf573426483c");
    let original: serde_json::Value = serde_json::from_str(raw).unwrap();
    let operation = crate::control::decode_operation(&Default::default(), raw).unwrap();
    assert_eq!(serde_json::to_value(&operation).unwrap(), original);
    let spec = operation.plan.spec();
    let change: ComputeManagementChangeV2 = serde_json::from_value(spec.desired_state.clone()).unwrap();
    change.validate_shape().unwrap();
    assert!(change.edit.is_none());
    assert_eq!(serde_json::to_value(change).unwrap(), spec.desired_state);
    let mutation: hiroute_domain::ComputeManagementMutationV2 = serde_json::from_value(operation.plan.control()["compute_management_mutation"].clone()).unwrap();
    mutation.validate_shape(spec).unwrap();
    assert!(mutation.desired().is_some());
    assert_eq!(serde_json::to_value(&mutation).unwrap(), operation.plan.control()["compute_management_mutation"]);
    assert_eq!(mutation.desired().unwrap().digest().unwrap(), *mutation.desired_digest());
    let accepted = CanonicalDigest::of(&("hiroute.compute-management-preview/v2", spec,
        &operation.expected_revisions, operation.plan.control(), operation.plan.secrets())).unwrap();
    assert_eq!(accepted, operation.accepted_digest);
    assert_eq!(accepted.as_str(), "sha256:1c55a5e135365f9e40fc739446d253ad367696a372b061c86cf13c883c3ce461");
    assert_eq!(mutation.source_id(), "source/managed-c71f3083486a232251e0c7f1");
    assert_eq!(mutation.desired().unwrap().models.len(), 2);
    assert_eq!(mutation.desired().unwrap().credentials.len(), 1);
}
