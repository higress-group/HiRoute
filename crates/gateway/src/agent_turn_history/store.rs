use std::collections::{BTreeMap, VecDeque};
use std::mem::size_of;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use parking_lot::Mutex;
use sha2::Sha256;

use super::projection::analyze_request;
#[cfg(test)]
use super::projection::project_user;
use super::{
    AcceptedExecution, AgentTurnBegin, AgentTurnHistoryError, AgentTurnHistoryKey,
    AgentTurnHistorySnapshot, AgentTurnStatus, AgentTurnTicket, AssessmentTarget,
    CompletedAgentTurn, ContextDecisionFacts, ExecutionAttribution, PlanSnapshot, VisibleAgentTurn,
    VisibleContentPart,
};
use crate::replay::ReplayStore;
use crate::server::core_runtime::model_ir::ModelRequestIRV1;
use crate::server::core_runtime::profiles::BranchDecisionV1;

type HmacSha256 = Hmac<Sha256>;

#[path = "store/output.rs"]
mod output;

pub(crate) const DEFAULT_MAX_MEMORY_BYTES: usize = 500_000_000;
pub(crate) const DEFAULT_IDLE_TTL: Duration = Duration::from_secs(60 * 60);

pub(crate) struct AgentTurnHistoryStore {
    max_memory_bytes: usize,
    idle_ttl: Duration,
    instance_nonce: [u8; 32],
    inner: Arc<Mutex<StoreInner>>,
}

pub(super) struct StoreInner {
    entries: BTreeMap<AgentTurnHistoryKey, SessionEntry>,
    next_token: u64,
    next_request_token: u64,
    next_id: u64,
    accounted_bytes: usize,
    peak_accounted_bytes: usize,
}

struct SessionEntry {
    entry_token: u64,
    revision: u64,
    last_access: Instant,
    history_partial: bool,
    checkpoint: TranscriptCheckpoint,
    active: ActiveTurn,
    closed: VecDeque<ClosedTurn>,
    current_segment: Option<SegmentState>,
    next_turn_ordinal: u64,
    pending_request: Option<u64>,
    snapshot_pins: u32,
    accounted_bytes: usize,
}

pub(crate) struct SnapshotPin {
    inner: Weak<Mutex<StoreInner>>,
    key: AgentTurnHistoryKey,
    entry_token: u64,
}

impl std::fmt::Debug for SnapshotPin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SnapshotPin")
            .finish_non_exhaustive()
    }
}

impl Drop for SnapshotPin {
    fn drop(&mut self) {
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        let mut inner = inner.lock();
        if let Some(entry) = inner.entries.get_mut(&self.key)
            && entry.entry_token == self.entry_token
        {
            entry.snapshot_pins = entry.snapshot_pins.saturating_sub(1);
        }
    }
}

#[derive(Clone)]
struct TranscriptCheckpoint {
    active_user_index: usize,
    message_count: usize,
}

struct ActiveTurn {
    agent_turn_id: String,
    ordinal: u64,
    user: Vec<VisibleContentPart>,
    steps: Vec<Vec<VisibleContentPart>>,
    output: output::AcceptedOutput,
    plan: PlanSnapshot,
    decision: Option<BranchDecisionV1>,
    executions: Vec<AcceptedExecution>,
    started_at_ms: u64,
    first_request_id: Option<String>,
    last_request_id: Option<String>,
    last_request_status: AgentTurnStatus,
    capture_partial: bool,
    finalized: bool,
}

struct ClosedTurn {
    wire: Arc<VisibleAgentTurn>,
    completed: CompletedAgentTurn,
}

#[derive(Clone)]
struct SegmentState {
    segment_id: String,
    key: SegmentKey,
    first_turn_id: String,
    through_turn_id: String,
    first_ordinal: u64,
    through_ordinal: u64,
    partial: bool,
}

#[derive(Clone, Eq, PartialEq)]
struct SegmentKey {
    plan: PlanSnapshot,
    selected_branch_id: String,
    executed_branch_id: String,
    model_configuration_id: String,
    profile_digest: String,
}

