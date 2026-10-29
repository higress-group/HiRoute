//! Custom extensions choose a group; no HiRoute judgment policy crosses this boundary.
use hiroute_domain::{DecisionDefinitionV1, ExecutionGroupV1, SMART_SAVING_SCOPE_ID};
use serde::Deserialize;
use serde_json::{Value, json, value::RawValue};

use super::protocol::{ClassifierAssessment, ClassifierResponse};
use super::{CallFailure, PreparedDecisionRequest};
use crate::agent_turn_history::{AgentTurnHistorySnapshot, ExecutionAttribution};
use crate::server::core_runtime::adapters::{ProtocolAdapterError, prepare_replay_json_template};
use crate::server::request_plan::RestBranchClassifierAuthorityV1;

pub(super) fn request_template<'a>(
    request: impl Into<super::DecisionInput<'a>>,
    history: &AgentTurnHistorySnapshot,
    authority: &RestBranchClassifierAuthorityV1,
    current_input_origin_unverified: bool,
) -> Result<PreparedDecisionRequest, ProtocolAdapterError> {
    let mut refs = Vec::new();
    let mut latest_user = super::protocol::latest_user_wire_value(
        request,
        &mut refs,
        current_input_origin_unverified,
    )?;
    super::protocol::disclose_history_evidence(&mut latest_user, history);
    let route = match &authority.decision {
        DecisionDefinitionV1::Ordinal { .. } => {
            json!({"kind":"smart_saving","groups":["regular","primary"]})
        }
        DecisionDefinitionV1::Categorical { options, .. } => {
            let categories = options.iter().map(|option| json!({
                "id":option.id,"condition":option.criterion,
                "groups": if option.refinement.is_some() { vec!["regular","primary"] } else { vec!["regular"] }
            })).collect::<Vec<_>>();
            json!({"kind":"branches","categories":categories,"default_category":authority.default_category})
        }
    };
    let target = preceding_stage(history, authority);
    let target_category = target
        .as_ref()
        .and_then(|value| value["category"].as_str())
        .map(str::to_owned);
    let body = json!({"route":route,"latest_user":latest_user,"visible_conversation":history.visible_conversation,
        "history_partial":history.history_partial,"preceding_stage":target});
    Ok(PreparedDecisionRequest {
        template: prepare_replay_json_template(&body, refs)?,
        target_partial: false,
        target_category,
        bindings: None,
        routing_observation: None,
    })
}

fn preceding_stage(
    history: &AgentTurnHistorySnapshot,
    authority: &RestBranchClassifierAuthorityV1,
) -> Option<Value> {
    let identity = authority.extension_identity.as_ref()?;
    let target = history.assessment_target.as_ref()?;
    if authority
        .plan_identity
        .as_ref()
        .is_none_or(|(id, revision)| {
            *id != target.plan.plan_id || *revision != target.plan.plan_revision
        })
    {
        return None;
    }
    let from = history
        .assessment_from
        .filter(|from| *from < history.visible_conversation.len())?;
    let ExecutionAttribution::Single {
        selected_branch_id,
        executed_branch_id,
        ..
    } = &target.attribution
    else {
        return None;
    };
    if selected_branch_id != executed_branch_id {
        return None;
    }
    let execution = target.branch_execution.as_ref()?;
    if execution.policy.extension.as_ref() != Some(identity) {
        return None;
    }
    let in_route = match &authority.decision {
        DecisionDefinitionV1::Ordinal { .. } => executed_branch_id == SMART_SAVING_SCOPE_ID,
        DecisionDefinitionV1::Categorical { options, .. } => options
            .iter()
            .any(|option| option.id == *executed_branch_id),
    };
    if !in_route {
        return None;
    }
    Some(
        json!({"from":from,"category":executed_branch_id,"group":execution.group,"complete":!target.target_partial}),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Output {
    #[serde(default, deserialize_with = "present_field")]
    category: Option<Box<RawValue>>,
    #[serde(default)]
    group: Option<Box<RawValue>>,
    #[serde(default, deserialize_with = "present_field")]
    reason: Option<Box<RawValue>>,
    #[serde(default, deserialize_with = "present_field")]
    assessment: Option<Box<RawValue>>,
}
fn present_field<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Box<RawValue>>, D::Error> {
    Box::<RawValue>::deserialize(deserializer).map(Some)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Assessment {
    score: f64,
    partial: bool,
}

pub(super) fn parse_response(
    body: &[u8],
    definition: &DecisionDefinitionV1,
    target_category: Option<&str>,
) -> Result<ClassifierResponse, CallFailure> {
    let output: Output = serde_json::from_slice(body).map_err(|_| CallFailure::InvalidOutput)?;
    let reason = output
        .reason
        .map(|raw| serde_json::from_str::<String>(raw.get()))
        .transpose()
        .map_err(|_| CallFailure::InvalidOutput)?;
    if reason
        .as_ref()
        .is_some_and(|reason| reason.is_empty() || reason.chars().count() > 1024)
    {
        return Err(CallFailure::InvalidOutput);
    }
    let (branch_id, dual) = match definition {
        DecisionDefinitionV1::Ordinal { .. } => {
            if output.category.is_some() {
                return Err(CallFailure::InvalidOutput);
            }
            (SMART_SAVING_SCOPE_ID.to_owned(), true)
        }
        DecisionDefinitionV1::Categorical { options, .. } => {
            let category = output.category.ok_or(CallFailure::InvalidOutput)?;
            let category: String =
                serde_json::from_str(category.get()).map_err(|_| CallFailure::InvalidOutput)?;
            let option = options
                .iter()
                .find(|option| option.id == category)
                .ok_or(CallFailure::InvalidOutput)?;
            (category, option.refinement.is_some())
        }
    };
    let group = if dual {
        output
            .group
            .and_then(|raw| serde_json::from_str::<ExecutionGroupV1>(raw.get()).ok())
    } else {
        Some(ExecutionGroupV1::Regular)
    };
    let mut invalid_assessment = false;
    let assessment = output.assessment.and_then(|raw| {
        let value = serde_json::from_str::<Assessment>(raw.get())
            .ok()
            .filter(|value| {
                target_category == Some(branch_id.as_str())
                    && value.score.is_finite()
                    && (0.0..=1.0).contains(&value.score)
            });
        invalid_assessment = value.is_none();
        value.map(|value| ClassifierAssessment {
            score: value.score,
            partial: value.partial,
            reason: None,
        })
    });
    Ok(ClassifierResponse {
        branch_id,
        probabilities: None,
        degree_failed: group.is_none(),
        assessment,
        invalid_assessment,
        applicability: None,
        native_shape_complete: false,
        extension_group: group,
        extension_reason: reason,
        extension_decision: true,
    })
}
