//! One request summary, released after its existing bounded observation work.
//! No response waits for this owner and no payload or persisted DTO is changed.

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use hiroute_diagnostics::context::DiagnosticContext;
use hiroute_diagnostics::event::{
    DiagnosticEvent, IngressProtocol, RequestAsyncTiming, RequestLocalTiming, RequestOutcome,
    RequestTimingSummary, RequestWaitTiming,
};
use hiroute_diagnostics::identity::CorrelationToken;
use hiroute_gateway_core::runtime::timing::{RequestClock, TimingSnapshot};

#[derive(Clone)]
pub(in crate::server::core_runtime) struct RequestLatency(Arc<Inner>);

struct Inner {
    context: DiagnosticContext,
    protocol: Option<IngressProtocol>,
    token: Mutex<Option<CorrelationToken>>,
    status: AtomicU16,
    clock: RequestClock,
    finished: OnceLock<Finished>,
    request_outcome: OnceLock<RequestOutcome>,
    records: AtomicU64,
    delivered: AtomicU64,
    dropped: AtomicU64,
    queue_wait_max_us: AtomicU64,
    sink_work_us: AtomicU64,
    capture_jobs: AtomicU64,
    capture_dropped_jobs: AtomicU64,
    capture_queue_wait_max_us: AtomicU64,
    capture_work_us: AtomicU64,
}

struct Finished {
    at: Instant,
    timing: TimingSnapshot,
    outcome: RequestOutcome,
    status: Option<u16>,
}

tokio::task_local! {
    static CURRENT: RequestLatency;
}

impl RequestLatency {
    pub(in crate::server::core_runtime) fn new(
        context: DiagnosticContext,
        protocol: Option<IngressProtocol>,
        started: Instant,
    ) -> Self {
        Self(Arc::new(Inner {
            context,
            protocol,
            token: Mutex::new(None),
            status: AtomicU16::new(0),
            clock: RequestClock::started_at(started),
            finished: OnceLock::new(),
            request_outcome: OnceLock::new(),
            records: AtomicU64::new(0),
            delivered: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            queue_wait_max_us: AtomicU64::new(0),
            sink_work_us: AtomicU64::new(0),
            capture_jobs: AtomicU64::new(0),
            capture_dropped_jobs: AtomicU64::new(0),
            capture_queue_wait_max_us: AtomicU64::new(0),
            capture_work_us: AtomicU64::new(0),
        }))
    }

    pub(in crate::server::core_runtime) fn current() -> Option<Self> {
        CURRENT.try_with(Clone::clone).ok()
    }

    pub(in crate::server::core_runtime) fn context(&self) -> DiagnosticContext {
        self.0.context.clone()
    }

    pub(in crate::server::core_runtime) fn response_status(&self, status: u16) {
        self.0.status.store(status, Relaxed);
    }

    pub(in crate::server::core_runtime) fn status(&self) -> Option<u16> {
        match self.0.status.load(Relaxed) {
            0 => None,
            status => Some(status),
        }
    }

    pub(in crate::server::core_runtime) fn bind(&self, token: Option<CorrelationToken>) {
        *self.0.token.lock().unwrap_or_else(|e| e.into_inner()) = token;
    }

    pub(in crate::server::core_runtime) async fn run<F: Future>(&self, future: F) -> F::Output {
        CURRENT.scope(self.clone(), self.0.clock.run(future)).await
    }

    pub(in crate::server::core_runtime) fn finish(
        &self,
        outcome: RequestOutcome,
        status: Option<u16>,
    ) {
        self.0.finished.get_or_init(|| Finished {
            timing: self.0.clock.finish(),
            at: Instant::now(),
            outcome: if outcome == RequestOutcome::Completed {
                self.0.request_outcome.get().copied().unwrap_or(outcome)
            } else {
                outcome
            },
            status,
        });
    }

    pub(in crate::server::core_runtime) fn request_outcome(&self, outcome: RequestOutcome) {
        let _ = self.0.request_outcome.set(outcome);
    }

    pub(super) fn record(&self) -> AsyncRecordTiming {
        self.0.records.fetch_add(1, Relaxed);
        AsyncRecordTiming(Arc::new(RecordInner {
            request: self.clone(),
            queued_at: Instant::now(),
            resolved: AtomicBool::new(false),
        }))
    }

