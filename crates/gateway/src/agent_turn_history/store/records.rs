//! Sealed execution revisions, snapshots and their memory accounting.
use super::*;

pub(super) fn active_is_empty(active: &ActiveTurn) -> bool {
    active.first_request_id.is_none()
        && active.last_request_id.is_none()
        && active.steps.is_empty()
        && active.executions.is_empty()
}

pub(super) fn trim_oldest_closed_turn(
    inner: &mut StoreInner,
    key: &AgentTurnHistoryKey,
) -> Result<bool, AgentTurnHistoryError> {
    let changed = {
        let entry = inner
            .entries
            .get_mut(key)
            .ok_or(AgentTurnHistoryError::Resource)?;
        if entry.closed.is_empty() {
            None
        } else {
            entry.history_partial = true;
            if let Some(removed) = entry.closed.front()
                && let Some(segment) = entry.current_segment.as_mut()
                && removed.completed.segment_id == segment.segment_id
            {
                segment.partial = true;
            }
            if let Some(removed) = entry.closed.front()
                && let Some(segment) = entry.referenced_segment.as_mut()
                && removed.completed.segment_id == segment.segment_id
            {
                segment.partial = true;
            }
            entry.closed.pop_front();
            let old = entry.accounted_bytes;
            entry.accounted_bytes = entry_bytes(key, entry);
            Some((old, entry.accounted_bytes))
        }
    };
    let Some((old, new)) = changed else {
        return Ok(false);
    };
    inner.replace_accounting(old, new);
    Ok(true)
}

pub(super) fn matching_entry_mut<'a>(
    inner: &'a mut StoreInner,
    ticket: &AgentTurnTicket,
) -> Result<&'a mut SessionEntry, AgentTurnHistoryError> {
    let entry = inner
        .entries
        .get_mut(&ticket.key)
        .ok_or(AgentTurnHistoryError::TurnContextUnavailable)?;
    if entry.entry_token != ticket.entry_token
        || entry.revision != ticket.revision
        || entry.pending_request != Some(ticket.request_token)
        || entry.active.agent_turn_id != ticket.agent_turn_id
    {
        return Err(AgentTurnHistoryError::TurnContextUnavailable);
    }
    Ok(entry)
}

/// Freezing immutable execution membership does not itself prove a stage ended.
/// Keep that proof separate from whether all accepted content was captured.
pub(super) enum StageClosure {
    RequestTerminal,
    NewActivation,
    ConfigurationChanged,
    AssociationLost,
}

impl StageClosure {
    fn proven(&self, status: AgentTurnStatus) -> bool {
        match self {
            Self::RequestTerminal => status != AgentTurnStatus::Unknown,
            Self::NewActivation | Self::ConfigurationChanged => true,
            Self::AssociationLost => false,
        }
    }
}

fn segment_has_unproven_closure(entry: &SessionEntry, segment: &SegmentState) -> bool {
    entry
        .closed
        .iter()
        .any(|turn| turn.completed.segment_id == segment.segment_id && !turn.closure_proven)
}

