use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::runtime::telemetry::{ExecutorOutcome, RequestTelemetry};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ExecutorKind {
    GatewayIo,
    SidecallIo,
    Compute,
    BlockingControl,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutorSnapshot {
    pub queued: usize,
    pub running: usize,
    pub admitted: usize,
    pub overloaded: usize,
    pub cancelled: usize,
    pub panicked: usize,
    pub late_dropped: usize,
}

#[derive(Debug)]
struct ExecutorMetrics {
    queued: AtomicUsize,
    running: AtomicUsize,
    admitted: AtomicUsize,
    overloaded: AtomicUsize,
    cancelled: AtomicUsize,
    panicked: AtomicUsize,
    late_dropped: AtomicUsize,
}

#[derive(Clone, Debug)]
pub struct BoundedExecutor {
    kind: ExecutorKind,
    queue_limit: usize,
    semaphore: Arc<Semaphore>,
    metrics: Arc<ExecutorMetrics>,
    telemetry: Option<RequestTelemetry>,
    backend: ExecutorBackend,
}

#[derive(Clone, Debug)]
enum ExecutorBackend {
    AsyncInline,
    Fixed(Arc<FixedBackend>),
}

type WorkerJob = Box<dyn FnOnce() + Send + 'static>;

struct FixedBackend {
    sender: Mutex<Option<SyncSender<WorkerJob>>>,
    workers: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl std::fmt::Debug for FixedBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FixedBackend")
            .finish_non_exhaustive()
    }
}

impl FixedBackend {
    fn new(
        kind: ExecutorKind,
        concurrency: usize,
        queue_limit: usize,
    ) -> Result<Self, ExecutorError> {
        let (sender, receiver) = sync_channel::<WorkerJob>(queue_limit);
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers: Vec<std::thread::JoinHandle<()>> = Vec::with_capacity(concurrency);
        for index in 0..concurrency {
            let receiver = Arc::clone(&receiver);
            let worker = match std::thread::Builder::new()
                .name(format!("hiroute-{kind:?}-{index}"))
                .spawn(move || {
                    loop {
                        let job = receiver
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .recv();
                        let Ok(job) = job else {
                            break;
                        };
                        // A single malformed compute/control callback must
                        // never retire a fixed worker and silently reduce the
                        // process-wide concurrency bound.
                        let _ = catch_unwind(AssertUnwindSafe(job));
                    }
                }) {
                Ok(worker) => worker,
                Err(_) => {
                    drop(sender);
                    for worker in workers {
                        let _ = worker.join();
                    }
                    return Err(ExecutorError::BackendUnavailable);
                }
            };
            workers.push(worker);
        }
        Ok(Self {
            sender: Mutex::new(Some(sender)),
            workers: Mutex::new(workers),
        })
    }

    fn submit(&self, job: WorkerJob) -> Result<(), ExecutorError> {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let sender = sender.as_ref().ok_or(ExecutorError::BackendUnavailable)?;
        sender.try_send(job).map_err(|error| match error {
            TrySendError::Full(_) => ExecutorError::BackendOverloaded,
            TrySendError::Disconnected(_) => ExecutorError::BackendUnavailable,
        })
    }
}

impl Drop for FixedBackend {
    fn drop(&mut self) {
        let sender = self
            .sender
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        drop(sender);

        let workers = self
            .workers
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for worker in workers.drain(..) {
            // Worker jobs deliberately do not retain the backend Arc, so the
            // last owner must be a request/application owner, never this
            // worker. Keep this guard defensive if that invariant regresses.
            if worker.thread().id() != std::thread::current().id() {
                let _ = worker.join();
            }
        }
    }
}

#[derive(Clone, Debug)]
struct ExecutorObserver {
    kind: ExecutorKind,
    metrics: Arc<ExecutorMetrics>,
    telemetry: Option<RequestTelemetry>,
}

impl ExecutorObserver {
    fn snapshot(&self) -> ExecutorSnapshot {
        ExecutorSnapshot {
            queued: self.metrics.queued.load(Ordering::Acquire),
            running: self.metrics.running.load(Ordering::Acquire),
            admitted: self.metrics.admitted.load(Ordering::Acquire),
            overloaded: self.metrics.overloaded.load(Ordering::Acquire),
            cancelled: self.metrics.cancelled.load(Ordering::Acquire),
            panicked: self.metrics.panicked.load(Ordering::Acquire),
            late_dropped: self.metrics.late_dropped.load(Ordering::Acquire),
        }
    }

