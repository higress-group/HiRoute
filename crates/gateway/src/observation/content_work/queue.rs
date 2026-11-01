//! One ready signal per request; controls do not compete with source bytes.
use std::collections::VecDeque;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
#[cfg(test)]
use std::sync::Condvar;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Instant;
use tokio::sync::mpsc::{Sender, channel};

use super::{
    CanonicalFrameDelivery, ChannelProducer, ObservationWorkReservation, ReplayStore,
    RequestObservation, RequestSnapshot,
};

pub(super) const JOB_OVERHEAD: usize = 1024;
fn status_bytes() -> usize {
    // Includes the concrete pinned traversal future, not a maximum inline
    // payload or eagerly allocated cursor buffers. Reserve before spawn/pin.
    (std::mem::size_of::<ContentStatus>()
        + std::mem::size_of_val(&run_ready(None))
        + 3 * (std::mem::size_of::<Job>() + 2 * std::mem::size_of::<usize>())
        + std::mem::size_of::<super::CaptureRun>()
        + 4096)
        .next_power_of_two()
}

#[cfg(test)]
pub(super) type Gate = Option<Arc<(Mutex<bool>, Condvar)>>;
#[cfg(not(test))]
pub(super) type Gate = ();

#[derive(Default)]
pub(in super::super) struct ContentStatus {
    failed: AtomicBool,
    pub(super) response_begin: AtomicBool,
    pub(super) response_terminal: AtomicBool,
    state: Mutex<RequestWork>,
}

#[derive(Default)]
struct RequestWork {
    _control: Option<ObservationWorkReservation>,
    queued: bool,
    next: u64,
    jobs: VecDeque<Box<Job>>,
    begin: Option<Box<Job>>,
    terminal: Option<Box<Job>>,
    failure: Option<&'static str>,
    failure_emitted: bool,
    #[cfg(test)]
    barriers: VecDeque<Box<Job>>,
}

pub(super) struct Job {
    ordinal: u64,
    queued: Instant,
    occurred_at: u64,
    command: Command,
    retained: Option<ObservationWorkReservation>,
}

pub(super) enum Command {
    Request {
        snapshot: Box<RequestSnapshot>,
        replay: ReplayStore,
    },
    Delivery {
        attempt_id: String,
        frame_id: String,
        delivery: CanonicalFrameDelivery,
    },
    BeginResponse(String),
    TerminalResponse {
        phase: &'static str,
        reason: Option<&'static str>,
    },
    Abort(&'static str),
    #[cfg(test)]
    Barrier(SyncSender<()>),
}

struct ReadyRequest {
    request: RequestObservation,
    status: Arc<ContentStatus>,
}

pub(super) struct WorkQueue {
    sender: Sender<Box<ReadyRequest>>,
}

fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|error| error.into_inner())
}

impl ContentStatus {
    pub(in super::super) fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    pub(in super::super) fn note_failure(&self, reason: &'static str) -> bool {
        let mut state = lock(&self.state);
        if state.failure.is_some() || state.failure_emitted {
            return false;
        }
        state.failure = Some(reason);
        self.failed.store(true, Ordering::Release);
        true
    }

    pub(in super::super) fn stop(&self) {
        self.note_failure("content_capture_unavailable");
    }
}

impl RequestWork {
    fn ordinal(&mut self) -> u64 {
        self.next = self
            .next
            .checked_add(1)
            .expect("content work ordinal overflow");
        self.next
    }

    fn has_work(&self) -> bool {
        if self.failure.is_some() {
            return true;
        }
        #[cfg(test)]
        if !self.barriers.is_empty() {
            return true;
        }
        !self.jobs.is_empty() || self.begin.is_some() || self.terminal.is_some()
    }