    pub(super) fn capture_completed(&self, queued: Instant, started: Instant, dropped: bool) {
        self.0.capture_jobs.fetch_add(1, Relaxed);
        self.0.capture_queue_wait_max_us.fetch_max(
            started
                .saturating_duration_since(queued)
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
            Relaxed,
        );
        self.0
            .capture_work_us
            .fetch_add(elapsed_us(started), Relaxed);
        if dropped {
            self.0.capture_dropped_jobs.fetch_add(1, Relaxed);
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        let Some(finished) = self.finished.get() else {
            return;
        };
        let [
            ingress_us,
            replay_us,
            parse_us,
            externalize_us,
            prevalidate_us,
            history_us,
            decision_us,
            plan_us,
            prepare_us,
            stream_us,
            other_us,
        ] = finished.timing.local_us;
        let local = RequestLocalTiming {
            ingress_us,
            replay_us,
            parse_us,
            externalize_us,
            prevalidate_us,
            history_us,
            decision_us,
            plan_us,
            prepare_us,
            stream_us,
            other_us,
        };
        let [
            client_read_us,
            decision_us,
            connect_us,
            upstream_write_us,
            upstream_read_us,
            downstream_write_us,
            local_work_us,
            other_us,
        ] = finished.timing.wait_us;
        self.context.emit(DiagnosticEvent::RequestTimingSummary(
            RequestTimingSummary {
                request_token: *self.token.lock().unwrap_or_else(|e| e.into_inner()),
                protocol: self.protocol,
                outcome: finished.outcome,
                http_status: finished.status,
                elapsed_us: finished.timing.elapsed_us,
                local,
                wait: RequestWaitTiming {
                    client_read_us,
                    decision_us,
                    connect_us,
                    upstream_write_us,
                    upstream_read_us,
                    downstream_write_us,
                    local_work_us,
                    other_us,
                },
                asynchronous: RequestAsyncTiming {
                    capture_jobs: self.capture_jobs.load(Relaxed),
                    capture_dropped_jobs: self.capture_dropped_jobs.load(Relaxed),
                    capture_queue_wait_max_us: self.capture_queue_wait_max_us.load(Relaxed),
                    capture_work_us: self.capture_work_us.load(Relaxed),
                    records: self.records.load(Relaxed),
                    delivered: self.delivered.load(Relaxed),
                    dropped: self.dropped.load(Relaxed),
                    queue_wait_max_us: self.queue_wait_max_us.load(Relaxed),
                    sink_work_us: self.sink_work_us.load(Relaxed),
                    after_response_us: if self.records.load(Relaxed) == 0 {
                        0
                    } else {
                        elapsed_us(finished.at)
                    },
                },
            },
        ));
    }
}

#[derive(Clone)]
pub(super) struct AsyncRecordTiming(Arc<RecordInner>);

struct RecordInner {
    request: RequestLatency,
    queued_at: Instant,
    resolved: AtomicBool,
}

impl std::fmt::Debug for AsyncRecordTiming {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncRecordTiming").finish_non_exhaustive()
    }
}

impl AsyncRecordTiming {
    pub(super) fn begin_delivery(&self) -> Instant {
        self.0
            .request
            .0
            .queue_wait_max_us
            .fetch_max(elapsed_us(self.0.queued_at), Relaxed);
        Instant::now()
    }

    pub(super) fn end_delivery(&self, started: Instant, delivered: bool) {
        if self.0.resolved.swap(true, Relaxed) {
            return;
        }
        self.0
            .request
            .0
            .sink_work_us
            .fetch_add(elapsed_us(started), Relaxed);
        let count = if delivered {
            &self.0.request.0.delivered
        } else {
            &self.0.request.0.dropped
        };
        count.fetch_add(1, Relaxed);
    }
}

impl Drop for RecordInner {
    fn drop(&mut self) {
        if !self.resolved.load(Relaxed) {
            self.request.0.dropped.fetch_add(1, Relaxed);
        }
    }
}

fn elapsed_us(started: Instant) -> u64 {
    started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
#[path = "latency/tests.rs"]
mod tests;
