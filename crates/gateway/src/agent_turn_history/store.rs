use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering};
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
    CompletedAgentTurn, ExecutionAttribution, PlanSnapshot, TurnDecisionInputs, VisibleAgentTurn,
    VisibleContentPart,
};
use crate::replay::ReplayStore;
use crate::server::core_runtime::model_ir::ModelRequestIRV1;
use crate::server::core_runtime::profiles::BranchDecisionV1;

type HmacSha256 = Hmac<Sha256>;

#[path = "store/gap.rs"]
mod gap;
#[path = "store/output.rs"]
mod output;
#[path = "store/records.rs"]
mod records;
use records::*;
#[path = "store/branches.rs"]
mod branches;
#[path = "store/evaluation.rs"]
mod evaluation;
#[path = "store/maintenance.rs"]
mod maintenance;
#[path = "store/opening.rs"]
mod opening;
#[path = "store/retry.rs"]
mod retry;

pub(crate) const DEFAULT_MAX_MEMORY_BYTES: usize = 500_000_000;
pub(crate) const DEFAULT_IDLE_TTL: Duration = Duration::from_secs(60 * 60);

pub(crate) struct AgentTurnHistoryStore {
    max_memory_bytes: usize,
    idle_ttl: Duration,
    instance_nonce: [u8; 32],
    capture_gap_epochs: [AtomicU64; 256],
    inner: Arc<Mutex<StoreInner>>,
}

pub(super) struct StoreInner {
    entries: BTreeMap<AgentTurnHistoryKey, SessionEntry>,
    scopes: BTreeMap<AgentTurnHistoryKey, BTreeSet<AgentTurnHistoryKey>>,
    next_token: u64,
    next_request_token: u64,
    next_id: u64,
    accounted_bytes: usize,
    peak_accounted_bytes: usize,
    expiry_cursor: Option<AgentTurnHistoryKey>,
}

struct SessionEntry {
    scope: AgentTurnHistoryKey,
    branch_id: String,
    admission_sequence: u64,
    branch_relation: branches::HistoryRelation,
    entry_token: u64,
    revision: u64,
    last_access: Instant,
    history_partial: bool,
    capture_gap_epoch: u64,
    checkpoint: TranscriptCheckpoint,
    response_boundary: Option<super::boundary::BoundaryEvidence>,
    branch_cursor: (Option<[u8; 32]>, Option<[u8; 32]>),
    active: ActiveTurn,
    closed: VecDeque<ClosedTurn>,
    current_segment: Option<SegmentState>,
    referenced_segment: Option<SegmentState>,
    next_turn_ordinal: u64,
    pending_request: Option<u64>,
    snapshot_pins: u32,
    accounted_bytes: usize,
}

pub(crate) struct SnapshotPin {
    inner: Weak<Mutex<StoreInner>>,
    key: AgentTurnHistoryKey,
    entry_token: u64,
    reserved_bytes: usize,
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
        inner.accounted_bytes = inner.accounted_bytes.saturating_sub(self.reserved_bytes);
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
    digest: Option<[u8; 32]>,
    retry_setup: Option<[u8; 32]>,
}

struct CaptureAdmission {
    scope: AgentTurnHistoryKey,
    now: Instant,
    gap_epoch: u64,
}

struct ActiveTurn {
    completed_requests: usize,
    request_ids: Vec<String>,
    request_ids_clipped: bool,
    source_unverified: bool,
    association_unverified: bool,
    opening_cause: hiroute_domain::AgentTurnOpeningCauseV1,
    agent_turn_id: String,
    native_turn: Option<[u8; 32]>,
    ordinal: u64,
    user: Vec<VisibleContentPart>,
    steps: Vec<Vec<VisibleContentPart>>,
    output: output::AcceptedOutput,
    boundary: super::boundary::BoundaryCapture,
    plan: PlanSnapshot,
    decision: Option<BranchDecisionV1>,
    executions: Vec<AcceptedExecution>,
    started_at_ms: u64,
    first_request_id: Option<String>,
    last_request_id: Option<String>,
    last_request_status: AgentTurnStatus,
    request_output_accepted: bool,
    // Provider outcomes are facts about every actual request, independent of
    // whether the routing stage is still waiting for a tool continuation.
    provider_outcomes_known: bool,
    capture_partial: bool,
    finalized: bool,
}

