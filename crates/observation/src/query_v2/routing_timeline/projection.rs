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
                request.identities.insert((
                    model_configuration_id.clone(),
                    profile_digest.as_str().to_owned(),
                ));
                request.models.insert(*ordinal, request_model.clone());
                request
                    .credentials
                    .insert(*ordinal, (credential_ref.clone(), key_id.clone()));
            }
            ExecutionFactV1::RequestFinished {
                accepted_attempt_ordinal,
                attempts_started,
                facts_completeness,
                ..
            } => {
                request.request_finished = true;
                request.identity_gap = *facts_completeness != FactsCompleteness::Complete
                    || usize::try_from(*attempts_started)
                        .ok()
                        .is_none_or(|n| n != request.models.len())
                    || accepted_attempt_ordinal.is_some_and(|n| !request.models.contains_key(&n));
                request.view.native_model =
                    accepted_attempt_ordinal.and_then(|n| request.models.get(&n).cloned());
                if let Some(ordinal) = accepted_attempt_ordinal {
                    request.candidate = attempted
                        .get(&(fact.request_id.as_str(), *ordinal))
                        .cloned();
                }
            }
            ExecutionFactV1::AgentTurnFinished { .. } => request.stage_finished = true,
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
    include_presentation: bool,
) -> Result<Vec<Arc<Projected>>, ObservationV2Error> {
    let old: BTreeMap<_, _> = old.iter().map(|t| (t.view.turn_id.as_str(), t)).collect();
    let mut next_bases = BTreeMap::new();
    // Keep maintenance request receipts and accounting intact, but do not
    // manufacture pending user turns for known title/compaction calls. Missing
    // historical purpose evidence never licenses hiding a request.
    let mut ordered: Vec<_> = requests
        .values()
        .filter(|request| {
            !request
                .decision
                .as_ref()
                .is_some_and(|d| d.native_auxiliary)
        })
        .collect();
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
    let mut opening_causes = BTreeMap::new();
    let mut identities = BTreeMap::new();
    let mut finished = BTreeMap::<_, (u64, &Fact)>::new();
    let admissions = admission::collect(facts)?;
    let mut admitted_stages = BTreeMap::<_, Vec<_>>::new();
    for (id, admission) in &admissions {
        admitted_stages
            .entry((admission.epoch, admission.value.agent_turn_id.as_str()))
            .or_default()
            .push((*id, *admission));
    }
    for fact in facts {
        if let ExecutionFactV1::AgentTurnFinished {
            agent_turn_id,
            history_membership,
            ..
        } = &fact.value
        {
            let revision = history_membership.as_ref().map_or(0, |m| m.revision);
            let entry = finished
                .entry((&fact.producer_epoch, agent_turn_id))
                .or_insert((revision, fact));
            if revision >= entry.0 {
                *entry = (revision, fact);
            }
        }
    }
    let finished_stages: BTreeMap<_, _> = finished
        .iter()
        .map(|(&(epoch, id), (_, fact))| ((epoch.as_str(), id.as_str()), fact.position))
        .collect();
    for (_, fact) in finished.into_values() {
        let ExecutionFactV1::AgentTurnFinished {
            agent_turn_id,
            history_membership,
            opening_cause,
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
            attribution,
            ..
        } = &fact.value
        else {
            continue;
        };
        // A previously finished stage may admit another actual continuation.
        // Its old frozen membership and score remain immutable, but cannot
        // close or assess the newly admitted request.
        let admitted = admitted_stages
            .get(&(fact.producer_epoch.as_str(), agent_turn_id.as_str()))
            .map(Vec::as_slice)
            .unwrap_or_default();
        if !admitted.is_empty() {
            let Some(membership) = history_membership else {
                return Err(ObservationV2Error::Unavailable);
            };
            if admitted.iter().any(|(_, a)| {
                a.value.branch_id != membership.branch_id
                    || a.value.ordinal != *ordinal
                    || opening_cause.is_some_and(|cause| cause != a.value.opening_cause)
            }) {
                return Err(ObservationV2Error::Unavailable);
            }
            if admitted
                .iter()
                .any(|(_, a)| a.value.revision > membership.revision)
            {
                continue;
            }
        }
        let mut members = if let Some(membership) = history_membership {
            membership
                .request_ids
                .iter()
                .filter_map(|id| requests.get(id))
                .collect::<Vec<_>>()
        } else {
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
            let start =
                by_time.partition_point(|r| r.view.started_at_ms <= opening.view.started_at_ms);
            let end =
                by_time.partition_point(|r| r.view.started_at_ms < closing.view.started_at_ms);
            let mut legacy_members = if start < end {
                by_time[start..end].to_vec()
            } else {
                Vec::new()
            };
            legacy_members.push(opening);
            if opening.view.request_id != closing.view.request_id {
                legacy_members.push(closing);
            }
            legacy_members
        };
        if members.iter().any(|r| {
            admissions.get(r.view.request_id.as_str()).is_some_and(|a| {
                a.epoch != fact.producer_epoch || a.value.agent_turn_id != *agent_turn_id
            })
        }) {
            return Err(ObservationV2Error::Unavailable);
        }
        let missing_members = history_membership
            .as_ref()
            .is_some_and(|m| m.clipped || m.request_ids.len() != members.len());
        // Admission can preserve exact identity even when bounded membership
        // storage clipped. This improves display only; frozen score membership
        // below still has to cover every displayed request.
        for (id, _) in admitted {
            if let Some(request) = requests.get(*id)
                && !members.iter().any(|r| r.view.request_id == *id)
            {
                if history_membership.as_ref().is_none_or(|m| !m.clipped) {
                    return Err(ObservationV2Error::Unavailable);
                }
                members.push(request);
            }
        }
        if members.is_empty() {
            continue;
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
        if let Some(cause) = opening_cause {
            opening_causes.insert(key.clone(), *cause);
        }
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
        turn.view.competence_floor_millis = branch_execution
            .as_ref()
            .and_then(|e| e.policy.floor_millis);
        turn.view.status = serde_json::to_value(status)
            .map_err(|_| ObservationV2Error::Unavailable)?
            .as_str()
            .ok_or(ObservationV2Error::Unavailable)?
            .into();
        turn.view.evidence_partial |= *history_partial || missing_members;
        turn.view.presentation = Some(presentation(
            members,
            turn.view.group,
            Some(*attribution),
            false,
            !missing_members,
        ));
        ordinals.insert(turn.view.turn_id.clone(), *ordinal);
        let turn = Arc::new(turn);
        next_bases.insert(turn.view.turn_id.clone(), turn.clone());
        result.push(turn);
    }
    for pending in admission::pending(&ordered, &covered, &admissions) {
        let mut turn = base_turn(&pending, previous, bases, &mut next_bases)?;
        let opening_admission = admissions.get(pending[0].view.request_id.as_str());
        if include_presentation
            && let Some(position) = opening_admission.and_then(|admission| {
                finished_stages.get(&(admission.epoch, admission.value.agent_turn_id.as_str()))
            })
            && *position > turn.position.facts
        {
            Arc::make_mut(&mut turn).position.facts = *position;
        }
        if turn.position.newer_than(previous) || turn.view.presentation.is_none() {
            let view = &mut Arc::make_mut(&mut turn).view;
            let unclosed_opening = opening_admission.is_some_and(|admission| {
                !finished_stages
                    .contains_key(&(admission.epoch, admission.value.agent_turn_id.as_str()))
            });
            view.presentation = Some(presentation(
                &pending,
                view.group,
                None,
                unclosed_opening,
                pending
                    .iter()
                    .all(|request| admissions.contains_key(request.view.request_id.as_str())),
            ));
        }
        if let Some(admission) = opening_admission {
            let id = turn.view.turn_id.clone();
            if turn.view.agent_turn_id.as_ref() != Some(&admission.value.agent_turn_id) {
                let view = &mut Arc::make_mut(&mut turn).view;
                view.agent_turn_id = Some(admission.value.agent_turn_id.clone());
            }
            ordinals.insert(id.clone(), admission.value.ordinal);
            opening_causes.insert(id.clone(), admission.value.opening_cause);
            identities.insert(
                id.clone(),
                (admission.value.ordinal, admission.epoch.to_owned()),
            );
        }
        next_bases.insert(turn.view.turn_id.clone(), turn.clone());
        result.push(turn);
    }
    result.sort_by(|a, b| {
        (a.view.started_at_ms, &a.view.turn_id).cmp(&(b.view.started_at_ms, &b.view.turn_id))
    });
    // Index assessment candidates by their segment instead of comparing every
    // assessment with every turn. Positions include cross-turn dependencies.
    let mut assessments: BTreeMap<&str, Vec<&Fact>> = BTreeMap::new();
    for fact in facts {
        if let Some(assessment) = fact.value.assessment_record() {
            assessments
                .entry(assessment.segment_id)
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
                let Some(assessment) = fact.value.assessment_record() else {
                    return false;
                };
                let hiroute_domain::AssessmentRecordRef {
                    plan_id,
                    plan_revision,
                    history_membership,
                    model_configuration_id,
                    target_from_ordinal,
                    target_through_ordinal,
                    ..
                } = assessment;
                base.view.plan_id.as_deref() == Some(plan_id.as_str())
                    && base.view.plan_revision == Some(plan_revision)
                    && base.view.model_configuration_id.as_deref() == model_configuration_id
                    && history_membership.map_or_else(
                        || {
                            !base
                                .view
                                .requests
                                .iter()
                                .any(|r| admissions.contains_key(r.request_id.as_str()))
                                && ordinals.get(&base.view.turn_id).is_some_and(|n| {
                                    *n >= target_from_ordinal && *n <= target_through_ordinal
                                })
                        },
                        |membership| {
                            !membership.clipped
                                && base
                                    .view
                                    .requests
                                    .iter()
                                    .all(|r| membership.request_ids.contains(&r.request_id))
                        },
                    )
            })
            .max_by_key(|fact| {
                (
                    fact.value
                        .assessment_record()
                        .expect("assessment candidate")
                        .assessed_at_ms,
                    fact.position,
                )
            });
        if let Some(fact) = score {
            dependency.facts = dependency.facts.max(fact.position);
            let assessment = fact
                .value
                .assessment_record()
                .expect("assessment candidate");
            if let Some(trigger) = requests.get(assessment.trigger_request_id.as_str()) {
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
        turn.view.boundary = match opening_causes.get(&turn.view.turn_id) {
            Some(AgentTurnOpeningCauseV1::UserMessage) => RoutingTurnBoundaryV1::NewUserMessage,
            Some(AgentTurnOpeningCauseV1::HistoryShortened) => {
                RoutingTurnBoundaryV1::ContextRebuilt
            }
            Some(AgentTurnOpeningCauseV1::NoComparableHistory) if initial => {
                RoutingTurnBoundaryV1::NewUserMessage
            }
            Some(_) => RoutingTurnBoundaryV1::Unknown,
            None if initial || (fresh_decision && continuing) => {
                RoutingTurnBoundaryV1::NewUserMessage
            }
            None => RoutingTurnBoundaryV1::Unknown,
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
            let assessment = fact
                .value
                .assessment_record()
                .expect("assessment candidate");
            let hiroute_domain::AssessmentRecordRef {
                trigger_request_id,
                target_from_turn_id,
                target_through_turn_id,
                target_from_ordinal,
                target_through_ordinal,
                assessed_at_ms,
                score,
                partial,
                ..
            } = assessment;
            turn.view.assessment_state = if partial { "partial" } else { "scored" }.into();
            turn.view.assessment = Some(PlanQualityAssessment {
                event_id: fact.event_id.clone(),
                trigger_request_id: trigger_request_id.to_string(),
                assessed_at_ms: assessed_at_ms as i64,
                target_from_turn_id: target_from_turn_id.to_string(),
                target_through_turn_id: target_through_turn_id.to_string(),
                target_from_ordinal,
                target_through_ordinal,
                score,
                partial,
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
        let mut digest_view = turn.view.clone();
        if !include_presentation {
            digest_view.presentation = None;
        }
        turn.view.content_digest = CanonicalDigest::of(&digest_view)
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
    let selection = decision
        .and_then(|d| d.complexity.clone())
        .map(RoutingTimelineSelectionV1::from);
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
                .and_then(|s| s.policy.as_ref().and_then(|p| p.floor_millis)),
            selection,
            tool_continuations: members
                .iter()
                .filter(|r| {
                    // Model affinity also applies to freshly classified user messages.
                    // Only the recorded inheritance reason proves a tool continuation.
                    r.decision
                        .as_ref()
                        .and_then(|d| d.complexity.as_ref())
                        .is_some_and(|d| {
                            d.reason_codes
                                .contains(&ComplexityReasonCodeV1::InheritedToolContinuation)
                        })
                })
                .count() as u64,
            relays,
            transitions: Vec::new(),
            assessment_state: "pending".into(),
            assessment: None,
            protected_turn_ids: Vec::new(),
            // Request completion does not prove stage completion. Missing a
            // stage-closing fact must not manufacture perpetual activity either.
            status: if members.iter().any(|r| r.view.outcome.is_none()) {
                "running"
            } else {
                match members.last().and_then(|r| r.view.outcome.as_deref()) {
                    Some("cancelled" | "postcommit_transport_failed") => "interrupted",
                    Some("failed") => "failed",
                    _ => "unknown",
                }
            }
            .into(),
            requests: members.iter().map(|r| r.view.clone()).collect(),
            cost_by_model: costs.into_values().collect(),
            evidence_partial: members.iter().any(|r| r.partial),
            presentation: None,
            content_digest: String::new(),
        },
    })
}

fn presentation(
    members: &[&Request],
    group: Option<ExecutionGroupV1>,
    finished_attribution: Option<AgentTurnAttributionV1>,
    admitted_opening: bool,
    membership_known: bool,
) -> RoutingTimelinePresentationV1 {
    let identities: std::collections::BTreeSet<_> = members
        .iter()
        .flat_map(|request| request.identities.iter())
        .collect();
    let attribution = finished_attribution.unwrap_or_else(|| {
        if identities.len() >= 2 {
            AgentTurnAttributionV1::Mixed
        } else if identities.len() == 1
            && membership_known
            && members.iter().all(|r| !r.partial && !r.identity_gap)
            && identities
                .iter()
                .all(|identity| identity.1.as_str() != "unknown")
        {
            AgentTurnAttributionV1::Single
        } else {
            AgentTurnAttributionV1::Unknown
        }
    });
    let pending = admitted_opening
        && members
            .first()
            .is_some_and(|opening| opening.decision.is_none() && opening.view.outcome.is_none())
        && members.iter().all(|r| {
            !r.partial && r.identities.is_empty() && !r.request_finished && !r.stage_finished
        });
    let group_state = if group.is_some() {
        RoutingTimelineGroupStateV1::Resolved
    } else if attribution == AgentTurnAttributionV1::Mixed {
        RoutingTimelineGroupStateV1::Mixed
    } else if pending {
        RoutingTimelineGroupStateV1::Pending
    } else {
        RoutingTimelineGroupStateV1::Unavailable
    };
    RoutingTimelinePresentationV1 {
        group_state,
        execution_attribution: attribution,
        request_models: members
            .iter()
            .map(|r| RoutingTimelineRequestModelV1 {
                request_id: r.view.request_id.clone(),
                model_configuration_id: r.candidate.as_ref().map(|candidate| candidate.0.clone()),
            })
            .collect(),
    }
}
