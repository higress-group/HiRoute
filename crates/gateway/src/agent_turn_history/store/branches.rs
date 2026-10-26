//! Bounded branch selection. A session is a namespace, never a serial execution slot.
use super::*;
use crate::context_hold::HistoryContinuity;

const MAX_HEADS: usize = 32;
const MAX_TERMINAL_RECORDS: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HistoryRelation {
    Root,
    ExactPrefix,
    AmbiguousPrefix,
    Contextual,
}

impl From<HistoryRelation> for hiroute_domain::HistoryRelationV1 {
    fn from(value: HistoryRelation) -> Self {
        match value {
            HistoryRelation::Root => Self::Root,
            HistoryRelation::ExactPrefix => Self::ExactPrefix,
            HistoryRelation::AmbiguousPrefix => Self::AmbiguousPrefix,
            HistoryRelation::Contextual => Self::Contextual,
        }
    }
}

pub(super) struct BranchAdmission {
    pub(super) key: AgentTurnHistoryKey,
    pub(super) scope: AgentTurnHistoryKey,
    pub(super) relation: HistoryRelation,
    projected: bool,
    retry: bool,
    pub(super) inputs: TurnDecisionInputs,
    seed: VecDeque<ClosedTurn>,
    segment: Option<SegmentState>,
    partial: bool,
    forked: bool,
    opening_cause: Option<hiroute_domain::AgentTurnOpeningCauseV1>,
    completed: Option<Box<CompletedAgentTurn>>,
}

impl AgentTurnHistoryStore {
    pub(crate) fn prepare_scoped_request(
        &self,
        scope: &AgentTurnHistoryKey,
        request: &ModelRequestIRV1,
        replay: &ReplayStore,
    ) -> Result<super::super::projection::AnalyzedRequest, AgentTurnHistoryError> {
        let mut positions = {
            let inner = self
                .inner
                .try_lock()
                .ok_or(AgentTurnHistoryError::Resource)?;
            inner
                .scope_entries(scope)
                .map(|(_, e)| e)
                .take(MAX_HEADS)
                .map(|e| e.checkpoint.message_count)
                .collect::<Vec<_>>()
        };
        positions.sort_unstable();
        positions.dedup();
        let mut analyzed = analyze_request(request, replay)?;
        analyzed.measured = true;
        analyzed.retry_setup = crate::context_hold::observed_history_with_replay(
            request,
            &self.instance_nonce,
            Some(replay),
        )
        .map(|history| history.instruction_digest);
        let mut boundary_budget = crate::context_hold::fingerprint::FingerprintBudget::default();
        analyzed.boundaries = positions
            .iter()
            .filter_map(|position| {
                super::super::boundary::incoming_evidence(
                    request,
                    *position,
                    replay,
                    &mut boundary_budget,
                )
                .map(|digest| (*position, digest))
            })
            .collect();
        analyzed.anchors = crate::context_hold::branch_anchors(
            request,
            &self.instance_nonce,
            Some(replay),
            &positions,
        );
        Ok(analyzed)
    }