#[derive(Clone)]
struct ClosedTurn {
    decision: BranchDecisionV1,
    wire: Arc<VisibleAgentTurn>,
    completed: CompletedAgentTurn,
    closure_proven: bool,
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
    branch_execution: Option<hiroute_domain::BranchExecutionV1>,
    plan: PlanSnapshot,
    selected_branch_id: String,
    executed_branch_id: String,
    model_configuration_id: Option<String>,
    profile_digest: Option<String>,
    evaluation_scope: Option<hiroute_domain::StageEvaluationScopeV1>,
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
    #[cfg(test)]
    pub(crate) fn with_lock_for_runtime_test<T>(&self, action: impl FnOnce() -> T) -> T {
        let _guard = self.inner.lock();
        action()
    }

    #[cfg(test)]
    pub(crate) fn with_exhausted_request_tokens_for_runtime_test<T>(
        &self,
        action: impl FnOnce() -> T,
    ) -> T {
        let previous = {
            let mut inner = self.inner.lock();
            std::mem::replace(&mut inner.next_request_token, u64::MAX)
        };
        let result = action();
        self.inner.lock().next_request_token = previous;
        result
    }

    #[cfg(test)]
    pub(crate) fn clear_decision_for_runtime_test(&self, ticket: &AgentTurnTicket) {
        let mut inner = self.inner.lock();
        let entry = inner
            .entries
            .get_mut(&ticket.key)
            .expect("retained session");
        assert_eq!(entry.entry_token, ticket.entry_token);
        assert_eq!(entry.active.agent_turn_id, ticket.agent_turn_id);
        entry.active.decision = None;
    }

