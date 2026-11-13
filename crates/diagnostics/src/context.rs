//! Emission handles and explicit span context.
//!
//! A [`DiagnosticHandle`] may be a no-op (production CLI paths) or attached to one running
//! [`crate::runtime::DiagnosticRuntime`]. Calls are always non-blocking: filtering reads an
//! atomic level, serialization respects the single-record limit, and queue overflow is
//! counted instead of waited on. Span correlation is carried explicitly by
//! [`DiagnosticContext`] instead of `thread_local` state, which does not survive across
//! async tasks.

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::correlation::{CorrelationDomain, Tokenizer};
use crate::event::{DiagnosticEvent, ProcessRole};
use crate::identity::{BootId, CorrelationToken, SessionId, SpanId};
use crate::level::{DiagnosticLevel, LevelSource};
use crate::queue::{PushError, QueueSender, QueueSnapshot};
use crate::record::{Component, DiagnosticRecordV1};

/// Normal per-request Debug budget, shared across every context clone of that request.
pub const REQUEST_DEBUG_BUDGET: u32 = 128;

#[derive(Debug, Default)]
pub struct Counters {
    dropped: AtomicU64,
    rejected: AtomicU64,
    filtered: AtomicU64,
    budget_exhausted: AtomicU64,
    queued: AtomicU64,
}

impl Counters {
    pub fn record(&self, outcome: EmitOutcome) {
        match outcome {
            EmitOutcome::Queued => {
                self.queued.fetch_add(1, Ordering::Relaxed);
            }
            EmitOutcome::Filtered => {
                self.filtered.fetch_add(1, Ordering::Relaxed);
            }
            EmitOutcome::BudgetExhausted => {
                self.budget_exhausted.fetch_add(1, Ordering::Relaxed);
            }
            EmitOutcome::Dropped => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            EmitOutcome::Rejected => {
                self.rejected.fetch_add(1, Ordering::Relaxed);
            }
            EmitOutcome::Unavailable => {}
        }
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn rejected(&self) -> u64 {
        self.rejected.load(Ordering::Relaxed)
    }

    pub fn filtered(&self) -> u64 {
        self.filtered.load(Ordering::Relaxed)
    }

    pub fn budget_exhausted(&self) -> u64 {
        self.budget_exhausted.load(Ordering::Relaxed)
    }

    pub fn queued(&self) -> u64 {
        self.queued.load(Ordering::Relaxed)
    }
}

/// Outcome of one `try_emit`/`emit` call. Call sites normally ignore it; tests assert it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitOutcome {
    /// The record was queued for the writer.
    Queued,
    /// The event severity is above the current threshold.
    Filtered,
    /// The per-request Debug budget is spent; only Debug events are affected.
    BudgetExhausted,
    /// The queue is at its event or byte limit; the record was not queued.
    Dropped,
    /// The record could not be encoded within the single-record limit.
    Rejected,
    /// No runtime is attached (no-op handle).
    Unavailable,
}

#[derive(Debug)]
pub struct Emitter {
    queue: QueueSender,
    level: AtomicU8,
    revision: AtomicU64,
    component: Component,
    role: ProcessRole,
    boot_id: BootId,
    parent_session_id: Option<SessionId>,
    /// The correlation key may be read from disk after the emit plane exists; until then
    /// tokens stay null instead of falling back to an unsalted identifier.
    tokenizer: Arc<OnceLock<Tokenizer>>,
    started: Instant,
    sequence: Arc<AtomicU64>,
    counters: Arc<Counters>,
}

