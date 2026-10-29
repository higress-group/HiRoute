use std::collections::{BTreeMap, BTreeSet};

use hiroute_domain::{
    AttemptOutcomeV1, CandidateDecisionFactV1, ExecutionFactEnvelopeV1, ExecutionFactV1,
};

use super::FactProjectionError;

pub(super) fn ordered_facts(
    facts: &[ExecutionFactEnvelopeV1],
    require_complete: bool,
) -> Result<(), FactProjectionError> {
    let first = facts
        .first()
        .ok_or(FactProjectionError::MissingPrerequisite)?;
    let mut previous_sequence = 0;
    let mut event_ids = BTreeSet::new();
    let mut route_seen = false;
    let mut input_selection_seen = false;
    let mut candidates: BTreeMap<String, &CandidateDecisionFactV1> = BTreeMap::new();
    let mut attempts = BTreeMap::new();
    let mut finished_attempts = BTreeMap::new();
    let mut value_seen = false;
    let mut request_finish = None;
    let mut missing_candidate_reference = false;
    let mut missing_attempt_reference = false;

    for envelope in facts {
        let after_request_finish = request_finish.is_some();
        if envelope.producer != first.producer
            || envelope.correlation != first.correlation
            || envelope.trust != first.trust
            || envelope.sequence <= previous_sequence
            || !event_ids.insert(&envelope.event_id)
            || after_request_finish
                && !matches!(envelope.fact, ExecutionFactV1::UsageAndCache { .. })
        {
            return Err(FactProjectionError::ImmutableConflict);
        }
        previous_sequence = envelope.sequence;
        match &envelope.fact {
            ExecutionFactV1::RequestInputSelection(_) => {
                if input_selection_seen {
                    return Err(FactProjectionError::ImmutableConflict);
                }
                input_selection_seen = true;
            }
            ExecutionFactV1::RouteDecision(route) => {
                if route_seen
                    || route.plan_id != envelope.trust.agent_plan_id
                    || route.route != envelope.trust.route
                    || route.requirements.ingress_protocol != envelope.trust.ingress_protocol
                {
                    return Err(FactProjectionError::ImmutableConflict);
                }
                route_seen = true;
            }
            ExecutionFactV1::CandidateDecision(candidate) => {
                if candidates
                    .insert(candidate.candidate_id.clone(), candidate.as_ref())
                    .is_some()
                {
                    return Err(FactProjectionError::ImmutableConflict);
                }
            }
            ExecutionFactV1::CredentialLease { .. }
            | ExecutionFactV1::RuntimeState { .. }
            | ExecutionFactV1::AgentTurnAdmitted(_)
            | ExecutionFactV1::AgentTurnFinished { .. }
            | ExecutionFactV1::StageAssessmentRecorded(_)
            | ExecutionFactV1::BranchAssessmentRecorded { .. } => {}
            ExecutionFactV1::AttemptStarted {
                ordinal,
                candidate_id,
                stable_binding_id,
                profile_digest,
                provider_name,
                request_model,
                upstream_protocol,
                model_configuration_id,
                adapter_revision,
                previous_attempt_id,
                ..
            } => {
                let attempt_id = envelope
                    .attempt_id
                    .as_ref()
                    .ok_or(FactProjectionError::Invalid)?;
                let candidate_mismatch = candidates.get(candidate_id).is_some_and(|candidate| {
                    candidate.stable_binding_id.as_str() != stable_binding_id.as_str()
                        || candidate.profile_digest != *profile_digest
                        || candidate.provider_id.as_str() != provider_name.as_str()
                        || candidate.native_model.as_str() != request_model.as_str()
                        || candidate.upstream_protocol != *upstream_protocol
                        || candidate.model_configuration_id.as_str()
                            != model_configuration_id.as_str()
                        || candidate.adapter_revision.as_str() != adapter_revision.as_str()
                });
                missing_candidate_reference |= !candidates.contains_key(candidate_id);
                if candidate_mismatch
                    || attempts
                        .values()
                        .any(|attempt: &AttemptStart<'_>| attempt.ordinal == *ordinal)
                    || attempts.contains_key(attempt_id)
                    || previous_attempt_id.as_ref().is_some_and(|previous| {
                        previous == attempt_id
                            || attempts
                                .get(previous)
                                .is_some_and(|attempt| attempt.ordinal >= *ordinal)
                    })
                {
                    return Err(FactProjectionError::ImmutableConflict);
                }
                attempts.insert(
                    attempt_id,
                    AttemptStart {
                        ordinal: *ordinal,
                        stable_binding_id,
                    },
                );
            }
            ExecutionFactV1::AttemptFinished(finished) => {
                let attempt_id = envelope
                    .attempt_id
                    .as_ref()
                    .ok_or(FactProjectionError::Invalid)?;
                missing_attempt_reference |= !attempts.contains_key(attempt_id);
                if attempts.get(attempt_id).is_some_and(|started| {
                    started.ordinal != finished.ordinal
                        || started.stable_binding_id != &finished.stable_binding_id
                }) || finished_attempts
                    .insert(attempt_id, finished.outcome)
                    .is_some()
                {
                    return Err(FactProjectionError::ImmutableConflict);
                }
            }
            ExecutionFactV1::SemanticCommit { ordinal, .. }
            | ExecutionFactV1::UsageAndCache { ordinal, .. } => {
                let attempt_id = envelope
                    .attempt_id
                    .as_ref()
                    .ok_or(FactProjectionError::Invalid)?;
                missing_attempt_reference |= !attempts.contains_key(attempt_id);
                if attempts
                    .get(attempt_id)
                    .is_some_and(|attempt| attempt.ordinal != *ordinal)
                {
                    return Err(FactProjectionError::ImmutableConflict);
                }
            }
            ExecutionFactV1::ValueSnapshot { value, .. } => {
                if value_seen || Some(&value.agent_plan_id) != envelope.trust.agent_plan_id.as_ref()
                {
                    return Err(FactProjectionError::ImmutableConflict);
                }
                value_seen = true;
            }
            ExecutionFactV1::RequestFinished {
                attempts_started,
                attempts_finished,
                accepted_attempt_ordinal,
                ..
            } => {
                if usize::try_from(*attempts_started).map_or(true, |count| count < attempts.len())
                    || usize::try_from(*attempts_finished)
                        .map_or(true, |count| count < finished_attempts.len())
                    || accepted_attempt_ordinal.is_some_and(|ordinal| {
                        attempts.iter().any(|(attempt_id, attempt)| {
                            attempt.ordinal == ordinal
                                && finished_attempts
                                    .get(attempt_id)
                                    .is_some_and(|outcome| !was_transport_accepted(*outcome))
                        })
                    })
                {
                    return Err(FactProjectionError::ImmutableConflict);
                }
                request_finish = Some((
                    *attempts_started,
                    *attempts_finished,
                    *accepted_attempt_ordinal,
                ));
            }
        }
    }

    if require_complete {
        let (attempts_started, attempts_finished, accepted_ordinal) =
            request_finish.ok_or(FactProjectionError::MissingPrerequisite)?;
        let accepted_attempt = accepted_ordinal.and_then(|ordinal| {
            attempts.iter().find_map(|(attempt_id, attempt)| {
                (attempt.ordinal == ordinal).then_some(*attempt_id)
            })
        });
        if !route_seen
            || missing_candidate_reference
            || missing_attempt_reference
            || usize::try_from(attempts_started).ok() != Some(attempts.len())
            || usize::try_from(attempts_finished).ok() != Some(finished_attempts.len())
            || accepted_ordinal.is_some() != accepted_attempt.is_some()
            || accepted_attempt.is_some_and(|attempt_id| {
                finished_attempts
                    .get(attempt_id)
                    .is_none_or(|outcome| !was_transport_accepted(*outcome))
            })
        {
            return Err(FactProjectionError::MissingPrerequisite);
        }
    }
    Ok(())
}

fn was_transport_accepted(outcome: AttemptOutcomeV1) -> bool {
    // Transport acceptance survives a later downstream failure or cancellation.
    matches!(
        outcome,
        AttemptOutcomeV1::Accepted
            | AttemptOutcomeV1::PostcommitTransportFailed
            | AttemptOutcomeV1::PostcommitCancelled
    )
}

struct AttemptStart<'a> {
    ordinal: u32,
    stable_binding_id: &'a String,
}
