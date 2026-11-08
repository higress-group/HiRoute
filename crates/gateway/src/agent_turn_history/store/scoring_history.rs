//! The scope budget owns optional scoring data. The global ledger also owns
//! checkpoints, relay state and the lifetime of in-flight assessment snapshots.
use super::*;

pub(super) fn content_bytes(parts: &[VisibleContentPart], capacity: usize) -> usize {
    capacity * size_of::<VisibleContentPart>()
        + parts
            .iter()
            .map(VisibleContentPart::retained_bytes)
            .sum::<usize>()
}

pub(super) fn steps_bytes(steps: &Vec<Vec<VisibleContentPart>>) -> usize {
    steps.capacity() * size_of::<Vec<VisibleContentPart>>()
        + steps
            .iter()
            .map(|step| content_bytes(step, step.capacity()))
            .sum::<usize>()
}

fn execution_payload(execution: &AcceptedExecution) -> usize {
    execution
        .branch_execution
        .as_ref()
        .map_or(0, |e| e.policy.name.capacity() + 80)
        + execution.model_configuration_id.capacity()
        + execution.profile_digest.capacity()
        + execution.executed_branch_id.capacity()
        + execution.request_id.capacity()
}

fn grown_capacity<T>(items: &Vec<T>, extra: usize) -> usize {
    if items.len() + extra <= items.capacity() {
        items.capacity()
    } else {
        (items.capacity() * 2).max(items.len() + extra).max(4)
    }
}

impl ActiveTurn {
    pub(super) fn scoring_bytes(&self) -> usize {
        self.request_ids.capacity() * size_of::<String>()
            + self.request_id_bytes
            + self.executions.capacity() * size_of::<AcceptedExecution>()
            + self.execution_bytes
            + self.user_bytes
            + self.steps_bytes
            + self.output.retained_bytes()
    }

    pub(super) fn retain_request_id(&mut self, id: &str) {
        let id = id.to_owned();
        self.request_id_bytes += id.capacity();
        let capacity = grown_capacity(&self.request_ids, 1);
        self.request_ids
            .reserve_exact(capacity - self.request_ids.len());
        self.request_ids.push(id);
    }

    pub(super) fn retain_executions(&mut self, executions: Vec<AcceptedExecution>) {
        self.execution_bytes += executions.iter().map(execution_payload).sum::<usize>();
        let capacity = grown_capacity(&self.executions, executions.len());
        self.executions
            .reserve_exact(capacity - self.executions.len());
        self.executions.extend(executions);
    }

    pub(super) fn record_charge(
        &self,
        id: Option<&str>,
        executions: &[AcceptedExecution],
    ) -> usize {
        let ids = id.map_or(0, |id| {
            id.len()
                + (grown_capacity(&self.request_ids, 1) - self.request_ids.capacity())
                    * size_of::<String>()
        });
        ids + (grown_capacity(&self.executions, executions.len()) - self.executions.capacity())
            * size_of::<AcceptedExecution>()
            + executions.iter().map(execution_payload).sum::<usize>()
    }

    pub(super) fn clear_records(&mut self) {
        self.request_ids = Vec::new();
        self.request_id_bytes = 0;
        self.executions = Vec::new();
        self.execution_bytes = 0;
        self.request_ids_clipped = true;
    }

    pub(super) fn clear_content(&mut self) {
        self.user = Vec::new();
        self.user_bytes = 0;
        self.clear_steps();
    }

    pub(super) fn clear_steps(&mut self) {
        self.steps = Vec::new();
        self.steps_bytes = 0;
        self.output = Default::default();
    }
}

impl SessionEntry {
    pub(super) fn pop_closed_back(&mut self) -> Option<ClosedTurn> {
        let turn = self.closed.pop_back()?;
        self.closed_bytes -= turn.bytes;
        Some(turn)
    }

    pub(super) fn mark_scope_partial(&mut self) {
        self.active.scope_partial = true;
        self.history_partial = true;
    }
}

impl StoreInner {
    pub(super) fn scope_history_bytes(&self, scope: &AgentTurnHistoryKey) -> usize {
        self.scope_entries(scope)
            .map(|(_, entry)| entry.active.scoring_bytes() + entry.closed_bytes)
            .sum()
    }

    pub(super) fn reserve_scoring_bytes(
        &mut self,
        scope: &AgentTurnHistoryKey,
        charge: usize,
    ) -> bool {
        let Some(limit) = self.scope_history_limit.checked_sub(charge) else {
            return false;
        };
        while self.scope_history_bytes(scope) > limit {
            let oldest = self
                .scope_entries(scope)
                .filter_map(|(key, entry)| {
                    let first = entry.closed.front()?;
                    Some((
                        (
                            entry.snapshot_pins != 0,
                            first.completed.started_at_ms,
                            entry.admission_sequence,
                        ),
                        key.clone(),
                    ))
                })
                .min_by_key(|(priority, _)| *priority)
                .map(|(_, key)| key);
            let Some(key) = oldest else { return false };
            if !trim_oldest_closed_turn(self, &key).unwrap_or(false) {
                return false;
            }
        }
        true
    }

    /// A reopen can move previously sealed data back into the active capture.
    /// If other branches occupy its scope, keep the routing state and release
    /// optional data until the store is back within the retention budget.
    pub(super) fn limit_scoring_history(&mut self, key: &AgentTurnHistoryKey) {
        let scope = self.entries[key].scope.clone();
        if self.reserve_scoring_bytes(&scope, 0) {
            return;
        }
        let entry = self.entries.get_mut(key).expect("retained entry");
        entry.active.clear_content();
        entry.active.clear_records();
        if let Some(pending) = &mut entry.pending_request {
            pending.has_record = false;
        }
        entry.mark_scope_partial();
        let old = entry.accounted_bytes;
        entry.accounted_bytes = entry_bytes(key, entry);
        let new = entry.accounted_bytes;
        self.replace_accounting(old, new);
        debug_assert!(self.scope_history_bytes(&scope) <= self.scope_history_limit);
    }
}