    fn observe(&self, outcome: ExecutorOutcome, queue_wait: Duration) {
        if let Some(telemetry) = &self.telemetry {
            telemetry.executor(self.kind, outcome, self.snapshot(), queue_wait);
        }
    }
}

impl BoundedExecutor {
    pub fn new(
        kind: ExecutorKind,
        concurrency: usize,
        queue_limit: usize,
    ) -> Result<Self, ExecutorError> {
        if concurrency == 0 || queue_limit == 0 {
            return Err(ExecutorError::InvalidLimit);
        }
        let backend = match kind {
            ExecutorKind::Compute | ExecutorKind::BlockingControl => {
                ExecutorBackend::Fixed(Arc::new(FixedBackend::new(kind, concurrency, queue_limit)?))
            }
            ExecutorKind::GatewayIo | ExecutorKind::SidecallIo => ExecutorBackend::AsyncInline,
        };
        Ok(Self {
            kind,
            queue_limit,
            semaphore: Arc::new(Semaphore::new(concurrency)),
            metrics: Arc::new(ExecutorMetrics {
                queued: AtomicUsize::new(0),
                running: AtomicUsize::new(0),
                admitted: AtomicUsize::new(0),
                overloaded: AtomicUsize::new(0),
                cancelled: AtomicUsize::new(0),
                panicked: AtomicUsize::new(0),
                late_dropped: AtomicUsize::new(0),
            }),
            telemetry: None,
            backend,
        })
    }

    pub fn with_telemetry(mut self, telemetry: RequestTelemetry) -> Self {
        self.telemetry = Some(telemetry);
        self
    }

    pub fn kind(&self) -> ExecutorKind {
        self.kind
    }

    /// Admission happens before the caller copies its payload or creates the
    /// expensive future.
    pub fn try_admit(
        &self,
        estimated_payload_bytes: usize,
    ) -> Result<QueuedAdmission, ExecutorError> {
        if estimated_payload_bytes == usize::MAX {
            return Err(ExecutorError::PayloadEstimateOverflow);
        }
        let result =
            self.metrics
                .queued
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                    (queued < self.queue_limit).then_some(queued + 1)
                });
        if result.is_err() {
            self.metrics.overloaded.fetch_add(1, Ordering::Relaxed);
            self.observe(ExecutorOutcome::Overloaded, Duration::ZERO);
            return Err(ExecutorError::Overloaded);
        }
        self.metrics.admitted.fetch_add(1, Ordering::Relaxed);
        self.observe(ExecutorOutcome::Admitted, Duration::ZERO);
        Ok(QueuedAdmission {
            executor: self.clone(),
            queued_at: Instant::now(),
            consumed: false,
        })
    }

    pub fn snapshot(&self) -> ExecutorSnapshot {
        self.observer().snapshot()
    }

    fn observe(&self, outcome: ExecutorOutcome, queue_wait: Duration) {
        self.observer().observe(outcome, queue_wait);
    }

    fn observer(&self) -> ExecutorObserver {
        ExecutorObserver {
            kind: self.kind,
            metrics: Arc::clone(&self.metrics),
            telemetry: self.telemetry.clone(),
        }
    }
}

#[derive(Debug)]
pub struct QueuedAdmission {
    executor: BoundedExecutor,
    queued_at: Instant,
    consumed: bool,
}

