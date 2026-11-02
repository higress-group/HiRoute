//! Endpoint-scoped decision facts never supply ordinary inference capabilities.
use crate::DecisionContextBasisV1;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataDecisionConditionV1 {
    Always,
    TypesafeLongContext,
    TypesafeDefault,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataDecisionConstraintV1 {
    pub constraint_key: String,
    pub interface_key: String,
    pub condition: MetadataDecisionConditionV1,
    pub request_tokens: u32,
    pub per_question_tokens: Option<u32>,
    pub basis: DecisionContextBasisV1,
    pub note: String,
    pub evidence_refs: Vec<String>,
}

pub(super) fn valid_decision_constraints(
    constraints: &[MetadataDecisionConstraintV1],
    interfaces: &[String],
    evidence: &BTreeSet<&str>,
) -> bool {
    constraints.len() <= 16
        && constraints
            .windows(2)
            .all(|v| v[0].constraint_key < v[1].constraint_key)
        && constraints.iter().all(|c| {
            !c.constraint_key.is_empty()
                && c.constraint_key.len() <= 512
                && interfaces.contains(&c.interface_key)
                && c.request_tokens > 0
                && c.per_question_tokens
                    .is_none_or(|cap| cap > 0 && cap <= c.request_tokens)
                && !c.note.is_empty()
                && c.note.len() <= 2048
                && !c.evidence_refs.is_empty()
                && c.evidence_refs.windows(2).all(|v| v[0] < v[1])
                && c.evidence_refs
                    .iter()
                    .all(|r| evidence.contains(r.as_str()))
        })
}
