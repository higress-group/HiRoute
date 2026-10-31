pub(crate) use std::collections::VecDeque;
pub(crate) use std::future::Future;
pub(crate) use std::net::SocketAddr;
pub(crate) use std::pin::Pin;
pub(crate) use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
pub(crate) use std::sync::{Arc, Mutex};
pub(crate) use std::task::{Context, Poll};
pub(crate) use std::time::{Duration, Instant};

pub(crate) use async_trait::async_trait;
pub(crate) use bytes::Bytes;
pub(crate) use hiroute_gateway_core::core::execution_plan::{
    AttemptBodyPlans, AttemptTimeouts, PlanRevision,
};
pub(crate) use hiroute_gateway_core::runtime::attempt::{
    AcceptBlockedReason, AttemptError, AttemptExchange, AttemptGeneration, AttemptId,
    AttemptTimeoutKind, AttemptTransport, CommitFence, Disposition, PrecommitEvent,
    PreparedAttemptBody, PreparedAttemptHttpRequest, PreparedRequestHead, RequestId,
    TransportPrecommitEvent, TransportPrecommitReceipt, WriterGate, WriterState,
};
pub(crate) use hiroute_gateway_core::runtime::body::{
    BodyError, BodyPlan, BudgetTree, ChargedBodyQueue, ChargedBytes, MemoryRole, RequestLeaseBook,
    StreamBudget,
};
pub(crate) use hiroute_gateway_core::runtime::driver::{LogicalRequestDriver, RequestState};
pub(crate) use hiroute_gateway_core::runtime::executor::{
    BoundedExecutor, ChildScope, ExecutorError, ExecutorKind,
};
pub(crate) use hiroute_gateway_core::runtime::scope::ScopeSupervisor;
pub(crate) use hiroute_gateway_core::test_support::plain_target;
pub(crate) use hiroute_gateway_core::transport::HttpProtocol;
pub(crate) use http::{HeaderMap, Method, StatusCode};
pub(crate) use proptest::prelude::*;
pub(crate) use static_assertions::assert_not_impl_any;

assert_not_impl_any!(hiroute_gateway_core::runtime::attempt::DispositionPublishPermit: Clone);

#[derive(Debug, Default)]
pub(crate) struct TransportStats {
    pub(crate) connects: Vec<SocketAddr>,
    pub(crate) heads: usize,
    pub(crate) body_bytes: usize,
    pub(crate) finishes: usize,
    pub(crate) resets: usize,
    pub(crate) response_polls: usize,
    pub(crate) retained_wire: Vec<Bytes>,
}

pub(crate) struct MockTransport {
    pub(crate) stats: Arc<Mutex<TransportStats>>,
    pub(crate) connect_failures: usize,
    pub(crate) write_head_fails: bool,
    pub(crate) reset_hangs: bool,
    pub(crate) events: VecDeque<TransportPrecommitEvent>,
    pub(crate) protocol: HttpProtocol,
}

#[async_trait]
impl AttemptTransport for MockTransport {
    async fn connect(
        &mut self,
        _target: &hiroute_gateway_core::core::execution_plan::TransportTarget,
        address: SocketAddr,
    ) -> Result<(), AttemptError> {
        self.stats.lock().unwrap().connects.push(address);
        if self.connect_failures > 0 {
            self.connect_failures -= 1;
            Err(AttemptError::ConnectFailed)
        } else {
            Ok(())
        }
    }

    async fn write_request_head(
        &mut self,
        _head: &PreparedRequestHead,
    ) -> Result<(), AttemptError> {
        self.stats.lock().unwrap().heads += 1;
        if self.write_head_fails {
            Err(AttemptError::Transport("partial header failure".into()))
        } else {
            Ok(())
        }
    }

    async fn write_request_body(
        &mut self,
        body: Bytes,
        _end_stream: bool,
    ) -> Result<(), AttemptError> {
        let mut stats = self.stats.lock().unwrap();
        stats.body_bytes += body.len();
        stats.retained_wire.push(body);
        Ok(())
    }

    async fn finish_request_body(&mut self) -> Result<(), AttemptError> {
        let mut stats = self.stats.lock().unwrap();
        stats.finishes += 1;
        stats.retained_wire.clear();
        Ok(())
    }

    fn poll_precommit(
        &mut self,
        _context: &mut Context<'_>,
    ) -> Poll<Result<Option<TransportPrecommitEvent>, AttemptError>> {
        self.stats.lock().unwrap().response_polls += 1;
        Poll::Ready(Ok(self.events.pop_front()))
    }

    async fn cancel_reset(&mut self) -> Result<(), AttemptError> {
        {
            let mut stats = self.stats.lock().unwrap();
            stats.resets += 1;
            stats.retained_wire.clear();
        }
        if self.reset_hangs {
            std::future::pending::<()>().await;
        }
        Ok(())
    }

    fn protocol(&self) -> HttpProtocol {
        self.protocol
    }
}

