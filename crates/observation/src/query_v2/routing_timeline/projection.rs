use super::*;
use std::sync::Arc;

fn held(request: &Request, code: LedgerReasonCodeV1) -> bool {
    request
        .decision
        .as_ref()
        .is_some_and(|d| d.reason_ledger.iter().any(|r| r.code == code))
}

pub(super) fn hydrate(requests: &mut BTreeMap<String, Request>, facts: &[Fact]) {
    let candidates: BTreeMap<_, _> = facts
        .iter()
        .filter_map(|fact| match &fact.value {
            ExecutionFactV1::CandidateDecision(candidate) => Some((
                (fact.request_id.as_str(), candidate.candidate_id.as_str()),
                candidate.as_ref(),
            )),
            _ => None,
        })
        .collect();
    let mut attempted = BTreeMap::new();
    for fact in facts {
        let Some(request) = requests.get_mut(&fact.request_id) else {
            continue;
        };
        match &fact.value {
            ExecutionFactV1::RouteDecision(decision) => {
                request.decision = Some((**decision).clone());
            }
            ExecutionFactV1::AttemptStarted {
                ordinal,
                candidate_id,
                model_configuration_id,
                profile_digest,
                request_model,
                credential_ref,
                key_id,
                ..
            } => {
                let reasoning = candidates
                    .get(&(fact.request_id.as_str(), candidate_id.as_str()))
                    .filter(|candidate| {
                        candidate.model_configuration_id == *model_configuration_id
                            && candidate.profile_digest == *profile_digest
                    })
                    .and_then(|candidate| candidate.reasoning_profile_id.clone());
                let identity = (model_configuration_id.clone(), reasoning);
                attempted.insert((fact.request_id.as_str(), *ordinal), identity.clone());
                request.candidate = Some(identity);
                request.models.insert(*ordinal, request_model.clone());
                request
                    .credentials
                    .insert(*ordinal, (credential_ref.clone(), key_id.clone()));
            }
            ExecutionFactV1::RequestFinished {
                accepted_attempt_ordinal,
                ..
            } => {
                request.view.native_model =
                    accepted_attempt_ordinal.and_then(|n| request.models.get(&n).cloned());
                if let Some(ordinal) = accepted_attempt_ordinal {
                    request.candidate = attempted
                        .get(&(fact.request_id.as_str(), *ordinal))
                        .cloned();
                }
            }
            _ => {}
        }
    }
    for request in requests.values_mut() {
        if request.view.native_model.is_none() {
            request.view.native_model = request
                .models
                .last_key_value()
                .map(|(_, model)| model.clone());
        }
    }
}

