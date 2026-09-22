use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use hiroute_gateway_core::runtime::attempt::PrecommitEvent;

use crate::server::core_runtime::profiles::CandidateProtocolProfile;

use super::super::RequestObservation;
use super::super::request::AttemptObservation;

#[path = "capture/tracker.rs"]
mod tracker;

use tracker::CanonicalResponseTracker;

const CAPTURE_EVENT_SLOTS: usize = 256;
const CAPTURE_POLL_INTERVAL: Duration = Duration::from_millis(10);
const CAPTURE_METADATA_BYTES: usize = 512;
const CAPTURE_BEGIN_BYTES: usize = 4 * 1024;
// Reserve before copying. The native multiplier conservatively covers the
// raw copy, canonical decoder state and rendered correlation units; accepted
// bytes need only their queue copy because the native reservation stays live
// until the capture reaches a terminal accepted frame.
const CAPTURE_NATIVE_EXPANSION: usize = 8;
const CAPTURE_ACCEPTED_EXPANSION: usize = 2;
const MAX_CAPTURE_FRAME_BYTES: usize = 64 * 1024;

#[derive(Clone, Default)]
pub(in crate::server::core_runtime::observation) struct CanonicalCaptureProducer {
    shared: Arc<OnceLock<Option<Arc<CaptureShared>>>>,
    capacity_bytes: usize,
}

struct CaptureShared {
    sender: SyncSender<CaptureJob>,
    budget: Arc<CaptureBudget>,
    next_id: AtomicU64,
}

struct CaptureBudget {
    capacity: usize,
    used: AtomicUsize,
}

struct CaptureReservation {
    budget: Arc<CaptureBudget>,
    bytes: usize,
}

impl Drop for CaptureReservation {
    fn drop(&mut self) {
        let previous = self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
        debug_assert!(previous >= self.bytes);
    }
}

struct CaptureStatus {
    finished: AtomicBool,
    completion: tokio::sync::Notify,
    failed: AtomicBool,
    terminal_enqueued: AtomicBool,
    reason: Mutex<Option<&'static str>>,
}

#[derive(Clone)]
pub(in crate::server::core_runtime::observation) struct CanonicalCaptureHandle {
    id: u64,
    shared: Arc<CaptureShared>,
    status: Arc<CaptureStatus>,
}

pub(super) struct PreparedNativeCapture {
    handle: CanonicalCaptureHandle,
    event: CapturedNativeEvent,
    reservation: CaptureReservation,
}

enum CaptureJob {
    Begin {
        id: u64,
        request: RequestObservation,
        profile: Arc<CandidateProtocolProfile>,
        tool_id_projection: crate::server::core_runtime::adapters::ToolIdProjection,
        chat_tool_projection: Option<crate::server::core_runtime::adapters::ChatToolProjection>,
        streaming: bool,
        alias: String,
        status: Arc<CaptureStatus>,
        reservation: CaptureReservation,
    },
    Native {
        id: u64,
        event: CapturedNativeEvent,
        reservation: CaptureReservation,
    },
    Accepted {
        id: u64,
        request: RequestObservation,
        attempt: Arc<AttemptObservation>,
        frame_id: String,
        bytes: Vec<u8>,
        end_stream: bool,
        reservation: CaptureReservation,
    },
    Wake,
    Cancel {
        id: u64,
        status: Arc<CaptureStatus>,
        reason: &'static str,
    },
}

enum CapturedNativeEvent {
    Head(u16),
    Body(Vec<u8>),
    EndStream,
}

struct CaptureWorkerState {
    request: RequestObservation,
    tracker: CanonicalResponseTracker,
    status: Arc<CaptureStatus>,
    reservations: Vec<CaptureReservation>,
    accepted_started: bool,
}

impl Drop for CaptureWorkerState {
    fn drop(&mut self) {
        self.status.finish();
    }
}

impl CaptureStatus {
    fn finish(&self) {
        self.finished.store(true, Ordering::Release);
        self.completion.notify_one();
    }
}

impl CanonicalCaptureProducer {
    pub(in crate::server::core_runtime::observation) fn new(
        enabled: bool,
        capacity_bytes: usize,
    ) -> Self {
        if !enabled || capacity_bytes == 0 {
            return Self::default();
        }
        Self {
            shared: Arc::new(OnceLock::new()),
            capacity_bytes,
        }
    }