impl QueuedAdmission {
    pub async fn run<F, T>(
        mut self,
        deadline: Instant,
        cancel: CancellationToken,
        future: F,
    ) -> Result<JobCompletion<T>, ExecutorError>
    where
        F: Future<Output = T> + Send,
        T: Send,
    {
        if !matches!(self.executor.backend, ExecutorBackend::AsyncInline) {
            return Err(ExecutorError::WrongExecutionMode);
        }
        let permit = match wait_for_permit(&self.executor, deadline, &cancel).await {
            Ok(permit) => permit,
            Err(error) => return Err(error),
        };
        self.executor.metrics.queued.fetch_sub(1, Ordering::AcqRel);
        self.consumed = true;
        self.executor.metrics.running.fetch_add(1, Ordering::AcqRel);
        let running = RunningGuard {
            metrics: Arc::clone(&self.executor.metrics),
            _permit: permit,
        };
        let started_at = Instant::now();
        let remaining = deadline.saturating_duration_since(started_at);
        let value = tokio::select! {
            _ = cancel.cancelled() => {
                self.executor.metrics.cancelled.fetch_add(1, Ordering::Relaxed);
                self.executor.observe(
                    ExecutorOutcome::Cancelled,
                    started_at.saturating_duration_since(self.queued_at),
                );
                return Err(ExecutorError::Cancelled);
            }
            result = tokio::time::timeout(remaining, future) => {
                match result {
                    Ok(value) => value,
                    Err(_) => {
                        self.executor.metrics.cancelled.fetch_add(1, Ordering::Relaxed);
                        self.executor.observe(
                            ExecutorOutcome::Cancelled,
                            started_at.saturating_duration_since(self.queued_at),
                        );
                        return Err(ExecutorError::DeadlineExceeded);
                    }
                }
            }
        };
        let io_completed_at = Instant::now();
        drop(running);
        self.executor.observe(
            ExecutorOutcome::Completed,
            started_at.saturating_duration_since(self.queued_at),
        );
        Ok(JobCompletion {
            value,
            queued_at: self.queued_at,
            started_at,
            io_completed_at,
        })
    }

    pub async fn run_offloaded<F, T>(
        self,
        deadline: Instant,
        cancel: CancellationToken,
        job: F,
    ) -> Result<JobCompletion<T>, ExecutorError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.run_offloaded_with_guard(deadline, cancel, job, None)
            .await
    }

    async fn run_offloaded_with_guard<F, T>(
        mut self,
        deadline: Instant,
        cancel: CancellationToken,
        job: F,
        child: Option<ChildRegistration>,
    ) -> Result<JobCompletion<T>, ExecutorError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let ExecutorBackend::Fixed(backend) = &self.executor.backend else {
            return Err(ExecutorError::WrongExecutionMode);
        };
        let permit = wait_for_permit(&self.executor, deadline, &cancel).await?;
        self.executor.metrics.queued.fetch_sub(1, Ordering::AcqRel);
        self.consumed = true;
        self.executor.metrics.running.fetch_add(1, Ordering::AcqRel);
        let running = RunningGuard {
            metrics: Arc::clone(&self.executor.metrics),
            _permit: permit,
        };
        let queued_at = self.queued_at;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        // The observer intentionally excludes the backend Arc. This lets the
        // request/application owner close the bounded channel and join every
        // fixed worker without a worker becoming the backend's last owner.
        let observer = self.executor.observer();
        backend.submit(Box::new(move || {
            let started_at = Instant::now();
            let value = catch_unwind(AssertUnwindSafe(job));
            let panicked = value.is_err();
            let io_completed_at = Instant::now();
            drop(running);
            let completion = match value {
                Ok(value) => Ok(JobCompletion {
                    value,
                    queued_at,
                    started_at,
                    io_completed_at,
                }),
                Err(_) => {
                    observer.metrics.panicked.fetch_add(1, Ordering::Relaxed);
                    Err(ExecutorError::JobPanicked)
                }
            };
            if sender.send(completion).is_err() {
                observer
                    .metrics
                    .late_dropped
                    .fetch_add(1, Ordering::Relaxed);
                observer.observe(
                    ExecutorOutcome::LateResultDropped,
                    started_at.saturating_duration_since(queued_at),
                );
            } else if !panicked {
                observer.observe(
                    ExecutorOutcome::Completed,
                    started_at.saturating_duration_since(queued_at),
                );
            } else {
                observer.observe(
                    ExecutorOutcome::Panicked,
                    started_at.saturating_duration_since(queued_at),
                );
            }
            // Joining this scope also joins rejected-result cleanup and the
            // terminal observation, not only the callback's return.
            drop(child);
        }))?;

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            self.executor
                .metrics
                .cancelled
                .fetch_add(1, Ordering::Relaxed);
            return Err(ExecutorError::DeadlineExceeded);
        }
        tokio::select! {
            _ = cancel.cancelled() => {
                self.executor.metrics.cancelled.fetch_add(1, Ordering::Relaxed);
                self.executor.observe(ExecutorOutcome::Cancelled, queued_at.elapsed());
                Err(ExecutorError::Cancelled)
            }
            result = tokio::time::timeout(remaining, receiver) => {
                match result {
                    Ok(Ok(Ok(completion))) => Ok(completion),
                    Ok(Ok(Err(error))) => Err(error),
                    Ok(Err(_)) => Err(ExecutorError::BackendUnavailable),
                    Err(_) => {
                        self.executor.metrics.cancelled.fetch_add(1, Ordering::Relaxed);
                        self.executor.observe(ExecutorOutcome::Cancelled, queued_at.elapsed());
                        Err(ExecutorError::DeadlineExceeded)
                    }
                }
            }
        }
    }
}