impl Default for AgentTurnHistoryStore {
    fn default() -> Self {
        let max_memory_bytes = std::env::var("HIROUTE_AGENT_TURN_HISTORY_MAX_BYTES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_MAX_MEMORY_BYTES);
        let idle_ttl = std::env::var("HIROUTE_AGENT_TURN_HISTORY_IDLE_TTL_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_IDLE_TTL);
        Self::new(max_memory_bytes, idle_ttl)
    }
}

impl AgentTurnHistoryStore {
    pub(crate) fn new(max_memory_bytes: usize, idle_ttl: Duration) -> Self {
        let mut instance_nonce = [0_u8; 32];
        let _ = getrandom::fill(&mut instance_nonce);
        Self {
            max_memory_bytes: max_memory_bytes.max(1),
            idle_ttl,
            instance_nonce,
            inner: Arc::new(Mutex::new(StoreInner {
                entries: BTreeMap::new(),
                next_token: 1,
                next_request_token: 1,
                next_id: 1,
                accounted_bytes: store_fixed_bytes(),
                peak_accounted_bytes: store_fixed_bytes(),
            })),
        }
    }

    pub(crate) fn scope_key(
        &self,
        workspace_id: &str,
        session_id: &str,
    ) -> Result<AgentTurnHistoryKey, AgentTurnHistoryError> {
        let mut mac = HmacSha256::new_from_slice(&self.instance_nonce)
            .map_err(|_| AgentTurnHistoryError::Integrity)?;
        mac.update(b"agent-turn-scope/v1");
        mac.update(&(workspace_id.len() as u64).to_be_bytes());
        mac.update(workspace_id.as_bytes());
        mac.update(&(session_id.len() as u64).to_be_bytes());
        mac.update(session_id.as_bytes());
        Ok(AgentTurnHistoryKey(mac.finalize().into_bytes().into()))
    }

    #[cfg(test)]
    pub(crate) fn begin(
        &self,
        key: AgentTurnHistoryKey,
        plan: PlanSnapshot,
        request: &ModelRequestIRV1,
        replay: &ReplayStore,
        now: Instant,
    ) -> Result<AgentTurnBegin, AgentTurnHistoryError> {
        self.begin_with_context(
            key,
            plan,
            request,
            replay,
            ContextDecisionFacts {
                history_continues: true,
                has_hold_preference: true,
            },
            now,
        )
    }

    pub(crate) fn begin_with_context(
        &self,
        key: AgentTurnHistoryKey,
        plan: PlanSnapshot,
        request: &ModelRequestIRV1,
        replay: &ReplayStore,
        context: ContextDecisionFacts,
        now: Instant,
    ) -> Result<AgentTurnBegin, AgentTurnHistoryError> {
        let analyzed = analyze_request(request, replay)?;
        let latest = analyzed
            .turns
            .last()
            .ok_or(AgentTurnHistoryError::Integrity)?;
        let mut inner = self.inner.lock();
        inner.purge_expired(now, self.idle_ttl);
        if inner
            .entries
            .get(&key)
            .is_some_and(|entry| entry.pending_request.is_some())
        {
            return Err(AgentTurnHistoryError::SessionTurnConflict);
        }

        if !inner.entries.contains_key(&key) {
            let missing_history =
                analyzed.turns.len() > 1 || analyzed.message_count > latest.user_index + 1;
            let entry_token = inner.allocate_token()?;
            let request_token = inner.allocate_request_token()?;
            let turn_id = self.allocate_id(&mut inner, "turn")?;
            let active = ActiveTurn {
                agent_turn_id: turn_id.clone(),
                ordinal: 1,
                user: latest.user.clone(),
                steps: Vec::new(),
                output: Default::default(),
                plan,
                decision: None,
                executions: Vec::new(),
                started_at_ms: unix_ms(),
                first_request_id: None,
                last_request_id: None,
                last_request_status: AgentTurnStatus::Unknown,
                capture_partial: missing_history,
                finalized: false,
            };
            let checkpoint = TranscriptCheckpoint {
                active_user_index: latest.user_index,
                message_count: analyzed.message_count,
            };
            let mut entry = SessionEntry {
                entry_token,
                revision: 1,
                last_access: now,
                history_partial: missing_history,
                checkpoint,
                active,
                closed: VecDeque::new(),
                current_segment: None,
                next_turn_ordinal: 2,
                pending_request: Some(request_token),
                snapshot_pins: 0,
                accounted_bytes: 0,
            };
            entry.accounted_bytes = entry_bytes(&key, &entry);
            if entry.accounted_bytes.saturating_add(store_fixed_bytes()) > self.max_memory_bytes {
                entry.active.user.clear();
                entry.active.steps.clear();
                entry.active.output = Default::default();
                entry.active.capture_partial = true;
                entry.history_partial = true;
                entry.accounted_bytes = entry_bytes(&key, &entry);
            }
            if entry.accounted_bytes.saturating_add(store_fixed_bytes()) > self.max_memory_bytes {
                return Err(AgentTurnHistoryError::Resource);
            }
            inner.accounted_bytes = inner.accounted_bytes.saturating_add(entry.accounted_bytes);
            inner.entries.insert(key.clone(), entry);
            inner.evict_to_budget(self.max_memory_bytes, Some(&key));
            if inner.accounted_bytes > self.max_memory_bytes {
                inner.remove(&key);
                return Err(AgentTurnHistoryError::Resource);
            }
            inner.note_peak();
            let ticket = AgentTurnTicket {
                key,
                entry_token,
                revision: 1,
                request_token,
                agent_turn_id: turn_id,
                new_turn: true,
            };
            return Ok(AgentTurnBegin::NewTurn {
                ticket,
                history: AgentTurnHistorySnapshot {
                    visible_conversation: Vec::new(),
                    history_partial: missing_history,
                    assessment_from: None,
                    assessment_target: None,
                    _pin: None,
                },
                completed: None,
            });
        }

        let entry = inner
            .entries
            .get(&key)
            .ok_or(AgentTurnHistoryError::Integrity)?;
        let appended_user =
            context.history_continues && latest.user_index > entry.checkpoint.active_user_index;
        let inheritable_decision = entry.active.decision.is_some() && entry.active.plan == plan;
        let needs_decision = !inheritable_decision || appended_user || !context.has_hold_preference;

        let request_token = inner.allocate_request_token()?;
        if !needs_decision {
            let (decision, ticket, old_bytes, new_bytes) = {
                let entry = inner
                    .entries
                    .get_mut(&key)
                    .ok_or(AgentTurnHistoryError::Integrity)?;
                let Some(decision) = entry.active.decision.clone() else {
                    return Err(AgentTurnHistoryError::TurnContextUnavailable);
                };
                entry.revision = entry
                    .revision
                    .checked_add(1)
                    .ok_or(AgentTurnHistoryError::Resource)?;
                entry.last_access = now;
                entry.pending_request = Some(request_token);
                if entry.active.finalized {
                    output::reopen_finalized(entry, &latest.user);
                }
                output::apply_tool_results(
                    &mut entry.active,
                    &analyzed.tool_results,
                    entry.checkpoint.message_count,
                );
                entry.active.output.next_step();
                entry.active.last_request_status = AgentTurnStatus::Unknown;
                entry.checkpoint = TranscriptCheckpoint {
                    active_user_index: latest.user_index,
                    message_count: analyzed.message_count,
                };
                let ticket = AgentTurnTicket {
                    key: key.clone(),
                    entry_token: entry.entry_token,
                    revision: entry.revision,
                    request_token,
                    agent_turn_id: entry.active.agent_turn_id.clone(),
                    new_turn: false,
                };
                let old_bytes = entry.accounted_bytes;
                entry.accounted_bytes = entry_bytes(&key, entry);
                (decision, ticket, old_bytes, entry.accounted_bytes)
            };
            inner.replace_accounting(old_bytes, new_bytes);
            inner.evict_to_budget(self.max_memory_bytes, Some(&key));
            if inner.accounted_bytes > self.max_memory_bytes {
                let (old_bytes, new_bytes) = {
                    let entry = inner
                        .entries
                        .get_mut(&key)
                        .ok_or(AgentTurnHistoryError::Resource)?;
                    entry.active.steps.clear();
                    entry.active.output = Default::default();
                    entry.active.capture_partial = true;
                    entry.history_partial = true;
                    let old_bytes = entry.accounted_bytes;
                    entry.accounted_bytes = entry_bytes(&key, entry);
                    (old_bytes, entry.accounted_bytes)
                };
                inner.replace_accounting(old_bytes, new_bytes);
            }
            while inner.accounted_bytes > self.max_memory_bytes {
                let changed = trim_oldest_closed_turn(&mut inner, &key)?;
                if !changed {
                    self.abort_locked(&mut inner, &ticket);
                    return Err(AgentTurnHistoryError::Resource);
                }
            }
            inner.note_peak();
            return Ok(AgentTurnBegin::Continuation { ticket, decision });
        }

        let close_previous = inner.entries.get(&key).is_some_and(|entry| {
            !entry.active.finalized
                && !active_is_empty(&entry.active)
                && entry.active.decision.is_some()
        });
        let next_segment_id = close_previous
            .then(|| self.allocate_id(&mut inner, "segment"))
            .transpose()?;
        let completed = {
            let entry = inner
                .entries
                .get_mut(&key)
                .ok_or(AgentTurnHistoryError::Integrity)?;
            if context.history_continues {
                output::apply_tool_results(
                    &mut entry.active,
                    &analyzed.tool_results,
                    entry.checkpoint.message_count,
                );
            }
            if let Some(next_segment_id) = next_segment_id {
                if appended_user && entry.active.last_request_status == AgentTurnStatus::Unknown {
                    entry.active.last_request_status = AgentTurnStatus::Interrupted;
                }
                Some(close_active(entry, next_segment_id, unix_ms())?)
            } else {
                None
            }
        };
        let turn_id = self.allocate_id(&mut inner, "turn")?;
        let (ticket, old_bytes, new_bytes) = {
            let entry = inner
                .entries
                .get_mut(&key)
                .ok_or(AgentTurnHistoryError::Integrity)?;
            let ordinal = if !entry.active.finalized && active_is_empty(&entry.active) {
                entry.active.ordinal
            } else {
                let ordinal = entry.next_turn_ordinal;
                entry.next_turn_ordinal = ordinal
                    .checked_add(1)
                    .ok_or(AgentTurnHistoryError::Resource)?;
                ordinal
            };
            entry.revision = entry
                .revision
                .checked_add(1)
                .ok_or(AgentTurnHistoryError::Resource)?;
            entry.last_access = now;
            entry.pending_request = Some(request_token);
            entry.active = ActiveTurn {
                agent_turn_id: turn_id.clone(),
                ordinal,
                user: latest.user.clone(),
                steps: Vec::new(),
                output: Default::default(),
                plan,
                decision: None,
                executions: Vec::new(),
                started_at_ms: unix_ms(),
                first_request_id: None,
                last_request_id: None,
                last_request_status: AgentTurnStatus::Unknown,
                capture_partial: false,
                finalized: false,
            };
            entry.checkpoint = TranscriptCheckpoint {
                active_user_index: latest.user_index,
                message_count: analyzed.message_count,
            };
            let ticket = AgentTurnTicket {
                key: key.clone(),
                entry_token: entry.entry_token,
                revision: entry.revision,
                request_token,
                agent_turn_id: turn_id,
                new_turn: true,
            };
            let old_bytes = entry.accounted_bytes;
            entry.accounted_bytes = entry_bytes(&key, entry);
            (ticket, old_bytes, entry.accounted_bytes)
        };
        inner.replace_accounting(old_bytes, new_bytes);
        inner.evict_to_budget(self.max_memory_bytes, Some(&key));
        inner.note_peak();
        while inner.accounted_bytes > self.max_memory_bytes {
            if !trim_oldest_closed_turn(&mut inner, &key)? {
                break;
            }
        }
        if inner.accounted_bytes > self.max_memory_bytes {
            let (old_bytes, new_bytes) = {
                let entry = inner
                    .entries
                    .get_mut(&key)
                    .ok_or(AgentTurnHistoryError::Resource)?;
                entry.active.user.clear();
                entry.active.steps.clear();
                entry.active.output = Default::default();
                entry.active.capture_partial = true;
                entry.history_partial = true;
                let old_bytes = entry.accounted_bytes;
                entry.accounted_bytes = entry_bytes(&key, entry);
                (old_bytes, entry.accounted_bytes)
            };
            inner.replace_accounting(old_bytes, new_bytes);
        }
        if inner.accounted_bytes > self.max_memory_bytes {
            self.abort_locked(&mut inner, &ticket);
            return Err(AgentTurnHistoryError::Resource);
        }
        let pin = {
            let entry = inner
                .entries
                .get_mut(&key)
                .ok_or(AgentTurnHistoryError::Resource)?;
            entry.snapshot_pins = entry
                .snapshot_pins
                .checked_add(1)
                .ok_or(AgentTurnHistoryError::Resource)?;
            Arc::new(SnapshotPin {
                inner: Arc::downgrade(&self.inner),
                key: key.clone(),
                entry_token: entry.entry_token,
            })
        };
        let history = {
            let entry = inner
                .entries
                .get(&key)
                .ok_or(AgentTurnHistoryError::Resource)?;
            snapshot(entry, pin)
        };
        Ok(AgentTurnBegin::NewTurn {
            ticket,
            history,
            completed: completed.map(Box::new),
        })
    }

    pub(crate) fn commit_decision(
        &self,
        ticket: &AgentTurnTicket,
        decision: BranchDecisionV1,
    ) -> Result<(), AgentTurnHistoryError> {
        let mut inner = self.inner.lock();
        let entry = matching_entry_mut(&mut inner, ticket)?;
        if !ticket.new_turn || entry.active.decision.is_some() {
            return Err(AgentTurnHistoryError::Integrity);
        }
        entry.active.decision = Some(decision);
        Ok(())
    }

    pub(crate) fn finish_request(
        &self,
        ticket: &AgentTurnTicket,
        request_id: String,
        status: AgentTurnStatus,
        executions: Vec<AcceptedExecution>,
        terminal: bool,
        now: Instant,
    ) -> Result<Option<CompletedAgentTurn>, AgentTurnHistoryError> {
        let mut inner = self.inner.lock();
        let next_segment_id = terminal
            .then(|| self.allocate_id(&mut inner, "segment"))
            .transpose()?;
        let (old, new, completed) = {
            let entry = matching_entry_mut(&mut inner, ticket)?;
            if entry.active.decision.is_none() {
                return Err(AgentTurnHistoryError::Integrity);
            }
            entry.pending_request = None;
            entry.last_access = now;
            if entry.active.first_request_id.is_none() {
                entry.active.first_request_id = Some(request_id.clone());
            }
            entry.active.last_request_id = Some(request_id);
            entry.active.last_request_status = if terminal {
                status
            } else {
                AgentTurnStatus::Unknown
            };
            entry.active.executions.extend(executions);
            let completed = if let Some(next_segment_id) = next_segment_id {
                Some(close_active(entry, next_segment_id, unix_ms())?)
            } else {
                None
            };
            let old = entry.accounted_bytes;
            entry.accounted_bytes = entry_bytes(&ticket.key, entry);
            (old, entry.accounted_bytes, completed)
        };
        inner.replace_accounting(old, new);
        inner.evict_to_budget(self.max_memory_bytes, Some(&ticket.key));
        inner.note_peak();
        Ok(completed)
    }

    pub(crate) fn abort(&self, ticket: &AgentTurnTicket) {
        let mut inner = self.inner.lock();
        self.abort_locked(&mut inner, ticket);
    }

    fn abort_locked(&self, inner: &mut StoreInner, ticket: &AgentTurnTicket) {
        let remove = inner.entries.get(&ticket.key).is_some_and(|entry| {
            entry.entry_token == ticket.entry_token
                && entry.revision == ticket.revision
                && entry.pending_request == Some(ticket.request_token)
                && ticket.new_turn
                && entry.active.decision.is_none()
                && entry.closed.is_empty()
        });
        if remove {
            inner.remove(&ticket.key);
            return;
        }
        if let Some(entry) = inner.entries.get_mut(&ticket.key)
            && entry.entry_token == ticket.entry_token
            && entry.revision == ticket.revision
            && entry.pending_request == Some(ticket.request_token)
        {
            entry.pending_request = None;
            if active_is_empty(&entry.active) {
                return;
            }
            // An externally dropped request cannot drain its capture worker.
            // Keep the accepted prefix, but never advertise a complete turn.
            entry.active.capture_partial = true;
            entry.history_partial = true;
        }
    }

    fn allocate_id(
        &self,
        inner: &mut StoreInner,
        kind: &str,
    ) -> Result<String, AgentTurnHistoryError> {
        let value = inner.next_id;
        inner.next_id = value
            .checked_add(1)
            .ok_or(AgentTurnHistoryError::Resource)?;
        opaque_id(kind, &self.instance_nonce, value)
    }
}

fn active_is_empty(active: &ActiveTurn) -> bool {
    active.first_request_id.is_none()
        && active.last_request_id.is_none()
        && active.steps.is_empty()
        && active.executions.is_empty()
}

fn trim_oldest_closed_turn(
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

fn matching_entry_mut<'a>(
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

fn close_active(
    entry: &mut SessionEntry,
    next_segment_id: String,
    finished_at_ms: u64,
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
    let (segment_id, segment_partial) = match &attribution {
        ExecutionAttribution::Single {
            executed_branch_id,
            model_configuration_id,
            profile_digest,
            ..
        } => {
            let segment_key = SegmentKey {
                plan: entry.active.plan.clone(),
                selected_branch_id: selected_branch_id.clone(),
                executed_branch_id: executed_branch_id.clone(),
                model_configuration_id: model_configuration_id.clone(),
                profile_digest: profile_digest.clone(),
            };
            if entry
                .current_segment
                .as_ref()
                .is_some_and(|segment| segment.key == segment_key)
            {
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
        }
        ExecutionAttribution::Mixed | ExecutionAttribution::Unknown => {
            entry.current_segment = None;
            (next_segment_id, true)
        }
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
        wire,
        completed: completed.clone(),
    });
    entry.active.finalized = true;
    entry.active.output = Default::default();
    Ok(completed)
}

fn execution_attribution(
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

fn snapshot(entry: &SessionEntry, pin: Arc<SnapshotPin>) -> AgentTurnHistorySnapshot {
    let visible_conversation = entry
        .closed
        .iter()
        .map(|turn| Arc::clone(&turn.wire))
        .collect::<Vec<_>>();
    let target_segment = entry.current_segment.as_ref();
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
            ExecutionAttribution::Single { .. } => Some(AssessmentTarget {
                segment_id: segment.segment_id.clone(),
                first_turn_id: segment.first_turn_id.clone(),
                through_turn_id: segment.through_turn_id.clone(),
                first_ordinal: segment.first_ordinal,
                through_ordinal: segment.through_ordinal,
                target_partial: segment.partial,
                plan: last.completed.plan.clone(),
                attribution: last.completed.attribution.clone(),
            }),
            ExecutionAttribution::Mixed | ExecutionAttribution::Unknown => None,
        }
    });
    AgentTurnHistorySnapshot {
        visible_conversation,
        history_partial: entry.history_partial,
        assessment_from,
        assessment_target,
        _pin: Some(pin),
    }
}

impl StoreInner {
    fn allocate_token(&mut self) -> Result<u64, AgentTurnHistoryError> {
        let token = self.next_token;
        self.next_token = token
            .checked_add(1)
            .ok_or(AgentTurnHistoryError::Resource)?;
        Ok(token)
    }