    pub(super) fn select_branch(
        &self,
        inner: &mut StoreInner,
        scope: &AgentTurnHistoryKey,
        analyzed: &super::super::projection::AnalyzedRequest,
        mut inputs: TurnDecisionInputs,
    ) -> Result<BranchAdmission, AgentTurnHistoryError> {
        let mut candidates = inner
            .scope_entries(scope)
            .map(|(key, entry)| {
                let input_prefix = analyzed.anchors.as_ref().is_some_and(|anchors| {
                    entry.checkpoint.digest.is_some_and(|digest| {
                        anchors
                            .prefixes
                            .contains(&(entry.checkpoint.message_count, digest))
                    })
                });
                let response = input_prefix
                    .then(|| {
                        let accepted = entry.response_boundary?;
                        let incoming = analyzed
                            .boundaries
                            .iter()
                            .find(|(position, _)| *position == entry.checkpoint.message_count)?
                            .1;
                        accepted.matches(incoming)
                    })
                    .flatten();
                let prefix = response.is_some();
                let turn =
                    inputs.native_turn.is_some() && inputs.native_turn == entry.active.native_turn;
                let cursor = analyzed.branch_cursor != (None, None)
                    && analyzed.branch_cursor == entry.branch_cursor;
                (
                    key.clone(),
                    prefix,
                    entry.checkpoint.message_count,
                    turn,
                    cursor,
                    entry.current_segment.is_some()
                        || entry.referenced_segment.is_some()
                        || (!entry.active.finalized && !entry.active.executions.is_empty()),
                    entry.admission_sequence,
                    response.is_some_and(|projected| {
                        projected || analyzed.anchors.as_ref().is_some_and(|a| a.contextual)
                    }),
                )
            })
            .collect::<Vec<_>>();
        // An exact retry of an ingress with a capture gap must retain that gap.
        // This is only a ranking hint: it never establishes response continuity.
        let retry_matches = |key: &AgentTurnHistoryKey| {
            let entry = &inner.entries[key];
            analyzed.anchors.as_ref().is_some_and(|anchors| {
                entry.checkpoint.digest == Some(anchors.complete)
                    && entry.checkpoint.message_count == analyzed.message_count
            })
        };
        let native_retries = candidates
            .iter()
            .filter(|candidate| {
                super::retry::same_native_request(
                    &inner.entries[&candidate.0],
                    analyzed,
                    inputs.native_turn,
                )
            })
            .map(|candidate| candidate.0.clone())
            .collect::<Vec<_>>();
        // Count every matching branch, including active or unobserved siblings.
        // Native turn identity cannot select among ambiguous equal requests.
        let retry_key = (native_retries.len() == 1)
            .then(|| &native_retries[0])
            .filter(|key| super::retry::failed_before_output(&inner.entries[*key]));
        // Admission order breaks equally plausible ties, not response completion.
        candidates.sort_by(|a, b| {
            (
                b.1,
                b.1.then_some(b.2),
                retry_key == Some(&b.0),
                retry_matches(&b.0),
                b.3,
                b.4,
                analyzed.measured && b.5,
                b.6,
                &b.0,
            )
                .cmp(&(
                    a.1,
                    a.1.then_some(a.2),
                    retry_key == Some(&a.0),
                    retry_matches(&a.0),
                    a.3,
                    a.4,
                    analyzed.measured && a.5,
                    a.6,
                    &a.0,
                ))
        });
        let selected = candidates.first();
        let exact = selected.is_some_and(|c| c.1);
        let retry = !exact && selected.is_some_and(|c| retry_key == Some(&c.0));
        let ambiguous = selected.is_some_and(|first| {
            first.1
                && candidates.iter().skip(1).any(|c| {
                    c.1 && c.2 == first.2 && (first.7 || c.7 || c.3 == first.3 && c.4 == first.4)
                })
        });
        let projected = selected.is_some_and(|c| c.7) && !ambiguous;
        let selected_entry = selected.and_then(|c| inner.entries.get(&c.0));
        // Old internal callers without measured anchors retain their explicit
        // continuity contract. Production always supplies measured evidence.
        let legacy_exact = !analyzed.measured
            && selected_entry.is_some()
            && inputs.history_continuity.semantic_continuity();
        let reusable = selected_entry.is_some_and(|entry| {
            entry.pending_request.is_none()
                && (entry.active.source_unverified || !analyzed.user_origin_unverified)
        }) && (exact && !ambiguous || retry || legacy_exact);
        let opening_cause = (analyzed.measured && !reusable).then(|| {
            super::opening::fork_opening_cause(
                inner.scope_entries(scope).map(|(_, entry)| entry),
                analyzed,
            )
        });
        if let Some(entry) = selected_entry
            && analyzed.measured
        {
            inputs.history_continuity = if retry {
                HistoryContinuity::NativeSameTurn
            } else if exact && !ambiguous {
                if inputs.native_turn.is_some()
                    && inputs.native_turn == entry.active.native_turn
                    && !super::opening::has_appended_input(&entry.checkpoint, analyzed)
                {
                    HistoryContinuity::NativeSameTurn
                } else if inputs.native_turn.is_some()
                    && entry.active.native_turn.is_some()
                    && super::opening::has_appended_input(&entry.checkpoint, analyzed)
                {
                    HistoryContinuity::NativeNewTurn
                } else {
                    HistoryContinuity::Continuing
                }
            } else if analyzed.message_count < entry.checkpoint.message_count {
                HistoryContinuity::Shortened
            } else {
                HistoryContinuity::Rewritten
            };
        }
        let relation = if ambiguous {
            HistoryRelation::AmbiguousPrefix
        } else if projected {
            HistoryRelation::Contextual
        } else if exact || legacy_exact {
            HistoryRelation::ExactPrefix
        } else if selected_entry.is_some() {
            HistoryRelation::Contextual
        } else {
            HistoryRelation::Root
        };
        let mut completed = None;
        let sealed_key = selected.filter(|_| !reusable).map(|c| c.0.clone());
        if let Some(key) = &sealed_key {
            let seal = inner.entries.get(key).is_some_and(|e| {
                e.pending_request.is_none()
                    && !e.active.finalized
                    && !active_is_empty(&e.active)
                    && e.active.decision.is_some()
            });
            if seal {
                let segment = self.allocate_id(inner, "segment")?;
                let entry = inner.entries.get_mut(key).expect("selected entry");
                // Only a witnessed activation after the accepted output can
                // establish this boundary. A missing edge freezes evidence,
                // but cannot declare the still-running tool stage complete.
                let new_activation = exact
                    && !ambiguous
                    && super::opening::has_appended_input(&entry.checkpoint, analyzed);
                let closure = if new_activation {
                    StageClosure::NewActivation
                } else {
                    StageClosure::AssociationLost
                };
                completed = Some(Box::new(close_active(entry, segment, unix_ms(), closure)?));
                let old = entry.accounted_bytes;
                entry.accounted_bytes = entry_bytes(key, entry);
                let new = entry.accounted_bytes;
                inner.replace_accounting(old, new);
            }
        }
        let selected_entry = selected.and_then(|c| inner.entries.get(&c.0));
        let mut seed = VecDeque::new();
        let mut segment = None;
        let mut partial = false;
        if !reusable && let Some(entry) = selected_entry {
            seed = entry.closed.clone();
            segment = entry
                .current_segment
                .clone()
                .or_else(|| entry.referenced_segment.clone());
            partial = entry.history_partial;
            // A still-running request is never a frozen assessment target.
            if segment.as_ref().is_some_and(|s| {
                !seed
                    .iter()
                    .any(|t| t.completed.agent_turn_id == s.through_turn_id)
            }) {
                segment = None;
            }
        }
        let key = if reusable {
            selected.expect("reusable candidate").0.clone()
        } else if candidates.is_empty() && !inner.entries.contains_key(scope) {
            scope.clone()
        } else {
            self.make_branch_room(inner, scope)?;
            let token = inner.allocate_token()?;
            let mut mac = HmacSha256::new_from_slice(&self.instance_nonce)
                .map_err(|_| AgentTurnHistoryError::Integrity)?;
            mac.update(b"execution-branch/v2");
            mac.update(&scope.0);
            mac.update(&token.to_be_bytes());
            AgentTurnHistoryKey(mac.finalize().into_bytes().into())
        };
        if !reusable && inputs.history_continuity.semantic_continuity() {
            inputs.history_continuity = HistoryContinuity::NoCheckpoint;
        }
        Ok(BranchAdmission {
            key,
            scope: scope.clone(),
            relation,
            projected,
            retry,
            inputs,
            seed,
            segment,
            partial,
            forked: !reusable,
            opening_cause,
            completed,
        })
    }

