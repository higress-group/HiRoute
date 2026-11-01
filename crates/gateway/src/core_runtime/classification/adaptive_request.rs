//! Adaptive C preparation. Current input/questions are protected; historical reservation is final.
use super::{
    CallFailure, DecisionInput, PreparedDecisionRequest, adaptive_history as history_budget,
    adaptive_replay,
    context_budget::{self, BudgetRejection, Remaining},
    openai_decisions,
    protocol::{assessment_definition_result, latest_user_wire_value},
    system_one::{base_questions, question_id},
};
use crate::agent_turn_history::{AgentTurnHistorySnapshot, ExecutionAttribution, VisibleAgentTurn};
use crate::server::request_plan::RestBranchClassifierAuthorityV1;
use hiroute_domain::*;
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Serialize)]
struct State<'a> {
    latest_user: &'a Value,
    prior_context: &'a [history_budget::TurnView<'a>],
    assessment_stage: &'a [history_budget::TurnView<'a>],
    history_partial: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_context: Option<&'a Value>,
}

pub(super) fn request_template(
    authority: &RestBranchClassifierAuthorityV1,
    input: DecisionInput<'_>,
    history: &AgentTurnHistorySnapshot,
    origin_unverified: bool,
) -> Result<PreparedDecisionRequest, CallFailure> {
    let context = authority
        .context_budget
        .as_ref()
        .ok_or(CallFailure::RejectedInput)?;
    let frozen = authority
        .native_protocol
        .as_ref()
        .ok_or(CallFailure::RejectedInput)?;
    let model = authority
        .system_one_model
        .as_deref()
        .ok_or(CallFailure::RejectedInput)?;
    if frozen.context_budget.as_ref() != Some(context) || !context.validate() {
        return Err(CallFailure::RejectedInput);
    }
    let mut refs = Vec::new();
    let mut latest = latest_user_wire_value(input, &mut refs, origin_unverified)
        .map_err(|_| CallFailure::RejectedInput)?;
    super::protocol::disclose_history_evidence(&mut latest, history);
    let target_result = assessment_definition_result(history, authority);
    let omitted_eligibility = target_result
        .as_ref()
        .err()
        .copied()
        .unwrap_or(ProtectionEligibilityV1::Unrated);
    let target = target_result.ok();
    let (mut questions, mut bindings) = base_questions(authority);
    let scope_id = history
        .assessment_target
        .as_ref()
        .and_then(|t| t.evaluation_policy())
        .map(|(id, _)| id);
    let scope = scope_id.and_then(|id| frozen.scopes.get(id));
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
            .is_some_and(|t| matches!(t.attribution, ExecutionAttribution::Single { .. }))
    {
        let id = question_id(&questions);
        questions.insert(id.clone(), frozen.protocol.fixed.applicability.clone());
        bindings.applicability = Some(id);
    }
    let mut core = json!({"model":model,"questions":questions,"state":{
        "latest_user":latest,"prior_context":[],"assessment_stage":[],"history_partial":false
    }});
    let with_replay = frozen.protocol.schema == SYSTEM_ONE_C_V4;
    if with_replay {
        core["state"]["request_context"] = super::request_context::empty();
    }
    let measurement =
        context_budget::measure_core(authority.wire_protocol, &core, refs.clone(), context)?;
    let mut budget = match measurement.allowance {
        Ok(remaining) => remaining,
        Err(rejection) => {
            // Verify the mandatory current questions against the same measured state.
            // Historical question removal never creates another admission allowance.
            let mut mandatory = questions.clone();
            for id in bindings
                .assessment
                .iter()
                .chain(bindings.applicability.iter())
            {
                mandatory.remove(id);
            }
            measurement.mandatory_allowance(authority.wire_protocol, model, &mandatory, context)?;
            Remaining {
                bytes: 0,
                values: 0,
                limiting: rejection.limiting,
            }
        }
    };
    let reservation_failed = measurement.allowance.is_err();
    let from = target.as_ref().map(|t| t.from);
    let stage: &[Arc<VisibleAgentTurn>] =
        from.map_or(&[], |from| &history.visible_conversation[from..]);
    let prior = from.map_or(history.visible_conversation.as_slice(), |from| {
        &history.visible_conversation[..from]
    });
    let selected_stage = if reservation_failed {
        history_budget::Selection::empty(stage.len())
    } else {
        history_budget::admit(stage, usize::MAX, true, &mut budget)?
    };
    let target_partial =
        initially_partial || selected_stage.partial || reservation_failed && target.is_some();
    let selected_prior = if !reservation_failed && !target_partial {
        history_budget::admit(
            prior,
            usize::from(frozen.protocol.prior_context_max_turns),
            false,
            &mut budget,
        )?
    } else {
        history_budget::Selection::empty(prior.len())
    };
    let replay = if with_replay {
        adaptive_replay::admit(input, &mut budget)?
    } else {
        adaptive_replay::ReplaySelection::empty()
    };
    refs.extend(replay.refs.iter().cloned());
    // No second admission pass follows either removal.
    if target_partial || !selected_stage.execution_evidence {
        if let Some(id) = bindings.applicability.take() {
            questions.remove(&id);
        }
    }
    if !selected_stage.execution_evidence {
        if let Some(id) = bindings.assessment.take() {
            questions.remove(&id);
        }
    }
    let state = State {
        latest_user: &latest,
        prior_context: &selected_prior.turns,
        assessment_stage: &selected_stage.turns,
        history_partial: history.history_partial
            || selected_prior.turns.len() < prior.len()
            || target_partial,
        request_context: with_replay.then_some(&replay.value),
    };
    let template = openai_decisions::prepare_native_state(
        authority.wire_protocol,
        model,
        &questions,
        &state,
        refs,
    )?;
    let question_bytes = context_budget::final_question_bytes(
        authority.wire_protocol,
        model,
        &questions,
        template.wire_len,
        &measurement.question_envelopes,
    )?;
    if template.wire_len > context.request_usable_tokens() as usize
        || question_bytes > context.per_question_usable_tokens() as usize
    {
        return Err(CallFailure::BudgetRejected(BudgetRejection {
            reason: DecisionContextNoFitV1::Accounting,
            limiting: budget.limiting,
            request_bytes: Some(template.wire_len as u64),
            question_bytes: Some(question_bytes as u64),
        }));
    }
    let requested = bindings.applicability.is_some();
    let mut observation = base_observation(authority, history, scope_id.unwrap_or_default());
    observation.target_complete =
        target.is_some() && !target_partial && selected_stage.execution_evidence;
    observation.eligibility = if target.is_none() {
        omitted_eligibility
    } else if target_partial {
        ProtectionEligibilityV1::PartialTarget
    } else {
        ProtectionEligibilityV1::Unrated
    };
    observation.applicability_requested = requested;
    observation.applicability_state = if requested {
        ApplicabilityStateV1::Unavailable
    } else {
        ApplicabilityStateV1::NotRequested
    };
    observation.applicability_reason = if requested {
        ApplicabilityReasonV1::Pending
    } else if target_partial {
        ApplicabilityReasonV1::PartialTarget
    } else {
        ApplicabilityReasonV1::NoTarget
    };
    observation.request_bytes =
        u32::try_from(template.wire_len).map_err(|_| CallFailure::RejectedInput)?;
    observation.prior_retained = selected_prior.turns.len() as u32;
    observation.prior_removed = (prior.len() - selected_prior.turns.len()) as u32;
    observation.stage_retained = selected_stage.turns.len() as u32;
    observation.stage_removed = (stage.len() - selected_stage.turns.len()) as u32;
    let mut diagnostic = empty_diagnostic(context, budget.limiting);
    diagnostic.estimated_request_tokens = Some(template.wire_len as u64);
    diagnostic.estimated_largest_question_tokens = Some(question_bytes as u64);
    diagnostic.encoded_bytes = Some(template.wire_len as u64);
    diagnostic.target_turns_retained = observation.stage_retained;
    diagnostic.target_turns_removed = observation.stage_removed;
    diagnostic.target_steps_retained = selected_stage.steps_retained as u32;
    diagnostic.target_steps_removed = if initially_partial {
        None
    } else {
        selected_stage.steps_removed.map(|count| count as u32)
    };
    diagnostic.prior_turns_retained = observation.prior_retained;
    diagnostic.prior_turns_removed = observation.prior_removed;
    diagnostic.prior_steps_retained = selected_prior.steps_retained as u32;
    diagnostic.prior_steps_removed = selected_prior.steps_removed.map(|count| count as u32);
    diagnostic.replay_messages_retained = replay.retained as u32;
    diagnostic.replay_messages_omitted = replay.omitted.map(|count| count as u32);
    diagnostic.replay_envelopes_scanned = replay.scanned as u32;
    diagnostic.replay_envelopes_unscanned = replay.unscanned.map(|count| count as u32);
    diagnostic.replay_truncated = replay.truncated;
    diagnostic.replay_boundary_unavailable = replay.boundary_unavailable;
    diagnostic.replay_budget_omitted = replay.budget_omitted;
    diagnostic.target_partial = target_partial;
    observation.context_budget = Some(diagnostic);
    Ok(PreparedDecisionRequest {
        template,
        target_partial,
        target_category: None,
        bindings: Some(bindings),
        routing_observation: Some(observation),
    })
}

