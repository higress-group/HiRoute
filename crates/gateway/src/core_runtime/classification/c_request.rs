//! Deterministic allocation: reserve Q3, admit the target, then optional background.
//! Dropping a historical question never starts another history admission pass.
use super::protocol::{assessment_definition_result, latest_user_wire_value};
use super::system_one::{base_questions, question_id};
use super::{CallFailure, PreparedDecisionRequest};
use crate::agent_turn_history::{AgentTurnHistorySnapshot, ExecutionAttribution, VisibleAgentTurn};
use crate::server::core_runtime::adapters::prepare_replay_json_template;
use crate::server::request_plan::RestBranchClassifierAuthorityV1;
use hiroute_domain::{
    ApplicabilityReasonV1 as R, ApplicabilityStateV1 as S, HistoricalProtectionActionV1 as A,
    ProtectionEligibilityV1 as E, RoutingObservationV1,
};
use serde_json::json;
use std::sync::Arc;

pub(super) const MAX_REQUEST_BYTES: usize = 256 * 1024;

pub(super) fn request_template<'a>(
    authority: &RestBranchClassifierAuthorityV1,
    request: impl Into<super::DecisionInput<'a>>,
    history: &AgentTurnHistorySnapshot,
    current_input_origin_unverified: bool,
) -> Result<PreparedDecisionRequest, CallFailure> {
    let protocol = authority
        .native_protocol
        .as_ref()
        .ok_or(CallFailure::RejectedInput)?;
    let mut refs = Vec::new();
    let mut latest = latest_user_wire_value(request, &mut refs, current_input_origin_unverified)
        .map_err(|_| CallFailure::RejectedInput)?;
    super::protocol::disclose_history_evidence(&mut latest, history);
    let target_result = assessment_definition_result(history, authority);
    let omitted_eligibility = target_result.as_ref().err().copied().unwrap_or(E::Unrated);
    let target = target_result.ok();
    let (mut questions, mut bindings) = base_questions(authority);
    let scope = history
        .assessment_target
        .as_ref()
        .and_then(|t| t.evaluation_policy())
        .and_then(|(branch, _)| protocol.scopes.get(branch));
    if target.is_some() {
        let scope = scope.ok_or(CallFailure::RejectedInput)?;
        let id = question_id(&questions);
        questions.insert(id.clone(), scope.assessment.clone());
        bindings.assessment = Some(id);
    }
    let initially_partial = target.is_some()
        && history
            .assessment_target
            .as_ref()
            .is_none_or(|t| t.target_partial);
    if target.is_some()
        && !initially_partial
        && history
            .assessment_target
            .as_ref()
            .is_some_and(|target| matches!(target.attribution, ExecutionAttribution::Single { .. }))
    {
        let id = question_id(&questions);
        questions.insert(id.clone(), protocol.protocol.fixed.applicability.clone());
        bindings.applicability = Some(id);
    }
    let mut body = json!({"model":authority.system_one_model,"questions":questions,"state":{
        "latest_user":latest,"prior_context":[],"assessment_stage":[],"history_partial":false
    }});
    let base = prepare_replay_json_template(&body, refs.clone())
        .map_err(|_| CallFailure::RejectedInput)?;
    let from = target.as_ref().map(|t| t.from);
    let stage = from.map_or(&[][..], |i| &history.visible_conversation[i..]);
    let background = from.map_or(history.visible_conversation.as_slice(), |i| {
        &history.visible_conversation[..i]
    });
    // Snapshot turns already have boundaries. Execution status does not invalidate
    // their user requirements, and filtering individual turns would break chronology.
    let cap_start = background
        .len()
        .saturating_sub(usize::from(protocol.protocol.prior_context_max_turns));
    let mut budget = MAX_REQUEST_BYTES.saturating_sub(base.wire_len);
    let admitted_stage = if base.wire_len > MAX_REQUEST_BYTES {
        0
    } else {
        admit_suffix(stage, &mut budget)?
    };
    let target_partial = initially_partial
        || admitted_stage < stage.len()
        || (target.is_some() && base.wire_len > MAX_REQUEST_BYTES);
    // This decision is final while Q3 is still reserved. Freed question bytes cannot refill it.
    let admitted_prior = if base.wire_len <= MAX_REQUEST_BYTES && admitted_stage == stage.len() {
        admit_suffix(&background[cap_start..], &mut budget)?
    } else {
        0
    };
    if target_partial && let Some(id) = bindings.applicability.take() {
        body["questions"].as_object_mut().unwrap().remove(&id);
    }
    if admitted_stage == 0
        && let Some(id) = bindings.assessment.take()
    {
        body["questions"].as_object_mut().unwrap().remove(&id);
    }
    body["state"]["assessment_stage"] =
        serde_json::to_value(&stage[stage.len() - admitted_stage..])
            .map_err(|_| CallFailure::RejectedInput)?;
    body["state"]["prior_context"] =
        serde_json::to_value(&background[background.len() - admitted_prior..])
            .map_err(|_| CallFailure::RejectedInput)?;
    body["state"]["history_partial"] =
        json!(history.history_partial || admitted_prior < background.len() || target_partial);
    let template =
        prepare_replay_json_template(&body, refs).map_err(|_| CallFailure::RejectedInput)?;
    if template.wire_len > MAX_REQUEST_BYTES {
        return Err(CallFailure::RejectedInput);
    }
    let requested = bindings.applicability.is_some();
    let observation = RoutingObservationV1 {
        protocol: protocol.protocol.schema.clone(),
        content_digest: protocol.digest.clone(),
        bundle_digest: hiroute_domain::CanonicalDigest::of(&protocol.protocol.fixed)
            .map_err(|_| CallFailure::RejectedInput)?,
        provider: authority.decision_provider.clone(),
        model: authority.system_one_model.clone().unwrap_or_default(),
        standard: scope.map_or("custom", |s| s.standard.as_str()).into(),
        target_segment: history
            .assessment_target
            .as_ref()
            .map(|t| t.segment_id.clone()),
        target_complete: target.is_some() && !target_partial && admitted_stage > 0,
        eligibility: if target.is_none() {
            omitted_eligibility
        } else if target_partial {
            E::PartialTarget
        } else {
            E::Unrated
        },
        applicability_requested: requested,
        applicability_state: if requested {
            S::Unavailable
        } else {
            S::NotRequested
        },
        applicability_reason: if requested {
            R::Pending
        } else if target_partial {
            R::PartialTarget
        } else {
            R::NoTarget
        },
        applicability_probability: None,
        protection_action: A::NotConsidered,
        simple_probability: None,
        simple_threshold_millis: scope.map_or(800, |s| s.simple_threshold_millis),
        score: None,
        competence_floor_millis: scope.map_or(500, |s| s.competence_floor_millis),
        applicability_boundary_millis: 500,
        request_bytes: template.wire_len as u32,
        prior_retained: admitted_prior as u32,
        prior_removed: (background.len() - admitted_prior) as u32,
        stage_retained: admitted_stage as u32,
        stage_removed: (stage.len() - admitted_stage) as u32,
    };
    Ok(PreparedDecisionRequest {
        target_category: None,
        template,
        target_partial,
        bindings: Some(bindings),
        routing_observation: Some(observation),
    })
}

fn admit_suffix(turns: &[Arc<VisibleAgentTurn>], budget: &mut usize) -> Result<usize, CallFailure> {
    let mut count = 0;
    for turn in turns.iter().rev() {
        let Some(available) = budget.checked_sub(usize::from(count > 0)) else {
            break;
        };
        let mut writer = super::system_one::ByteBudget(available);
        if let Err(error) = serde_json::to_writer(&mut writer, turn) {
            if error.io_error_kind() == Some(std::io::ErrorKind::FileTooLarge) {
                break;
            }
            return Err(CallFailure::RejectedInput);
        }
        *budget = writer.0;
        count += 1;
    }
    Ok(count)
}
