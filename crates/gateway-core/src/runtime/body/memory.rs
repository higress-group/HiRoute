use super::*;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum MemoryRole {
    RawRequest = 0,
    ModelIrBacking = 1,
    AttemptWire = 2,
    Retry = 3,
    ResponsePrefix = 4,
    SseFrame = 5,
    SemanticState = 6,
    OutputQueue = 7,
    TransportInflight = 8,
}

impl MemoryRole {
    pub const ALL: [Self; 9] = [
        Self::RawRequest,
        Self::ModelIrBacking,
        Self::AttemptWire,
        Self::Retry,
        Self::ResponsePrefix,
        Self::SseFrame,
        Self::SemanticState,
        Self::OutputQueue,
        Self::TransportInflight,
    ];
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct StreamBudgetId(u64);

#[derive(Clone, Debug)]
pub struct BudgetTree {
    inner: Arc<Mutex<BudgetState>>,
}

#[derive(Debug)]
struct BudgetState {
    process_limit: usize,
    worker_limit: usize,
    process_live: usize,
    worker_live: usize,
    process_peak: usize,
    worker_peak: usize,
    process_rejected: usize,
    worker_rejected: usize,
    process_role_live: [usize; 9],
    process_role_peak: [usize; 9],
    worker_role_live: [usize; 9],
    worker_role_peak: [usize; 9],
    next_stream_id: u64,
    streams: HashMap<StreamBudgetId, StreamState>,
}

#[derive(Debug)]
struct StreamState {
    limit: usize,
    live: usize,
    peak: usize,
    rejected: usize,
    role_live: [usize; 9],
    role_peak: [usize; 9],
}

impl BudgetTree {
    pub fn new(process_limit: usize, worker_limit: usize) -> Result<Self, BodyError> {
        if process_limit == 0 || worker_limit == 0 || worker_limit > process_limit {
            return Err(BodyError::InvalidBudgetLimit);
        }
        Ok(Self {
            inner: Arc::new(Mutex::new(BudgetState {
                process_limit,
                worker_limit,
                process_live: 0,
                worker_live: 0,
                process_peak: 0,
                worker_peak: 0,
                process_rejected: 0,
                worker_rejected: 0,
                process_role_live: [0; 9],
                process_role_peak: [0; 9],
                worker_role_live: [0; 9],
                worker_role_peak: [0; 9],
                next_stream_id: 0,
                streams: HashMap::new(),
            })),
        })
    }

    pub fn stream(&self, limit: usize) -> Result<StreamBudget, BodyError> {
        self.stream_inner(limit, None)
    }

    pub fn stream_with_telemetry(
        &self,
        limit: usize,
        telemetry: RequestTelemetry,
    ) -> Result<StreamBudget, BodyError> {
        self.stream_inner(limit, Some(telemetry))
    }

    fn stream_inner(
        &self,
        limit: usize,
        telemetry: Option<RequestTelemetry>,
    ) -> Result<StreamBudget, BodyError> {
        let mut inner = lock(&self.inner);
        if limit == 0 || limit > inner.worker_limit {
            return Err(BodyError::InvalidBudgetLimit);
        }
        inner.next_stream_id = inner.next_stream_id.wrapping_add(1);
        let id = StreamBudgetId(inner.next_stream_id);
        inner.streams.insert(
            id,
            StreamState {
                limit,
                live: 0,
                peak: 0,
                rejected: 0,
                role_live: [0; 9],
                role_peak: [0; 9],
            },
        );
        Ok(StreamBudget {
            handle: Arc::new(StreamBudgetHandle {
                id,
                inner: Arc::clone(&self.inner),
                telemetry,
            }),
        })
    }

    pub fn snapshot(&self) -> BudgetTreeSnapshot {
        let inner = lock(&self.inner);
        BudgetTreeSnapshot {
            process_live: inner.process_live,
            process_peak: inner.process_peak,
            worker_live: inner.worker_live,
            worker_peak: inner.worker_peak,
            active_streams: inner.streams.len(),
        }
    }