    fn take(&mut self) -> (Option<Box<Job>>, VecDeque<Box<Job>>) {
        if let Some(reason) = self.failure.take() {
            self.failure_emitted = true;
            let discard = std::mem::take(&mut self.jobs);
            self.begin = None;
            self.terminal = None;
            return (
                Some(Box::new(Job {
                    ordinal: self.next,
                    queued: Instant::now(),
                    occurred_at: super::super::unix_nanos(),
                    command: Command::Abort(reason),
                    retained: None,
                })),
                discard,
            );
        }
        let mut next = self.jobs.front().map_or(u64::MAX, |job| job.ordinal);
        if let Some(begin) = &self.begin {
            next = next.min(begin.ordinal);
        }
        if let Some(terminal) = &self.terminal {
            next = next.min(terminal.ordinal);
        }
        #[cfg(test)]
        if let Some(barrier) = self.barriers.front() {
            next = next.min(barrier.ordinal);
        }
        let job = if self.begin.as_ref().is_some_and(|job| job.ordinal == next) {
            self.begin.take()
        } else if self
            .terminal
            .as_ref()
            .is_some_and(|job| job.ordinal == next)
        {
            self.terminal.take()
        } else if self.jobs.front().is_some_and(|job| job.ordinal == next) {
            self.jobs.pop_front()
        } else {
            #[cfg(test)]
            {
                self.barriers.pop_front()
            }
            #[cfg(not(test))]
            {
                None
            }
        };
        (job, VecDeque::new())
    }
}

impl WorkQueue {
    pub(super) fn spawn(content: &ChannelProducer, gate: Gate) -> Option<Self> {
        let slots = (content.content_capacity()? / status_bytes()).max(1) + 1;
        let storage = slots
            * (std::mem::size_of::<Box<ReadyRequest>>() + 2 * std::mem::size_of::<usize>())
            + JOB_OVERHEAD;
        let storage = content.reserve_control_work(storage)?;
        let (sender, mut receiver) = channel::<Box<ReadyRequest>>(slots);
        let runtime = tokio::runtime::Builder::new_current_thread().build().ok()?;
        std::thread::Builder::new().name("hiroute-content-capture".into()).spawn(move || {
            let _storage = storage;
            // Test-only gate blocks this thread before, never inside, its runtime.
            wait_gate(&gate);
            runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
                let mut running = tokio::task::JoinSet::new();
                loop {
                    tokio::select! {
                        Some(result) = running.join_next(), if !running.is_empty() => {
                            // Individual job panics are converted to capture loss
                            // inside run_ready; a task-level failure is not hidden.
                            if let Err(error) = result { eprintln!("content task failed: {error}"); }
                        }
                        ready = receiver.recv() => match ready {
                            Some(ready) => { running.spawn_local(run_ready(Some(ready))); }
                            None => { while running.join_next().await.is_some() {} return; }
                        }
                    }
                }
            }));
        }).ok()?;
        Some(Self { sender })
    }

    pub(super) fn register(&self, request: &RequestObservation, status: &ContentStatus) -> bool {
        let mut state = lock(&status.state);
        if state._control.is_none() {
            let Some(reservation) = request
                .inner
                .channels
                .content
                .reserve_control_work(status_bytes())
            else {
                return false;
            };
            state._control = Some(reservation);
        }
        true
    }

    pub(super) fn wake(&self, request: &RequestObservation, status: &Arc<ContentStatus>) -> bool {
        if !self.register(request, status) {
            return false;
        }
        {
            let mut state = lock(&status.state);
            if state.queued {
                return true;
            }
            state.queued = true;
        }
        if self
            .sender
            .try_send(Box::new(ReadyRequest {
                request: request.clone(),
                status: Arc::clone(status),
            }))
            .is_err()
        {
            lock(&status.state).queued = false;
            return false;
        }
        true
    }

    pub(super) fn push_data(
        &self,
        request: &RequestObservation,
        status: &Arc<ContentStatus>,
        retained: ObservationWorkReservation,
        command: Command,
    ) -> bool {
        {
            let mut state = lock(&status.state);
            if status.failed() {
                return false;
            }
            let ordinal = state.ordinal();
            state.jobs.push_back(Box::new(Job {
                ordinal,
                queued: Instant::now(),
                occurred_at: request
                    .content_occurred_at
                    .unwrap_or_else(super::super::unix_nanos),
                command,
                retained: Some(retained),
            }));
        }
        self.wake(request, status)
    }

    pub(super) fn push_control(
        &self,
        request: &RequestObservation,
        status: &Arc<ContentStatus>,
        command: Command,
    ) -> bool {
        if !self.register(request, status) {
            return false;
        }
        {
            let mut state = lock(&status.state);
            let ordinal = state.ordinal();
            let job = Box::new(Job {
                ordinal,
                queued: Instant::now(),
                occurred_at: request
                    .content_occurred_at
                    .unwrap_or_else(super::super::unix_nanos),
                command,
                retained: None,
            });
            match &job.command {
                Command::BeginResponse(_) => {
                    state.begin = Some(job);
                }
                Command::TerminalResponse { .. } => {
                    state.terminal = Some(job);
                }
                #[cfg(test)]
                Command::Barrier(_) => {
                    state.barriers.push_back(job);
                }
                _ => return false,
            }
        }
        self.wake(request, status)
    }
}