    fn allocate_request_token(&mut self) -> Result<u64, AgentTurnHistoryError> {
        let token = self.next_request_token;
        self.next_request_token = token
            .checked_add(1)
            .ok_or(AgentTurnHistoryError::Resource)?;
        Ok(token)
    }

    fn purge_expired(&mut self, now: Instant, ttl: Duration) {
        let expired = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                entry.pending_request.is_none()
                    && entry.snapshot_pins == 0
                    && now.saturating_duration_since(entry.last_access) >= ttl
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in expired {
            self.remove(&key);
        }
    }

    fn evict_to_budget(&mut self, budget: usize, protected: Option<&AgentTurnHistoryKey>) {
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

    fn remove(&mut self, key: &AgentTurnHistoryKey) {
        if let Some(entry) = self.entries.remove(key) {
            self.accounted_bytes = self.accounted_bytes.saturating_sub(entry.accounted_bytes);
        }
    }

    fn replace_accounting(&mut self, old: usize, new: usize) {
        self.accounted_bytes = self.accounted_bytes.saturating_sub(old).saturating_add(new);
    }

    fn note_peak(&mut self) {
        self.peak_accounted_bytes = self.peak_accounted_bytes.max(self.accounted_bytes);
    }
}

fn entry_bytes(key: &AgentTurnHistoryKey, entry: &SessionEntry) -> usize {
    size_of::<SessionEntry>()
        + size_of_val(key)
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
                    + execution.model_configuration_id.capacity()
                    + execution.profile_digest.capacity()
                    + execution.executed_branch_id.capacity()
                    + execution.request_id.capacity()
            })
            .sum::<usize>()
        + entry
            .closed
            .iter()
            .map(|turn| turn.wire.retained_bytes() + completed_bytes(&turn.completed))
            .sum::<usize>()
        + entry.current_segment.as_ref().map_or(0, segment_bytes)
}

