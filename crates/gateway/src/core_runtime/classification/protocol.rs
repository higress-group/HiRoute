//! One strict custom v1 contract, shared by native System One reduction.
use std::collections::BTreeMap;

use hiroute_domain::SMART_SAVING_SCOPE_ID;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::super::adapters::{ProtocolAdapterError, ReplacementEncoding, RequestedReplacement};
use super::super::model_ir::{ContentPart, ImageSource, MessageRole, ModelRequestIRV1};
use super::CallFailure;
use crate::agent_turn_history::{AgentTurnHistorySnapshot, ExecutionAttribution};
use crate::content_ref::ContentValueExt;
use crate::server::request_plan::RestBranchClassifierAuthorityV1;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ClassifierAssessment {
    pub(super) score: f64,
    pub(super) partial: bool,
    pub(super) reason: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ClassifierResponse {
    pub(super) extension_group: Option<hiroute_domain::ExecutionGroupV1>,
    pub(super) extension_reason: Option<String>,
    pub(super) extension_decision: bool,
    pub(super) branch_id: String,
    pub(super) probabilities: Option<BTreeMap<String, f64>>,
    pub(super) degree_failed: bool,
    pub(super) assessment: Option<ClassifierAssessment>,
    pub(super) invalid_assessment: bool,
    pub(super) applicability: Option<f64>,
    pub(super) native_shape_complete: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct AssessmentDefinition {
    pub(super) from: usize,
    pub(super) instructions: String,
    pub(super) criteria: [AssessmentCriterion; 3],
}
#[derive(Clone, Debug, Serialize)]
pub(super) struct AssessmentCriterion {
    score: f64,
    pub(super) criterion: String,
}

/// Target identity remains local; use only the standard frozen for its actual execution.
pub(super) fn assessment_definition(
    history: &AgentTurnHistorySnapshot,
    authority: &RestBranchClassifierAuthorityV1,
) -> Option<AssessmentDefinition> {
    assessment_definition_result(history, authority).ok()
}

pub(super) fn assessment_definition_result(
    history: &AgentTurnHistorySnapshot,
    authority: &RestBranchClassifierAuthorityV1,
) -> Result<AssessmentDefinition, hiroute_domain::ProtectionEligibilityV1> {
    use hiroute_domain::ProtectionEligibilityV1 as E;
    let target = history.assessment_target.as_ref().ok_or(E::NoTarget)?;
    let from = history
        .assessment_from
        .filter(|i| *i < history.visible_conversation.len())
        .ok_or(E::InvalidBoundary)?;
    let ExecutionAttribution::Single {
        executed_branch_id, ..
    } = &target.attribution
    else {
        return Err(E::AmbiguousAttribution);
    };
    let (judgment, name) = if executed_branch_id == SMART_SAVING_SCOPE_ID {
        (
            authority.smart_judgment.as_ref().ok_or(E::Unrated)?,
            "Smart saving",
        )
    } else {
        let branch = authority
            .branch_policies
            .iter()
            .find(|b| b.id == *executed_branch_id)
            .ok_or(E::DifferentCategory)?;
        (
            branch.judgment.as_ref().ok_or(E::Unrated)?,
            branch.name.as_str(),
        )
    };
    let policy = authority.native_protocol.as_ref().map_or_else(
        || judgment.execution_policy(name),
        |p| p.execution_policy(executed_branch_id, judgment, name),
    );
    if target.branch_execution.as_ref().ok_or(E::Unrated)?.policy != policy {
        return Err(E::DifferentPolicy);
    }
    Ok(AssessmentDefinition {
        from,
        instructions: judgment.competence.instructions.clone(),
        criteria: std::array::from_fn(|i| AssessmentCriterion {
            score: i as f64 / 2.0,
            criterion: judgment.competence.criteria[i].clone(),
        }),
    })
}

pub(super) fn latest_user_wire_value(
    request: &ModelRequestIRV1,
    refs: &mut Vec<RequestedReplacement>,
) -> Result<Value, ProtocolAdapterError> {
    let message = request
        .messages
        .iter()
        .rev()
        .find(|message| {
            message.role == MessageRole::User
                && message.content.iter().any(|part| {
                    matches!(part, ContentPart::Text { .. } | ContentPart::Image { .. })
                })
        })
        .ok_or_else(|| {
            ProtocolAdapterError::Serialization(
                "classification requires a current user message".into(),
            )
        })?;
    let values = message
        .content
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(project_text(text, refs)),
            ContentPart::Image {
                source: ImageSource::Url { .. },
            } => Some(json!({"kind":"unavailable","source_kind":"image_url"})),
            ContentPart::Image {
                source: ImageSource::Base64 { .. },
            } => Some(json!({"kind":"unavailable","source_kind":"image_base64"})),
            ContentPart::ToolCall { .. }
            | ContentPart::ToolResult { .. }
            | ContentPart::ProviderState { .. } => None,
        })
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Err(ProtocolAdapterError::Serialization(
            "classification current user message is empty".into(),
        ));
    }
    Ok(Value::Array(values))
}

fn project_text(value: &str, refs: &mut Vec<RequestedReplacement>) -> Value {
    if let Some(content) = value.content_ref() {
        refs.push(RequestedReplacement {
            content,
            encoding: ReplacementEncoding::JsonString,
        });
    }
    json!({"kind":"text","text":value})
}

pub(super) fn normalize_probabilities<'a>(
    mut probabilities: BTreeMap<String, f64>,
    expected: impl Iterator<Item = &'a str>,
) -> Result<BTreeMap<String, f64>, CallFailure> {
    let expected: std::collections::BTreeSet<_> = expected.collect();
    if probabilities.len() != expected.len()
        || probabilities.iter().any(|(id, p)| {
            !expected.contains(id.as_str()) || !p.is_finite() || !(0.0..=1.0).contains(p)
        })
    {
        return Err(CallFailure::InvalidOutput);
    }
    let sum: f64 = probabilities.values().sum();
    if (sum - 1.0).abs() > 1e-6 || sum == 0.0 {
        return Err(CallFailure::InvalidOutput);
    }
    probabilities.values_mut().for_each(|p| *p /= sum);
    Ok(probabilities)
}

/// serde maps ordinarily overwrite duplicates; decisions must never silently choose the last.
pub(super) struct UniqueMap<T>(pub(super) BTreeMap<String, T>);
impl<'de, T: Deserialize<'de>> Deserialize<'de> for UniqueMap<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Visitor<T> {
            type Value = UniqueMap<T>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object with unique keys")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut access: A,
            ) -> Result<Self::Value, A::Error> {
                let mut entries = BTreeMap::new();
                while let Some((key, value)) = access.next_entry::<String, T>()? {
                    if entries.insert(key, value).is_some() {
                        return Err(serde::de::Error::custom("duplicate decision key"));
                    }
                }
                Ok(UniqueMap(entries))
            }
        }
        d.deserialize_map(Visitor(std::marker::PhantomData))
    }
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