pub(crate) struct TimedTransport {
    pub(crate) connect_pending: bool,
    pub(crate) write_pending: bool,
    pub(crate) head_written: bool,
    pub(crate) events: VecDeque<(Duration, TransportPrecommitEvent)>,
    pub(crate) event_timer: Option<Pin<Box<tokio::time::Sleep>>>,
}

#[async_trait]
impl AttemptTransport for TimedTransport {
    async fn connect(
        &mut self,
        _target: &hiroute_gateway_core::core::execution_plan::TransportTarget,
        _address: SocketAddr,
    ) -> Result<(), AttemptError> {
        if self.connect_pending {
            std::future::pending::<()>().await;
        }
        Ok(())
    }

    async fn write_request_head(
        &mut self,
        _head: &PreparedRequestHead,
    ) -> Result<(), AttemptError> {
        if self.write_pending {
            std::future::pending::<()>().await;
        }
        self.head_written = true;
        Ok(())
    }

    async fn write_request_body(
        &mut self,
        _body: Bytes,
        _end_stream: bool,
    ) -> Result<(), AttemptError> {
        Ok(())
    }

    async fn finish_request_body(&mut self) -> Result<(), AttemptError> {
        Ok(())
    }

    fn poll_precommit(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Result<Option<TransportPrecommitEvent>, AttemptError>> {
        if !self.head_written {
            return Poll::Pending;
        }
        let Some((delay, _)) = self.events.front() else {
            return Poll::Pending;
        };
        if self.event_timer.is_none() {
            self.event_timer = Some(Box::pin(tokio::time::sleep(*delay)));
        }
        let timer = self
            .event_timer
            .as_mut()
            .expect("event timer was initialized");
        if timer.as_mut().poll(context).is_pending() {
            return Poll::Pending;
        }
        self.event_timer.take();
        Poll::Ready(Ok(self.events.pop_front().map(|(_, event)| event)))
    }

    async fn cancel_reset(&mut self) -> Result<(), AttemptError> {
        Ok(())
    }

    fn protocol(&self) -> HttpProtocol {
        HttpProtocol::Http1
    }
}

pub(crate) struct SuppressionRaceTransport {
    pub(crate) head_written: bool,
    pub(crate) stage: u8,
    pub(crate) head_received_at: Option<Instant>,
    pub(crate) suppression_started: Option<Instant>,
    pub(crate) suppression_completed: Duration,
    pub(crate) reported_suppression: Arc<Mutex<Option<Duration>>>,
    pub(crate) upstream_gap: Option<Pin<Box<tokio::time::Sleep>>>,
}

#[async_trait]
impl AttemptTransport for SuppressionRaceTransport {
    async fn connect(
        &mut self,
        _target: &hiroute_gateway_core::core::execution_plan::TransportTarget,
        _address: SocketAddr,
    ) -> Result<(), AttemptError> {
        Ok(())
    }

    async fn write_request_head(
        &mut self,
        _head: &PreparedRequestHead,
    ) -> Result<(), AttemptError> {
        self.head_written = true;
        let received_at = Instant::now();
        self.head_received_at = Some(received_at);
        self.suppression_started = Some(received_at);
        Ok(())
    }

    async fn write_request_body(
        &mut self,
        _body: Bytes,
        _end_stream: bool,
    ) -> Result<(), AttemptError> {
        Ok(())
    }

    async fn finish_request_body(&mut self) -> Result<(), AttemptError> {
        Ok(())
    }

    fn poll_precommit(
        &mut self,
        _context: &mut Context<'_>,
    ) -> Poll<Result<Option<TransportPrecommitEvent>, AttemptError>> {
        Poll::Pending
    }

