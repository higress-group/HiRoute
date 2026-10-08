//! One strict custom v1 contract, shared by native System One reduction.
use std::collections::BTreeMap;

use hiroute_domain::{DecisionDefinitionV1, OrdinalLevelV1, SMART_SAVING_SCOPE_ID};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json, value::RawValue};

use super::super::adapters::{
    ProtocolAdapterError, ReplacementEncoding, RequestedReplacement, prepare_replay_json_template,
};
use super::super::model_ir::{ContentPart, ImageSource, MessageRole, ModelRequestIRV1};
use super::{CallFailure, PreparedDecisionRequest};
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
    pub(super) branch_id: String,
    pub(super) probabilities: Option<BTreeMap<String, f64>>,
    pub(super) degree_failed: bool,
    pub(super) assessment: Option<ClassifierAssessment>,
    pub(super) invalid_assessment: bool,
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
    let target = history.assessment_target.as_ref()?;
    let from = history
        .assessment_from
        .filter(|i| *i < history.visible_conversation.len())?;
    let ExecutionAttribution::Single {
        executed_branch_id, ..
    } = &target.attribution
    else {
        return None;
    };
    let (judgment, name) = if executed_branch_id == SMART_SAVING_SCOPE_ID {
        (authority.smart_judgment.as_ref()?, "Smart saving")
    } else {
        let branch = authority
            .branch_policies
            .iter()
            .find(|b| b.id == *executed_branch_id)?;
        (&branch.judgment, branch.name.as_str())
    };
    if target.branch_execution.as_ref()?.policy != judgment.execution_policy(name) {
        return None;
    }
    Some(AssessmentDefinition {
        from,
        instructions: judgment.competence.instructions.clone(),
        criteria: std::array::from_fn(|i| AssessmentCriterion {
            score: i as f64 / 2.0,
            criterion: judgment.competence.criteria[i].clone(),
        }),
    })
}

pub(super) fn classifier_request_template(
    request: &ModelRequestIRV1,
    history: &AgentTurnHistorySnapshot,
    authority: &RestBranchClassifierAuthorityV1,
) -> Result<PreparedDecisionRequest, ProtocolAdapterError> {
    let mut refs = Vec::new();
    let latest_user = latest_user_wire_value(request, &mut refs)?;
    let target = assessment_definition(history, authority);
    let body = json!({
        "decision": authority.decision,
        "latest_user": latest_user,
        "visible_conversation": history.visible_conversation,
        "history_partial": history.history_partial,
        "assessment_target": target,
    });
    Ok(PreparedDecisionRequest {
        template: prepare_replay_json_template(&body, refs)?,
        has_target: target.is_some(),
        target_partial: false,
        bindings: None,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassifierOutput {
    decision: Box<RawValue>,
    #[serde(default)]
    assessment: Option<Box<RawValue>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CategoricalOutput {
    kind: String,
    choice: String,
    #[serde(default)]
    refinement: Option<Box<RawValue>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OrdinalOutput {
    kind: String,
    probabilities: UniqueMap<f64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssessmentOutput {
    score: f64,
    partial: bool,
    #[serde(default)]
    reason: Option<String>,
}

/// Raw nested values isolate malformed optional assessment/refinement from valid categories.
pub(super) fn parse_classifier_response(
    body: &[u8],
    definition: &DecisionDefinitionV1,
    has_target: bool,
) -> Result<ClassifierResponse, CallFailure> {
    let output: ClassifierOutput =
        serde_json::from_slice(body).map_err(|_| CallFailure::InvalidOutput)?;
    let (branch_id, probabilities, degree_failed) = match definition {
        DecisionDefinitionV1::Ordinal { levels, .. } => {
            let parsed = parse_ordinal(output.decision.get(), levels);
            (
                SMART_SAVING_SCOPE_ID.into(),
                parsed.clone().ok(),
                parsed.is_err(),
            )
        }
        DecisionDefinitionV1::Categorical { options, .. } => {
            let selected: CategoricalOutput = serde_json::from_str(output.decision.get())
                .map_err(|_| CallFailure::InvalidOutput)?;
            if selected.kind != "categorical" {
                return Err(CallFailure::InvalidOutput);
            }
            let option = options
                .iter()
                .find(|o| o.id == selected.choice)
                .ok_or(CallFailure::InvalidOutput)?;
            match (&option.refinement, selected.refinement) {
                (Some(degree), raw) => {
                    let parsed = raw
                        .ok_or(CallFailure::InvalidOutput)
                        .and_then(|raw| parse_ordinal(raw.get(), &degree.levels));
                    (selected.choice, parsed.clone().ok(), parsed.is_err())
                }
                (None, None) => (selected.choice, None, false),
                (None, Some(_)) => return Err(CallFailure::InvalidOutput),
            }
        }
    };
    let mut invalid_assessment = false;
    let assessment = output.assessment.and_then(|raw| {
        let parsed = serde_json::from_str::<AssessmentOutput>(raw.get()).ok();
        let valid = has_target
            && parsed.as_ref().is_some_and(|a| {
                a.score.is_finite()
                    && (0.0..=1.0).contains(&a.score)
                    && a.reason
                        .as_ref()
                        .is_none_or(|r| !r.is_empty() && r.chars().count() <= 1024)
            });
        if !valid {
            invalid_assessment = true;
            return None;
        }
        parsed.map(|a| ClassifierAssessment {
            score: a.score,
            partial: a.partial,
            reason: a.reason,
        })
    });
    Ok(ClassifierResponse {
        branch_id,
        probabilities,
        degree_failed,
        assessment,
        invalid_assessment,
    })
}

fn parse_ordinal(
    raw: &str,
    levels: &[OrdinalLevelV1],
) -> Result<BTreeMap<String, f64>, CallFailure> {
    let output: OrdinalOutput =
        serde_json::from_str(raw).map_err(|_| CallFailure::InvalidOutput)?;
    if output.kind != "ordinal" {
        return Err(CallFailure::InvalidOutput);
    }
    normalize_probabilities(output.probabilities.0, levels.iter().map(|l| l.id.as_str()))
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