async fn caught<F: Future>(future: F) -> Result<F::Output, ()> {
    let mut future = std::pin::pin!(future);
    std::future::poll_fn(|context| {
        match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(context))) {
            Ok(Poll::Ready(value)) => Poll::Ready(Ok(value)),
            Ok(Poll::Pending) => Poll::Pending,
            Err(_) => Poll::Ready(Err(())),
        }
    })
    .await
}

async fn process(request: &RequestObservation, command: Command) -> bool {
    match command {
        Command::Request { snapshot, replay } => {
            request
                .capture_request_payload_run(
                    &snapshot.instructions,
                    &snapshot.messages,
                    &snapshot.tools,
                    &replay,
                )
                .await
        }
        Command::Delivery {
            attempt_id,
            frame_id,
            delivery,
        } => {
            request
                .emit_response_content_at_run(&attempt_id, &frame_id, delivery)
                .await;
            true
        }
        Command::BeginResponse(attempt_id) => {
            request
                .begin_content_at_run("response_delivered", Some(&attempt_id))
                .await;
            true
        }
        Command::TerminalResponse { phase, reason } => {
            request
                .terminal_content_run("response_delivered", phase, reason)
                .await;
            true
        }
        Command::Abort(reason) => {
            request
                .terminal_content_run("request_input", "abort", Some(reason))
                .await;
            request
                .terminal_content_run("response_delivered", "abort", Some(reason))
                .await;
            true
        }
        #[cfg(test)]
        Command::Barrier(sender) => {
            let _ = sender.send(());
            true
        }
    }
}

async fn run_ready(ready: Option<Box<ReadyRequest>>) {
    let Some(ready) = ready else {
        return;
    };
    loop {
        let (job, discard) = {
            let mut state = lock(&ready.status.state);
            if !state.has_work() {
                state.queued = false;
                return;
            }
            state.take()
        };
        // Backing handles and large sources are dropped outside the state lock.
        drop(discard);
        let Some(mut job) = job else {
            continue;
        };
        if let Some(retained) = &mut job.retained {
            ready
                .request
                .inner
                .channels
                .content
                .activate_content_work(retained, super::CURSOR_BUFFERS)
                .await;
        }
        let mut request = ready.request.clone();
        request.content_background = true;
        request.content_run = Some(Arc::new(super::CaptureRun::new()));
        request.content_occurred_at = Some(job.occurred_at);
        let started = Instant::now();
        let result = caught(process(&request, job.command)).await;
        match result {
            Ok(true) => {}
            Ok(false) => request.fail_content_work(&ready.status, "content_capture_unavailable"),
            Err(()) => request.fail_content_work(&ready.status, "content_capture_worker_panicked"),
        }
        // Source and cursor ownership outlives the whole traversal future.
        drop(job.retained);
        if let Some(latency) = &request.inner.latency {
            latency.capture_completed(job.queued, started, ready.status.failed());
        }
        tokio::task::yield_now().await;
    }
}

#[cfg(test)]
fn wait_gate(gate: &Gate) {
    if let Some(gate) = gate {
        let mut open = lock(&gate.0);
        while !*open {
            open = gate.1.wait(open).unwrap();
        }
    }
}
#[cfg(not(test))]
fn wait_gate(_: &Gate) {}

#[cfg(test)]
pub(in super::super) struct ContentGate(Arc<(Mutex<bool>, Condvar)>);

#[cfg(test)]
impl ContentGate {
    pub(super) fn new() -> (Gate, Self) {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        (Some(Arc::clone(&gate)), Self(gate))
    }
    pub(super) fn release(&self) {
        *lock(&self.0.0) = true;
        self.0.1.notify_all();
    }
}
#[cfg(test)]
impl Drop for ContentGate {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_request_controls_and_pointer_queue_fit_the_reserved_metadata() {
        assert!(
            std::mem::size_of::<ContentStatus>() + 3 * std::mem::size_of::<Job>() + 512
                <= status_bytes()
        );
        assert!(std::mem::size_of::<Job>() + 2 * std::mem::size_of::<usize>() <= JOB_OVERHEAD);
    }
}