    fn poll_precommit_receipt(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Result<Option<TransportPrecommitReceipt>, AttemptError>> {
        if !self.head_written {
            return Poll::Pending;
        }
        match self.stage {
            0 => {
                self.stage = 1;
                Poll::Ready(Ok(Some(TransportPrecommitReceipt {
                    event: TransportPrecommitEvent::ResponseHead {
                        status: StatusCode::OK,
                        headers: HeaderMap::new(),
                    },
                    received_at: self
                        .head_received_at
                        .expect("transport received the head when it wrote the request"),
                    local_read_suppressed: Duration::ZERO,
                    local_read_suppression_total_at_receipt: Some(Duration::ZERO),
                })))
            }
            1 => {
                self.suppression_completed = self
                    .suppression_started
                    .take()
                    .expect("suppression starts after the response head")
                    .elapsed();
                *self.reported_suppression.lock().unwrap() = Some(self.suppression_completed);
                self.stage = 2;
                self.upstream_gap = Some(Box::pin(tokio::time::sleep(Duration::from_millis(10))));
                self.poll_precommit_receipt(context)
            }
            2 => {
                let gap = self.upstream_gap.as_mut().expect("upstream gap is active");
                if gap.as_mut().poll(context).is_pending() {
                    return Poll::Pending;
                }
                self.upstream_gap.take();
                self.stage = 3;
                Poll::Ready(Ok(Some(TransportPrecommitReceipt {
                    event: TransportPrecommitEvent::Body(Bytes::from_static(
                        b"after-local-backpressure",
                    )),
                    received_at: Instant::now(),
                    // Deliberately report the same interval through the
                    // receipt and the cumulative counter. Attempt must choose
                    // the cumulative counter as its one authoritative source.
                    local_read_suppressed: self.suppression_completed,
                    local_read_suppression_total_at_receipt: Some(self.suppression_completed),
                })))
            }
            _ => Poll::Pending,
        }
    }

    fn local_read_suppression_total(&self) -> Option<Duration> {
        Some(
            self.suppression_completed.saturating_add(
                self.suppression_started
                    .map_or(Duration::ZERO, |started| started.elapsed()),
            ),
        )
    }

    async fn cancel_reset(&mut self) -> Result<(), AttemptError> {
        Ok(())
    }

    fn protocol(&self) -> HttpProtocol {
        HttpProtocol::Http1
    }
}

pub(crate) fn plans() -> AttemptBodyPlans {
    AttemptBodyPlans {
        attempt_request: BodyPlan::StreamingReplay {
            max_chunk_bytes: 4,
            max_replay_bytes: 64 * 1024,
        },
        attempt_response_precommit: BodyPlan::PassThrough {
            max_chunk_bytes: 64 * 1024,
        },
    }
}

pub(crate) fn timeouts() -> AttemptTimeouts {
    AttemptTimeouts {
        request_write: Duration::from_secs(1),
        first_byte: Duration::from_secs(1),
        stream_idle: Duration::from_secs(1),
    }
}

pub(crate) fn request(
    book: &RequestLeaseBook,
    budget: &StreamBudget,
    bytes: &[u8],
) -> PreparedAttemptHttpRequest {
    let plans = plans();
    request_for_plan(book, budget, bytes, &plans.attempt_request)
}

pub(crate) fn request_for_plan(
    book: &RequestLeaseBook,
    budget: &StreamBudget,
    bytes: &[u8],
    plan: &BodyPlan,
) -> PreparedAttemptHttpRequest {
    let mut body =
        ChargedBodyQueue::new(budget, MemoryRole::AttemptWire, plan, 64 * 1024, 64).unwrap();
    for chunk in bytes.chunks(4) {
        body.push_back(
            ChargedBytes::copy_from_opaque(budget, MemoryRole::AttemptWire, chunk).unwrap(),
        )
        .unwrap();
    }
    PreparedAttemptHttpRequest {
        network_route: None,
        head: PreparedRequestHead {
            method: Method::POST,
            path_and_query: Arc::from("/v1/chat"),
            headers: HeaderMap::new(),
        },
        body: PreparedAttemptBody::new(body, book.acquire().unwrap()).unwrap(),
    }
}

pub(crate) fn exchange(
    transport: MockTransport,
    book: &RequestLeaseBook,
    bytes: &[u8],
) -> AttemptExchange<MockTransport> {
    exchange_with_plans(transport, book, bytes, plans())
}

pub(crate) fn exchange_with_plans(
    transport: MockTransport,
    book: &RequestLeaseBook,
    bytes: &[u8],
    body_plans: AttemptBodyPlans,
) -> AttemptExchange<MockTransport> {
    let budgets = BudgetTree::new(1024 * 1024, 1024 * 1024).unwrap();
    let budget = budgets.stream(512 * 1024).unwrap();
    let request = request_for_plan(book, &budget, bytes, &body_plans.attempt_request);
    AttemptExchange::new(
        RequestId(1),
        AttemptId(2),
        AttemptGeneration(3),
        PlanRevision(4),
        plain_target("127.0.0.1:8080".parse().unwrap(), 9),
        transport,
        request,
        body_plans,
        timeouts(),
        Instant::now() + Duration::from_secs(5),
        16,
        budget,
        4,
    )
    .unwrap()
}

pub(crate) fn exchange_with_transport<T: AttemptTransport>(
    transport: T,
    target: hiroute_gateway_core::core::execution_plan::TransportTarget,
    phase_timeouts: AttemptTimeouts,
    attempt_deadline: Instant,
    precommit_capacity: usize,
) -> AttemptExchange<T> {
    let budgets = BudgetTree::new(1024 * 1024, 1024 * 1024).unwrap();
    let budget = budgets.stream(512 * 1024).unwrap();
    let leases = RequestLeaseBook::new();
    AttemptExchange::new(
        RequestId(901),
        AttemptId(902),
        AttemptGeneration(1),
        PlanRevision(903),
        target,
        transport,
        request(&leases, &budget, b""),
        plans(),
        phase_timeouts,
        attempt_deadline,
        precommit_capacity,
        budget,
        4,
    )
    .unwrap()
}

pub(crate) fn short_timeouts() -> AttemptTimeouts {
    AttemptTimeouts {
        request_write: Duration::from_millis(20),
        first_byte: Duration::from_millis(20),
        stream_idle: Duration::from_millis(25),
    }
}

pub(crate) fn timed_transport(
    events: impl IntoIterator<Item = (Duration, TransportPrecommitEvent)>,
) -> TimedTransport {
    TimedTransport {
        connect_pending: false,
        write_pending: false,
        head_written: false,
        events: events.into_iter().collect(),
        event_timer: None,
    }
}