impl Drop for QueuedAdmission {
    fn drop(&mut self) {
        if !self.consumed {
            self.executor.metrics.queued.fetch_sub(1, Ordering::AcqRel);
            self.executor
                .metrics
                .cancelled
                .fetch_add(1, Ordering::Relaxed);
            self.executor
                .observe(ExecutorOutcome::Cancelled, self.queued_at.elapsed());
        }
    }
}

async fn wait_for_permit(
    executor: &BoundedExecutor,
    deadline: Instant,
    cancel: &CancellationToken,
) -> Result<OwnedSemaphorePermit, ExecutorError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(ExecutorError::DeadlineExceeded);
    }
    tokio::select! {
        _ = cancel.cancelled() => Err(ExecutorError::Cancelled),
        result = tokio::time::timeout(remaining, Arc::clone(&executor.semaphore).acquire_owned()) => {
            result
                .map_err(|_| ExecutorError::DeadlineExceeded)?
                .map_err(|_| ExecutorError::Closed)
        }
    }
}

struct RunningGuard {
    metrics: Arc<ExecutorMetrics>,
    _permit: OwnedSemaphorePermit,
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.metrics.running.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Debug)]
pub struct JobCompletion<T> {
    value: T,
    pub queued_at: Instant,
    pub started_at: Instant,
    pub io_completed_at: Instant,
}

impl<T> JobCompletion<T> {
    pub fn value(&self) -> &T {
        &self.value
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResumeTiming {
    pub io_completed_at: Instant,
    pub request_resumed_at: Instant,
    pub scheduler_resume_lag: Duration,
}

#[derive(Clone, Debug)]
pub struct ChildScope {
    deadline: Instant,
    cancel: CancellationToken,
    active: Arc<AtomicUsize>,
    cancelled: Arc<AtomicBool>,
    joined: Arc<AtomicBool>,
    registration: Arc<Mutex<()>>,
    notify: Arc<Notify>,
}

impl ChildScope {
    pub fn new(deadline: Instant) -> Self {
        Self::with_cancellation(deadline, CancellationToken::new())
    }

    pub fn with_cancellation(deadline: Instant, parent: CancellationToken) -> Self {
        Self {
            deadline,
            cancel: parent.child_token(),
            active: Arc::new(AtomicUsize::new(0)),
            cancelled: Arc::new(AtomicBool::new(false)),
            joined: Arc::new(AtomicBool::new(false)),
            registration: Arc::new(Mutex::new(())),
            notify: Arc::new(Notify::new()),
        }
    }

    pub async fn run_inline<F, T>(&self, future: F) -> Result<T, ExecutorError>
    where
        F: Future<Output = T> + Send,
        T: Send,
    {
        let registration = self
            .registration
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.cancelled.load(Ordering::Acquire) {
            return Err(ExecutorError::ScopeFinalized);
        }
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ExecutorError::DeadlineExceeded);
        }
        self.active.fetch_add(1, Ordering::AcqRel);
        drop(registration);
        let guard = ChildRegistration {
            active: Arc::clone(&self.active),
            notify: Arc::clone(&self.notify),
        };
        let result = tokio::select! {
            _ = self.cancel.cancelled() => Err(ExecutorError::Cancelled),
            result = tokio::time::timeout(remaining, future) => {
                result.map_err(|_| ExecutorError::DeadlineExceeded)
            }
        };
        drop(guard);
        result
    }

