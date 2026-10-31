//! Ordered, optional content encoding. Runtime accepted-output history is
//! updated before work reaches this lane and never waits for its completion.
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use hiroute_gateway_core::runtime::body::{MemoryRole, Reservation};
use serde_json::Value;

use crate::replay::ReplayStore;
use crate::server::core_runtime::model_ir::{
    CanonicalInstruction, CanonicalMessage, CanonicalTool, ContentPart, ImageSource,
    ModelRequestIRV1, OpaqueProviderState, ToolOutput,
};

use super::RequestObservation;
use super::content::CanonicalFrameDelivery;
use super::producer::{ChannelProducer, ObservationWorkReservation};
use super::request::AttemptObservation;
use super::schema::LifecycleFactV1;

const JOB_SLOTS: usize = 256;
const JOB_OVERHEAD: usize = 1024;
// Optional capture may park for credit; it never extends a model deadline.
// Bound each stalled credit wait; successful records keep making progress without
// an arbitrary whole-transcript deadline or a polling/retry loop.
const CREDIT_WAIT_LIMIT: std::time::Duration = std::time::Duration::from_secs(1);

fn queue_storage_bytes() -> usize {
    JOB_SLOTS * (std::mem::size_of::<Box<Job>>() + 2 * std::mem::size_of::<usize>()) + JOB_OVERHEAD
}

#[derive(Clone, Default)]
pub(super) struct ContentWorkProducer {
    shared: Arc<OnceLock<Arc<WorkQueue>>>,
    initialize: Arc<Mutex<()>>,
    #[cfg(test)]
    gate: Option<Arc<(Mutex<bool>, std::sync::Condvar)>>,
}

struct WorkQueue {
    sender: SyncSender<Box<Job>>,
}

#[derive(Default)]
pub(super) struct ContentStatus {
    failed: AtomicBool,
    response_begin: AtomicBool,
    response_terminal: AtomicBool,
}

impl ContentStatus {
    pub(super) fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    pub(super) fn stop(&self) {
        self.failed.store(true, Ordering::Release);
    }
}

struct Job {
    request: RequestObservation,
    status: Arc<ContentStatus>,
    queued: Instant,
    command: Command,
    _retained: ObservationWorkReservation,
}

enum Command {
    #[cfg(test)]
    Barrier(SyncSender<()>),
    Request {
        snapshot: Box<RequestSnapshot>,
        replay: ReplayStore,
    },
    BeginResponse(Arc<AttemptObservation>),
    Delivery {
        attempt: Arc<AttemptObservation>,
        frame_id: String,
        delivery: CanonicalFrameDelivery,
    },
    TerminalResponse {
        phase: &'static str,
        reason: Option<&'static str>,
    },
}

struct RequestSnapshot {
    instructions: Vec<CanonicalInstruction>,
    messages: Vec<CanonicalMessage>,
    tools: Vec<CanonicalTool>,
    _reservation: Reservation,
}

impl ContentWorkProducer {
    #[cfg(test)]
    pub(super) fn paused() -> (Self, ContentGate) {
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        (
            Self {
                gate: Some(Arc::clone(&gate)),
                ..Self::default()
            },
            ContentGate(gate),
        )
    }