impl Emitter {
    /// An emitter whose correlation key is not available yet. Events are still encoded with
    /// null tokens and the key becomes usable as soon as it is read. `level` is `None` until
    /// the persisted level has been read: nothing is filtered by a guessed default, and the
    /// first real level also judges the records still waiting in the queue.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        queue: QueueSender,
        level: Option<DiagnosticLevel>,
        component: Component,
        role: ProcessRole,
        boot_id: BootId,
        parent_session_id: Option<SessionId>,
        started: Instant,
        sequence: Arc<AtomicU64>,
        counters: Arc<Counters>,
    ) -> Self {
        Self {
            queue,
            level: AtomicU8::new(level.map_or(LEVEL_UNKNOWN, encode_level)),
            revision: AtomicU64::new(0),
            component,
            role,
            boot_id,
            parent_session_id,
            tokenizer: Arc::new(OnceLock::new()),
            started,
            sequence,
            counters,
        }
    }

    /// Install the correlation key once it has been read; later calls are ignored.
    pub(crate) fn set_tokenizer(&self, tokenizer: Tokenizer) {
        let _ = self.tokenizer.set(tokenizer);
    }

    /// Install the level this process will use. The first call ends the "no level known yet"
    /// window and drops the queued records the level does not admit, so a record admitted
    /// under no known level is never judged more loosely than the process it belongs to.
    pub fn set_level(&self, level: DiagnosticLevel, revision: u64) {
        let previous = self.level.swap(encode_level(level), Ordering::SeqCst);
        if previous == LEVEL_UNKNOWN {
            let discarded = self.queue.discard_below(level);
            if discarded > 0 {
                self.counters
                    .filtered
                    .fetch_add(discarded as u64, Ordering::Relaxed);
            }
        }
        self.revision.store(revision, Ordering::SeqCst);
    }

    /// The installed level, or `None` while no level is known yet.
    pub fn level(&self) -> Option<DiagnosticLevel> {
        match self.level.load(Ordering::SeqCst) {
            LEVEL_UNKNOWN => None,
            value => Some(decode_level(value)),
        }
    }

    fn admits(&self, severity: DiagnosticLevel) -> bool {
        match self.level() {
            None => true,
            Some(level) => level.admits(severity),
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn component(&self) -> Component {
        self.component
    }

    pub fn role(&self) -> ProcessRole {
        self.role
    }

    pub fn boot_id(&self) -> BootId {
        self.boot_id
    }

    pub fn parent_session_id(&self) -> Option<SessionId> {
        self.parent_session_id
    }

    pub fn tokenizer(&self) -> Option<&Tokenizer> {
        self.tokenizer.get()
    }

    pub fn counters(&self) -> &Arc<Counters> {
        &self.counters
    }

    pub fn queue_snapshot(&self) -> QueueSnapshot {
        self.queue.snapshot()
    }

    pub(crate) fn emit(
        &self,
        event: DiagnosticEvent,
        span_id: Option<SpanId>,
        parent_span_id: Option<SpanId>,
    ) -> EmitOutcome {
        let severity = event.level();
        let exempt = event.bypasses_level_filter();
        if !exempt && !self.admits(severity) {
            self.counters.filtered.fetch_add(1, Ordering::Relaxed);
            return EmitOutcome::Filtered;
        }
        let record = DiagnosticRecordV1 {
            schema: crate::record::RecordSchema,
            timestamp_ms: now_epoch_ms(),
            monotonic_ms: self.started.elapsed().as_millis() as u64,
            component: self.component,
            boot_id: self.boot_id,
            sequence: self.sequence.fetch_add(1, Ordering::Relaxed),
            level: severity,
            level_revision: self.revision(),
            parent_session_id: self.parent_session_id,
            span_id,
            parent_span_id,
            event,
        };
        let bytes = match record.encode_jsonl() {
            Ok(bytes) => bytes,
            Err(_) => {
                self.counters.rejected.fetch_add(1, Ordering::Relaxed);
                return EmitOutcome::Rejected;
            }
        };
        let outcome = match self.queue.try_push(severity, bytes) {
            Ok(()) => EmitOutcome::Queued,
            Err(PushError::Full | PushError::Closed) => EmitOutcome::Dropped,
        };
        self.counters.record(outcome);
        outcome
    }
}

fn encode_level(level: DiagnosticLevel) -> u8 {
    match level {
        DiagnosticLevel::Error => 0,
        DiagnosticLevel::Warn => 1,
        DiagnosticLevel::Info => 2,
        DiagnosticLevel::Debug => 3,
    }
}

/// No level has been installed yet; every event is admitted (bounded by the queue) until
/// the process knows the level it must use.
const LEVEL_UNKNOWN: u8 = u8::MAX;

fn decode_level(value: u8) -> DiagnosticLevel {
    match value {
        0 => DiagnosticLevel::Error,
        1 => DiagnosticLevel::Warn,
        2 => DiagnosticLevel::Info,
        _ => DiagnosticLevel::Debug,
    }
}

pub(crate) fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

/// Debug-budget shared by every clone of one request context.
#[derive(Debug)]
pub struct DebugBudget {
    remaining: std::sync::atomic::AtomicU32,
}

impl DebugBudget {
    pub fn new(limit: u32) -> Self {
        Self {
            remaining: std::sync::atomic::AtomicU32::new(limit),
        }
    }

    fn try_consume(&self) -> bool {
        let mut current = self.remaining.load(Ordering::Relaxed);
        loop {
            if current == 0 {
                return false;
            }
            match self.remaining.compare_exchange_weak(
                current,
                current - 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(observed) => current = observed,
            }
        }
    }

    pub fn remaining(&self) -> u32 {
        self.remaining.load(Ordering::Relaxed)
    }
}

/// A cloneable emission handle. `noop()` never records anything.
#[derive(Clone, Debug, Default)]
pub struct DiagnosticHandle {
    emitter: Option<Arc<Emitter>>,
}

impl DiagnosticHandle {
    pub fn noop() -> Self {
        Self { emitter: None }
    }

    pub(crate) fn attached(emitter: Arc<Emitter>) -> Self {
        Self {
            emitter: Some(emitter),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.emitter.is_some()
    }

    pub fn try_emit(&self, event: DiagnosticEvent) -> EmitOutcome {
        match &self.emitter {
            Some(emitter) => emitter.emit(event, None, None),
            None => EmitOutcome::Unavailable,
        }
    }

    /// Start a new span with a fresh Debug budget. Call once per user request; use
    /// [`DiagnosticContext::child`] for nested work so the budget is shared.
    pub fn context(&self) -> DiagnosticContext {
        let span_id = SpanId::random().ok();
        DiagnosticContext {
            handle: self.clone(),
            span_id,
            parent_span_id: None,
            budget: self
                .emitter
                .as_ref()
                .map(|_| Arc::new(DebugBudget::new(REQUEST_DEBUG_BUDGET))),
        }
    }

    /// A context without its own Debug budget; used for process-lifetime events.
    pub fn root_context(&self) -> DiagnosticContext {
        DiagnosticContext {
            handle: self.clone(),
            span_id: SpanId::random().ok(),
            parent_span_id: None,
            budget: None,
        }
    }

    pub fn tokenizer(&self) -> Option<&Tokenizer> {
        self.emitter
            .as_ref()
            .and_then(|emitter| emitter.tokenizer())
    }

    pub fn token(&self, domain: CorrelationDomain, original: &str) -> Option<CorrelationToken> {
        self.tokenizer()
            .and_then(|tokenizer| tokenizer.token(domain, original))
    }

    /// A service revision can group test outcomes and wire failures without exposing its id.
    pub fn decision_service_token(&self, id: &str, revision: u64) -> Option<CorrelationToken> {
        if id.len() > 256 {
            return None;
        }
        self.tokenizer()?.token(
            CorrelationDomain::Decision,
            &format!("service:{id}@{revision}"),
        )
    }

    pub fn boot_id(&self) -> Option<BootId> {
        self.emitter.as_ref().map(|emitter| emitter.boot_id())
    }

    pub fn session_id(&self) -> Option<SessionId> {
        self.emitter
            .as_ref()
            .and_then(|emitter| emitter.parent_session_id())
    }

    pub fn level(&self) -> Option<DiagnosticLevel> {
        self.emitter.as_ref().and_then(|emitter| emitter.level())
    }

    pub fn counters(&self) -> Option<&Arc<Counters>> {
        self.emitter.as_ref().map(|emitter| emitter.counters())
    }

    pub fn queue_snapshot(&self) -> Option<QueueSnapshot> {
        self.emitter
            .as_ref()
            .map(|emitter| emitter.queue_snapshot())
    }

    pub fn level_source(&self) -> LevelSource {
        LevelSource::Default
    }
}

/// Explicit span context for one unit of work.
#[derive(Clone, Debug)]
pub struct DiagnosticContext {
    handle: DiagnosticHandle,
    span_id: Option<SpanId>,
    parent_span_id: Option<SpanId>,
    budget: Option<Arc<DebugBudget>>,
}

impl DiagnosticContext {
    pub fn handle(&self) -> &DiagnosticHandle {
        &self.handle
    }

    pub fn span_id(&self) -> Option<SpanId> {
        self.span_id
    }

    pub fn parent_span_id(&self) -> Option<SpanId> {
        self.parent_span_id
    }

    pub fn token(&self, domain: CorrelationDomain, original: &str) -> Option<CorrelationToken> {
        self.handle.token(domain, original)
    }

    pub fn emit(&self, event: DiagnosticEvent) -> EmitOutcome {
        let emitter = match &self.handle.emitter {
            Some(emitter) => emitter,
            None => return EmitOutcome::Unavailable,
        };
        if event.level() == DiagnosticLevel::Debug
            && let Some(budget) = &self.budget
            && !budget.try_consume()
        {
            emitter
                .counters
                .budget_exhausted
                .fetch_add(1, Ordering::Relaxed);
            return EmitOutcome::BudgetExhausted;
        }
        emitter.emit(event, self.span_id, self.parent_span_id)
    }

    /// Derive a nested context. The Debug budget is shared with the parent.
    pub fn child(&self) -> DiagnosticContext {
        DiagnosticContext {
            handle: self.handle.clone(),
            span_id: SpanId::random().ok(),
            parent_span_id: self.span_id,
            budget: self.budget.clone(),
        }
    }

    /// Derive a nested context that carries a specific parent span.
    pub fn child_of(&self, parent: SpanId) -> DiagnosticContext {
        DiagnosticContext {
            handle: self.handle.clone(),
            span_id: SpanId::random().ok(),
            parent_span_id: Some(parent),
            budget: self.budget.clone(),
        }
    }

    /// Derive the context for one independently budgeted call of a long-lived client,
    /// anchored to this context's span when it has one. A per-call budget keeps one busy
    /// call from spending another call's Debug allowance.
    pub fn call_span(&self) -> DiagnosticContext {
        let call = self.handle.context();
        match self.span_id {
            Some(parent) => call.child_of(parent),
            None => call,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{PanicObserved, ProcessRole};
    use crate::identity::SourceFileRef;
    use crate::queue::bounded_queue;

    fn test_emitter(
        level: Option<DiagnosticLevel>,
    ) -> (DiagnosticHandle, crate::queue::QueueReceiver) {
        let (sender, receiver) = bounded_queue();
        let emitter = Emitter::new(
            sender,
            level,
            Component::Diagnostics,
            ProcessRole::Daemon,
            BootId::random().expect("boot id"),
            None,
            Instant::now(),
            Arc::new(AtomicU64::new(0)),
            Arc::new(Counters::default()),
        );
        (DiagnosticHandle::attached(Arc::new(emitter)), receiver)
    }

    fn panic_event() -> DiagnosticEvent {
        DiagnosticEvent::PanicObserved(PanicObserved {
            source_file: SourceFileRef::parse("crates/diagnostics/src/context.rs").expect("path"),
            line: 1,
        })
    }

    fn stage_event() -> DiagnosticEvent {
        DiagnosticEvent::StageBegin(crate::event::StageBegin {
            stage: crate::event::StartupStage::ReadyWait,
        })
    }

    #[test]
    fn level_threshold_filters_before_encoding() {
        let (handle, receiver) = test_emitter(Some(DiagnosticLevel::Error));
        let outcome = handle.try_emit(stage_event());
        assert_eq!(outcome, EmitOutcome::Filtered);
        assert_eq!(handle.counters().expect("counters").filtered(), 1);
        assert!(
            receiver
                .pop_timeout(std::time::Duration::from_millis(1))
                .is_none()
        );
    }

    /// While no level is known every severity is admitted (the queue still bounds the
    /// records); the first installed level then judges the queued records by that level.
    /// A Debug startup stage is therefore kept when Debug was saved, and a saved Error
    /// never writes the earlier Info/Debug records.
    #[test]
    fn an_unknown_level_admits_everything_and_the_first_level_judges_the_queue() {
        let (handle, receiver) = test_emitter(None);
        let emitter = handle.emitter.as_ref().expect("emitter").clone();
        assert_eq!(emitter.level(), None);
        assert_eq!(handle.try_emit(stage_event()), EmitOutcome::Queued);
        assert_eq!(handle.try_emit(panic_event()), EmitOutcome::Queued);

        emitter.set_level(DiagnosticLevel::Debug, 7);
        assert_eq!(emitter.level(), Some(DiagnosticLevel::Debug));
        assert_eq!(handle.counters().expect("counters").filtered(), 0);
        let first = receiver
            .pop_timeout(std::time::Duration::from_millis(1))
            .expect("stage record");
        assert_eq!(
            DiagnosticRecordV1::parse_line(&first)
                .expect("record")
                .level,
            DiagnosticLevel::Debug
        );
        let second = receiver
            .pop_timeout(std::time::Duration::from_millis(1))
            .expect("panic record");
        assert_eq!(
            DiagnosticRecordV1::parse_line(&second)
                .expect("record")
                .level,
            DiagnosticLevel::Error
        );

        let (handle, receiver) = test_emitter(None);
        let emitter = handle.emitter.as_ref().expect("emitter").clone();
        assert_eq!(handle.try_emit(stage_event()), EmitOutcome::Queued);
        assert_eq!(handle.try_emit(panic_event()), EmitOutcome::Queued);
        emitter.set_level(DiagnosticLevel::Error, 8);
        // The queued Debug stage was dropped with the level it never had; the Error record
        // is still there, and the drop is counted as filtered.
        assert_eq!(handle.counters().expect("counters").filtered(), 1);
        let record = receiver
            .pop_timeout(std::time::Duration::from_millis(1))
            .expect("error record");
        assert_eq!(
            DiagnosticRecordV1::parse_line(&record)
                .expect("record")
                .level,
            DiagnosticLevel::Error
        );
        assert!(
            receiver
                .pop_timeout(std::time::Duration::from_millis(1))
                .is_none()
        );
    }

    #[test]
    fn error_events_pass_every_threshold() {
        let (handle, receiver) = test_emitter(Some(DiagnosticLevel::Error));
        assert_eq!(handle.try_emit(panic_event()), EmitOutcome::Queued);
        let bytes = receiver
            .pop_timeout(std::time::Duration::from_millis(5))
            .expect("record");
        let record = DiagnosticRecordV1::parse_line(&bytes).expect("valid record");
        assert_eq!(record.level, DiagnosticLevel::Error);
        assert_eq!(record.span_id, None);
    }

    #[test]
    fn debug_budget_is_shared_across_children_and_only_drops_debug() {
        let (handle, receiver) = test_emitter(Some(DiagnosticLevel::Debug));
        let context = handle.context();
        let child = context.child();
        for _ in 0..REQUEST_DEBUG_BUDGET {
            assert_eq!(
                child.emit(DiagnosticEvent::StageBegin(crate::event::StageBegin {
                    stage: crate::event::StartupStage::Spawn,
                })),
                EmitOutcome::Queued
            );
        }
        assert_eq!(
            context.emit(DiagnosticEvent::StageBegin(crate::event::StageBegin {
                stage: crate::event::StartupStage::Spawn,
            })),
            EmitOutcome::BudgetExhausted
        );
        // Warnings and errors are never dropped by the Debug budget.
        assert_eq!(context.emit(panic_event()), EmitOutcome::Queued);
        assert!(handle.counters().expect("counters").budget_exhausted() >= 1);
        let mut drained = 0;
        while receiver
            .pop_timeout(std::time::Duration::from_millis(1))
            .is_some()
        {
            drained += 1;
        }
        assert_eq!(drained, REQUEST_DEBUG_BUDGET as usize + 1);
    }

    #[test]
    fn noop_handle_records_nothing() {
        let handle = DiagnosticHandle::noop();
        assert_eq!(handle.try_emit(panic_event()), EmitOutcome::Unavailable);
        assert!(handle.tokenizer().is_none());
        assert!(handle.boot_id().is_none());
    }
}
