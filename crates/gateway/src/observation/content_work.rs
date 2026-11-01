//! Optional Content source ownership. Canonical history/usage are updated first.
use super::RequestObservation;
use super::content::CanonicalFrameDelivery;
use super::producer::{ChannelProducer, ObservationWorkReservation};
use super::request::AttemptObservation;
use super::schema::LifecycleFactV1;
use crate::replay::ReplayStore;
use crate::server::core_runtime::model_ir::{
    CanonicalInstruction, CanonicalMessage, CanonicalTool, ContentPart, ImageSource,
    ModelRequestIRV1, OpaqueProviderState, ToolOutput,
};
use serde_json::Value;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

#[path = "content_work/queue.rs"]
mod queue;
#[cfg(test)]
pub(super) use queue::ContentGate;
pub(super) use queue::ContentStatus;
use queue::{Command, JOB_OVERHEAD, WorkQueue};

const SNAPSHOT_LIMIT: usize = 16 * 1024 * 1024;
// Lazy cursor buffers, borrowed object indexes/stack are charged separately.
const CURSOR_BUFFERS: usize = 96 * 1024;

#[derive(Clone, Default)]
pub(super) struct ContentWorkProducer {
    shared: Arc<OnceLock<Arc<WorkQueue>>>,
    initialize: Arc<Mutex<()>>,
    #[cfg(test)]
    gate: queue::Gate,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct InputCaptureSelection {
    pub(crate) start: usize,
    pub(crate) ordinal_base: usize,
    pub(crate) current_ordinal: Option<u32>,
    pub(crate) boundary_verified: bool,
    pub(crate) turn_anchor: Option<String>,
}

struct RequestSnapshot {
    instructions: Vec<CanonicalInstruction>,
    messages: Vec<CanonicalMessage>,
    tools: Vec<CanonicalTool>,
}

pub(super) struct CaptureRun(Mutex<(usize, usize, Instant)>);
impl CaptureRun {
    fn new() -> Self {
        Self(Mutex::new((0, 0, Instant::now())))
    }
}

/// The synchronous fallback has no runtime, blocking executor or polling loop.
/// Its same traversal has no suspension points when background is false.
pub(super) fn inline<F: std::future::Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    match future.as_mut().poll(&mut context) {
        std::task::Poll::Ready(value) => value,
        std::task::Poll::Pending => panic!("Inline Content unexpectedly suspended"),
    }
}

impl RequestObservation {
    pub(super) async fn content_checkpoint(&self, bytes: usize, force: bool) {
        let Some(run) = &self.content_run else {
            return;
        };
        let should_yield = {
            let mut state = run.0.lock().unwrap_or_else(|error| error.into_inner());
            state.0 += 1;
            state.1 = state.1.saturating_add(bytes);
            if force
                || state.0 >= 32
                || state.1 >= 256 * 1024
                || state.2.elapsed() >= std::time::Duration::from_millis(2)
            {
                *state = (0, 0, Instant::now());
                true
            } else {
                false
            }
        };
        if should_yield {
            tokio::task::yield_now().await;
        }
    }