pub(super) fn close_active(
    entry: &mut SessionEntry,
    next_segment_id: String,
    finished_at_ms: u64,
    closure: StageClosure,
) -> Result<CompletedAgentTurn, AgentTurnHistoryError> {
    if entry.active.finalized
        && entry
            .closed
            .back()
            .is_some_and(|turn| turn.completed.agent_turn_id == entry.active.agent_turn_id)
    {
        entry.closed.pop_back();
    }
    let decision = entry
        .active
        .decision
        .as_ref()
        .ok_or(AgentTurnHistoryError::TurnContextUnavailable)?;
    let selected_branch_id = decision.branch_id.clone();
    let attribution = execution_attribution(&entry.active.executions, &selected_branch_id);
    let key = super::evaluation::segment_key(&entry.active, &attribution);
    let evaluation_scope = key.as_ref().and_then(|key| key.evaluation_scope.clone());
    let (segment_id, segment_partial) = if let Some(segment_key) = key {
        let reusable = entry.current_segment.as_ref().is_some_and(|segment| {
            segment.key == segment_key
                // A composite assessment covers this task only. It must not
                // combine different tasks merely because they share a rubric.
                && (evaluation_scope.is_none()
                    || segment.through_turn_id == entry.active.agent_turn_id)
                && !((segment.partial || segment_has_unproven_closure(entry, segment))
                    && !entry.active.capture_partial
                    && entry.active.opening_cause
                        == hiroute_domain::AgentTurnOpeningCauseV1::UserMessage
                    && segment.through_turn_id != entry.active.agent_turn_id)
        });
        if reusable {
            let segment = entry.current_segment.as_mut().expect("checked");
            segment.through_turn_id = entry.active.agent_turn_id.clone();
            segment.through_ordinal = entry.active.ordinal;
            segment.partial |= entry.active.capture_partial;
            (segment.segment_id.clone(), segment.partial)
        } else {
            let segment_id = next_segment_id;
            entry.current_segment = Some(SegmentState {
                segment_id: segment_id.clone(),
                key: segment_key,
                first_turn_id: entry.active.agent_turn_id.clone(),
                through_turn_id: entry.active.agent_turn_id.clone(),
                first_ordinal: entry.active.ordinal,
                through_ordinal: entry.active.ordinal,
                partial: entry.active.capture_partial,
            });
            (segment_id, entry.active.capture_partial)
        }
    } else {
        entry.current_segment = None;
        // Missing executor/rubric attribution is not evidence of missing bytes.
        (next_segment_id, entry.active.capture_partial)
    };
    let status = entry.active.last_request_status;
    let executed_branch_id = match &attribution {
        ExecutionAttribution::Single {
            executed_branch_id, ..
        } if executed_branch_id != &selected_branch_id => Some(executed_branch_id.clone()),
        _ => None,
    };
    let wire = Arc::new(VisibleAgentTurn {
        branch_id: Some(selected_branch_id.clone()),
        executed_branch_id,
        user: std::mem::take(&mut entry.active.user),
        status,
        steps: std::mem::take(&mut entry.active.steps),
    });
    let completed = CompletedAgentTurn {
        evaluation_scope,
        history_membership: Some(hiroute_domain::HistoryMembershipV1 {
            version: 1,
            branch_id: entry.branch_id.clone(),
            revision: entry.revision,
            request_ids: entry.active.request_ids.clone(),
            clipped: entry.active.request_ids_clipped,
            relation: if entry.active.association_unverified {
                branches::HistoryRelation::Contextual.into()
            } else {
                entry.branch_relation.into()
            },
            source_unverified: entry.active.source_unverified,
            association_unverified: entry.active.association_unverified
                || matches!(
                    entry.branch_relation,
                    branches::HistoryRelation::Contextual
                        | branches::HistoryRelation::AmbiguousPrefix
                ),
            capture_partial: entry.active.capture_partial || segment_partial,
        }),
        opening_cause: entry.active.opening_cause,
        branch_execution: if matches!(attribution, ExecutionAttribution::Single { .. }) {
            entry
                .active
                .executions
                .first()
                .and_then(|e| e.branch_execution.clone())
        } else {
            None
        },
        agent_turn_id: entry.active.agent_turn_id.clone(),
        segment_id,
        ordinal: entry.active.ordinal,
        plan: entry.active.plan.clone(),
        selected_branch_id,
        attribution,
        started_at_ms: entry.active.started_at_ms,
        finished_at_ms,
        status,
        history_partial: entry.active.capture_partial || segment_partial,
        first_request_id: entry.active.first_request_id.clone(),
        last_request_id: entry.active.last_request_id.clone(),
    };
    entry.closed.push_back(ClosedTurn {
        decision: decision.clone(),
        wire,
        completed: completed.clone(),
        closure_proven: closure.proven(status) && entry.active.provider_outcomes_known,
    });
    entry.active.finalized = true;
    // Keep the bounded identities of accepted tools: an interrupted stage can
    // resume, and actual later tool results must still address the sealed steps.
    // Reopening restores those steps, without mutating any frozen snapshot.
    entry.active.output.next_step();
    Ok(completed)
}