fn base_observation(
    authority: &RestBranchClassifierAuthorityV1,
    history: &AgentTurnHistorySnapshot,
    scope: &str,
) -> RoutingObservationV1 {
    let mut observation = RoutingObservationV1::not_dispatched(
        authority.native_protocol.as_ref().expect("C authority"),
        scope,
    );
    observation.provider = authority.decision_provider.clone();
    observation.model = authority.system_one_model.clone().unwrap_or_default();
    observation.target_segment = history
        .assessment_target
        .as_ref()
        .map(|t| t.segment_id.clone());
    observation
}

fn empty_diagnostic(
    context: &ResolvedDecisionContextV1,
    limiting: DecisionContextConstraintV1,
) -> DecisionContextObservationV1 {
    DecisionContextObservationV1 {
        schema: DecisionContextObservationSchemaV1::V1,
        context_digest: context.context_digest.clone(),
        request_source: context.request_source,
        question_source: context.per_question_source,
        request_window_tokens: context.request_window_tokens,
        question_window_tokens: context.per_question_window_tokens,
        request_usable_tokens: context.request_usable_tokens(),
        question_usable_tokens: context.per_question_usable_tokens(),
        estimator: context.accounting_algorithm.clone(),
        estimated_request_tokens: None,
        estimated_largest_question_tokens: None,
        encoded_bytes: None,
        target_turns_retained: 0,
        target_turns_removed: 0,
        target_steps_retained: 0,
        target_steps_removed: None,
        prior_turns_retained: 0,
        prior_turns_removed: 0,
        prior_steps_retained: 0,
        prior_steps_removed: None,
        replay_messages_retained: 0,
        replay_messages_omitted: None,
        replay_envelopes_scanned: 0,
        replay_envelopes_unscanned: None,
        replay_truncated: false,
        replay_boundary_unavailable: false,
        replay_budget_omitted: false,
        target_partial: false,
        limiting_constraint: limiting,
        no_fit_reason: None,
    }
}