    pub(crate) fn owns_stream(&self, budget: &StreamBudget) -> bool {
        Arc::ptr_eq(&self.inner, &budget.handle.inner)
    }
}

#[derive(Clone, Debug)]
pub struct StreamBudget {
    handle: Arc<StreamBudgetHandle>,
}

#[derive(Debug)]
struct StreamBudgetHandle {
    id: StreamBudgetId,
    inner: Arc<Mutex<BudgetState>>,
    telemetry: Option<RequestTelemetry>,
}

impl Drop for StreamBudgetHandle {
    fn drop(&mut self) {
        let mut inner = lock(&self.inner);
        if inner
            .streams
            .get(&self.id)
            .is_some_and(|stream| stream.live == 0)
        {
            inner.streams.remove(&self.id);
        }
    }
}

impl StreamBudget {
    pub fn reserve(&self, role: MemoryRole, bytes: usize) -> Result<Reservation, BodyError> {
        if bytes == 0 {
            return Ok(Reservation {
                stream: self.clone(),
                role,
                bytes,
                released: false,
            });
        }
        let mut inner = lock(&self.handle.inner);
        let process_next = inner
            .process_live
            .checked_add(bytes)
            .ok_or(BodyError::BudgetExceeded)?;
        let worker_next = inner
            .worker_live
            .checked_add(bytes)
            .ok_or(BodyError::BudgetExceeded)?;
        let (stream_next, role_next, stream_limit) = {
            let stream = inner
                .streams
                .get(&self.handle.id)
                .ok_or(BodyError::UnknownStreamBudget)?;
            (
                stream
                    .live
                    .checked_add(bytes)
                    .ok_or(BodyError::BudgetExceeded)?,
                stream.role_live[role as usize]
                    .checked_add(bytes)
                    .ok_or(BodyError::BudgetExceeded)?,
                stream.limit,
            )
        };
        if process_next > inner.process_limit
            || worker_next > inner.worker_limit
            || stream_next > stream_limit
        {
            inner.process_rejected += 1;
            inner.worker_rejected += 1;
            inner
                .streams
                .get_mut(&self.handle.id)
                .ok_or(BodyError::UnknownStreamBudget)?
                .rejected += 1;
            let snapshot = snapshot_for_observation(&inner, self.handle.id)?;
            drop(inner);
            self.observe_memory(role, snapshot);
            return Err(BodyError::BudgetExceeded);
        }
        inner.process_live = process_next;
        inner.worker_live = worker_next;
        inner.process_peak = inner.process_peak.max(process_next);
        inner.worker_peak = inner.worker_peak.max(worker_next);
        inner.process_role_live[role as usize] = inner.process_role_live[role as usize]
            .checked_add(bytes)
            .ok_or(BodyError::BudgetExceeded)?;
        inner.worker_role_live[role as usize] = inner.worker_role_live[role as usize]
            .checked_add(bytes)
            .ok_or(BodyError::BudgetExceeded)?;
        inner.process_role_peak[role as usize] =
            inner.process_role_peak[role as usize].max(inner.process_role_live[role as usize]);
        inner.worker_role_peak[role as usize] =
            inner.worker_role_peak[role as usize].max(inner.worker_role_live[role as usize]);
        let stream = inner
            .streams
            .get_mut(&self.handle.id)
            .ok_or(BodyError::UnknownStreamBudget)?;
        stream.live = stream_next;
        stream.peak = stream.peak.max(stream_next);
        stream.role_live[role as usize] = role_next;
        stream.role_peak[role as usize] = stream.role_peak[role as usize].max(role_next);
        let snapshot = snapshot_for_observation(&inner, self.handle.id)?;
        drop(inner);
        self.observe_memory(role, snapshot);
        Ok(Reservation {
            stream: self.clone(),
            role,
            bytes,
            released: false,
        })
    }

    pub fn snapshot(&self) -> Result<StreamBudgetSnapshot, BodyError> {
        let inner = lock(&self.handle.inner);
        let stream = inner
            .streams
            .get(&self.handle.id)
            .ok_or(BodyError::UnknownStreamBudget)?;
        Ok(snapshot_from_stream(stream))
    }

    fn observe_memory(&self, role: MemoryRole, snapshot: BudgetMemorySnapshot) {
        if let Some(telemetry) = &self.handle.telemetry {
            telemetry.memory_role(role, snapshot);
        }
    }
}

#[derive(Debug)]
pub struct Reservation {
    stream: StreamBudget,
    role: MemoryRole,
    bytes: usize,
    released: bool,
}

/// Cloneable, opaque ownership of already-admitted queue metadata. The
/// reservation is shared only among units allocated from the same fixed
/// output queue and is released with the final downstream unit/drop.
#[derive(Clone, Debug, Default)]
pub struct BodyMetadataOwner {
    reservation: Option<Arc<Reservation>>,
}

impl BodyMetadataOwner {
    pub(crate) fn from_reservation(reservation: Option<Arc<Reservation>>) -> Self {
        Self { reservation }
    }

    pub fn is_empty(&self) -> bool {
        self.reservation.is_none()
    }
}

impl Reservation {
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn role(&self) -> MemoryRole {
        self.role
    }