    pub(crate) fn select_input_capture(
        &self,
        request: &ModelRequestIRV1,
        start: Option<usize>,
        current: Option<usize>,
        turn_anchor: Option<String>,
    ) {
        let start = start.unwrap_or(0).min(request.messages.len());
        let _ = self.inner.input_capture.set(InputCaptureSelection {
            start,
            ordinal_base: request.instructions.len().saturating_add(start),
            current_ordinal: current
                .and_then(|n| u32::try_from(request.instructions.len().checked_add(n)?).ok()),
            boundary_verified: start > 0,
            turn_anchor,
        });
    }

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
        self.inner.content_spool.bind(replay.overflow_factory());
        if self.inner.input_capture.get().is_none() {
            self.select_input_capture(
                request,
                None,
                request
                    .messages
                    .iter()
                    .rposition(|m| m.role == crate::model_ir::MessageRole::User),
                None,
            );
        }
        let start = self.inner.input_capture.get().map_or(0, |s| s.start);
        let messages = &request.messages[start..];
        let bytes = snapshot_bytes(messages);
        let retained = bytes
            .saturating_add(replay.content_lease_memory_bytes())
            .saturating_add(snapshot_cursor_bytes(messages));
        if bytes > SNAPSHOT_LIMIT
            || !self
                .inner
                .channels
                .content
                .work_can_fit(retained.saturating_add(JOB_OVERHEAD))
        {
            self.capture_request_fallback(request, replay);
            return;
        }
        let Some(queue) = self
            .inner
            .channels
            .content_work
            .queue(&self.inner.channels.content)
        else {
            self.capture_request_fallback(request, replay);
            return;
        };
        let status = self
            .inner
            .async_content
            .get_or_init(|| Arc::new(ContentStatus::default()));
        if !queue.register(self, status) {
            self.capture_request_fallback(request, replay);
            return;
        }
        let Some(reservation) = self
            .inner
            .channels
            .content
            .reserve_work(retained.saturating_add(JOB_OVERHEAD))
        else {
            self.capture_request_fallback(request, replay);
            return;
        };
        let Ok(lease) = replay.content_lease() else {
            drop(reservation);
            self.capture_request_fallback(request, replay);
            return;
        };
        let snapshot = Box::new(RequestSnapshot {
            instructions: Vec::new(),
            messages: messages.to_vec(),
            tools: Vec::new(),
        });
        if !queue.push_data(
            self,
            status,
            reservation,
            Command::Request {
                snapshot,
                replay: lease,
            },
        ) {
            self.fail_content_work(status, "content_capture_queue_unavailable");
        }
    }

    fn capture_request_fallback(&self, _request: &ModelRequestIRV1, _replay: &ReplayStore) {
        let status = self
            .inner
            .async_content
            .get_or_init(|| Arc::new(ContentStatus::default()));
        // Optional archive pressure cannot run a full history traversal in the
        // forwarding lane. The accepted request remains serviceable.
        self.fail_content_work(status, "content_capture_queue_unavailable");
    }

    pub(super) fn begin_response_content(&self, attempt: &AttemptObservation) {
        if !self.captures_content() {
            return;
        }
        let Some(status) = self.inner.async_content.get() else {
            self.begin_content("response_delivered", Some(attempt));
            return;
        };
        if status
            .response_begin
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return;
        }
        self.enqueue_content_control(status, Command::BeginResponse(attempt.attempt_id.clone()));
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
        if status
            .response_terminal
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return;
        }
        self.enqueue_content_control(status, Command::TerminalResponse { phase, reason });
    }

    fn enqueue_content_control(&self, status: &Arc<ContentStatus>, command: Command) {
        let queue = self
            .inner
            .channels
            .content_work
            .queue(&self.inner.channels.content);
        if !queue.is_some_and(|queue| queue.push_control(self, status, command)) {
            self.fail_content_work(status, "content_capture_control_unavailable");
        }
    }

    pub(super) fn defer_response_content(
        &self,
        attempt: &AttemptObservation,
        frame_id: &str,
        mut delivery: CanonicalFrameDelivery,
    ) {
        let Some(status) = self.inner.async_content.get() else {
            self.emit_response_content(attempt, frame_id, delivery);
            return;
        };
        if status.failed() {
            return;
        }
        let retained = delivery
            .reservations
            .iter()
            .fold(0usize, |size, item| size.saturating_add(item.bytes()));
        let retained = if retained == 0 {
            let mut size = SerializedSize(0);
            if serde_json::to_writer(&mut size, &delivery.events).is_err() {
                self.fail_content_work(status, "content_capture_unavailable");
                return;
            }
            size.0.saturating_mul(4).saturating_add(512)
        } else {
            retained
        };
        let bytes = attempt
            .attempt_id
            .len()
            .saturating_add(frame_id.len())
            .saturating_add(retained)
            .saturating_add(
                delivery
                    .events
                    .capacity()
                    .saturating_mul(std::mem::size_of::<
                        crate::server::core_runtime::model_ir::ModelStreamEventV1,
                    >()),
            )
            .saturating_add(JOB_OVERHEAD);
        let Some(queue) = self
            .inner
            .channels
            .content_work
            .queue(&self.inner.channels.content)
        else {
            self.fail_content_work(status, "content_capture_queue_unavailable");
            return;
        };
        let Some(reservation) = self.inner.channels.content.reserve_work(bytes) else {
            self.fail_content_work(status, "content_capture_queue_unavailable");
            return;
        };
        // Typed ownership transfers only after independent Content admission.
        // Wire bytes have their own owner and never enter this transfer.
        delivery.reservations.clear();
        if !queue.push_data(
            self,
            status,
            reservation,
            Command::Delivery {
                attempt_id: attempt.attempt_id.clone(),
                frame_id: frame_id.into(),
                delivery,
            },
        ) {
            self.fail_content_work(status, "content_capture_queue_unavailable");
        }
    }

    pub(super) fn fail_content_work(&self, status: &Arc<ContentStatus>, reason: &'static str) {
        if !status.note_failure(reason) {
            return;
        }
        self.inner.channels.content.declare_work_loss();
        // Core records only bounded metadata; abort encoding/backing cleanup
        // belongs to the Content thread, not the transport/history owner.
        if let Some(queue) = self
            .inner
            .channels
            .content_work
            .queue(&self.inner.channels.content)
        {
            let _ = queue.wake(self, status);
        }
    }
}

