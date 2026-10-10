use super::*;

mod cleanup;
mod constructor;
mod disposition;
mod io;

/// A fixed-capacity mailbox with an inline first slot. Production plans use a
/// single precommit event, so the common response-head handoff does not need a
/// second heap allocation after the request future has been created. Wider
/// windows retain their remaining events in FIFO order behind that slot.
struct PrecommitWindow {
    first: Option<PrecommitEvent>,
    overflow: VecDeque<PrecommitEvent>,
}

impl PrecommitWindow {
    fn new() -> Self {
        Self {
            first: None,
            overflow: VecDeque::new(),
        }
    }

    fn len(&self) -> usize {
        usize::from(self.first.is_some()) + self.overflow.len()
    }

    fn push_back(&mut self, event: PrecommitEvent) {
        if self.first.is_none() {
            debug_assert!(self.overflow.is_empty());
            self.first = Some(event);
        } else {
            self.overflow.push_back(event);
        }
    }

    fn pop_front(&mut self) -> Option<PrecommitEvent> {
        let event = self.first.take()?;
        self.first = self.overflow.pop_front();
        Some(event)
    }

    fn release_storage(&mut self) {
        self.first = None;
        self.overflow = VecDeque::new();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptSnapshot {
    pub semantic_upstream_calls: usize,
    pub connection_sub_attempts: usize,
    pub writer_state: WriterState,
    pub upstream_request_fence: CommitFence,
    pub downstream_header_fence: CommitFence,
    pub downstream_semantic_fence: CommitFence,
    pub accepted_response_scope_created: bool,
    pub published: Option<Disposition>,
    pub reset_count: usize,
    pub finalized: bool,
}

/// Linear owner of one semantic upstream exchange. It is intentionally not
/// Clone and never selects a second provider or binding.
pub struct AttemptExchange<T: AttemptTransport> {
    request_id: RequestId,
    attempt_id: AttemptId,
    generation: AttemptGeneration,
    plan_revision: PlanRevision,
    target: AttemptTargetOwner,
    transport: Option<T>,
    request: PreparedAttemptHttpRequest,
    semantic_upstream_calls: usize,
    connection_sub_attempts: usize,
    connected: bool,
    negotiated_protocol: Option<HttpProtocol>,
    request_framing_reconciled: bool,
    pending_request_write: Option<Bytes>,
    writer_state: WriterState,
    upstream_request_fence: CommitFence,
    downstream_header_fence: CommitFence,
    downstream_semantic_fence: CommitFence,
    accepted_response_scope_created: bool,
    response_window: PrecommitWindow,
    response_window_capacity: usize,
    response_mailbox_reservation: Option<Reservation>,
    transport_codec_reservation: Option<Reservation>,
    precommit_body_owner: BodyPlanExecutor,
    body_plans: AttemptBodyPlans,
    timeouts: AttemptTimeouts,
    attempt_deadline: Instant,
    attempt_started_at: Instant,
    connect_elapsed: Option<Duration>,
    request_write_started_at: Option<Instant>,
    request_write_elapsed: Option<Duration>,
    first_upstream_receipt_at: Option<Instant>,
    last_upstream_progress_at: Option<Instant>,
    local_read_suppressed: Duration,
    transport_suppression_observed: Duration,
    idle_suppression_credit: Duration,
    upstream_body_bytes: u64,
    timeout_kind: Option<AttemptTimeoutKind>,
    budget: StreamBudget,
    response_live: bool,
    cancellation: CancellationToken,
    candidate: Option<Disposition>,
    accept_blocked: bool,
    permit_nonce: u64,
    published: Option<Disposition>,
    reset_count: usize,
    response_window_high_water: usize,
    finalized: bool,
    telemetry: Option<RequestTelemetry>,
}

enum AttemptTargetOwner {
    Dynamic(TransportTarget),
    Compiled(Arc<CompiledAttemptPlan>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DuplexIoProgress {
    ResponseBuffered,
    WriteComplete,
}

impl AttemptTargetOwner {
    fn target(&self) -> &TransportTarget {
        match self {
            Self::Dynamic(target) => target,
            Self::Compiled(plan) => &plan.transport_target,
        }
    }
}