    pub fn transfer_role(&mut self, role: MemoryRole) -> Result<(), BodyError> {
        if self.released {
            return Err(BodyError::ReservationReleased);
        }
        if self.role == role || self.bytes == 0 {
            self.role = role;
            return Ok(());
        }
        let mut inner = lock(&self.stream.handle.inner);
        inner.process_role_live[self.role as usize] -= self.bytes;
        inner.worker_role_live[self.role as usize] -= self.bytes;
        inner.process_role_live[role as usize] += self.bytes;
        inner.worker_role_live[role as usize] += self.bytes;
        inner.process_role_peak[role as usize] =
            inner.process_role_peak[role as usize].max(inner.process_role_live[role as usize]);
        inner.worker_role_peak[role as usize] =
            inner.worker_role_peak[role as usize].max(inner.worker_role_live[role as usize]);
        let stream = inner
            .streams
            .get_mut(&self.stream.handle.id)
            .ok_or(BodyError::UnknownStreamBudget)?;
        stream.role_live[self.role as usize] -= self.bytes;
        let next = stream.role_live[role as usize]
            .checked_add(self.bytes)
            .ok_or(BodyError::BudgetExceeded)?;
        stream.role_live[role as usize] = next;
        stream.role_peak[role as usize] = stream.role_peak[role as usize].max(next);
        let snapshot = snapshot_for_observation(&inner, self.stream.handle.id)?;
        let old_role = self.role;
        self.role = role;
        drop(inner);
        self.stream.observe_memory(old_role, snapshot);
        self.stream.observe_memory(role, snapshot);
        Ok(())
    }

    fn release(&mut self) {
        if self.released {
            return;
        }
        let mut inner = lock(&self.stream.handle.inner);
        inner.process_live -= self.bytes;
        inner.worker_live -= self.bytes;
        inner.process_role_live[self.role as usize] -= self.bytes;
        inner.worker_role_live[self.role as usize] -= self.bytes;
        let snapshot = if let Some(stream) = inner.streams.get_mut(&self.stream.handle.id) {
            stream.live -= self.bytes;
            stream.role_live[self.role as usize] -= self.bytes;
            snapshot_for_observation(&inner, self.stream.handle.id).ok()
        } else {
            None
        };
        drop(inner);
        if let Some(snapshot) = snapshot {
            self.stream.observe_memory(self.role, snapshot);
        }
        self.released = true;
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Debug)]
pub struct ChargedBytes {
    bytes: Bytes,
    reservation: Reservation,
    metadata: BodyMetadataOwner,
}

/// Reserve-before-allocate builder for transforms whose exact or maximum
/// output size is known before materialization.
pub struct ChargedBytesBuilder {
    bytes: Vec<u8>,
    capacity: usize,
    reservation: Reservation,
}

impl ChargedBytesBuilder {
    pub fn new(
        budget: &StreamBudget,
        role: MemoryRole,
        capacity: usize,
    ) -> Result<Self, BodyError> {
        let reservation = budget.reserve(role, capacity)?;
        Ok(Self {
            bytes: Vec::with_capacity(capacity),
            capacity,
            reservation,
        })
    }