    fn start_worker(&self) -> Option<Arc<CaptureShared>> {
        let capacity_bytes = self.capacity_bytes;
        if capacity_bytes == 0 {
            return None;
        }
        let slots = (capacity_bytes / CAPTURE_METADATA_BYTES).clamp(1, CAPTURE_EVENT_SLOTS);
        let (sender, receiver) = sync_channel(slots);
        let budget = Arc::new(CaptureBudget {
            capacity: capacity_bytes,
            used: AtomicUsize::new(0),
        });
        let shared = Arc::new(CaptureShared {
            sender,
            budget,
            next_id: AtomicU64::new(1),
        });
        if thread::Builder::new()
            .name("hiroute-canonical-capture".into())
            .spawn(move || capture_worker(receiver))
            .is_err()
        {
            return None;
        }
        Some(shared)
    }

    pub(super) fn begin(
        &self,
        request: RequestObservation,
        profile: Arc<CandidateProtocolProfile>,
        tool_id_projection: crate::server::core_runtime::adapters::ToolIdProjection,
        chat_tool_projection: Option<&crate::server::core_runtime::adapters::ChatToolProjection>,
        streaming: bool,
        alias: String,
    ) -> Option<CanonicalCaptureHandle> {
        let shared = Arc::clone(self.shared.get_or_init(|| self.start_worker()).as_ref()?);
        let projection_bytes = match chat_tool_projection {
            Some(projection) => projection.retained_bytes()?,
            None => 0,
        };
        let begin_bytes = CAPTURE_BEGIN_BYTES.checked_add(projection_bytes)?;
        let reservation = reserve(&shared.budget, begin_bytes)?;
        let chat_tool_projection = chat_tool_projection.cloned();
        let handle = CanonicalCaptureHandle {
            id: shared.next_id.fetch_add(1, Ordering::Relaxed),
            shared,
            status: Arc::new(CaptureStatus {
                finished: AtomicBool::new(false),
                completion: tokio::sync::Notify::new(),
                failed: AtomicBool::new(false),
                terminal_enqueued: AtomicBool::new(false),
                reason: Mutex::new(None),
            }),
        };
        let job = CaptureJob::Begin {
            id: handle.id,
            request,
            profile,
            tool_id_projection,
            chat_tool_projection,
            streaming,
            alias,
            status: Arc::clone(&handle.status),
            reservation,
        };
        if handle.try_send(job).is_err() {
            return None;
        }
        Some(handle)
    }
}

impl CanonicalCaptureHandle {
    pub(super) fn prepare_native(&self, event: &PrecommitEvent) -> Option<PreparedNativeCapture> {
        if self.status.failed.load(Ordering::Acquire) {
            return None;
        }
        let (event, bytes) = match event {
            PrecommitEvent::ResponseHead(head) => {
                (CapturedNativeEvent::Head(head.status().as_u16()), 0)
            }
            PrecommitEvent::Body(bytes) => {
                let source = bytes.bytes();
                if source.len() > MAX_CAPTURE_FRAME_BYTES {
                    self.fail("canonical_capture_native_frame_limit");
                    return None;
                }
                let charge = capture_charge(source.len(), CAPTURE_NATIVE_EXPANSION)?;
                let reservation = reserve(&self.shared.budget, charge).or_else(|| {
                    self.fail("canonical_capture_budget_exceeded");
                    None
                })?;
                return Some(PreparedNativeCapture {
                    handle: self.clone(),
                    event: CapturedNativeEvent::Body(source.to_vec()),
                    reservation,
                });
            }
            PrecommitEvent::EndStream => (CapturedNativeEvent::EndStream, 0),
            PrecommitEvent::SseEvent { .. } => {
                self.fail("canonical_capture_raw_sse_bypass");
                return None;
            }
        };
        let reservation = reserve(
            &self.shared.budget,
            capture_charge(bytes, CAPTURE_NATIVE_EXPANSION)?,
        )
        .or_else(|| {
            self.fail("canonical_capture_budget_exceeded");
            None
        })?;
        Some(PreparedNativeCapture {
            handle: self.clone(),
            event,
            reservation,
        })
    }