    #[cfg(test)]
    pub(super) fn flush(&self, request: &RequestObservation) {
        let queue = self.shared.get().expect("started content worker");
        let (sender, receiver) = sync_channel(1);
        let reservation = request
            .inner
            .channels
            .content
            .reserve_work(JOB_OVERHEAD)
            .unwrap();
        let mut job = Box::new(Job {
            request: request.clone(),
            status: Arc::new(ContentStatus::default()),
            queued: Instant::now(),
            command: Command::Barrier(sender),
            _retained: reservation,
        });
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match queue.sender.try_send(job) {
                Ok(()) => break,
                Err(std::sync::mpsc::TrySendError::Full(returned)) if Instant::now() < deadline => {
                    job = returned;
                    std::thread::yield_now();
                }
                Err(_) => panic!("content worker barrier unavailable"),
            }
        }
        receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
    }

    fn queue(&self, content: &ChannelProducer) -> Option<Arc<WorkQueue>> {
        if let Some(queue) = self.shared.get() {
            return Some(Arc::clone(queue));
        }
        // Initialization failure is transient; do not cache an unavailable
        // worker forever after one request encounters capacity pressure.
        let _initializing = self.initialize.try_lock().ok()?;
        if let Some(queue) = self.shared.get() {
            return Some(Arc::clone(queue));
        }
        let slots = content.reserve_work(queue_storage_bytes())?;
        let (sender, receiver) = sync_channel::<Box<Job>>(JOB_SLOTS);
        #[cfg(test)]
        let gate = self.gate.clone();
        std::thread::Builder::new()
            .name("hiroute-content-capture".into())
            .spawn(move || {
                let _slots = slots;
                while let Ok(job) = receiver.recv() {
                    let mut job = *job;
                    #[cfg(test)]
                    if let Some(gate) = &gate {
                        let mut open = gate.0.lock().unwrap();
                        while !*open {
                            open = gate.1.wait(open).unwrap();
                        }
                    }
                    if matches!(&job.command, Command::Request { .. }) {
                        // The active snapshot remains fully charged by its
                        // request/process reservation. Transfer its queue
                        // charge before emitting encoded records; otherwise
                        // the same payload unnecessarily occupies both slots.
                        job._retained.shrink_to(JOB_OVERHEAD);
                    }
                    let started = Instant::now();
                    let mut request = job.request.clone();
                    request.content_credit_wait = Some(CREDIT_WAIT_LIMIT);
                    let failed = job.status.failed.load(Ordering::Acquire);
                    if !failed {
                        let result =
                            catch_unwind(AssertUnwindSafe(|| process(&request, job.command)));
                        match result {
                            Ok(true) => {}
                            Ok(false) => request
                                .fail_content_work(&job.status, "content_capture_unavailable"),
                            Err(_) => request
                                .fail_content_work(&job.status, "content_capture_worker_panicked"),
                        }
                    }
                    if let Some(latency) = &request.inner.latency {
                        latency.capture_completed(
                            job.queued,
                            started,
                            failed || job.status.failed.load(Ordering::Acquire),
                        );
                    }
                }
            })
            .ok()?;
        let queue = Arc::new(WorkQueue { sender });
        let _ = self.shared.set(Arc::clone(&queue));
        Some(queue)
    }
}

fn process(request: &RequestObservation, command: Command) -> bool {
    match command {
        #[cfg(test)]
        Command::Barrier(sender) => {
            let _ = sender.send(());
            true
        }
        Command::Request { snapshot, replay } => request.capture_request_payload(
            &snapshot.instructions,
            &snapshot.messages,
            &snapshot.tools,
            &replay,
        ),
        Command::BeginResponse(attempt) => {
            request.begin_content("response_delivered", Some(&attempt));
            true
        }
        Command::Delivery {
            attempt,
            frame_id,
            delivery,
        } => {
            request.emit_response_content(&attempt, &frame_id, delivery);
            true
        }
        Command::TerminalResponse { phase, reason } => {
            request.terminal_content("response_delivered", phase, reason);
            true
        }
    }
}

#[cfg(test)]
pub(super) struct ContentGate(Arc<(Mutex<bool>, std::sync::Condvar)>);

#[cfg(test)]
impl ContentGate {
    pub(super) fn release(&self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
    }
}

#[cfg(test)]
impl Drop for ContentGate {
    fn drop(&mut self) {
        self.release();
    }
}