    pub fn extend_from_slice(&mut self, bytes: &[u8]) -> Result<(), BodyError> {
        if self.bytes.len().saturating_add(bytes.len()) > self.capacity {
            return Err(BodyError::BodyLimitExceeded);
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    pub fn push(&mut self, byte: u8) -> Result<(), BodyError> {
        if self.bytes.len() == self.capacity {
            return Err(BodyError::BodyLimitExceeded);
        }
        self.bytes.push(byte);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn resize_zeroed(&mut self, len: usize) -> Result<(), BodyError> {
        if len > self.capacity {
            return Err(BodyError::BodyLimitExceeded);
        }
        self.bytes.resize(len, 0);
        Ok(())
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    pub fn truncate(&mut self, len: usize) {
        self.bytes.truncate(len);
    }

    pub fn finish(self) -> ChargedBytes {
        ChargedBytes {
            bytes: Bytes::from(self.bytes),
            reservation: self.reservation,
            metadata: BodyMetadataOwner::default(),
        }
    }
}

struct DropTrackedBytesOwner {
    bytes: Bytes,
    _reservation: Reservation,
    _metadata: BodyMetadataOwner,
}

impl AsRef<[u8]> for DropTrackedBytesOwner {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

/// Fixed-capacity, byte-bounded owner for charged body chunks. Queue metadata
/// is reserved up front so an "empty" mailbox cannot hide unaccounted heap
/// capacity, and every stored chunk must already carry the queue's role.
#[derive(Debug)]
pub struct ChargedBodyQueue {
    chunks: VecDeque<ChargedBytes>,
    role: MemoryRole,
    max_chunks: usize,
    max_chunk_bytes: usize,
    max_visible_bytes: usize,
    visible_bytes: usize,
    high_water_bytes: usize,
    high_water_chunks: usize,
    metadata_reservation: Option<Reservation>,
}

impl ChargedBodyQueue {
    pub fn new(
        budget: &StreamBudget,
        role: MemoryRole,
        plan: &BodyPlan,
        hard_total_limit: usize,
        max_chunks: usize,
    ) -> Result<Self, BodyError> {
        plan.validate()?;
        if hard_total_limit == 0 || max_chunks == 0 {
            return Err(BodyError::InvalidPlanLimit);
        }
        let max_chunk_bytes = plan.max_chunk_bytes().min(hard_total_limit);
        let max_visible_bytes = plan
            .max_retained_bytes()
            .unwrap_or(hard_total_limit)
            .min(hard_total_limit);
        // Account for the requested fixed queue before asking the allocator
        // for its backing. A rejected stream therefore cannot transiently
        // allocate an uncharged mailbox.
        let metadata_bytes = max_chunks
            .checked_mul(size_of::<ChargedBytes>())
            .ok_or(BodyError::BudgetExceeded)?;
        let metadata_reservation = budget.reserve(role, metadata_bytes)?;
        // Allocate the fixed backing only after its complete logical capacity
        // has been admitted. This keeps allocator capacity and accounting in
        // lockstep even for queues containing only zero-byte units.
        let chunks = VecDeque::with_capacity(max_chunks);
        Ok(Self {
            chunks,
            role,
            max_chunks,
            max_chunk_bytes,
            max_visible_bytes,
            visible_bytes: 0,
            high_water_bytes: 0,
            high_water_chunks: 0,
            metadata_reservation: Some(metadata_reservation),
        })
    }

    pub fn push_back(&mut self, chunk: ChargedBytes) -> Result<(), BodyError> {
        if chunk.role() != self.role {
            return Err(BodyError::WrongMemoryRole);
        }
        if chunk.bytes().len() > self.max_chunk_bytes || self.chunks.len() >= self.max_chunks {
            return Err(BodyError::BodyLimitExceeded);
        }
        let next = self
            .visible_bytes
            .checked_add(chunk.bytes().len())
            .ok_or(BodyError::BodyLimitExceeded)?;
        if next > self.max_visible_bytes {
            return Err(BodyError::BodyLimitExceeded);
        }
        self.visible_bytes = next;
        self.chunks.push_back(chunk);
        self.high_water_bytes = self.high_water_bytes.max(self.visible_bytes);
        self.high_water_chunks = self.high_water_chunks.max(self.chunks.len());
        Ok(())
    }

    /// A protocol prefix has no message-count quota. Grow its queue against
    /// the request budget while ordinary bounded handoff queues keep theirs.
    pub fn push_back_growing(
        &mut self,
        budget: &StreamBudget,
        chunk: ChargedBytes,
    ) -> Result<(), BodyError> {
        if self.chunks.len() == self.max_chunks {
            let capacity = self
                .max_chunks
                .checked_mul(2)
                .ok_or(BodyError::BodyLimitExceeded)?;
            let bytes = capacity
                .checked_mul(size_of::<ChargedBytes>())
                .ok_or(BodyError::BodyLimitExceeded)?;
            let reservation = budget.reserve(self.role, bytes)?;
            let mut replacement = VecDeque::with_capacity(capacity);
            replacement.extend(self.chunks.drain(..));
            self.chunks = replacement;
            self.metadata_reservation = Some(reservation);
            self.max_chunks = capacity;
        }
        self.push_back(chunk)
    }

    pub fn pop_front(&mut self) -> Option<ChargedBytes> {
        let chunk = self.chunks.pop_front()?;
        self.visible_bytes -= chunk.bytes().len();
        Some(chunk)
    }

    pub fn chunks(&self) -> impl Iterator<Item = &ChargedBytes> {
        self.chunks.iter()
    }

    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    pub fn visible_bytes(&self) -> usize {
        self.visible_bytes
    }

    pub fn high_water_bytes(&self) -> usize {
        self.high_water_bytes
    }

    pub fn high_water_chunks(&self) -> usize {
        self.high_water_chunks
    }

    pub fn max_chunk_bytes(&self) -> usize {
        self.max_chunk_bytes
    }

    pub fn allocated_chunk_capacity(&self) -> usize {
        self.chunks.capacity()
    }

    pub fn clear_and_release(&mut self) {
        // Free both elements and allocation before releasing its accounting;
        // VecDeque::clear would retain capacity while reporting zero live
        // metadata bytes.
        let chunks = std::mem::take(&mut self.chunks);
        self.visible_bytes = 0;
        drop(chunks);
        self.metadata_reservation.take();
    }
}

impl ChargedBytes {
    /// Copies an opaque transport slice into an exact-sized allocation. This
    /// avoids charging a tiny visible slice while pinning unknown large backing.
    pub fn copy_from_opaque(
        budget: &StreamBudget,
        role: MemoryRole,
        bytes: &[u8],
    ) -> Result<Self, BodyError> {
        let reservation = budget.reserve(role, bytes.len())?;
        let mut exact = Vec::with_capacity(bytes.len());
        exact.extend_from_slice(bytes);
        Ok(Self {
            bytes: Bytes::from(exact),
            reservation,
            metadata: BodyMetadataOwner::default(),
        })
    }

    pub fn from_exact_vec(
        budget: &StreamBudget,
        role: MemoryRole,
        mut bytes: Vec<u8>,
    ) -> Result<Self, BodyError> {
        if bytes.capacity() != bytes.len() {
            bytes.shrink_to_fit();
        }
        let retained_capacity = bytes.capacity();
        let reservation = budget.reserve(role, retained_capacity)?;
        Ok(Self {
            bytes: Bytes::from(bytes),
            reservation,
            metadata: BodyMetadataOwner::default(),
        })
    }

    pub fn bytes(&self) -> &Bytes {
        &self.bytes
    }

    pub fn retained_capacity(&self) -> usize {
        self.reservation.bytes()
    }

    pub fn role(&self) -> MemoryRole {
        self.reservation.role()
    }

    pub(crate) fn metadata(&self) -> BodyMetadataOwner {
        self.metadata.clone()
    }

    pub fn transfer_role(mut self, role: MemoryRole) -> Result<Self, BodyError> {
        self.reservation.transfer_role(role)?;
        Ok(self)
    }

    pub(crate) fn with_metadata(mut self, metadata: BodyMetadataOwner) -> Self {
        self.metadata = metadata;
        self
    }

    /// Moves both bytes and their reservation into a `Bytes` owner. Any codec
    /// clone keeps the reservation alive, and the charge is released exactly
    /// when the final transport clone drops.
    pub fn into_tracked_bytes(self) -> Bytes {
        Bytes::from_owner(DropTrackedBytesOwner {
            bytes: self.bytes,
            _reservation: self.reservation,
            _metadata: self.metadata,
        })
    }

    pub(crate) fn into_drop_tracked_bytes(self) -> Bytes {
        self.into_tracked_bytes()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BudgetTreeSnapshot {
    pub process_live: usize,
    pub process_peak: usize,
    pub worker_live: usize,
    pub worker_peak: usize,
    pub active_streams: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamBudgetSnapshot {
    pub live: usize,
    pub peak: usize,
    pub rejected: usize,
    pub role_live: [usize; 9],
    pub role_peak: [usize; 9],
}

/// One atomic view of the stream and both enclosing admission levels. It is
/// captured under the budget-tree mutex so telemetry never reconstructs a
/// process/worker gauge by adding racing per-request snapshots.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BudgetMemorySnapshot {
    pub stream: StreamBudgetSnapshot,
    pub process_rejected: usize,
    pub worker_rejected: usize,
    pub process_role_live: [usize; 9],
    pub process_role_peak: [usize; 9],
    pub worker_role_live: [usize; 9],
    pub worker_role_peak: [usize; 9],
}

fn snapshot_from_stream(stream: &StreamState) -> StreamBudgetSnapshot {
    StreamBudgetSnapshot {
        live: stream.live,
        peak: stream.peak,
        rejected: stream.rejected,
        role_live: stream.role_live,
        role_peak: stream.role_peak,
    }
}

fn snapshot_for_observation(
    state: &BudgetState,
    stream_id: StreamBudgetId,
) -> Result<BudgetMemorySnapshot, BodyError> {
    let stream = state
        .streams
        .get(&stream_id)
        .ok_or(BodyError::UnknownStreamBudget)?;
    Ok(BudgetMemorySnapshot {
        stream: snapshot_from_stream(stream),
        process_rejected: state.process_rejected,
        worker_rejected: state.worker_rejected,
        process_role_live: state.process_role_live,
        process_role_peak: state.process_role_peak,
        worker_role_live: state.worker_role_live,
        worker_role_peak: state.worker_role_peak,
    })
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