pub(super) fn project(
    session: &str,
    requests: &BTreeMap<String, Request>,
    facts: &[Fact],
    previous: Position,
    bases: &mut BTreeMap<String, Arc<Projected>>,
    old: &[Arc<Projected>],
) -> Result<Vec<Arc<Projected>>, ObservationV2Error> {
    let old: BTreeMap<_, _> = old.iter().map(|t| (t.view.turn_id.as_str(), t)).collect();
    let mut next_bases = BTreeMap::new();
    let mut ordered: Vec<_> = requests.values().collect();
    ordered.sort_by_key(|r| (r.first_fact, &r.view.request_id));
    let positions: BTreeMap<_, _> = ordered
        .iter()
        .enumerate()
        .map(|(i, r)| (r.view.request_id.as_str(), i))
        .collect();
    let mut by_time = ordered.clone();
    by_time.sort_by_key(|r| (r.view.started_at_ms, &r.view.request_id));
    let mut covered = std::collections::BTreeSet::new();
    let mut result = Vec::new();
    let mut ordinals = BTreeMap::new();
    let mut identities = BTreeMap::new();
    let mut finished = BTreeMap::new();
    for fact in facts {
        if let ExecutionFactV1::AgentTurnFinished { agent_turn_id, .. } = &fact.value {
            finished.insert(agent_turn_id, fact);
        }
    }
    for fact in finished.into_values() {
        let ExecutionFactV1::AgentTurnFinished {
            agent_turn_id,
            segment_id,
            ordinal,
            plan_id,
            plan_revision,
            selected_branch_id,
            executed_branch_id,
            model_configuration_id,
            branch_execution,
            started_at_ms,
            finished_at_ms,
            status,
            first_request_id,
            last_request_id,
            history_partial,
            ..
        } = &fact.value
        else {
            continue;
        };
        let Some((first, last)) = first_request_id
            .as_ref()
            .zip(last_request_id.as_ref())
            .and_then(|(first, last)| {
                positions
                    .get(first.as_str())
                    .zip(positions.get(last.as_str()))
            })
        else {
            continue;
        };
        if first > last {
            return Err(ObservationV2Error::Unavailable);
        }
        let opening = ordered[*first];
        let closing = ordered[*last];
        // Use the same endpoint/time rule as retained analysis. Timestamp ties
        // outside the recorded endpoints remain unassigned in both readers.
        let start = by_time.partition_point(|r| r.view.started_at_ms <= opening.view.started_at_ms);
        let end = by_time.partition_point(|r| r.view.started_at_ms < closing.view.started_at_ms);
        let mut members = if start < end {
            by_time[start..end].to_vec()
        } else {
            Vec::new()
        };
        members.push(opening);
        if opening.view.request_id != closing.view.request_id {
            members.push(closing);
        }
        members.sort_by_key(|r| (r.first_fact, &r.view.request_id));
        let members = members.as_slice();
        if members
            .iter()
            .any(|r| covered.contains(r.view.request_id.as_str()))
        {
            // Concurrent or conflicting ranges cannot manufacture a second assignment.
            return Err(ObservationV2Error::Unavailable);
        }
        covered.extend(members.iter().map(|r| r.view.request_id.as_str()));
        let mut position = Position {
            facts: fact.position,
            ..Default::default()
        };
        for r in members {
            position.include(r.position);
        }
        let key = &members[0].view.request_id;
        identities.insert(key.clone(), (*ordinal, fact.producer_epoch.clone()));
        if !position.newer_than(previous)
            && let Some(turn) = bases.get(key)
        {
            ordinals.insert(key.clone(), *ordinal);
            next_bases.insert(key.clone(), turn.clone());
            result.push(turn.clone());
            continue;
        }
        let mut turn = make_turn(members)?;
        turn.position.facts = turn.position.facts.max(fact.position);
        turn.view.agent_turn_id = Some(agent_turn_id.clone());
        turn.view.segment_id = Some(segment_id.clone());
        turn.view.started_at_ms =
            i64::try_from(*started_at_ms).map_err(|_| ObservationV2Error::Unavailable)?;
        turn.view.finished_at_ms =
            Some(i64::try_from(*finished_at_ms).map_err(|_| ObservationV2Error::Unavailable)?);
        turn.view.plan_id = Some(plan_id.as_str().to_owned());
        turn.view.plan_revision = Some(*plan_revision);
        turn.view.branch_id = executed_branch_id
            .clone()
            .or_else(|| Some(selected_branch_id.clone()));
        turn.view.model_configuration_id = model_configuration_id.clone();
        turn.view.group = branch_execution.as_ref().map(|e| e.group);
        turn.view.competence_floor_millis =
            branch_execution.as_ref().map(|e| e.policy.floor_millis);
        turn.view.status = serde_json::to_value(status)
            .map_err(|_| ObservationV2Error::Unavailable)?
            .as_str()
            .ok_or(ObservationV2Error::Unavailable)?
            .into();
        turn.view.evidence_partial |= *history_partial;
        ordinals.insert(turn.view.turn_id.clone(), *ordinal);
        let turn = Arc::new(turn);
        next_bases.insert(turn.view.turn_id.clone(), turn.clone());
        result.push(turn);
    }
    // Requests arrive before AgentTurnFinished. Keep their opening-request ID stable.
    let mut pending = Vec::new();
    for request in &ordered {
        if covered.contains(request.view.request_id.as_str()) {
            if !pending.is_empty() {
                result.push(base_turn(&pending, previous, bases, &mut next_bases)?);
                pending.clear();
            }
            continue;
        }
        if !pending.is_empty() && !held(request, LedgerReasonCodeV1::ContextHoldApplied) {
            result.push(base_turn(&pending, previous, bases, &mut next_bases)?);
            pending.clear();
        }
        pending.push(*request);
    }
    if !pending.is_empty() {
        result.push(base_turn(&pending, previous, bases, &mut next_bases)?);
    }
    result.sort_by(|a, b| {
        (a.view.started_at_ms, &a.view.turn_id).cmp(&(b.view.started_at_ms, &b.view.turn_id))
    });
    // Index assessment candidates by their segment instead of comparing every
    // assessment with every turn. Positions include cross-turn dependencies.
    let mut assessments: BTreeMap<&str, Vec<&Fact>> = BTreeMap::new();
    for fact in facts {
        if let ExecutionFactV1::BranchAssessmentRecorded { segment_id, .. } = &fact.value {
            assessments
                .entry(segment_id.as_str())
                .or_default()
                .push(fact);
        }
    }
    let mut protections: BTreeMap<&str, Vec<(String, Position)>> = BTreeMap::new();
    for turn in &result {
        if let Some(trigger) = turn
            .view
            .selection
            .as_ref()
            .and_then(|s| s.competence_trigger.as_ref())
        {
            protections
                .entry(trigger.segment_id.as_str())
                .or_default()
                .push((turn.view.turn_id.clone(), turn.position));
        }
    }
    let mut output = Vec::with_capacity(result.len());
    for (index, base) in result.iter().enumerate() {
        let mut dependency = base.position;
        if index > 0 {
            dependency.include(result[index - 1].position);
        }
        if index + 1 < result.len() {
            dependency.include(result[index + 1].position);
        }
        let candidates = base
            .view
            .segment_id
            .as_deref()
            .and_then(|s| assessments.get(s));
        let score = candidates
            .into_iter()
            .flatten()
            .filter(|fact| {
                let ExecutionFactV1::BranchAssessmentRecorded {
                    plan_id,
                    plan_revision,
                    model_configuration_id,
                    target_from_ordinal,
                    target_through_ordinal,
                    ..
                } = &fact.value
                else {
                    return false;
                };
                base.view.plan_id.as_deref() == Some(plan_id.as_str())
                    && base.view.plan_revision == Some(*plan_revision)
                    && base.view.model_configuration_id.as_ref() == Some(model_configuration_id)
                    && ordinals
                        .get(&base.view.turn_id)
                        .is_some_and(|n| n >= target_from_ordinal && n <= target_through_ordinal)
            })
            .max_by_key(|fact| match &fact.value {
                ExecutionFactV1::BranchAssessmentRecorded { assessed_at_ms, .. } => {
                    (*assessed_at_ms, fact.position)
                }
                _ => unreachable!(),
            });
        if let Some(fact) = score {
            dependency.facts = dependency.facts.max(fact.position);
            if let ExecutionFactV1::BranchAssessmentRecorded {
                trigger_request_id, ..
            } = &fact.value
                && let Some(trigger) = requests.get(trigger_request_id.as_str())
            {
                dependency.include(trigger.position);
            }
        }
        let protected = base
            .view
            .segment_id
            .as_deref()
            .and_then(|s| protections.get(s));
        for (_, position) in protected.into_iter().flatten() {
            dependency.include(*position);
        }
        if !dependency.newer_than(previous)
            && let Some(turn) = old.get(base.view.turn_id.as_str())
        {
            output.push((*turn).clone());
            continue;
        }
        let mut turn = (**base).clone();
        turn.position = dependency;
        let first = &requests[&turn.view.turn_id];
        let fresh_decision = first
            .decision
            .as_ref()
            .and_then(|d| d.complexity.as_ref())
            .is_some_and(|s| s.decision_source != ComplexityDecisionSourceV1::Inherited);
        let continuing = held(first, LedgerReasonCodeV1::ContextHoldApplied)
            || held(first, LedgerReasonCodeV1::ContextHoldInvalidated);
        // A retained tail with a later recorded ordinal is not the session's first turn.
        let initial = index == 0 && ordinals.get(&turn.view.turn_id).is_none_or(|n| *n == 1);
        turn.view.boundary = if initial || (fresh_decision && continuing) {
            RoutingTurnBoundaryV1::NewUserMessage
        } else {
            RoutingTurnBoundaryV1::Unknown
        };
        turn.view.assessment_state = if index + 1 < result.len()
            || matches!(turn.view.status.as_str(), "failed" | "interrupted")
        {
            "unrated"
        } else {
            "pending"
        }
        .into();
        if let Some(fact) = score {
            let ExecutionFactV1::BranchAssessmentRecorded {
                trigger_request_id,
                target_from_turn_id,
                target_through_turn_id,
                target_from_ordinal,
                target_through_ordinal,
                assessed_at_ms,
                score,
                partial,
                ..
            } = &fact.value
            else {
                unreachable!()
            };
            turn.view.assessment_state = if *partial { "partial" } else { "scored" }.into();
            turn.view.assessment = Some(PlanQualityAssessment {
                event_id: fact.event_id.clone(),
                trigger_request_id: trigger_request_id.to_string(),
                assessed_at_ms: *assessed_at_ms as i64,
                target_from_turn_id: target_from_turn_id.to_string(),
                target_through_turn_id: target_through_turn_id.to_string(),
                target_from_ordinal: *target_from_ordinal,
                target_through_ordinal: *target_through_ordinal,
                score: *score,
                partial: *partial,
                reason: None,
                evidence_available: requests.contains_key(trigger_request_id.as_str()),
            });
        }
        turn.view.protected_turn_ids = protected
            .into_iter()
            .flatten()
            .map(|(id, _)| id.clone())
            .collect();
        turn.view.transitions = transitions::classify(
            session,
            &turn.view,
            identities.get(&turn.view.turn_id),
            index
                .checked_sub(1)
                .map(|i| (&result[i].view, identities.get(&result[i].view.turn_id))),
            requests,
        );
        turn.view.content_digest = CanonicalDigest::of(&turn.view)
            .map_err(|_| ObservationV2Error::Unavailable)?
            .to_string();
        output.push(Arc::new(turn));
    }
    *bases = next_bases;
    Ok(output)
}