impl RequestObservation {
    pub(crate) fn capture_request_async(&self, request: &ModelRequestIRV1, replay: &ReplayStore) {
        if !self.is_enabled() {
            return;
        }
        self.emit_lifecycle(LifecycleFactV1::CanonicalRequestAccepted {
            canonicalization_version: "hiroute.model-request-ir/v1".into(),
        });
        if !self.captures_content() {
            return;
        }
        let bytes = snapshot_bytes(request);
        if !self.inner.channels.content.work_can_fit(
            bytes
                .saturating_add(JOB_OVERHEAD)
                .saturating_add(queue_storage_bytes()),
        ) {
            #[cfg(feature = "e2e-test-control")]
            eprintln!(
                "capture request fallback: reason=snapshot_size bytes={bytes} state={:?}",
                self.inner.channels.content.work_state()
            );
            self.capture_request_fallback(request, replay);
            return;
        }
        // Shared Replay bytes are not a second payload in the Content queue.
        // Charge their optional extended lifetime to the existing request /
        // process budget, including disk bytes. Under pressure, retain the
        // original synchronous path rather than lose previously capturable data.
        let retained = bytes.saturating_add(replay.content_retention_bytes());
        let Ok(memory) = replay
            .budget()
            .reserve(MemoryRole::ModelIrBacking, retained)
        else {
            #[cfg(feature = "e2e-test-control")]
            eprintln!(
                "capture request fallback: reason=request_budget bytes={bytes} retained={retained} available={:?}",
                replay.budget().available_bytes()
            );
            self.capture_request_fallback(request, replay);
            return;
        };
        let Ok(lease) = replay.content_lease() else {
            #[cfg(feature = "e2e-test-control")]
            eprintln!("capture request fallback: reason=content_lease bytes={bytes}");
            drop(memory);
            self.capture_request_fallback(request, replay);
            return;
        };
        let Some((queue, reservation)) = self.content_work_reservation(bytes) else {
            #[cfg(feature = "e2e-test-control")]
            eprintln!(
                "capture request fallback: reason=shared_credit bytes={bytes} state={:?}",
                self.inner.channels.content.work_state()
            );
            // Another request can occupy the optional snapshot allowance even
            // when this snapshot fits alone. No content has been queued for
            // this request yet: keep the original nonblocking encoder instead
            // of declaring a gap merely because optional ownership was refused.
            drop(lease);
            drop(memory);
            self.capture_request_fallback(request, replay);
            return;
        };
        let status = self
            .inner
            .async_content
            .get_or_init(|| Arc::new(ContentStatus::default()));
        // Capture only the fields consumed by the existing observation encoder.
        // The native wire tree and routing/history metadata are never cloned.
        let snapshot = Box::new(RequestSnapshot {
            instructions: request.instructions.clone(),
            messages: request.messages.clone(),
            tools: request.tools.clone(),
            _reservation: memory,
        });
        self.send_content_work(
            status,
            queue,
            reservation,
            Command::Request {
                snapshot,
                replay: lease,
            },
        );
    }

    fn capture_request_fallback(&self, request: &ModelRequestIRV1, replay: &ReplayStore) {
        if !self.capture_request_payload(
            &request.instructions,
            &request.messages,
            &request.tools,
            replay,
        ) {
            let status = self
                .inner
                .async_content
                .get_or_init(|| Arc::new(ContentStatus::default()));
            self.fail_content_work(status, "content_capture_unavailable");
        }
    }

    pub(super) fn begin_response_content(&self, attempt: &AttemptObservation) {
        if !self.captures_content() {
            return;
        }
        let Some(status) = self.inner.async_content.get() else {
            self.begin_content("response_delivered", Some(attempt));
            return;
        };
        if status.response_begin.swap(true, Ordering::AcqRel) {
            return;
        }
        self.enqueue_content_work(status, attempt_bytes(attempt), || {
            Command::BeginResponse(Arc::new(attempt.clone()))
        });
    }

    pub(super) fn finish_response_content(
        &self,
        phase: &'static str,
        reason: Option<&'static str>,
    ) {
        if !self.captures_content() {
            return;
        }
        let Some(status) = self.inner.async_content.get() else {
            self.terminal_content("response_delivered", phase, reason);
            return;
        };
        if status.response_terminal.swap(true, Ordering::AcqRel) {
            return;
        }
        self.enqueue_content_work(status, JOB_OVERHEAD, || Command::TerminalResponse {
            phase,
            reason,
        });
    }