fn completed_bytes(value: &CompletedAgentTurn) -> usize {
    size_of::<CompletedAgentTurn>()
        + value.agent_turn_id.capacity()
        + value.segment_id.capacity()
        + value.plan.plan_id.capacity()
        + value.selected_branch_id.capacity()
        + attribution_bytes(&value.attribution)
        + value.first_request_id.as_ref().map_or(0, String::capacity)
        + value.last_request_id.as_ref().map_or(0, String::capacity)
}

fn attribution_bytes(value: &ExecutionAttribution) -> usize {
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

fn segment_bytes(value: &SegmentState) -> usize {
    size_of::<SegmentState>()
        + value.segment_id.capacity()
        + value.first_turn_id.capacity()
        + value.through_turn_id.capacity()
        + value.key.plan.plan_id.capacity()
        + value.key.selected_branch_id.capacity()
        + value.key.executed_branch_id.capacity()
        + value.key.model_configuration_id.capacity()
        + value.key.profile_digest.capacity()
}

fn store_fixed_bytes() -> usize {
    size_of::<AgentTurnHistoryStore>() + size_of::<StoreInner>() + 2_048
}

fn opaque_id(kind: &str, nonce: &[u8; 32], value: u64) -> Result<String, AgentTurnHistoryError> {
    let mut mac =
        HmacSha256::new_from_slice(nonce).map_err(|_| AgentTurnHistoryError::Integrity)?;
    mac.update(kind.as_bytes());
    mac.update(&value.to_be_bytes());
    Ok(format!("{kind}-{}", hex(&mac.finalize().into_bytes())))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut value, "{byte:02x}");
    }
    value
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