pub(super) fn execution_attribution(
    executions: &[AcceptedExecution],
    selected_branch_id: &str,
) -> ExecutionAttribution {
    let Some(first) = executions.first() else {
        return ExecutionAttribution::Unknown;
    };
    if executions.iter().skip(1).any(|execution| {
        execution.model_configuration_id != first.model_configuration_id
            || execution.profile_digest != first.profile_digest
            || execution.executed_branch_id != first.executed_branch_id
            || execution.branch_execution != first.branch_execution
    }) {
        return ExecutionAttribution::Mixed;
    }
    ExecutionAttribution::Single {
        selected_branch_id: selected_branch_id.to_owned(),
        executed_branch_id: first.executed_branch_id.clone(),
        model_configuration_id: first.model_configuration_id.clone(),
        profile_digest: first.profile_digest.clone(),
    }
}

pub(super) fn snapshot(
    entry: &SessionEntry,
    pin: Arc<SnapshotPin>,
    observed_assessment_segment: Option<&SegmentState>,
) -> AgentTurnHistorySnapshot {
    let visible_conversation = entry
        .closed
        .iter()
        .map(|turn| Arc::clone(&turn.wire))
        .collect::<Vec<_>>();
    let target_segment = entry
        .current_segment
        .as_ref()
        .or(observed_assessment_segment)
        .or(entry.referenced_segment.as_ref());
    let assessment_from = target_segment.and_then(|segment| {
        entry
            .closed
            .iter()
            .position(|turn| turn.completed.segment_id == segment.segment_id)
    });
    let assessment_target = target_segment.and_then(|segment| {
        let last = entry
            .closed
            .iter()
            .rev()
            .find(|turn| turn.completed.segment_id == segment.segment_id)?;
        match &last.completed.attribution {
            ExecutionAttribution::Single { .. } | ExecutionAttribution::Mixed
                if matches!(
                    last.completed.attribution,
                    ExecutionAttribution::Single { .. }
                ) || last.completed.evaluation_scope.is_some() =>
            {
                Some(AssessmentTarget {
                    evaluation_scope: last.completed.evaluation_scope.clone(),
                    history_membership: stage_membership(entry, segment),
                    branch_execution: last.completed.branch_execution.clone(),
                    segment_id: segment.segment_id.clone(),
                    first_turn_id: segment.first_turn_id.clone(),
                    through_turn_id: segment.through_turn_id.clone(),
                    first_ordinal: segment.first_ordinal,
                    through_ordinal: segment.through_ordinal,
                    target_partial: segment.partial || segment_has_unproven_closure(entry, segment),
                    plan: last.completed.plan.clone(),
                    attribution: last.completed.attribution.clone(),
                })
            }
            _ => None,
        }
    });
    AgentTurnHistorySnapshot {
        previous_decision: entry
            .closed
            .back()
            .map(|turn| (turn.completed.plan.clone(), turn.decision.clone())),
        visible_conversation,
        history_partial: entry.history_partial,
        assessment_from,
        assessment_target,
        _pin: Some(pin),
    }
}