    pub(super) fn defer_response_content(
        &self,
        attempt: &AttemptObservation,
        frame_id: &str,
        delivery: CanonicalFrameDelivery,
    ) {
        let Some(status) = self.inner.async_content.get() else {
            self.emit_response_content(attempt, frame_id, delivery);
            return;
        };
        let bytes = delivery
            .reservations
            .iter()
            .fold(
                attempt_bytes(attempt).saturating_add(frame_id.len()),
                |size, reservation| size.saturating_add(reservation.bytes()),
            )
            .saturating_add(
                delivery
                    .events
                    .capacity()
                    .saturating_mul(std::mem::size_of::<
                        crate::server::core_runtime::model_ir::ModelStreamEventV1,
                    >()),
            );
        self.enqueue_content_work(status, bytes, || Command::Delivery {
            attempt: Arc::new(attempt.clone()),
            frame_id: frame_id.into(),
            delivery,
        });
    }

    fn enqueue_content_work(
        &self,
        status: &Arc<ContentStatus>,
        bytes: usize,
        build: impl FnOnce() -> Command,
    ) {
        if let Some((queue, reservation)) = self.reserve_content_work(status, bytes) {
            self.send_content_work(status, queue, reservation, build());
        }
    }

    fn reserve_content_work(
        &self,
        status: &ContentStatus,
        bytes: usize,
    ) -> Option<(Arc<WorkQueue>, ObservationWorkReservation)> {
        if status.failed.load(Ordering::Acquire) {
            return None;
        }
        let result = self.content_work_reservation(bytes);
        if result.is_none() {
            #[cfg(feature = "e2e-test-control")]
            eprintln!(
                "capture work reservation refused: bytes={bytes} state={:?}",
                self.inner.channels.content.work_state()
            );
            self.fail_content_work(status, "content_capture_queue_unavailable");
        }
        result
    }

    fn content_work_reservation(
        &self,
        bytes: usize,
    ) -> Option<(Arc<WorkQueue>, ObservationWorkReservation)> {
        let queue = self
            .inner
            .channels
            .content_work
            .queue(&self.inner.channels.content)?;
        self.inner
            .channels
            .content
            .reserve_work(bytes.saturating_add(JOB_OVERHEAD))
            .map(|reservation| (queue, reservation))
    }

    fn send_content_work(
        &self,
        status: &Arc<ContentStatus>,
        queue: Arc<WorkQueue>,
        reservation: ObservationWorkReservation,
        command: Command,
    ) {
        let mut request = self.clone();
        request
            .content_occurred_at
            .get_or_insert_with(super::unix_nanos);
        // Allocate only admitted jobs. JOB_OVERHEAD covers this allocation;
        // the bounded channel retains pointers instead of 256 maximum-size jobs.
        let job = Box::new(Job {
            request,
            status: Arc::clone(status),
            queued: Instant::now(),
            command,
            _retained: reservation,
        });
        if queue.sender.try_send(job).is_err() {
            self.fail_content_work(status, "content_capture_queue_full");
        }
    }

    fn fail_content_work(&self, status: &ContentStatus, reason: &'static str) {
        if status.failed.swap(true, Ordering::AcqRel) {
            return;
        }
        // Reuse the independent bounded loss pulse. It cannot be trapped behind
        // full payload slots. Never invent a complete transcript after a gap.
        self.inner.channels.content.declare_work_loss();
        self.terminal_content("request_input", "abort", Some(reason));
        self.terminal_content("response_delivered", "abort", Some(reason));
        if let Some(latency) = &self.inner.latency {
            let now = Instant::now();
            latency.capture_completed(now, now, true);
        }
    }
}

fn attempt_bytes(attempt: &AttemptObservation) -> usize {
    [
        &attempt.attempt_id,
        &attempt.stable_binding_id,
        &attempt.credential_ref,
        &attempt.key_id,
        &attempt.provider_name,
        &attempt.request_model,
        &attempt.upstream_protocol,
        &attempt.model_configuration_id,
        &attempt.adapter_revision,
        &attempt.candidate_id,
        &attempt.profile_digest,
        &attempt.start_reason,
    ]
    .iter()
    .fold(
        std::mem::size_of::<AttemptObservation>() + JOB_OVERHEAD,
        |size, value| size.saturating_add(value.len()),
    )
    .saturating_add(attempt.previous_attempt_id.as_ref().map_or(0, String::len))
    .saturating_add(attempt.cost_class.as_ref().map_or(0, String::len))
}

