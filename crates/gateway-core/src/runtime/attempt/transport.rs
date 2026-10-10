use super::*;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RequestId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AttemptId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AttemptGeneration(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptTimeoutKind {
    Connect,
    RequestWrite,
    FirstByte,
    StreamIdle,
    AttemptDeadline,
}

#[derive(Clone, Debug)]
pub struct AttemptTransportFacts {
    pub started_at: Instant,
    /// Captured only after a successful connection; retained after cleanup.
    pub upstream_protocol: Option<HttpProtocol>,
    pub connect_elapsed: Option<Duration>,
    pub request_write_elapsed: Option<Duration>,
    /// Measured at the transport receipt boundary, never at downstream write.
    pub upstream_ttfb: Option<Duration>,
    pub last_upstream_progress_at: Option<Instant>,
    pub local_read_suppressed: Duration,
    pub upstream_body_bytes: u64,
    pub timeout: Option<AttemptTimeoutKind>,
}

#[derive(Clone, Debug)]
pub struct PreparedRequestHead {
    pub method: Method,
    pub path_and_query: Arc<str>,
    pub headers: HeaderMap,
}

#[derive(Debug)]
pub struct PreparedAttemptHttpRequest {
    pub head: PreparedRequestHead,
    pub body: PreparedAttemptBody,
}

/// One Attempt's independent sequential view of request wire bytes. The
/// logical replay owner remains outside the Attempt; this reader may retain
/// only bounded structural state and one charged output quantum.
pub trait AttemptRequestBodyReader: Send + std::fmt::Debug {
    fn visible_bytes(&self) -> usize;
    fn max_chunk_bytes(&self) -> usize;
    fn high_water_bytes(&self) -> usize {
        self.max_chunk_bytes().min(self.visible_bytes())
    }
    fn next_chunk(&mut self) -> Result<Option<ChargedBytes>, AttemptError>;
    fn release(&mut self);
}

#[derive(Debug)]
pub struct PreparedAttemptBody {
    source: Option<PreparedAttemptSource>,
    sequential_remaining: Option<usize>,
    sequential_quantum: Option<usize>,
    lease: Option<RequestBodyLease>,
}

#[derive(Debug)]
enum PreparedAttemptSource {
    Queue(ChargedBodyQueue),
    Sequential(Box<dyn AttemptRequestBodyReader>),
}

impl PreparedAttemptBody {
    pub fn new(chunks: ChargedBodyQueue, lease: RequestBodyLease) -> Result<Self, AttemptError> {
        if chunks
            .chunks()
            .any(|chunk| chunk.role() != MemoryRole::AttemptWire)
        {
            return Err(AttemptError::Body(BodyError::WrongMemoryRole));
        }
        Ok(Self {
            source: Some(PreparedAttemptSource::Queue(chunks)),
            sequential_remaining: None,
            sequential_quantum: None,
            lease: Some(lease),
        })
    }

    pub fn from_reader(
        reader: Box<dyn AttemptRequestBodyReader>,
        lease: RequestBodyLease,
    ) -> Result<Self, AttemptError> {
        let quantum = reader.max_chunk_bytes();
        if quantum == 0 {
            return Err(AttemptError::RequestChunkExceedsWriteQuantum);
        }
        let visible_bytes = reader.visible_bytes();
        Ok(Self {
            source: Some(PreparedAttemptSource::Sequential(reader)),
            sequential_remaining: Some(visible_bytes),
            sequential_quantum: Some(quantum),
            lease: Some(lease),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.visible_bytes() == 0
    }

    pub fn visible_bytes(&self) -> usize {
        match self.source.as_ref() {
            Some(PreparedAttemptSource::Queue(chunks)) => chunks.visible_bytes(),
            Some(PreparedAttemptSource::Sequential(_)) => {
                self.sequential_remaining.unwrap_or_default()
            }
            None => 0,
        }
    }

    pub(super) fn max_chunk_bytes(&self) -> usize {
        match self.source.as_ref() {
            Some(PreparedAttemptSource::Queue(chunks)) => chunks
                .chunks()
                .map(|chunk| chunk.bytes().len())
                .max()
                .unwrap_or(1),
            Some(PreparedAttemptSource::Sequential(_)) => {
                self.sequential_quantum.unwrap_or_default()
            }
            None => 0,
        }
    }

    pub(super) fn high_water_bytes(&self) -> usize {
        match self.source.as_ref() {
            Some(PreparedAttemptSource::Queue(chunks)) => chunks.high_water_bytes(),
            Some(PreparedAttemptSource::Sequential(_)) => {
                self.max_chunk_bytes().min(self.visible_bytes())
            }
            None => 0,
        }
    }

    pub(super) fn validate_body_plan(
        &self,
        owner: &mut BodyPlanExecutor,
    ) -> Result<(), AttemptError> {
        let mut remaining = self.visible_bytes();
        let quantum = self.max_chunk_bytes();
        if remaining != 0 && quantum == 0 {
            return Err(AttemptError::RequestChunkExceedsWriteQuantum);
        }
        while remaining != 0 {
            let chunk = remaining.min(quantum);
            owner.admit_chunk(chunk)?;
            remaining -= chunk;
        }
        owner.finish()?;
        Ok(())
    }

    /// Temporarily transfers the queued wire units to the RouteAttempt
    /// request-filter owner. The request-body lease remains attached to this
    /// object, so a callback cannot manufacture a lease-zero proof merely by
    /// moving bytes through the filter chain.
    pub(crate) fn take_filter_chunks(&mut self) -> Result<ChargedBodyQueue, AttemptError> {
        match self.source.take() {
            Some(PreparedAttemptSource::Queue(chunks)) => Ok(chunks),
            Some(source @ PreparedAttemptSource::Sequential(_)) => {
                self.source = Some(source);
                Err(AttemptError::SequentialBodyFilterUnsupported)
            }
            None => Err(AttemptError::SequentialBodyFilterUnsupported),
        }
    }

    pub(crate) fn replace_filter_chunks(
        &mut self,
        chunks: ChargedBodyQueue,
    ) -> Result<(), AttemptError> {
        if self.source.is_some()
            || chunks
                .chunks()
                .any(|chunk| chunk.role() != MemoryRole::AttemptWire)
        {
            return Err(AttemptError::Body(BodyError::WrongMemoryRole));
        }
        self.source = Some(PreparedAttemptSource::Queue(chunks));
        self.sequential_remaining = None;
        self.sequential_quantum = None;
        Ok(())
    }

    pub(super) fn begin_next_send(&mut self) -> Result<Option<Bytes>, AttemptError> {
        let chunk = match self.source.as_mut() {
            Some(PreparedAttemptSource::Queue(chunks)) => chunks.pop_front(),
            Some(PreparedAttemptSource::Sequential(reader)) => {
                let chunk = reader.next_chunk()?;
                let quantum = self
                    .sequential_quantum
                    .ok_or(AttemptError::SequentialBodyContractViolation)?;
                let remaining = self
                    .sequential_remaining
                    .as_mut()
                    .ok_or(AttemptError::SequentialBodyContractViolation)?;
                match chunk.as_ref() {
                    Some(chunk)
                        if chunk.bytes().is_empty()
                            || chunk.bytes().len() > quantum
                            || chunk.bytes().len() > *remaining =>
                    {
                        return Err(AttemptError::SequentialBodyContractViolation);
                    }
                    Some(chunk) => *remaining -= chunk.bytes().len(),
                    None if *remaining != 0 => {
                        return Err(AttemptError::SequentialBodyContractViolation);
                    }
                    None => {}
                }
                chunk
            }
            None => None,
        };
        let Some(chunk) = chunk else { return Ok(None) };
        let chunk = chunk.transfer_role(MemoryRole::TransportInflight)?;
        Ok(Some(chunk.into_tracked_bytes()))
    }

    pub(super) fn release(&mut self) {
        if let Some(source) = self.source.as_mut() {
            match source {
                PreparedAttemptSource::Queue(chunks) => chunks.clear_and_release(),
                PreparedAttemptSource::Sequential(reader) => reader.release(),
            }
        }
        self.source.take();
        self.sequential_remaining = None;
        self.sequential_quantum = None;
        self.lease.take();
    }
}

#[cfg(test)]
mod sequential_contract_tests {
    use super::*;
    use crate::runtime::body::{BudgetTree, ChargedBytes, MemoryRole, RequestLeaseBook};

    #[test]
    fn sequential_source_cannot_exceed_or_short_read_its_declared_body() {
        for chunks in [vec![3], Vec::new()] {
            let tree = BudgetTree::new(64, 64).expect("tree");
            let budget = tree.stream(64).expect("budget");
            let leases = RequestLeaseBook::new();
            let mut body = PreparedAttemptBody::from_reader(
                Box::new(ContractReader {
                    budget,
                    chunks,
                    declared: 2,
                }),
                leases.acquire().expect("lease"),
            )
            .expect("body");
            assert!(matches!(
                body.begin_next_send(),
                Err(AttemptError::SequentialBodyContractViolation)
            ));
        }
    }

    #[derive(Debug)]
    struct ContractReader {
        budget: StreamBudget,
        chunks: Vec<usize>,
        declared: usize,
    }

    impl AttemptRequestBodyReader for ContractReader {
        fn visible_bytes(&self) -> usize {
            self.declared
        }

        fn max_chunk_bytes(&self) -> usize {
            8
        }

        fn next_chunk(&mut self) -> Result<Option<ChargedBytes>, AttemptError> {
            let Some(len) = self.chunks.pop() else {
                return Ok(None);
            };
            Ok(Some(ChargedBytes::copy_from_opaque(
                &self.budget,
                MemoryRole::AttemptWire,
                &vec![0_u8; len],
            )?))
        }

        fn release(&mut self) {
            self.chunks.clear();
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransportPrecommitEvent {
    ResponseHead {
        status: StatusCode,
        headers: HeaderMap,
    },
    Body(Bytes),
    EndStream,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransportPrecommitReceipt {
    pub event: TransportPrecommitEvent,
    /// Monotonic instant at which the transport reader received protocol
    /// progress, before any request-local consumer or downstream work.
    pub received_at: Instant,
    /// Time the transport deliberately did not poll upstream because its
    /// bounded local mailbox had no capacity.
    pub local_read_suppressed: Duration,
    /// Authoritative cumulative suppression counter sampled immediately after
    /// this receipt was obtained from upstream. A transport that also exposes
    /// `local_read_suppression_total` uses this anchor to distinguish local
    /// backpressure before this receipt from suppression that happened while
    /// the receipt waited in a bounded mailbox.
    pub local_read_suppression_total_at_receipt: Option<Duration>,
}

impl TransportPrecommitReceipt {
    pub fn immediate(event: TransportPrecommitEvent) -> Self {
        Self {
            event,
            received_at: Instant::now(),
            local_read_suppressed: Duration::ZERO,
            local_read_suppression_total_at_receipt: None,
        }
    }
}

#[derive(Debug)]
pub struct ChargedResponseHead {
    pub(super) status: StatusCode,
    pub(super) headers: HeaderMap,
    pub(super) _reservation: Reservation,
}

impl ChargedResponseHead {
    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }
}

#[derive(Debug)]
pub enum PrecommitEvent {
    ResponseHead(ChargedResponseHead),
    Body(ChargedBytes),
    SseEvent {
        sequence: u64,
        bytes: ChargedBytes,
        provenance: SemanticProvenance,
    },
    EndStream,
}

#[async_trait]
pub trait AttemptTransport: Send {
    /// Production codecs that can retain opaque response backing request a
    /// pre-read window reservation. Deterministic in-memory transports may
    /// leave this false because they allocate no hidden codec buffers.
    fn requires_codec_reservation(&self) -> bool {
        false
    }
    async fn connect(
        &mut self,
        target: &TransportTarget,
        address: SocketAddr,
    ) -> Result<(), AttemptError>;
    async fn write_request_head(&mut self, head: &PreparedRequestHead) -> Result<(), AttemptError>;
    async fn write_request_body(
        &mut self,
        body: Bytes,
        end_stream: bool,
    ) -> Result<(), AttemptError>;
    async fn finish_request_body(&mut self) -> Result<(), AttemptError>;
    /// Starts the sole persistent response reader after the exchange has
    /// reserved opaque codec memory. Implementations without a detached
    /// reader may keep the default no-op and poll in `poll_precommit`.
    fn activate_precommit_reader(&mut self) -> Result<(), AttemptError> {
        Ok(())
    }
    /// Whether request-body/EOS writes can be polled while a response reader
    /// independently progresses. A true implementation must retain partial
    /// write ownership across `Pending` and cancellation.
    fn supports_duplex_request_body(&self) -> bool {
        false
    }
    fn poll_write_request_body(
        &mut self,
        _context: &mut Context<'_>,
        _body: &mut Bytes,
        _end_stream: bool,
    ) -> Poll<Result<(), AttemptError>> {
        Poll::Ready(Err(AttemptError::DuplexWriteUnsupported))
    }
    fn poll_finish_request_body(
        &mut self,
        _context: &mut Context<'_>,
    ) -> Poll<Result<(), AttemptError>> {
        Poll::Ready(Err(AttemptError::DuplexWriteUnsupported))
    }
    /// Polls one response event without taking response ownership away from
    /// the exchange owner loop. Implementations must be cancellation safe:
    /// returning `Pending` and recreating the poll on the next bounded writer
    /// turn cannot discard partially read protocol state.
    fn poll_precommit(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Result<Option<TransportPrecommitEvent>, AttemptError>>;
    /// Detached readers override this method to preserve the actual transport
    /// receipt instant and local backpressure suppression. Inline transports
    /// use the default because polling and receipt are the same operation.
    fn poll_precommit_receipt(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Result<Option<TransportPrecommitReceipt>, AttemptError>> {
        match self.poll_precommit(context) {
            Poll::Ready(Ok(Some(event))) => {
                Poll::Ready(Ok(Some(TransportPrecommitReceipt::immediate(event))))
            }
            Poll::Ready(Ok(None)) => Poll::Ready(Ok(None)),
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Pending => Poll::Pending,
        }
    }
    /// Returns a monotonic cumulative duration for intervals in which a
    /// detached reader deliberately did not poll upstream because its bounded
    /// local mailbox was full. The total includes an active interval up to the
    /// instant of this call. `None` means receipts carry any such accounting.
    fn local_read_suppression_total(&self) -> Option<Duration> {
        None
    }
    async fn cancel_reset(&mut self) -> Result<(), AttemptError>;
    /// Completes an accepted response. A reusable implementation may return
    /// the protocol session to its pool only after normal request EOS and
    /// normal response EOS have both been observed.
    async fn finish_accepted(&mut self, _reusable: bool) -> Result<(), AttemptError> {
        Ok(())
    }
    fn protocol(&self) -> HttpProtocol;
}

pub trait AttemptTransportFactory: Send + Sync + 'static {
    type Transport: AttemptTransport;

    fn create_transport(&self, connection_configs: &ConfigScopeSnapshot) -> Self::Transport;
}