    fn make_branch_room(
        &self,
        inner: &mut StoreInner,
        scope: &AgentTurnHistoryKey,
    ) -> Result<(), AgentTurnHistoryError> {
        if inner.scope_entries(scope).count() < MAX_HEADS {
            return Ok(());
        }
        let victim = inner
            .scope_entries(scope)
            .filter(|(_, e)| e.pending_request.is_none() && e.snapshot_pins == 0)
            .min_by_key(|(_, e)| e.admission_sequence)
            .map(|(k, _)| k.clone())
            .ok_or(AgentTurnHistoryError::Resource)?;
        inner.remove(&victim);
        Ok(())
    }

    pub(super) fn finish_branch_admission(
        &self,
        inner: &mut StoreInner,
        selection: BranchAdmission,
        begin: &mut AgentTurnBegin,
    ) -> Result<(), AgentTurnHistoryError> {
        let entry = inner
            .entries
            .get_mut(&selection.key)
            .ok_or(AgentTurnHistoryError::Resource)?;
        entry.scope = selection.scope.clone();
        entry.branch_relation = selection.relation;
        entry.active.association_unverified |= selection.projected || selection.retry;
        let ticket = match begin {
            AgentTurnBegin::NewTurn {
                ticket, history, ..
            } => {
                if let Some(cause) = selection.opening_cause {
                    // Only a new independent branch receives this evidence.
                    // It has no current scoring segment to repair or inherit;
                    // the old segment remains separately frozen and partial.
                    debug_assert!(selection.forked && entry.current_segment.is_none());
                    entry.active.opening_cause = cause;
                }
                if selection.projected
                    && let Some(membership) = history
                        .assessment_target
                        .as_mut()
                        .and_then(|target| target.history_membership.as_mut())
                {
                    membership.relation = HistoryRelation::Contextual.into();
                    membership.association_unverified = true;
                }
                ticket
            }
            AgentTurnBegin::Continuation { ticket, .. } => ticket,
        };
        // A visible projection is enough for actual-stage capture, never for
        // provider affinity or provider-state cleanup authorization.
        ticket.allow_affinity = !selection.projected && !selection.retry;
        if !selection.forked || selection.seed.is_empty() {
            return Ok(());
        }
        entry.closed = selection.seed;
        entry.referenced_segment = selection.segment.clone();
        entry.history_partial = selection.partial
            || (selection.relation != HistoryRelation::ExactPrefix && entry.history_partial);
        entry.next_turn_ordinal = entry
            .closed
            .back()
            .map_or(2, |t| t.completed.ordinal.saturating_add(2));
        entry.active.ordinal = entry.next_turn_ordinal.saturating_sub(1);
        while entry.closed.len() >= MAX_TERMINAL_RECORDS {
            entry.closed.pop_front();
            entry.history_partial = true;
        }
        // These references are assessment context only. A fork always starts a
        // separate segment and cannot inherit the selected branch's decision.
        let old = entry.accounted_bytes;
        entry.accounted_bytes = entry_bytes(&selection.key, entry);
        let new = entry.accounted_bytes;
        inner.replace_accounting(old, new);
        inner.limit_terminal_records(&selection.scope);
        inner.evict_to_budget(self.max_memory_bytes, Some(&selection.key));
        while inner.accounted_bytes > self.max_memory_bytes {
            if !trim_oldest_closed_turn(inner, &selection.key)? {
                return Err(AgentTurnHistoryError::Resource);
            }
        }
        let mut frozen = self
            .pinned_snapshot(inner, &selection.key, selection.segment.as_ref())
            .unwrap_or_else(|| AgentTurnHistorySnapshot::empty(true));
        let entry = inner
            .entries
            .get(&selection.key)
            .ok_or(AgentTurnHistoryError::Resource)?;
        frozen.previous_decision = None;
        if let Some(membership) = frozen
            .assessment_target
            .as_mut()
            .and_then(|t| t.history_membership.as_mut())
        {
            membership.relation = selection.relation.into();
            membership.association_unverified = selection.relation != HistoryRelation::ExactPrefix;
        }
        if frozen
            .assessment_target
            .as_ref()
            .is_some_and(|target| target.plan != entry.active.plan)
        {
            frozen.assessment_target = None;
            frozen.assessment_from = None;
        }
        if let AgentTurnBegin::NewTurn {
            history, completed, ..
        } = begin
        {
            **history = frozen;
            *completed = selection.completed;
        }
        inner.note_peak();
        Ok(())
    }
}