impl StoreInner {
    pub(super) fn scope_entries<'a>(
        &'a self,
        scope: &AgentTurnHistoryKey,
    ) -> impl Iterator<Item = (&'a AgentTurnHistoryKey, &'a SessionEntry)> {
        self.scopes
            .get(scope)
            .into_iter()
            .flat_map(|keys| keys.iter())
            .filter_map(|key| self.entries.get_key_value(key))
    }

    pub(super) fn retained_records(&self, scope: &AgentTurnHistoryKey) -> usize {
        self.scope_entries(scope)
            .map(|(_, e)| {
                let closed = e
                    .closed
                    .iter()
                    .map(|t| {
                        t.completed
                            .history_membership
                            .as_ref()
                            .map_or(1, |m| m.request_ids.len().max(1))
                    })
                    .sum::<usize>();
                let active_is_closed = e.active.finalized
                    && e.closed
                        .iter()
                        .any(|t| t.completed.agent_turn_id == e.active.agent_turn_id);
                let active = if active_is_closed {
                    0
                } else {
                    e.active.completed_requests
                };
                // Reserve space for each live completion as well. Conservative
                // duplicate references count toward the bound, not just stages.
                closed + active + usize::from(e.pending_request.is_some())
            })
            .sum()
    }

    pub(super) fn reserve_scope_record(&mut self, scope: &AgentTurnHistoryKey) -> bool {
        self.trim_scope_records(scope, 127)
    }

    pub(super) fn limit_terminal_records(&mut self, scope: &AgentTurnHistoryKey) {
        self.trim_scope_records(scope, 128);
    }

    fn trim_scope_records(&mut self, scope: &AgentTurnHistoryKey, limit: usize) -> bool {
        while self.retained_records(scope) > limit {
            let victim = self
                .scope_entries(scope)
                .filter(|(_, e)| !e.closed.is_empty())
                .min_by_key(|(_, e)| {
                    (
                        e.snapshot_pins != 0,
                        e.closed.front().unwrap().completed.started_at_ms,
                        e.admission_sequence,
                    )
                })
                .map(|(key, _)| key.clone());
            // Frozen snapshots own their Arc payload and retain a separate charge.
            // Trimming a live index never frees their bytes or changes membership.
            if let Some(key) = victim
                && trim_oldest_closed_turn(self, &key).unwrap_or(false)
            {
                continue;
            }
            // A long tool phase can retain many terminal HTTP records without
            // closing its stage. Retire its whole idle branch rather than drop
            // individual attempts and invent a different model attribution.
            let idle = self
                .scope_entries(scope)
                .filter(|(_, e)| e.pending_request.is_none() && e.snapshot_pins == 0)
                .min_by_key(|(_, e)| e.admission_sequence)
                .map(|(key, _)| key.clone());
            let Some(key) = idle else {
                return false;
            };
            self.remove(&key);
        }
        true
    }

    pub(super) fn note_capture_gap(&mut self, key: &AgentTurnHistoryKey) {
        let Some(entry) = self.entries.get_mut(key) else {
            return;
        };
        entry.history_partial = true;
        entry.current_segment = None;
        entry.referenced_segment = None;
        entry.response_boundary = None;
        if !active_is_empty(&entry.active) {
            entry.active.capture_partial = true;
        }
        let old = entry.accounted_bytes;
        entry.accounted_bytes = entry_bytes(key, entry);
        let new = entry.accounted_bytes;
        self.replace_accounting(old, new);
    }

    pub(super) fn allocate_token(&mut self) -> Result<u64, AgentTurnHistoryError> {
        let token = self.next_token;
        self.next_token = token
            .checked_add(1)
            .ok_or(AgentTurnHistoryError::Resource)?;
        Ok(token)
    }

    pub(super) fn allocate_request_token(&mut self) -> Result<u64, AgentTurnHistoryError> {
        let token = self.next_request_token;
        self.next_request_token = token
            .checked_add(1)
            .ok_or(AgentTurnHistoryError::Resource)?;
        Ok(token)
    }

    pub(super) fn evict_to_budget(
        &mut self,
        budget: usize,
        protected: Option<&AgentTurnHistoryKey>,
    ) {
        while self.accounted_bytes > budget {
            let victim = self
                .entries
                .iter()
                .filter(|(key, entry)| {
                    protected.is_none_or(|protected| protected != *key)
                        && entry.pending_request.is_none()
                        && entry.snapshot_pins == 0
                })
                .min_by_key(|(_, entry)| entry.last_access)
                .map(|(key, _)| key.clone());
            let Some(victim) = victim else {
                break;
            };
            self.remove(&victim);
        }
    }

    pub(super) fn remove(&mut self, key: &AgentTurnHistoryKey) {
        if let Some(entry) = self.entries.remove(key) {
            if let Some(keys) = self.scopes.get_mut(&entry.scope) {
                keys.remove(key);
                if keys.is_empty() {
                    self.scopes.remove(&entry.scope);
                }
            }
            self.accounted_bytes = self.accounted_bytes.saturating_sub(entry.accounted_bytes);
        }
    }

    pub(super) fn replace_accounting(&mut self, old: usize, new: usize) {
        self.accounted_bytes = self.accounted_bytes.saturating_sub(old).saturating_add(new);
    }

    pub(super) fn note_peak(&mut self) {
        self.peak_accounted_bytes = self.peak_accounted_bytes.max(self.accounted_bytes);
    }
}

