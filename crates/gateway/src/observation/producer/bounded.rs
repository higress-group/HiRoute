use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

use super::super::schema::{
    LossWatermarkV1, OBSERVATION_GAP_HEARTBEAT_SCHEMA, ObservationAckV2, ObservationGapHeartbeatV1,
    ObservationNackV1, ProducerDescriptorV1,
};
use super::feedback::{acknowledgement_covers_record, nack_matches_record};

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ObservationError {
    #[error("observation sink is full or unavailable")]
    Unavailable,
    #[error("observation channel capacity must be non-zero")]
    InvalidCapacity,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(super) const RECORD_ACCOUNTING_BYTES: usize = 128;
const MAX_PENDING_LOSS_RANGES: usize = 16;
const LOSS_POLL_INTERVAL: Duration = Duration::from_millis(10);
const GAP_RETRY_INITIAL_BACKOFF: Duration = Duration::from_millis(1);
const GAP_RETRY_MAX_BACKOFF: Duration = Duration::from_millis(50);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationProducerIdentity {
    pub channel: Arc<str>,
    pub component: Arc<str>,
    pub revision: Arc<str>,
    pub producer_id: Arc<str>,
    pub producer_epoch: Arc<str>,
    pub stream_id: Arc<str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationLossReason {
    PublishContended,
    QueueBytesExceeded,
    QueueEventsExceeded,
    EventTooLarge,
    SinkFailed,
    SinkPanicked,
    SinkNack,
    WorkerDisconnected,
    Compacted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationLossWatermark {
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub reason: ObservationLossReason,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationSequenceStamp {
    pub identity: ObservationProducerIdentity,
    pub sequence: u64,
    pub loss_watermark: Option<ObservationLossWatermark>,
}

#[derive(Clone, Debug)]
pub struct ObservationRecord {
    stamp: ObservationSequenceStamp,
    payload: Arc<[u8]>,
    accounted_bytes: usize,
    gap_heartbeat: bool,
}

impl ObservationRecord {
    pub fn stamp(&self) -> &ObservationSequenceStamp {
        &self.stamp
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn accounted_bytes(&self) -> usize {
        self.accounted_bytes
    }

    /// A producer-owned control record declaring loss without waiting for a
    /// later request record. It does not replace the missing data record.
    pub fn is_gap_heartbeat(&self) -> bool {
        self.gap_heartbeat
    }
}

pub type ObservationAck = ObservationAckV2;
/// The wire DTO remains `ObservationNackV1`; the sink ABI boxes it so every
/// data-plane producer message stays small regardless of the selected detail.
pub type ObservationNack = Box<ObservationNackV1>;

pub trait ObservationRecordSink: Send + Sync {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationPublishOutcome {
    Enqueued { sequence: u64 },
    Dropped { sequence: u64 },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ObservationProducerStats {
    pub queued_bytes: usize,
    pub queue_high_water_bytes: usize,
    pub enqueued: usize,
    pub delivered: usize,
    pub dropped: usize,
    pub sink_failures: usize,
    pub sink_panics: usize,
    pub sink_nacks: usize,
    pub gap_heartbeats_delivered: usize,
    pub gap_heartbeat_failures: usize,
}

#[derive(Clone)]
pub struct ByteBoundedObservationProducer {
    identity: ObservationProducerIdentity,
    sender: SyncSender<ProducerMessage>,
    shared: Arc<ProducerShared>,
}

impl fmt::Debug for ByteBoundedObservationProducer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ByteBoundedObservationProducer")
            .field("identity", &self.identity)
            .field("capacity_bytes", &self.shared.capacity_bytes)
            .field("stats", &self.stats())
            .finish()
    }
}

struct ProducerShared {
    capacity_bytes: usize,
    next_sequence: AtomicU64,
    queued_bytes: AtomicUsize,
    queue_high_water_bytes: AtomicUsize,
    enqueued: AtomicUsize,
    delivered: AtomicUsize,
    dropped: AtomicUsize,
    sink_failures: AtomicUsize,
    sink_panics: AtomicUsize,
    sink_nacks: AtomicUsize,
    gap_heartbeats_delivered: AtomicUsize,
    gap_heartbeat_failures: AtomicUsize,
    pending_loss: Mutex<VecDeque<ObservationLossWatermark>>,
}

enum ProducerMessage {
    Record(ObservationRecord),
    GapHeartbeat,
    #[cfg(test)]
    Flush {
        through_sequence: u64,
        acknowledge: SyncSender<()>,
    },
}

impl ByteBoundedObservationProducer {
    pub fn new(
        identity: ObservationProducerIdentity,
        capacity_bytes: usize,
        capacity_events: usize,
        sink: Arc<dyn ObservationRecordSink>,
    ) -> Result<Self, ObservationError> {
        if capacity_bytes == 0 || capacity_events == 0 {
            return Err(ObservationError::InvalidCapacity);
        }
        let (sender, receiver) = sync_channel(capacity_events);
        let shared = Arc::new(ProducerShared {
            capacity_bytes,
            next_sequence: AtomicU64::new(1),
            queued_bytes: AtomicUsize::new(0),
            queue_high_water_bytes: AtomicUsize::new(0),
            enqueued: AtomicUsize::new(0),
            delivered: AtomicUsize::new(0),
            dropped: AtomicUsize::new(0),
            sink_failures: AtomicUsize::new(0),
            sink_panics: AtomicUsize::new(0),
            sink_nacks: AtomicUsize::new(0),
            gap_heartbeats_delivered: AtomicUsize::new(0),
            gap_heartbeat_failures: AtomicUsize::new(0),
            pending_loss: Mutex::new(VecDeque::new()),
        });
        let worker_shared = Arc::clone(&shared);
        let worker_identity = identity.clone();
        thread::Builder::new()
            .name("hiroute-observation-producer".into())
            .spawn(move || producer_worker(receiver, sink, worker_shared, worker_identity))
            .map_err(|_| ObservationError::Unavailable)?;
        Ok(Self {
            identity,
            sender,
            shared,
        })
    }

    /// Reserves a monotonically increasing sequence before building the
    /// envelope. The closure is never allowed to await, and enqueue is always
    /// `try_send`; a data-plane caller therefore cannot inherit sink latency.
    pub fn try_publish<F>(&self, build: F) -> ObservationPublishOutcome
    where
        F: FnOnce(ObservationSequenceStamp) -> Vec<u8>,
    {
        // Concurrent builders can enqueue out of order. The worker holds
        // those records in sequence order before delivery, so a CPU-bound
        // build in one request does not turn another healthy request into a
        // synthetic loss.
        let sequence = self.shared.next_sequence.fetch_add(1, Ordering::Relaxed);
        let stamp = ObservationSequenceStamp {
            identity: self.identity.clone(),
            sequence,
            loss_watermark: None,
        };
        let payload: Arc<[u8]> = build(stamp.clone()).into();
        let accounted_bytes = payload.len().saturating_add(RECORD_ACCOUNTING_BYTES);
        if accounted_bytes > self.shared.capacity_bytes {
            self.restore_and_drop(sequence, ObservationLossReason::EventTooLarge);
            return ObservationPublishOutcome::Dropped { sequence };
        }
        if !reserve_bytes(&self.shared, accounted_bytes) {
            self.restore_and_drop(sequence, ObservationLossReason::QueueBytesExceeded);
            return ObservationPublishOutcome::Dropped { sequence };
        }
        let record = ObservationRecord {
            stamp,
            payload,
            accounted_bytes,
            gap_heartbeat: false,
        };
        match self.sender.try_send(ProducerMessage::Record(record)) {
            Ok(()) => {
                self.shared.enqueued.fetch_add(1, Ordering::Relaxed);
                ObservationPublishOutcome::Enqueued { sequence }
            }
            Err(TrySendError::Full(ProducerMessage::Record(record))) => {
                release_bytes(&self.shared, record.accounted_bytes);
                self.restore_and_drop(sequence, ObservationLossReason::QueueEventsExceeded);
                ObservationPublishOutcome::Dropped { sequence }
            }
            Err(TrySendError::Disconnected(ProducerMessage::Record(record))) => {
                release_bytes(&self.shared, record.accounted_bytes);
                self.restore_and_drop(sequence, ObservationLossReason::WorkerDisconnected);
                ObservationPublishOutcome::Dropped { sequence }
            }
            Err(TrySendError::Full(ProducerMessage::GapHeartbeat))
            | Err(TrySendError::Disconnected(ProducerMessage::GapHeartbeat)) => {
                unreachable!("try_publish sends only records")
            }
            #[cfg(test)]
            Err(TrySendError::Full(ProducerMessage::Flush { .. }))
            | Err(TrySendError::Disconnected(ProducerMessage::Flush { .. })) => {
                unreachable!("try_publish sends only records")
            }
        }
    }

    /// Test/control drain only. Request handling never calls this method.
    #[cfg(test)]
    pub fn flush(&self, timeout: Duration) -> Result<(), ObservationError> {
        let (sender, receiver) = sync_channel(1);
        let through_sequence = self
            .shared
            .next_sequence
            .load(Ordering::Acquire)
            .saturating_sub(1);
        self.sender
            .try_send(ProducerMessage::Flush {
                through_sequence,
                acknowledge: sender,
            })
            .map_err(|_| ObservationError::Unavailable)?;
        receiver
            .recv_timeout(timeout)
            .map_err(|_| ObservationError::Unavailable)
    }

    pub fn stats(&self) -> ObservationProducerStats {
        ObservationProducerStats {
            queued_bytes: self.shared.queued_bytes.load(Ordering::Acquire),
            queue_high_water_bytes: self.shared.queue_high_water_bytes.load(Ordering::Acquire),
            enqueued: self.shared.enqueued.load(Ordering::Acquire),
            delivered: self.shared.delivered.load(Ordering::Acquire),
            dropped: self.shared.dropped.load(Ordering::Acquire),
            sink_failures: self.shared.sink_failures.load(Ordering::Acquire),
            sink_panics: self.shared.sink_panics.load(Ordering::Acquire),
            sink_nacks: self.shared.sink_nacks.load(Ordering::Acquire),
            gap_heartbeats_delivered: self.shared.gap_heartbeats_delivered.load(Ordering::Acquire),
            gap_heartbeat_failures: self.shared.gap_heartbeat_failures.load(Ordering::Acquire),
        }
    }

    fn restore_and_drop(&self, sequence: u64, reason: ObservationLossReason) {
        record_loss(
            &self.shared,
            ObservationLossWatermark {
                first_sequence: sequence,
                last_sequence: sequence,
                reason,
            },
        );
        self.shared.dropped.fetch_add(1, Ordering::Relaxed);
        // This control pulse has its own event-capacity slot and no byte
        // reservation. If the event queue is already full, the worker checks
        // pending loss after draining every queued record.
        let _ = self.sender.try_send(ProducerMessage::GapHeartbeat);
    }
}

fn reserve_bytes(shared: &ProducerShared, bytes: usize) -> bool {
    let mut current = shared.queued_bytes.load(Ordering::Acquire);
    loop {
        let Some(next) = current.checked_add(bytes) else {
            return false;
        };
        if next > shared.capacity_bytes {
            return false;
        }
        match shared.queued_bytes.compare_exchange_weak(
            current,
            next,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                shared
                    .queue_high_water_bytes
                    .fetch_max(next, Ordering::Relaxed);
                return true;
            }
            Err(observed) => current = observed,
        }
    }
}

fn release_bytes(shared: &ProducerShared, bytes: usize) {
    let previous = shared.queued_bytes.fetch_sub(bytes, Ordering::AcqRel);
    debug_assert!(previous >= bytes);
}

fn record_loss(shared: &ProducerShared, loss: ObservationLossWatermark) {
    let mut pending = lock(&shared.pending_loss);
    pending.push_back(loss);
    pending
        .make_contiguous()
        .sort_by_key(|watermark| watermark.first_sequence);
    let mut merged: VecDeque<ObservationLossWatermark> = VecDeque::with_capacity(pending.len());
    for watermark in pending.drain(..) {
        if let Some(last) = merged.back_mut()
            && last.reason == watermark.reason
            && last
                .last_sequence
                .checked_add(1)
                .is_some_and(|next| next >= watermark.first_sequence)
        {
            last.last_sequence = last.last_sequence.max(watermark.last_sequence);
        } else {
            merged.push_back(watermark);
        }
    }
    *pending = merged;
    if pending.len() > MAX_PENDING_LOSS_RANGES {
        let first = pending.front().expect("pending loss exists").first_sequence;
        let last = pending
            .iter()
            .map(|watermark| watermark.last_sequence)
            .max()
            .expect("pending loss exists");
        pending.clear();
        pending.push_back(ObservationLossWatermark {
            first_sequence: first,
            last_sequence: last,
            reason: ObservationLossReason::Compacted,
        });
    }
}

fn producer_worker(
    receiver: Receiver<ProducerMessage>,
    sink: Arc<dyn ObservationRecordSink>,
    shared: Arc<ProducerShared>,
    identity: ObservationProducerIdentity,
) {
    let mut expected_sequence = 1_u64;
    let mut records = BTreeMap::<u64, ObservationRecord>::new();
    let mut flushes = Vec::<(u64, SyncSender<()>)>::new();
    let mut retry_at = None;
    let mut retry_backoff = GAP_RETRY_INITIAL_BACKOFF;
    let mut contiguous_ceiling: Option<u64> = None;
    let mut disconnected = false;

    loop {
        let timeout = retry_at.map_or(LOSS_POLL_INTERVAL, |deadline: Instant| {
            deadline.saturating_duration_since(Instant::now())
        });
        match receiver.recv_timeout(timeout) {
            Ok(ProducerMessage::Record(record)) => {
                if record.stamp.sequence < expected_sequence {
                    // A compacted loss declaration may conservatively cover
                    // a record that was still waiting in the event queue.
                    // Its sequence is already terminally declared as a gap.
                    release_bytes(&shared, record.accounted_bytes);
                    shared.dropped.fetch_add(1, Ordering::Relaxed);
                } else if let Some(replaced) = records.insert(record.stamp.sequence, record) {
                    release_bytes(&shared, replaced.accounted_bytes);
                    shared.dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            Ok(ProducerMessage::GapHeartbeat) => {}
            #[cfg(test)]
            Ok(ProducerMessage::Flush {
                through_sequence,
                acknowledge,
            }) => flushes.push((through_sequence, acknowledge)),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => disconnected = true,
        }

        loop {
            if retry_at.is_some_and(|deadline| Instant::now() < deadline) {
                break;
            }
            let loss = pending_loss_at(&shared, expected_sequence);
            if let Some(loss) = loss {
                let record = gap_heartbeat_record(&identity, loss);
                let result = catch_unwind(AssertUnwindSafe(|| sink.deliver(&record)));
                match result {
                    Ok(Ok(ack))
                        if acknowledgement_covers_record(&record, &ack, contiguous_ceiling) =>
                    {
                        shared
                            .gap_heartbeats_delivered
                            .fetch_add(1, Ordering::Relaxed);
                        let gap_ceiling = loss.first_sequence.saturating_sub(1);
                        contiguous_ceiling = Some(
                            contiguous_ceiling
                                .map_or(gap_ceiling, |current| current.min(gap_ceiling)),
                        );
                        discard_covered_records(&shared, &mut records, loss.last_sequence);
                        expected_sequence = loss.last_sequence.saturating_add(1);
                        retry_at = None;
                        retry_backoff = GAP_RETRY_INITIAL_BACKOFF;
                        continue;
                    }
                    Ok(Err(nack)) if nack_matches_record(&record, &nack) => {
                        shared.sink_nacks.fetch_add(1, Ordering::Relaxed);
                        shared.sink_failures.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(Ok(_)) | Ok(Err(_)) => {
                        shared.sink_failures.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        shared.sink_panics.fetch_add(1, Ordering::Relaxed);
                        shared.sink_failures.fetch_add(1, Ordering::Relaxed);
                    }
                }
                shared
                    .gap_heartbeat_failures
                    .fetch_add(1, Ordering::Relaxed);
                record_loss(&shared, loss);
                retry_at = Some(Instant::now() + retry_backoff);
                retry_backoff = retry_backoff
                    .checked_mul(2)
                    .unwrap_or(GAP_RETRY_MAX_BACKOFF)
                    .min(GAP_RETRY_MAX_BACKOFF);
                break;
            }

            let Some(record) = records.remove(&expected_sequence) else {
                break;
            };
            let sequence = record.stamp.sequence;
            let result = catch_unwind(AssertUnwindSafe(|| sink.deliver(&record)));
            release_bytes(&shared, record.accounted_bytes);
            match result {
                Ok(Ok(ack)) if acknowledgement_covers_record(&record, &ack, contiguous_ceiling) => {
                    shared.delivered.fetch_add(1, Ordering::Relaxed);
                    expected_sequence = sequence.saturating_add(1);
                }
                Ok(Err(nack)) if nack_matches_record(&record, &nack) => {
                    shared.sink_nacks.fetch_add(1, Ordering::Relaxed);
                    shared.sink_failures.fetch_add(1, Ordering::Relaxed);
                    restore_worker_loss(&shared, sequence, ObservationLossReason::SinkNack);
                }
                Ok(Ok(_)) | Ok(Err(_)) => {
                    shared.sink_failures.fetch_add(1, Ordering::Relaxed);
                    restore_worker_loss(&shared, sequence, ObservationLossReason::SinkFailed);
                }
                Err(_) => {
                    shared.sink_panics.fetch_add(1, Ordering::Relaxed);
                    shared.sink_failures.fetch_add(1, Ordering::Relaxed);
                    restore_worker_loss(&shared, sequence, ObservationLossReason::SinkPanicked);
                }
            }
        }

        flushes.retain(|(through_sequence, acknowledge)| {
            if expected_sequence > *through_sequence {
                let _ = acknowledge.try_send(());
                false
            } else {
                true
            }
        });
        if disconnected {
            for (_, record) in records {
                release_bytes(&shared, record.accounted_bytes);
            }
            return;
        }
    }
}

fn pending_loss_at(
    shared: &ProducerShared,
    expected_sequence: u64,
) -> Option<ObservationLossWatermark> {
    let mut pending = lock(&shared.pending_loss);
    while pending
        .front()
        .is_some_and(|loss| loss.last_sequence < expected_sequence)
    {
        pending.pop_front();
    }
    let mut loss = *pending.front()?;
    if loss.first_sequence > expected_sequence {
        return None;
    }
    pending.pop_front();
    loss.first_sequence = expected_sequence;
    Some(loss)
}

fn discard_covered_records(
    shared: &ProducerShared,
    records: &mut BTreeMap<u64, ObservationRecord>,
    through_sequence: u64,
) {
    let covered = records
        .range(..=through_sequence)
        .map(|(sequence, _)| *sequence)
        .collect::<Vec<_>>();
    for sequence in covered {
        if let Some(record) = records.remove(&sequence) {
            release_bytes(shared, record.accounted_bytes);
            shared.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn gap_heartbeat_record(
    identity: &ObservationProducerIdentity,
    loss: ObservationLossWatermark,
) -> ObservationRecord {
    let stamp = ObservationSequenceStamp {
        identity: identity.clone(),
        sequence: loss.last_sequence,
        loss_watermark: Some(loss),
    };
    let payload: Arc<[u8]> = serde_json::to_vec(&ObservationGapHeartbeatV1 {
        schema_version: OBSERVATION_GAP_HEARTBEAT_SCHEMA.into(),
        channel: identity.channel.to_string(),
        producer: ProducerDescriptorV1 {
            component: identity.component.to_string(),
            revision: identity.revision.to_string(),
            producer_id: identity.producer_id.to_string(),
            producer_epoch: identity.producer_epoch.to_string(),
            stream_id: identity.stream_id.to_string(),
        },
        sequence: loss.last_sequence,
        event_id: format!(
            "gap:{}:{}:{}:{}",
            identity.stream_id,
            loss.first_sequence,
            loss.last_sequence,
            loss_reason_label(loss.reason)
        ),
        loss_watermark: LossWatermarkV1 {
            first_sequence: loss.first_sequence,
            last_sequence: loss.last_sequence,
            reason: loss_reason_label(loss.reason).into(),
        },
        completeness_delta: "partial".into(),
    })
    .unwrap_or_else(|_| {
        br#"{"schema_version":"hiroute.observation.gap-heartbeat/v1","completeness_delta":"partial"}"#.to_vec()
    })
    .into();
    ObservationRecord {
        stamp,
        accounted_bytes: payload.len().saturating_add(RECORD_ACCOUNTING_BYTES),
        payload,
        gap_heartbeat: true,
    }
}

fn loss_reason_label(reason: ObservationLossReason) -> &'static str {
    match reason {
        ObservationLossReason::PublishContended => "publish_contended",
        ObservationLossReason::QueueBytesExceeded => "queue_bytes_exceeded",
        ObservationLossReason::QueueEventsExceeded => "queue_events_exceeded",
        ObservationLossReason::EventTooLarge => "event_too_large",
        ObservationLossReason::SinkFailed => "sink_failed",
        ObservationLossReason::SinkPanicked => "sink_panicked",
        ObservationLossReason::SinkNack => "sink_nack",
        ObservationLossReason::WorkerDisconnected => "worker_disconnected",
        ObservationLossReason::Compacted => "compacted",
    }
}

fn restore_worker_loss(shared: &ProducerShared, sequence: u64, reason: ObservationLossReason) {
    record_loss(
        shared,
        ObservationLossWatermark {
            first_sequence: sequence,
            last_sequence: sequence,
            reason,
        },
    );
    shared.dropped.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
#[path = "bounded/tests.rs"]
mod tests;