fn base_turn(
    members: &[&Request],
    previous: Position,
    old: &BTreeMap<String, Arc<Projected>>,
    next: &mut BTreeMap<String, Arc<Projected>>,
) -> Result<Arc<Projected>, ObservationV2Error> {
    let key = &members[0].view.request_id;
    let mut position = Position::default();
    for r in members {
        position.include(r.position);
    }
    let turn = if !position.newer_than(previous)
        && let Some(turn) = old.get(key)
    {
        turn.clone()
    } else {
        Arc::new(make_turn(members)?)
    };
    next.insert(key.clone(), turn.clone());
    Ok(turn)
}

fn make_turn(members: &[&Request]) -> Result<Projected, ObservationV2Error> {
    let first = members.first().ok_or(ObservationV2Error::Unavailable)?;
    let mut position = Position::default();
    type CostIdentity = (Option<String>, Option<String>, Option<String>);
    let mut costs: BTreeMap<CostIdentity, RoutingTimelineCostV1> = BTreeMap::new();
    let mut relays = Vec::new();
    for member in members {
        position.include(member.position);
        let models: Vec<_> = member.models.values().collect();
        for pair in models.windows(2).filter(|p| p[0] != p[1]) {
            relays.push(RoutingTimelineRelayV1 {
                request_id: member.view.request_id.clone(),
                from_model: Some(pair[0].clone()),
                to_model: Some(pair[1].clone()),
            });
        }
        for cost in &member.view.costs {
            let key = (
                cost.model_configuration_id.clone(),
                cost.currency.clone(),
                cost.valuation_kind.clone(),
            );
            match costs.entry(key) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(cost.clone());
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let value = entry.get_mut();
                    value.unknown_contributions = value
                        .unknown_contributions
                        .checked_add(cost.unknown_contributions)
                        .ok_or(ObservationV2Error::ScopeTooLarge)?;
                    value.known_micros = match (value.known_micros, cost.known_micros) {
                        (Some(a), Some(b)) => {
                            Some(a.checked_add(b).ok_or(ObservationV2Error::ScopeTooLarge)?)
                        }
                        (a, b) => a.or(b),
                    };
                }
            }
        }
    }
    let decision = first.decision.as_ref();
    let selection = decision.and_then(|d| d.complexity.clone());
    Ok(Projected {
        position,
        view: RoutingTimelineTurnV1 {
            turn_id: first.view.request_id.clone(),
            agent_turn_id: None,
            segment_id: None,
            started_at_ms: first.view.started_at_ms,
            finished_at_ms: None,
            boundary: RoutingTurnBoundaryV1::Unknown,
            plan_id: decision.and_then(|d| d.plan_id.as_ref().map(|id| id.as_str().to_owned())),
            plan_revision: decision.and_then(|d| d.route.plan_revision()),
            branch_id: selection.as_ref().map(|s| s.branch_id.clone()),
            model_configuration_id: first.candidate.as_ref().map(|c| c.0.clone()),
            reasoning_profile_id: first.candidate.as_ref().and_then(|c| c.1.clone()),
            group: selection.as_ref().map(|s| s.execution_group),
            competence_floor_millis: selection
                .as_ref()
                .and_then(|s| s.policy.as_ref().map(|p| p.floor_millis)),
            selection,
            tool_continuations: members
                .iter()
                .filter(|r| held(r, LedgerReasonCodeV1::ContextHoldApplied))
                .count() as u64,
            relays,
            transitions: Vec::new(),
            assessment_state: "pending".into(),
            assessment: None,
            protected_turn_ids: Vec::new(),
            status: "running".into(),
            requests: members.iter().map(|r| r.view.clone()).collect(),
            cost_by_model: costs.into_values().collect(),
            evidence_partial: members.iter().any(|r| r.partial),
            content_digest: String::new(),
        },
    })
}