    pub async fn run_offloaded<F, T>(
        &self,
        executor: &BoundedExecutor,
        admission: QueuedAdmission,
        job: F,
    ) -> Result<JobCompletion<T>, ExecutorError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let registration = self
            .registration
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.cancelled.load(Ordering::Acquire) {
            return Err(ExecutorError::ScopeFinalized);
        }
        self.active.fetch_add(1, Ordering::AcqRel);
        drop(registration);
        let guard = ChildRegistration {
            active: Arc::clone(&self.active),
            notify: Arc::clone(&self.notify),
        };
        debug_assert_eq!(executor.kind(), admission.executor.kind());
        admission
            .run_offloaded_with_guard(self.deadline, self.cancel.child_token(), job, Some(guard))
            .await
    }

    pub fn cancel(&self) {
        let registration = self
            .registration
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.cancelled.store(true, Ordering::Release);
        self.cancel.cancel();
        drop(registration);
    }

    pub async fn run<F, T>(
        &self,
        executor: &BoundedExecutor,
        admission: QueuedAdmission,
        future: F,
    ) -> Result<JobCompletion<T>, ExecutorError>
    where
        F: Future<Output = T> + Send,
        T: Send,
    {
        let registration = self
            .registration
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.cancelled.load(Ordering::Acquire) {
            return Err(ExecutorError::ScopeFinalized);
        }
        self.active.fetch_add(1, Ordering::AcqRel);
        drop(registration);
        let guard = ChildRegistration {
            active: Arc::clone(&self.active),
            notify: Arc::clone(&self.notify),
        };
        debug_assert_eq!(executor.kind(), admission.executor.kind());
        let result = admission
            .run(self.deadline, self.cancel.child_token(), future)
            .await;
        drop(guard);
        result
    }

    pub fn resume<T>(
        &self,
        executor: &BoundedExecutor,
        completion: JobCompletion<T>,
    ) -> Result<(T, ResumeTiming), ExecutorError> {
        if self.cancelled.load(Ordering::Acquire) || self.cancel.is_cancelled() {
            executor
                .metrics
                .late_dropped
                .fetch_add(1, Ordering::Relaxed);
            executor.observe(ExecutorOutcome::LateResultDropped, Duration::ZERO);
            return Err(ExecutorError::ResultDropped);
        }
        let request_resumed_at = Instant::now();
        let timing = ResumeTiming {
            io_completed_at: completion.io_completed_at,
            request_resumed_at,
            scheduler_resume_lag: request_resumed_at
                .saturating_duration_since(completion.io_completed_at),
        };
        Ok((completion.value, timing))
    }

    pub async fn cancel_and_finalize(&self, join_timeout: Duration) -> Result<(), ExecutorError> {
        if self.joined.load(Ordering::Acquire) {
            return Ok(());
        }
        self.cancel();
        let wait = async {
            loop {
                // A worker may finish between the counter read and the await.
                // notify_waiters covers a Notified future from its creation.
                let notified = self.notify.notified();
                if self.active.load(Ordering::Acquire) == 0 {
                    break;
                }
                notified.await;
            }
        };
        tokio::time::timeout(join_timeout, wait)
            .await
            .map_err(|_| ExecutorError::JoinTimeout)?;
        self.joined.store(true, Ordering::Release);
        Ok(())
    }

    pub fn active_children(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }
}

struct ChildRegistration {
    active: Arc<AtomicUsize>,
    notify: Arc<Notify>,
}

impl Drop for ChildRegistration {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
        self.notify.notify_waiters();
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ExecutorError {
    #[error("executor concurrency and queue limits must be non-zero")]
    InvalidLimit,
    #[error("executor queue is overloaded")]
    Overloaded,
    #[error("payload estimate overflow")]
    PayloadEstimateOverflow,
    #[error("executor admission deadline exceeded")]
    DeadlineExceeded,
    #[error("executor job was cancelled")]
    Cancelled,
    #[error("executor is closed")]
    Closed,
    #[error("child scope is finalized")]
    ScopeFinalized,
    #[error("late result was dropped")]
    ResultDropped,
    #[error("executor job used the wrong async/offloaded execution mode")]
    WrongExecutionMode,
    #[error("fixed executor backend is unavailable")]
    BackendUnavailable,
    #[error("fixed executor backend queue is overloaded")]
    BackendOverloaded,
    #[error("fixed executor job panicked")]
    JobPanicked,
    #[error("child scope bounded join timed out")]
    JoinTimeout,
}