    pub(in crate::server::core_runtime::observation) fn accepted_frame(
        &self,
        request: RequestObservation,
        attempt: Arc<AttemptObservation>,
        frame_id: String,
        bytes: &[u8],
        end_stream: bool,
    ) -> Result<(), &'static str> {
        if let Some(reason) = self.failure_reason() {
            return Err(reason);
        }
        if bytes.len() > MAX_CAPTURE_FRAME_BYTES {
            self.fail("canonical_capture_accepted_frame_limit");
            return Err("canonical_capture_accepted_frame_limit");
        }
        let charge = capture_charge(bytes.len(), CAPTURE_ACCEPTED_EXPANSION)
            .ok_or("canonical_capture_budget_overflow")?;
        let reservation = reserve(&self.shared.budget, charge).ok_or_else(|| {
            self.fail("canonical_capture_budget_exceeded");
            "canonical_capture_budget_exceeded"
        })?;
        let job = CaptureJob::Accepted {
            id: self.id,
            request,
            attempt,
            frame_id,
            bytes: bytes.to_vec(),
            end_stream,
            reservation,
        };
        self.try_send(job)?;
        if end_stream {
            self.status.terminal_enqueued.store(true, Ordering::Release);
        }
        Ok(())
    }

    pub(in crate::server::core_runtime::observation) fn cancel_unless_terminal(
        &self,
        reason: &'static str,
    ) {
        if !self.status.terminal_enqueued.load(Ordering::Acquire) {
            // Drain already accepted frames before terminating a partial stream.
            let _ = self.try_send(CaptureJob::Cancel {
                id: self.id,
                status: Arc::clone(&self.status),
                reason,
            });
        }
    }

    pub(in crate::server::core_runtime::observation) fn failure_reason(
        &self,
    ) -> Option<&'static str> {
        if !self.status.failed.load(Ordering::Acquire) {
            return None;
        }
        *self
            .status
            .reason
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn fail(&self, reason: &'static str) {
        if !self.status.failed.swap(true, Ordering::AcqRel) {
            *self
                .status
                .reason
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(reason);
        }
        let _ = self.shared.sender.try_send(CaptureJob::Wake);
    }

    fn try_send(&self, job: CaptureJob) -> Result<(), &'static str> {
        match self.shared.sender.try_send(job) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.fail("canonical_capture_queue_full");
                Err("canonical_capture_queue_full")
            }
            Err(TrySendError::Disconnected(_)) => {
                self.fail("canonical_capture_worker_unavailable");
                Err("canonical_capture_worker_unavailable")
            }
        }
    }
}

impl PreparedNativeCapture {
    pub(super) fn commit(self) {
        let job = CaptureJob::Native {
            id: self.handle.id,
            event: self.event,
            reservation: self.reservation,
        };
        let _ = self.handle.try_send(job);
    }

    pub(super) fn cancel(self, reason: &'static str) {
        self.handle.fail(reason);
    }
}

fn capture_charge(bytes: usize, expansion: usize) -> Option<usize> {
    bytes
        .checked_mul(expansion)
        .and_then(|bytes| bytes.checked_add(CAPTURE_METADATA_BYTES))
}

fn reserve(budget: &Arc<CaptureBudget>, bytes: usize) -> Option<CaptureReservation> {
    let mut used = budget.used.load(Ordering::Acquire);
    loop {
        let next = used.checked_add(bytes)?;
        if next > budget.capacity {
            return None;
        }
        match budget
            .used
            .compare_exchange_weak(used, next, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => {
                return Some(CaptureReservation {
                    budget: Arc::clone(budget),
                    bytes,
                });
            }
            Err(observed) => used = observed,
        }
    }
}