pub(super) fn entry_bytes(key: &AgentTurnHistoryKey, entry: &SessionEntry) -> usize {
    size_of::<SessionEntry>()
        + size_of_val(key) + 768 // charged scope and branch index nodes
        + entry.branch_id.capacity()
        + entry.active.decision.as_ref().map_or(0, decision_bytes)
        + entry.active.request_ids.capacity() * size_of::<String>()
        + entry
            .active
            .request_ids
            .iter()
            .map(String::capacity)
            .sum::<usize>()
        + entry.active.agent_turn_id.capacity()
        + entry
            .active
            .first_request_id
            .as_ref()
            .map_or(0, String::capacity)
        + entry
            .active
            .last_request_id
            .as_ref()
            .map_or(0, String::capacity)
        + entry.active.user.capacity() * size_of::<VisibleContentPart>()
        + entry
            .active
            .user
            .iter()
            .map(VisibleContentPart::retained_bytes)
            .sum::<usize>()
        + entry.active.steps.capacity() * size_of::<Vec<VisibleContentPart>>()
        + entry.active.output.retained_bytes()
        + entry.active.boundary.bytes()
        + entry
            .active
            .steps
            .iter()
            .map(|step| step.capacity() * size_of::<VisibleContentPart>())
            .sum::<usize>()
        + entry
            .active
            .steps
            .iter()
            .flat_map(|step| step.iter())
            .map(VisibleContentPart::retained_bytes)
            .sum::<usize>()
        + entry
            .active
            .executions
            .iter()
            .map(|execution| {
                size_of::<AcceptedExecution>()
                    + execution
                        .branch_execution
                        .as_ref()
                        .map_or(0, |e| e.policy.name.capacity() + 80)
                    + execution.model_configuration_id.capacity()
                    + execution.profile_digest.capacity()
                    + execution.executed_branch_id.capacity()
                    + execution.request_id.capacity()
            })
            .sum::<usize>()
        + entry
            .closed
            .iter()
            .map(|turn| {
                turn.wire.retained_bytes()
                    + completed_bytes(&turn.completed)
                    + decision_bytes(&turn.decision)
            })
            .sum::<usize>()
        + entry.current_segment.as_ref().map_or(0, segment_bytes)
        + entry.referenced_segment.as_ref().map_or(0, segment_bytes)
}

pub(super) fn decision_bytes(value: &BranchDecisionV1) -> usize {
    size_of::<BranchDecisionV1>()
        + value.strategy_id.capacity()
        + value.schema_version.capacity()
        + value.payload_digest.capacity()
        + value.branch_id.capacity()
        + value.reason_codes.capacity()
            * size_of::<crate::server::core_runtime::profiles::ComplexityReasonCodeV1>()
        + value.matched_user_phrase_ids.capacity() * size_of::<String>()
        + value
            .matched_user_phrase_ids
            .iter()
            .map(String::capacity)
            .sum::<usize>()
        + value
            .policy
            .as_ref()
            .map_or(0, |policy| policy.name.capacity() + 80)
        + value
            .competence_trigger
            .as_ref()
            .map_or(0, |trigger| trigger.segment_id.capacity())
}

pub(super) fn completed_bytes(value: &CompletedAgentTurn) -> usize {
    size_of::<CompletedAgentTurn>()
        + value
            .history_membership
            .as_ref()
            .map_or(0, membership_bytes)
        + value
            .branch_execution
            .as_ref()
            .map_or(0, |e| e.policy.name.capacity() + 80)
        + value.agent_turn_id.capacity()
        + value.segment_id.capacity()
        + value.plan.plan_id.capacity()
        + value.selected_branch_id.capacity()
        + super::evaluation::scope_bytes(&value.evaluation_scope)
        + attribution_bytes(&value.attribution)
        + value.first_request_id.as_ref().map_or(0, String::capacity)
        + value.last_request_id.as_ref().map_or(0, String::capacity)
}

pub(super) fn attribution_bytes(value: &ExecutionAttribution) -> usize {
    match value {
        ExecutionAttribution::Single {
            selected_branch_id,
            executed_branch_id,
            model_configuration_id,
            profile_digest,
        } => {
            selected_branch_id.capacity()
                + executed_branch_id.capacity()
                + model_configuration_id.capacity()
                + profile_digest.capacity()
        }
        ExecutionAttribution::Mixed | ExecutionAttribution::Unknown => 0,
    }
}

pub(super) fn segment_bytes(value: &SegmentState) -> usize {
    size_of::<SegmentState>()
        + value
            .key
            .branch_execution
            .as_ref()
            .map_or(0, |e| e.policy.name.capacity() + 80)
        + value.segment_id.capacity()
        + value.first_turn_id.capacity()
        + value.through_turn_id.capacity()
        + value.key.plan.plan_id.capacity()
        + value.key.selected_branch_id.capacity()
        + value.key.executed_branch_id.capacity()
        + value
            .key
            .model_configuration_id
            .as_ref()
            .map_or(0, String::capacity)
        + value
            .key
            .profile_digest
            .as_ref()
            .map_or(0, String::capacity)
        + super::evaluation::scope_bytes(&value.key.evaluation_scope)
}

