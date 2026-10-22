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
    pub(super) inputs: TurnDecisionInputs,
    seed: VecDeque<ClosedTurn>,
    segment: Option<SegmentState>,
    partial: bool,
    forked: bool,
    completed: Option<Box<CompletedAgentTurn>>,
}

impl AgentTurnHistoryStore {
    pub(crate) fn prepare_scoped_request(
        &self,
        scope: &AgentTurnHistoryKey,
        request: &ModelRequestIRV1,
        replay: &ReplayStore,
    ) -> Result<super::super::projection::AnalyzedRequest, AgentTurnHistoryError> {
        let positions = {
            let inner = self
                .inner
                .try_lock()
                .ok_or(AgentTurnHistoryError::Resource)?;
            inner
                .entries
                .values()
                .filter(|e| &e.scope == scope)
                .take(MAX_HEADS)
                .map(|e| e.checkpoint.message_count)
                .collect::<Vec<_>>()
        };
        let mut analyzed = analyze_request(request, replay)?;
        analyzed.anchors = crate::context_hold::observed_history_with_replay(
            request,
            &self.instance_nonce,
            Some(replay),
        )
        .and_then(|history| history.branch_anchors(&positions));
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
            .entries
            .iter()
            .filter(|(_, e)| &e.scope == scope)
            .map(|(key, entry)| {
                let prefix = analyzed.anchors.as_ref().is_some_and(|anchors| {
                    entry.checkpoint.digest.is_some_and(|digest| {
                        anchors
                            .prefixes
                            .contains(&(entry.checkpoint.message_count, digest))
                    })
                });
                let turn =
                    inputs.native_turn.is_some() && inputs.native_turn == entry.active.native_turn;
                (
                    key.clone(),
                    prefix,
                    entry.checkpoint.message_count,
                    turn,
                    entry.admission_sequence,
                )
            })
            .collect::<Vec<_>>();
        // Admission order breaks equally plausible ties, not response completion.
        candidates.sort_by(|a, b| {
            (b.1, b.1.then_some(b.2), b.3, b.4, &b.0).cmp(&(
                a.1,
                a.1.then_some(a.2),
                a.3,
                a.4,
                &a.0,
            ))
        });
        let selected = candidates.first();
        let exact = selected.is_some_and(|c| c.1);
        let ambiguous = selected.is_some_and(|first| {
            first.1
                && candidates
                    .iter()
                    .skip(1)
                    .any(|c| c.1 && c.2 == first.2 && c.3 == first.3)
        });
        let selected_entry = selected.and_then(|c| inner.entries.get(&c.0));
        // Old internal callers without measured anchors retain their explicit
        // continuity contract. Production always supplies measured evidence.
        let legacy_exact = analyzed.anchors.is_none()
            && selected_entry.is_some()
            && inputs.history_continuity.semantic_continuity();
        let reusable = selected_entry.is_some_and(|entry| entry.pending_request.is_none())
            && (exact && !ambiguous || legacy_exact);
        if let Some(entry) = selected_entry
            && analyzed.anchors.is_some()
        {
            inputs.history_continuity = if exact && !ambiguous {
                if inputs.native_turn.is_some() && inputs.native_turn == entry.active.native_turn {
                    HistoryContinuity::NativeSameTurn
                } else if inputs.native_turn.is_some() && entry.active.native_turn.is_some() {
                    HistoryContinuity::NativeNewTurn
                } else {
                    HistoryContinuity::Continuing
                }
            } else {
                HistoryContinuity::Rewritten
            };
        }
        let relation = if ambiguous {
            HistoryRelation::AmbiguousPrefix
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
                completed = Some(Box::new(close_active(entry, segment, unix_ms())?));
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
            segment = entry.current_segment.clone();
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
            inputs,
            seed,
            segment,
            partial,
            forked: !reusable,
            completed,
        })
    }

    fn make_branch_room(
        &self,
        inner: &mut StoreInner,
        scope: &AgentTurnHistoryKey,
    ) -> Result<(), AgentTurnHistoryError> {
        if inner.entries.values().filter(|e| &e.scope == scope).count() < MAX_HEADS {
            return Ok(());
        }
        let victim = inner
            .entries
            .iter()
            .filter(|(_, e)| {
                &e.scope == scope && e.pending_request.is_none() && e.snapshot_pins == 0
            })
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
        entry.scope = selection.scope;
        entry.branch_relation = selection.relation;
        if !selection.forked || selection.seed.is_empty() {
            return Ok(());
        }
        entry.closed = selection.seed;
        entry.history_partial = selection.partial;
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
        inner.evict_to_budget(self.max_memory_bytes, Some(&selection.key));
        while inner.accounted_bytes > self.max_memory_bytes {
            if !trim_oldest_closed_turn(inner, &selection.key)? {
                return Err(AgentTurnHistoryError::Resource);
            }
        }
        let entry = inner
            .entries
            .get_mut(&selection.key)
            .ok_or(AgentTurnHistoryError::Resource)?;
        entry.snapshot_pins = entry
            .snapshot_pins
            .checked_add(1)
            .ok_or(AgentTurnHistoryError::Resource)?;
        let pin = Arc::new(SnapshotPin {
            inner: Arc::downgrade(&self.inner),
            key: selection.key,
            entry_token: entry.entry_token,
        });
        let mut frozen = snapshot(entry, pin, selection.segment.as_ref());
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
            *history = Box::new(frozen);
            *completed = selection.completed;
        }
        inner.note_peak();
        Ok(())
    }
}