fn capture_worker(receiver: Receiver<CaptureJob>) {
    let mut states = BTreeMap::<u64, CaptureWorkerState>::new();
    loop {
        match receiver.recv_timeout(CAPTURE_POLL_INTERVAL) {
            Ok(job) => {
                let id = job_id(&job);
                if catch_unwind(AssertUnwindSafe(|| process_job(job, &mut states))).is_err()
                    && let Some(id) = id
                    && let Some(state) = states.get(&id)
                {
                    mark_failed(&state.status, "canonical_capture_worker_panicked");
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        cleanup_failed(&mut states);
    }
}

fn job_id(job: &CaptureJob) -> Option<u64> {
    match job {
        CaptureJob::Begin { id, .. }
        | CaptureJob::Native { id, .. }
        | CaptureJob::Accepted { id, .. }
        | CaptureJob::Cancel { id, .. } => Some(*id),
        CaptureJob::Wake => None,
    }
}

fn process_job(job: CaptureJob, states: &mut BTreeMap<u64, CaptureWorkerState>) {
    match job {
        CaptureJob::Begin {
            id,
            request,
            profile,
            tool_id_projection,
            chat_tool_projection,
            streaming,
            alias,
            status,
            reservation,
        } => {
            if status.failed.load(Ordering::Acquire) {
                status.finish();
                return;
            }
            states.insert(
                id,
                CaptureWorkerState {
                    request,
                    tracker: CanonicalResponseTracker::new(
                        profile,
                        tool_id_projection,
                        chat_tool_projection,
                        streaming,
                        alias,
                    ),
                    status,
                    reservations: vec![reservation],
                    accepted_started: false,
                },
            );
        }
        CaptureJob::Native {
            id,
            event,
            reservation,
        } => {
            let Some(state) = states.get_mut(&id) else {
                return;
            };
            state.reservations.push(reservation);
            if let Err(reason) = state.tracker.observe(event) {
                mark_failed(&state.status, reason);
            }
        }
        CaptureJob::Accepted {
            id,
            request,
            attempt,
            frame_id,
            bytes,
            end_stream,
            reservation: _reservation,
        } => {
            let Some(state) = states.get_mut(&id) else {
                abort_capture(&request, &attempt, "canonical_capture_state_unavailable");
                return;
            };
            state.accepted_started = true;
            state
                .request
                .begin_content("response_delivered", Some(&attempt));
            match state.tracker.correlate_accepted_output(&bytes, end_stream) {
                Ok(delivery) => state
                    .request
                    .emit_canonical_response_delivery(&attempt, &frame_id, delivery),
                Err(reason) => {
                    mark_failed(&state.status, reason);
                    state
                        .request
                        .terminal_content("response_delivered", "abort", Some(reason));
                }
            }
            if end_stream && !state.status.failed.load(Ordering::Acquire) {
                state
                    .request
                    .terminal_content("response_delivered", "finish", None);
                states.remove(&id);
            }
        }
        CaptureJob::Wake => {}
        CaptureJob::Cancel { id, status, reason } => {
            mark_failed(&status, reason);
            if let Some(state) = states.remove(&id)
                && state.accepted_started
            {
                state
                    .request
                    .terminal_content("response_delivered", "abort", Some(reason));
            }
            status.finish();
        }
    }
}

fn cleanup_failed(states: &mut BTreeMap<u64, CaptureWorkerState>) {
    let failed = states
        .iter()
        .filter_map(|(id, state)| state.status.failed.load(Ordering::Acquire).then_some(*id))
        .collect::<Vec<_>>();
    for id in failed {
        if let Some(state) = states.remove(&id)
            && state.accepted_started
        {
            state.request.terminal_content(
                "response_delivered",
                "abort",
                state_failure_reason(&state.status),
            );
        }
    }
}

fn mark_failed(status: &CaptureStatus, reason: &'static str) {
    if !status.failed.swap(true, Ordering::AcqRel) {
        *status
            .reason
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(reason);
    }
}

fn state_failure_reason(status: &CaptureStatus) -> Option<&'static str> {
    *status
        .reason
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn abort_capture(request: &RequestObservation, attempt: &AttemptObservation, reason: &'static str) {
    request.begin_content("response_delivered", Some(attempt));
    request.terminal_content("response_delivered", "abort", Some(reason));
}

impl RequestObservation {
    pub(in crate::server::core_runtime::observation) fn begin_response_capture(
        &self,
        stable_binding_id: &str,
        chat_tool_projection: Option<&crate::server::core_runtime::adapters::ChatToolProjection>,
    ) -> Option<CanonicalCaptureHandle> {
        let (profile, tool_id_projection, streaming) = {
            let state = self.lock_state();
            let candidate = state.candidates.get(stable_binding_id)?;
            (
                candidate.protocol_profile.clone()?,
                state.tool_id_projection.clone()?,
                candidate.streaming,
            )
        };
        self.inner.channels.response_capture.begin(
            self.clone(),
            profile,
            tool_id_projection,
            chat_tool_projection,
            streaming,
            self.metadata().served_model_id.clone(),
        )
    }

    pub(in crate::server::core_runtime::observation) fn bind_response_capture(
        &self,
        capture: CanonicalCaptureHandle,
    ) {
        self.lock_state().response_capture = Some(capture);
    }

    pub(in crate::server::core_runtime::observation) fn response_capture(
        &self,
    ) -> Option<CanonicalCaptureHandle> {
        self.lock_state().response_capture.clone()
    }

    pub(in crate::server::core_runtime::observation) fn clear_response_capture(&self) {
        if self.inner.agent_turn_output.get().is_none() {
            self.lock_state().response_capture = None;
        }
    }

    pub(crate) async fn finish_agent_turn_output(&self) {
        let Some((store, ticket)) = self.inner.agent_turn_output.get() else {
            return;
        };
        let capture = self.lock_state().response_capture.take();
        let Some(capture) = capture else {
            if self.has_accepted_attempt() {
                store.mark_output_partial(ticket);
            }
            return;
        };
        capture.cancel_unless_terminal("agent_turn_response_incomplete");
        // Only drain the existing bounded worker. A stuck decoder must never hold
        // the session lease indefinitely; late events are rejected by its ticket.
        let completed = tokio::time::timeout(Duration::from_millis(250), async {
            loop {
                let notified = capture.status.completion.notified();
                if capture.status.finished.load(Ordering::Acquire) {
                    break;
                }
                notified.await;
            }
        })
        .await
        .is_ok();
        if !completed || capture.failure_reason().is_some() {
            store.mark_output_partial(ticket);
            capture.fail("agent_turn_capture_incomplete");
        }
    }
}