pub(super) fn store_fixed_bytes() -> usize {
    size_of::<AgentTurnHistoryStore>() + size_of::<StoreInner>() + 2_048
}

pub(super) fn opaque_id(
    kind: &str,
    nonce: &[u8; 32],
    value: u64,
) -> Result<String, AgentTurnHistoryError> {
    let mut mac =
        HmacSha256::new_from_slice(nonce).map_err(|_| AgentTurnHistoryError::Integrity)?;
    mac.update(kind.as_bytes());
    mac.update(&value.to_be_bytes());
    Ok(format!("{kind}-{}", hex(&mac.finalize().into_bytes())))
}

pub(super) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut value, "{byte:02x}");
    }
    value
}

pub(super) fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn membership_bytes(value: &hiroute_domain::HistoryMembershipV1) -> usize {
    size_of_val(value)
        + value.branch_id.capacity()
        + value.request_ids.capacity() * size_of::<String>()
        + value
            .request_ids
            .iter()
            .map(String::capacity)
            .sum::<usize>()
}

fn stage_membership(
    entry: &SessionEntry,
    segment: &SegmentState,
) -> Option<hiroute_domain::HistoryMembershipV1> {
    let mut result = entry
        .closed
        .iter()
        .rev()
        .find(|t| t.completed.segment_id == segment.segment_id)?
        .completed
        .history_membership
        .clone()?;
    result.request_ids.clear();
    for turn in entry
        .closed
        .iter()
        .filter(|t| t.completed.segment_id == segment.segment_id)
    {
        let Some(membership) = &turn.completed.history_membership else {
            result.clipped = true;
            continue;
        };
        result.source_unverified |= membership.source_unverified;
        result.association_unverified |= membership.association_unverified;
        if result.association_unverified {
            use hiroute_domain::HistoryRelationV1;
            result.relation = if result.relation == HistoryRelationV1::AmbiguousPrefix
                || membership.relation == HistoryRelationV1::AmbiguousPrefix
            {
                HistoryRelationV1::AmbiguousPrefix
            } else {
                HistoryRelationV1::Contextual
            };
        }
        result.capture_partial |= membership.capture_partial;
        result.clipped |= membership.clipped;
        for id in &membership.request_ids {
            if result.request_ids.contains(id) {
                continue;
            }
            if result.request_ids.len() < 128 {
                result.request_ids.push(id.clone());
            } else {
                result.clipped = true;
            }
        }
    }
    Some(result)
}

impl AgentTurnHistoryStore {
    pub(super) fn pinned_snapshot(
        &self,
        inner: &mut StoreInner,
        key: &AgentTurnHistoryKey,
        segment: Option<&SegmentState>,
    ) -> Option<AgentTurnHistorySnapshot> {
        let entry = inner.entries.get(key)?;
        // Charge the immutable snapshot separately: reopening or trimming a
        // live branch cannot make retained Arc payloads disappear from budget.
        let charge = 2048usize.saturating_add(
            entry
                .closed
                .iter()
                .map(|turn| {
                    turn.wire.retained_bytes()
                        + completed_bytes(&turn.completed)
                        + decision_bytes(&turn.decision)
                })
                .sum::<usize>(),
        );
        inner.evict_to_budget(self.max_memory_bytes.saturating_sub(charge), Some(key));
        if inner.accounted_bytes.saturating_add(charge) > self.max_memory_bytes {
            return None;
        }
        let entry = inner.entries.get_mut(key)?;
        entry.snapshot_pins = entry.snapshot_pins.checked_add(1)?;
        let pin = Arc::new(SnapshotPin {
            inner: Arc::downgrade(&self.inner),
            key: key.clone(),
            entry_token: entry.entry_token,
            reserved_bytes: charge,
        });
        let result = snapshot(entry, pin, segment);
        inner.accounted_bytes = inner.accounted_bytes.saturating_add(charge);
        inner.note_peak();
        Some(result)
    }
}