pub(super) fn rejected_observation(
    authority: &RestBranchClassifierAuthorityV1,
    history: &AgentTurnHistorySnapshot,
    rejection: BudgetRejection,
) -> Option<RoutingObservationV1> {
    let context = authority.context_budget.as_ref()?;
    authority.native_protocol.as_ref()?;
    let scope = history
        .assessment_target
        .as_ref()
        .and_then(|t| t.evaluation_policy())
        .map_or("", |(scope, _)| scope);
    let mut observation = base_observation(authority, history, scope);
    let from = assessment_definition_result(history, authority)
        .ok()
        .map(|target| target.from);
    observation.prior_removed = from.unwrap_or(history.visible_conversation.len()) as u32;
    observation.stage_removed =
        from.map_or(0, |from| history.visible_conversation.len() - from) as u32;
    observation.eligibility = if from.is_some() {
        ProtectionEligibilityV1::PartialTarget
    } else {
        ProtectionEligibilityV1::NoTarget
    };
    observation.applicability_reason = ApplicabilityReasonV1::RejectedInput;
    let mut diagnostic = empty_diagnostic(context, rejection.limiting);
    diagnostic.no_fit_reason = Some(rejection.reason);
    diagnostic.estimated_request_tokens = rejection.request_bytes;
    diagnostic.estimated_largest_question_tokens = rejection.question_bytes;
    diagnostic.target_partial = from.is_some();
    diagnostic.target_turns_removed = observation.stage_removed;
    diagnostic.prior_turns_removed = observation.prior_removed;
    observation.context_budget = Some(diagnostic);
    Some(observation)
}