impl ContentWorkProducer {
    #[cfg(test)]
    pub(super) fn paused() -> (Self, ContentGate) {
        let (gate, control) = ContentGate::new();
        (
            Self {
                gate,
                ..Self::default()
            },
            control,
        )
    }
    #[cfg(test)]
    pub(super) fn flush(&self, request: &RequestObservation) {
        let queue = self.shared.get().expect("started content worker");
        let status = request
            .inner
            .async_content
            .get()
            .expect("registered Content request");
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        assert!(queue.push_control(request, status, Command::Barrier(sender)));
        receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
    }
    fn queue(&self, content: &ChannelProducer) -> Option<Arc<WorkQueue>> {
        if let Some(queue) = self.shared.get() {
            return Some(Arc::clone(queue));
        }
        let _initializing = self.initialize.try_lock().ok()?;
        if let Some(queue) = self.shared.get() {
            return Some(Arc::clone(queue));
        }
        #[cfg(test)]
        let gate = self.gate.clone();
        #[cfg(not(test))]
        let gate = ();
        let queue = Arc::new(WorkQueue::spawn(content, gate)?);
        let _ = self.shared.set(Arc::clone(&queue));
        Some(queue)
    }
}

struct SerializedSize(usize);

impl std::io::Write for SerializedSize {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("content size overflow"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn snapshot_bytes(messages: &[CanonicalMessage]) -> usize {
    messages
        .iter()
        .fold(std::mem::size_of::<RequestSnapshot>(), |size, message| {
            size.saturating_add(std::mem::size_of::<CanonicalMessage>())
                .saturating_add(message.name.as_ref().map_or(0, String::len))
                .saturating_add(parts_bytes(&message.content))
        })
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

// Only one part is traversed at a time. Reserve its maximum borrowed stack and
// sorted object index in addition to the immutable source, before cloning it.
fn snapshot_cursor_bytes(messages: &[CanonicalMessage]) -> usize {
    fn value_bytes(value: &Value) -> usize {
        let frame = crate::context_hold::CanonicalJsonReplayCursor::frame_bytes();
        match value {
            Value::Array(items) => 2 * frame + items.iter().map(value_bytes).max().unwrap_or(0),
            Value::Object(fields) => {
                2 * frame
                    + fields
                        .len()
                        .saturating_mul(2 * std::mem::size_of::<(&str, &Value)>())
                    + fields.values().map(value_bytes).max().unwrap_or(0)
            }
            _ => 2 * frame,
        }
    }
    fn parts_bytes(parts: &[ContentPart]) -> usize {
        parts
            .iter()
            .map(|part| match part {
                ContentPart::ToolCall { arguments, .. } => value_bytes(arguments),
                ContentPart::ToolResult {
                    output: ToolOutput::Json(value),
                    ..
                } => value_bytes(value),
                ContentPart::ProviderState { state } => value_bytes(&state.value),
                _ => 0,
            })
            .max()
            .unwrap_or(0)
    }
    messages
        .iter()
        .map(|item| parts_bytes(&item.content))
        .max()
        .unwrap_or(0)
}