    pub(crate) fn new(max_memory_bytes: usize, idle_ttl: Duration) -> Self {
        let mut instance_nonce = [0_u8; 32];
        let _ = getrandom::fill(&mut instance_nonce);
        Self {
            max_memory_bytes: max_memory_bytes.max(1),
            idle_ttl,
            instance_nonce,
            capture_gap_epochs: std::array::from_fn(|_| AtomicU64::new(0)),
            inner: Arc::new(Mutex::new(StoreInner {
                entries: BTreeMap::new(),
                scopes: BTreeMap::new(),
                next_token: 1,
                next_request_token: 1,
                next_id: 1,
                accounted_bytes: store_fixed_bytes(),
                peak_accounted_bytes: store_fixed_bytes(),
                expiry_cursor: None,
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
            TurnDecisionInputs {
                native_turn: None,
                history_continuity: crate::context_hold::HistoryContinuity::Continuing,
                reselect_on_user_message: true,
            },
            now,
        )
    }

    #[cfg(test)]
    pub(crate) fn begin_with_context(
        &self,
        key: AgentTurnHistoryKey,
        plan: PlanSnapshot,
        request: &ModelRequestIRV1,
        replay: &ReplayStore,
        decision_inputs: TurnDecisionInputs,
        now: Instant,
    ) -> Result<AgentTurnBegin, AgentTurnHistoryError> {
        self.begin_with_admission(
            key,
            plan,
            Self::prepare_request(request, replay)?,
            decision_inputs,
            now,
            || Some(()),
        )
        .map(|result| result.expect("unconditional test admission").0)
    }

    #[cfg(test)]
    pub(crate) fn prepare_request(
        request: &ModelRequestIRV1,
        replay: &ReplayStore,
    ) -> Result<super::projection::AnalyzedRequest, AgentTurnHistoryError> {
        analyze_request(request, replay)
    }

    pub(crate) fn begin_with_admission<T>(
        &self,
        key: AgentTurnHistoryKey,
        plan: PlanSnapshot,
        analyzed: super::projection::AnalyzedRequest,
        decision_inputs: TurnDecisionInputs,
        now: Instant,
        admit: impl FnOnce() -> Option<T>,
    ) -> Result<Option<(AgentTurnBegin, T)>, AgentTurnHistoryError> {
        if analyzed.turns.is_empty() {
            return Err(AgentTurnHistoryError::NoUserTurn);
        }
        // Contention is not capacity exhaustion. Only this in-memory admission
        // is serialized; projection/Replay reads happened before taking the lock
        // and the request's external execution never owns it.
        let mut inner = self.inner.lock();
        // Include gaps reported while this entrance was waiting. A later gap
        // still advances the fence and cannot be acknowledged by this admission.
        let gap_epoch = self.capture_gap_epoch(&key).load(Ordering::Acquire);
        inner.purge_expired(now, self.idle_ttl, &key);
        self.observe_capture_gap_epoch(&mut inner, &key, gap_epoch);
        // Projection ran before either cache lock. Validate and reserve the
        // checkpoint before changing active turns, tool outcomes or snapshots.
        // The callback must not wait or acquire the history lock again.
        let Some(admission) = admit() else {
            inner.note_capture_gap(&key);
            return Ok(None);
        };
        if !inner.reserve_scope_record(&key) {
            return Err(AgentTurnHistoryError::Resource);
        }
        let selection = self.select_branch(&mut inner, &key, &analyzed, decision_inputs)?;
        let branch_key = selection.key.clone();
        match self.begin_locked(
            &mut inner,
            branch_key.clone(),
            plan,
            analyzed,
            selection.inputs,
            CaptureAdmission {
                scope: selection.scope.clone(),
                now,
                gap_epoch,
            },
        ) {
            Ok(mut begin) => {
                if let Err(error) = self.finish_branch_admission(&mut inner, selection, &mut begin)
                {
                    // No lease has escaped yet. Keep an explicit scope gap and
                    // release only this failed admission's slot.
                    let ticket = match &begin {
                        AgentTurnBegin::NewTurn { ticket, .. }
                        | AgentTurnBegin::Continuation { ticket, .. } => ticket,
                    };
                    self.abort_locked(&mut inner, ticket);
                    inner.note_capture_gap(&key);
                    drop(inner);
                    drop(begin);
                    return Err(error);
                }
                Ok(Some((begin, admission)))
            }
            Err(error) => {
                inner.note_capture_gap(&key);
                Err(error)
            }
        }
    }

    fn begin_locked(
        &self,
        inner: &mut StoreInner,
        key: AgentTurnHistoryKey,
        plan: PlanSnapshot,
        analyzed: super::projection::AnalyzedRequest,
        decision_inputs: TurnDecisionInputs,
        admission: CaptureAdmission,
    ) -> Result<AgentTurnBegin, AgentTurnHistoryError> {
        let CaptureAdmission {
            scope,
            now,
            gap_epoch,
        } = admission;
        self.observe_capture_gap_epoch(inner, &key, gap_epoch);
        let latest = analyzed
            .turns
            .last()
            .ok_or(AgentTurnHistoryError::NoUserTurn)?;

        if !inner.entries.contains_key(&key) {
            if decision_inputs.history_continuity.uses_saved_user() {
                return Err(AgentTurnHistoryError::TurnContextUnavailable);
            }
            // A client may bring older messages when HiRoute first joins a session.
            // That prefix is missing from our history, but does not make the current
            // stage partial unless output after its user message was already missed.
            let missing_current_output = analyzed.output_after_latest_user;
            let missing_history = analyzed.prior_history_missing
                || analyzed.turns.len() > 1
                || missing_current_output;
            let entry_token = inner.allocate_token()?;
            let request_token = inner.allocate_request_token()?;
            let turn_id = self.allocate_id(inner, "turn")?;
            let active = ActiveTurn {
                completed_requests: 0,
                request_ids: Vec::new(),
                request_ids_clipped: false,
                source_unverified: analyzed.user_origin_unverified,
                association_unverified: false,
                opening_cause: match decision_inputs.history_continuity {
                    crate::context_hold::HistoryContinuity::Shortened => {
                        hiroute_domain::AgentTurnOpeningCauseV1::HistoryShortened
                    }
                    crate::context_hold::HistoryContinuity::Rewritten => {
                        hiroute_domain::AgentTurnOpeningCauseV1::HistoryRewrite
                    }
                    _ => hiroute_domain::AgentTurnOpeningCauseV1::NoComparableHistory,
                },
                agent_turn_id: turn_id.clone(),
                native_turn: decision_inputs.native_turn,
                ordinal: 1,
                user: latest.user.clone(),
                steps: Vec::new(),
                output: Default::default(),
                boundary: Default::default(),
                plan,
                decision: None,
                executions: Vec::new(),
                started_at_ms: unix_ms(),
                first_request_id: None,
                last_request_id: None,
                last_request_status: AgentTurnStatus::Unknown,
                request_output_accepted: false,
                provider_outcomes_known: true,
                capture_partial: missing_current_output,
                finalized: false,
            };
            let checkpoint = TranscriptCheckpoint {
                active_user_index: latest.user_index,
                message_count: analyzed.message_count,
                digest: analyzed.anchors.as_ref().map(|a| a.complete),
                retry_setup: analyzed.retry_setup,
            };
            let mut entry = SessionEntry {
                scope: scope.clone(),
                branch_id: hex(&key.0),
                admission_sequence: request_token,
                branch_relation: branches::HistoryRelation::Root,
                entry_token,
                revision: 1,
                last_access: now,
                history_partial: missing_history,
                capture_gap_epoch: gap_epoch,
                checkpoint,
                response_boundary: None,
                branch_cursor: analyzed.branch_cursor,
                active,
                closed: VecDeque::new(),
                current_segment: None,
                referenced_segment: None,
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
            inner.scopes.entry(scope).or_default().insert(key.clone());
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
                allow_affinity: true,
            };
            return Ok(AgentTurnBegin::NewTurn {
                ticket,
                history: Box::new(AgentTurnHistorySnapshot {
                    previous_decision: None,
                    visible_conversation: Vec::new(),
                    history_partial: missing_history,
                    assessment_from: None,
                    assessment_target: None,
                    _pin: None,
                }),
                completed: None,
                inherited_decision: None,
            });
        }

        let entry = inner
            .entries
            .get(&key)
            .ok_or(AgentTurnHistoryError::Integrity)?;
        if decision_inputs.history_continuity.uses_saved_user() {
            let original_user = if entry.active.finalized {
                entry
                    .closed
                    .back()
                    .filter(|turn| turn.completed.agent_turn_id == entry.active.agent_turn_id)
                    .map(|turn| turn.wire.user.as_slice())
            } else {
                Some(entry.active.user.as_slice())
            };
            if decision_inputs.native_turn.is_none()
                || decision_inputs.native_turn != entry.active.native_turn
                || entry.active.plan != plan
                || entry.active.decision.is_none()
                || original_user.is_none_or(|user| user.is_empty())
            {
                return Err(AgentTurnHistoryError::TurnContextUnavailable);
            }
        }
        let missed_user = !decision_inputs.history_continuity.uses_saved_user()
            && decision_inputs.history_continuity.semantic_continuity()
            && analyzed.turns.iter().any(|turn| {
                turn.user_index >= entry.checkpoint.message_count
                    && turn.user_index < latest.user_index
            });
        if missed_user {
            // An optional admission/adapter miss may have executed normally.
            // Native transcripts can expose its intervening user turn when
            // capture resumes; retain accepted output, but close that gap's
            // assessment boundary instead of claiming complete history.
            inner.note_capture_gap(&key);
        }
        let entry = inner
            .entries
            .get(&key)
            .ok_or(AgentTurnHistoryError::Integrity)?;
        let appended_user = if analyzed.measured {
            // An exact response edge permits continuation; a changed native ID
            // does not turn that continuation into a new observed input.
            decision_inputs.history_continuity.semantic_continuity()
                && opening::has_appended_input(&entry.checkpoint, &analyzed)
        } else {
            decision_inputs.history_continuity
                == crate::context_hold::HistoryContinuity::NativeNewTurn
                || matches!(
                    decision_inputs.history_continuity,
                    crate::context_hold::HistoryContinuity::Continuing
                        | crate::context_hold::HistoryContinuity::NativeSameTurn
                ) && latest.user_index > entry.checkpoint.active_user_index
        };
        let inheritable_decision = entry.active.decision.is_some() && entry.active.plan == plan;
        let needs_decision = !inheritable_decision
            || !decision_inputs.history_continuity.semantic_continuity()
            || (appended_user && decision_inputs.reselect_on_user_message);
        let starts_new_turn = appended_user || needs_decision;
        let opening_cause = match decision_inputs.history_continuity {
            crate::context_hold::HistoryContinuity::Shortened => {
                hiroute_domain::AgentTurnOpeningCauseV1::HistoryShortened
            }
            crate::context_hold::HistoryContinuity::Rewritten => {
                hiroute_domain::AgentTurnOpeningCauseV1::HistoryRewrite
            }
            crate::context_hold::HistoryContinuity::NoCheckpoint => {
                hiroute_domain::AgentTurnOpeningCauseV1::NoComparableHistory
            }
            crate::context_hold::HistoryContinuity::NativeNewTurn
            | crate::context_hold::HistoryContinuity::Continuing
                if appended_user =>
            {
                hiroute_domain::AgentTurnOpeningCauseV1::UserMessage
            }
            crate::context_hold::HistoryContinuity::Continuing
            | crate::context_hold::HistoryContinuity::NativeSameTurn
            | crate::context_hold::HistoryContinuity::NativeContinuation
            | crate::context_hold::HistoryContinuity::NativeNewTurn => {
                hiroute_domain::AgentTurnOpeningCauseV1::Other
            }
        };
        let reset_branch_context = !decision_inputs.history_continuity.semantic_continuity()
            && entry
                .active
                .decision
                .as_ref()
                .is_some_and(|decision| decision.policy.is_some());
        let inherited_decision = (!needs_decision)
            .then(|| entry.active.decision.clone())
            .flatten();

        let request_token = inner.allocate_request_token()?;
        if !starts_new_turn {
            let (decision, ticket, old_bytes, new_bytes) = {
                let entry = inner
                    .entries
                    .get_mut(&key)
                    .ok_or(AgentTurnHistoryError::Integrity)?;
                let Some(decision) = entry.active.continuation_decision() else {
                    return Err(AgentTurnHistoryError::TurnContextUnavailable);
                };
                entry.revision = entry
                    .revision
                    .checked_add(1)
                    .ok_or(AgentTurnHistoryError::Resource)?;
                entry.last_access = now;
                entry.pending_request = Some(request_token);
                entry.admission_sequence = request_token;
                if entry.active.finalized {
                    output::reopen_finalized(entry, &latest.user);
                }
                if decision_inputs.history_continuity.retains_message_prefix() {
                    output::apply_tool_results(
                        &mut entry.active,
                        &analyzed.tool_results,
                        entry.checkpoint.message_count,
                    );
                }
                entry.branch_cursor = analyzed.branch_cursor;
                if analyzed.measured
                    && let Some(native_turn) = decision_inputs.native_turn
                {
                    // Exact continuation proves the execution association. Keep
                    // its latest native hint without changing stage or input.
                    entry.active.native_turn = Some(native_turn);
                }
                entry.active.output.next_step();
                entry.active.boundary = Default::default();
                entry.response_boundary = None;
                entry.active.last_request_status = AgentTurnStatus::Unknown;
                entry.active.request_output_accepted = false;
                entry.checkpoint = TranscriptCheckpoint {
                    active_user_index: latest.user_index,
                    message_count: analyzed.message_count,
                    digest: analyzed.anchors.as_ref().map(|a| a.complete),
                    retry_setup: analyzed.retry_setup,
                };
                let ticket = AgentTurnTicket {
                    key: key.clone(),
                    entry_token: entry.entry_token,
                    revision: entry.revision,
                    request_token,
                    agent_turn_id: entry.active.agent_turn_id.clone(),
                    new_turn: false,
                    allow_affinity: true,
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
                let changed = trim_oldest_closed_turn(inner, &key)?;
                if !changed {
                    self.abort_locked(inner, &ticket);
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
            .then(|| self.allocate_id(inner, "segment"))
            .transpose()?;
        let completed = {
            let entry = inner
                .entries
                .get_mut(&key)
                .ok_or(AgentTurnHistoryError::Integrity)?;
            if decision_inputs.history_continuity
                == crate::context_hold::HistoryContinuity::Continuing
            {
                output::apply_tool_results(
                    &mut entry.active,
                    &analyzed.tool_results,
                    entry.checkpoint.message_count,
                );
            }
            if let Some(next_segment_id) = next_segment_id {
                // A new activation closes this routing stage, not the logical task.
                // Only a witnessed interruption may claim Interrupted.
                let closure = if appended_user {
                    StageClosure::NewActivation
                } else if entry.active.plan != plan {
                    StageClosure::ConfigurationChanged
                } else {
                    StageClosure::AssociationLost
                };
                Some(close_active(entry, next_segment_id, unix_ms(), closure)?)
            } else {
                None
            }
        };
        // Unverified native input may rewrite its prefix on every question.
        // Assess only the actual stage we captured before that rewrite. Keep
        // the new segment separate and never restore a segment fenced by a gap.
        let observed_assessment_segment = if analyzed.user_origin_unverified && reset_branch_context
        {
            inner
                .entries
                .get(&key)
                .filter(|entry| {
                    entry
                        .closed
                        .back()
                        .is_some_and(|turn| turn.completed.status == AgentTurnStatus::Completed)
                })
                .and_then(|entry| entry.current_segment.clone())
        } else {
            None
        };
        let turn_id = self.allocate_id(inner, "turn")?;
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
            entry.admission_sequence = request_token;
            if reset_branch_context {
                // Keep visible history, but a new context cannot extend the old
                // task's competence segment, even when it starts on the same model.
                entry.current_segment = None;
            }
            entry.branch_cursor = analyzed.branch_cursor;
            entry.response_boundary = None;
            entry.active = ActiveTurn {
                completed_requests: 0,
                request_ids: Vec::new(),
                request_ids_clipped: false,
                source_unverified: analyzed.user_origin_unverified,
                association_unverified: false,
                opening_cause,
                agent_turn_id: turn_id.clone(),
                native_turn: decision_inputs.native_turn,
                ordinal,
                user: latest.user.clone(),
                steps: Vec::new(),
                output: Default::default(),
                boundary: Default::default(),
                plan,
                decision: None,
                executions: Vec::new(),
                started_at_ms: unix_ms(),
                first_request_id: None,
                last_request_id: None,
                last_request_status: AgentTurnStatus::Unknown,
                request_output_accepted: false,
                provider_outcomes_known: true,
                capture_partial: false,
                finalized: false,
            };
            entry.checkpoint = TranscriptCheckpoint {
                active_user_index: latest.user_index,
                message_count: analyzed.message_count,
                digest: analyzed.anchors.as_ref().map(|a| a.complete),
                retry_setup: analyzed.retry_setup,
            };
            let ticket = AgentTurnTicket {
                key: key.clone(),
                entry_token: entry.entry_token,
                revision: entry.revision,
                request_token,
                agent_turn_id: turn_id,
                new_turn: true,
                allow_affinity: true,
            };
            let old_bytes = entry.accounted_bytes;
            entry.accounted_bytes = entry_bytes(&key, entry);
            (ticket, old_bytes, entry.accounted_bytes)
        };
        inner.replace_accounting(old_bytes, new_bytes);
        inner.evict_to_budget(self.max_memory_bytes, Some(&key));
        inner.note_peak();
        while inner.accounted_bytes > self.max_memory_bytes {
            if !trim_oldest_closed_turn(inner, &key)? {
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
            self.abort_locked(inner, &ticket);
            return Err(AgentTurnHistoryError::Resource);
        }
        if let Some(inherited_decision) = inherited_decision {
            return Ok(AgentTurnBegin::NewTurn {
                ticket,
                history: Box::new(AgentTurnHistorySnapshot {
                    previous_decision: None,
                    visible_conversation: Vec::new(),
                    history_partial: true,
                    assessment_from: None,
                    assessment_target: None,
                    _pin: None,
                }),
                completed: completed.map(Box::new),
                inherited_decision: Some(inherited_decision),
            });
        }
        let history = {
            let mut history = self
                .pinned_snapshot(inner, &key, observed_assessment_segment.as_ref())
                .unwrap_or_else(|| AgentTurnHistorySnapshot::empty(true));
            let entry = inner
                .entries
                .get(&key)
                .ok_or(AgentTurnHistoryError::Resource)?;
            let new_branch_task = !decision_inputs.history_continuity.semantic_continuity()
                && history
                    .previous_decision
                    .as_ref()
                    .is_some_and(|(_, decision)| decision.policy.is_some());
            let new_version = history
                .previous_decision
                .as_ref()
                .is_some_and(|(_, decision)| decision.policy.is_some())
                && history
                    .assessment_target
                    .as_ref()
                    .is_some_and(|target| target.plan != entry.active.plan);
            if (new_branch_task && observed_assessment_segment.is_none()) || new_version {
                history.assessment_from = None;
                history.assessment_target = None;
            }
            if new_branch_task {
                history.previous_decision = None;
            }
            history
        };
        Ok(AgentTurnBegin::NewTurn {
            ticket,
            history: Box::new(history),
            completed: completed.map(Box::new),
            inherited_decision: None,
        })
    }

    pub(crate) fn commit_decision(
        &self,
        ticket: &AgentTurnTicket,
        decision: BranchDecisionV1,
    ) -> Result<(), AgentTurnHistoryError> {
        let mut inner = self
            .inner
            .try_lock()
            .ok_or(AgentTurnHistoryError::Resource)?;
        self.commit_decision_locked(&mut inner, ticket, decision)
    }

    /// Only called after downstream delivery has ended. This uses the same
    /// bounded store lock as finalization, never a retry loop on the output path.
    pub(crate) fn commit_decision_after_delivery(
        &self,
        ticket: &AgentTurnTicket,
        decision: BranchDecisionV1,
    ) -> Result<(), AgentTurnHistoryError> {
        self.commit_decision_locked(&mut self.inner.lock(), ticket, decision)
    }

    fn commit_decision_locked(
        &self,
        inner: &mut StoreInner,
        ticket: &AgentTurnTicket,
        decision: BranchDecisionV1,
    ) -> Result<(), AgentTurnHistoryError> {
        let entry = matching_entry_mut(inner, ticket)?;
        if !ticket.new_turn || entry.active.decision.is_some() {
            return Err(AgentTurnHistoryError::Integrity);
        }
        let charge = decision_bytes(&decision);
        inner.evict_to_budget(
            self.max_memory_bytes.saturating_sub(charge),
            Some(&ticket.key),
        );
        if inner.accounted_bytes.saturating_add(charge) > self.max_memory_bytes {
            return Err(AgentTurnHistoryError::Resource);
        }
        let entry = matching_entry_mut(inner, ticket)?;
        let old = entry.accounted_bytes;
        entry.active.decision = Some(decision);
        entry.accounted_bytes = entry_bytes(&ticket.key, entry);
        let new = entry.accounted_bytes;
        inner.replace_accounting(old, new);
        inner.note_peak();
        Ok(())
    }

    pub(crate) fn bind_request(
        &self,
        ticket: &AgentTurnTicket,
        request_id: &str,
    ) -> Option<hiroute_domain::AgentTurnAdmissionV1> {
        let mut inner = self.inner.lock();
        let charge = request_id.len().saturating_mul(3).saturating_add(256);
        inner.evict_to_budget(
            self.max_memory_bytes.saturating_sub(charge),
            Some(&ticket.key),
        );
        let fits = inner.accounted_bytes.saturating_add(charge) <= self.max_memory_bytes;
        let entry = matching_entry_mut(&mut inner, ticket).ok()?;
        // Admission identity exists independently of bounded membership storage,
        // decision success and the optional affinity cache. Clipping the index
        // must not erase which actual stage admitted this request.
        let admission = hiroute_domain::AgentTurnAdmissionV1 {
            version: 1,
            agent_turn_id: entry.active.agent_turn_id.clone(),
            branch_id: entry.branch_id.clone(),
            revision: entry.revision,
            ordinal: entry.active.ordinal,
            opening_cause: entry.active.opening_cause,
        };
        if !fits || entry.active.request_ids.len() >= 128 {
            entry.active.request_ids_clipped = true;
            return Some(admission);
        }
        entry.active.request_ids.push(request_id.to_owned());
        entry
            .active
            .first_request_id
            .get_or_insert_with(|| request_id.to_owned());
        entry.active.last_request_id = Some(request_id.to_owned());
        let old = entry.accounted_bytes;
        entry.accounted_bytes = entry_bytes(&ticket.key, entry);
        let new = entry.accounted_bytes;
        inner.replace_accounting(old, new);
        inner.note_peak();
        Some(admission)
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
        let charge = executions
            .iter()
            .map(|e| {
                e.model_configuration_id.len()
                    + e.profile_digest.len()
                    + e.executed_branch_id.len()
                    + e.request_id.len()
                    + 512
            })
            .sum::<usize>()
            .saturating_mul(3)
            .saturating_add(request_id.len() * 3 + 4096);
        inner.evict_to_budget(
            self.max_memory_bytes.saturating_sub(charge),
            Some(&ticket.key),
        );
        if inner.accounted_bytes.saturating_add(charge) > self.max_memory_bytes {
            let entry = matching_entry_mut(&mut inner, ticket)?;
            entry.pending_request = None;
            inner.note_capture_gap(&ticket.key);
            return Err(AgentTurnHistoryError::Resource);
        }
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
            if entry.active.request_ids.contains(&request_id) {
                // The Gateway identifier was already bound before classification.
            } else if entry.active.request_ids.len() < 128 {
                entry.active.request_ids.push(request_id.clone());
            } else {
                entry.active.request_ids_clipped = true;
            }
            entry.active.last_request_id = Some(request_id);
            entry.active.provider_outcomes_known &= status != AgentTurnStatus::Unknown;
            entry.active.last_request_status = if terminal {
                status
            } else {
                AgentTurnStatus::Unknown
            };
            entry.active.completed_requests += 1;
            entry.active.executions.extend(executions);
            entry.response_boundary = (!entry.active.capture_partial)
                .then(|| entry.active.boundary.evidence())
                .flatten();
            let completed = if let Some(next_segment_id) = next_segment_id {
                Some(close_active(
                    entry,
                    next_segment_id,
                    unix_ms(),
                    StageClosure::RequestTerminal,
                )?)
            } else {
                None
            };
            let old = entry.accounted_bytes;
            entry.accounted_bytes = entry_bytes(&ticket.key, entry);
            (old, entry.accounted_bytes, completed)
        };
        inner.replace_accounting(old, new);
        let scope = inner.entries.get(&ticket.key).unwrap().scope.clone();
        inner.limit_terminal_records(&scope);
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
            // An externally dropped request cannot drain its capture worker.
            // Keep the accepted prefix, but never advertise a complete turn.
            inner.note_capture_gap(&ticket.key);
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

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