fn snapshot_bytes(request: &ModelRequestIRV1) -> usize {
    request
        .instructions
        .iter()
        .fold(
            std::mem::size_of::<RequestSnapshot>(),
            |size, instruction| {
                size.saturating_add(std::mem::size_of::<CanonicalInstruction>())
                    .saturating_add(parts_bytes(&instruction.content))
            },
        )
        .saturating_add(request.messages.iter().fold(0usize, |size, message| {
            size.saturating_add(std::mem::size_of::<CanonicalMessage>())
                .saturating_add(message.name.as_ref().map_or(0, String::len))
                .saturating_add(parts_bytes(&message.content))
        }))
        .saturating_add(request.tools.iter().fold(0usize, |size, tool| {
            size.saturating_add(std::mem::size_of::<CanonicalTool>())
                .saturating_add(tool.name.len())
                .saturating_add(tool.description.as_ref().map_or(0, String::len))
                .saturating_add(tool.input_schema.as_ref().map_or(0, json_bytes))
                .saturating_add(tool.format.as_ref().map_or(0, json_bytes))
        }))
}

fn parts_bytes(parts: &[ContentPart]) -> usize {
    parts.iter().fold(
        parts
            .len()
            .saturating_mul(std::mem::size_of::<ContentPart>()),
        |size, part| {
            let heap = match part {
                ContentPart::Text { text } => text.len(),
                ContentPart::Image {
                    source: ImageSource::Url { url },
                } => url.len(),
                ContentPart::Image {
                    source: ImageSource::Base64 { media_type, data },
                } => media_type.len().saturating_add(data.len()),
                ContentPart::ToolCall {
                    logical_id,
                    namespace,
                    name,
                    arguments,
                    encrypted_function_args,
                    raw_arguments,
                    ..
                } => logical_id
                    .len()
                    .saturating_add(namespace.as_ref().map_or(0, String::len))
                    .saturating_add(name.len())
                    .saturating_add(json_bytes(arguments))
                    .saturating_add(raw_arguments.as_ref().map_or(0, String::len))
                    .saturating_add(encrypted_function_args.as_ref().map_or(0, |paths| {
                        paths.iter().fold(0usize, |size, path| {
                            size.saturating_add(std::mem::size_of::<String>() + path.len())
                        })
                    })),
                ContentPart::ToolResult {
                    logical_id, output, ..
                } => logical_id.len().saturating_add(match output {
                    ToolOutput::Text(text) => text.len(),
                    ToolOutput::Json(value) => json_bytes(value),
                }),
                ContentPart::ProviderState { state } => std::mem::size_of::<OpaqueProviderState>()
                    .saturating_add(state.kind.len())
                    .saturating_add(json_bytes(&state.value)),
            };
            size.saturating_add(heap)
        },
    )
}

fn json_bytes(value: &Value) -> usize {
    match value {
        Value::String(text) => text.len(),
        Value::Array(items) => items.iter().fold(
            items.len().saturating_mul(std::mem::size_of::<Value>()),
            |size, value| size.saturating_add(json_bytes(value)),
        ),
        // Covers cloned entries, key/Value storage, map capacity and buckets in
        // both serde map implementations. Strings themselves are counted below.
        Value::Object(fields) => {
            fields
                .iter()
                .fold(fields.len().saturating_mul(256), |size, (key, value)| {
                    size.saturating_add(key.len())
                        .saturating_add(json_bytes(value))
                })
        }
        Value::Number(number) => number.to_string().len(),
        _ => 0,
    }
}

#[cfg(test)]
mod allocation_tests {
    use super::*;

    #[test]
    fn admitted_job_header_and_allocation_fit_the_existing_per_job_charge() {
        assert!(std::mem::size_of::<Job>() + 2 * std::mem::size_of::<usize>() <= JOB_OVERHEAD);
    }
}
