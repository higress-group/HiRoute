use serde::{Deserialize, Serialize};

use crate::DecisionFailureCodeV2;
pub use hiroute_domain::{PlanReferenceBlockerV1, PlanReferenceProtectionV1};

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionRemovalPreviewRequestV1 {
    pub id: String,
    pub expected_revision: u64,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionRemovalPreviewV1 {
    pub id: String,
    pub revision: u64,
    pub failure: Option<DecisionFailureCodeV2>,
    pub blockers: Vec<PlanReferenceBlockerV1>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_removal_preview_is_additive_and_has_no_mutation_authority() {
        let command = crate::command_by_id("decision.removal.preview").unwrap();
        assert_eq!(command.operation_id, "PreviewDecisionRemovalV1");
        let request = serde_json::json!({"id":"decision/fixture","expected_revision":2});
        assert!(serde_json::from_value::<DecisionRemovalPreviewRequestV1>(request.clone()).is_ok());
        let mut mutation = request;
        mutation["reclaim"] = serde_json::json!([]);
        assert!(serde_json::from_value::<DecisionRemovalPreviewRequestV1>(mutation).is_err());
        let preview = DecisionRemovalPreviewV1 {
            id: "decision/fixture".into(),
            revision: 2,
            failure: Some(DecisionFailureCodeV2::DecisionInUse),
            blockers: vec![PlanReferenceBlockerV1 {
                plan_id: Some("plan/fixture".into()),
                draft_id: None,
                revision: 1,
                protection: PlanReferenceProtectionV1::LkgPublication,
            }],
        };
        let value = serde_json::to_value(&preview).unwrap();
        assert_eq!(value["failure"], "DECISION_IN_USE");
        assert_eq!(value.as_object().unwrap().len(), 4);
        assert_eq!(
            serde_json::from_value::<DecisionRemovalPreviewV1>(value).unwrap(),
            preview
        );
    }
}
