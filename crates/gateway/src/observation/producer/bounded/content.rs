//! Content-only queue admission and workspace ownership. No durable IO is done here.
use super::*;
use crate::replay::{OverflowFactory, OverflowQuota, OverflowStats, OverflowWriter};
use std::sync::OnceLock;

const MIB: usize = 1024 * 1024;
pub(crate) const CONTENT_RECORD_BYTES: usize = 512;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ContentQueueLimits {
    pub bytes: usize,
    pub soft: usize,
    pub resume: usize,
    pub control: usize,
    pub progress: usize,
    pub active_source: usize,
    pub segment: usize,
    pub disk: usize,
}

impl ContentQueueLimits {
    pub(crate) fn for_capacity(bytes: usize) -> Self {
        let control = (bytes / 16)
            .min(MIB)
            .max(CONTENT_RECORD_BYTES * 2)
            .min(bytes);
        Self {
            bytes,
            soft: bytes.saturating_mul(3) / 4,
            resume: bytes / 2,
            control,
            progress: 4096.min(control.saturating_sub(CONTENT_RECORD_BYTES)),
            active_source: 32 * MIB,
            segment: 4 * MIB,
            disk: 512 * MIB,
        }
    }

    pub(super) fn slots(&self) -> usize {
        (self.bytes / CONTENT_RECORD_BYTES).max(1)
    }
}

/// Instantaneous accounting counters; neither an RSS estimate nor a persisted DTO.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ContentResourceSnapshot {
    pub queued_bytes: usize,
    pub queue_peak_bytes: usize,
    pub queued_source_bytes: usize,
    pub active_source_bytes: usize,
    pub record_slots: usize,
    pub encode_bytes: usize,
    pub encode_peak_bytes: usize,
    pub foreground_encode_bytes: usize,
    pub foreground_encode_peak_bytes: usize,
    pub drain_bytes: usize,
    pub drain_peak_bytes: usize,
    pub overflow_reserved_bytes: usize,
    pub overflow_file_bytes: u64,
    pub overflow_written_bytes: u64,
    pub overflow_read_bytes: u64,
    pub overflow_peak_bytes: usize,
    pub overflow_live_segments: usize,
    pub overflow_created_segments: usize,
    pub overflow_cleanup_failures: usize,
    pub io_failures: usize,
    pub integrity_failures: usize,
}

pub(crate) struct ContentResources {
    pub(super) limits: ContentQueueLimits,
    pub(super) quota: Arc<OverflowQuota>,
    pub(super) slots: AtomicUsize,
    pub(super) control_bytes: AtomicUsize,
    pub(super) progress_busy: AtomicBool,
    encoder_busy: Mutex<bool>,
    encoder_available: tokio::sync::Notify,
    pub(super) active_source_bytes: AtomicUsize,
    encode_bytes: AtomicUsize,
    encode_peak: AtomicUsize,
    foreground_encode_bytes: AtomicUsize,
    foreground_encode_peak: AtomicUsize,
    pub(super) drain_bytes: AtomicUsize,
    pub(super) drain_peak: AtomicUsize,
    pub(super) io_failures: AtomicUsize,
    pub(super) integrity_failures: AtomicUsize,
    pub(super) loss_wake_queued: AtomicBool,
}

use std::sync::atomic::AtomicBool;

impl ContentResources {
    pub(super) fn new(limits: ContentQueueLimits) -> Arc<Self> {
        Arc::new(Self {
            limits,
            quota: OverflowQuota::new(limits.disk),
            slots: AtomicUsize::new(0),
            control_bytes: AtomicUsize::new(0),
            progress_busy: AtomicBool::new(false),
            encoder_busy: Mutex::new(false),
            encoder_available: tokio::sync::Notify::new(),
            active_source_bytes: AtomicUsize::new(0),
            encode_bytes: AtomicUsize::new(0),
            encode_peak: AtomicUsize::new(0),
            foreground_encode_bytes: AtomicUsize::new(0),
            foreground_encode_peak: AtomicUsize::new(0),
            drain_bytes: AtomicUsize::new(0),
            drain_peak: AtomicUsize::new(0),
            io_failures: AtomicUsize::new(0),
            integrity_failures: AtomicUsize::new(0),
            loss_wake_queued: AtomicBool::new(false),
        })
    }

    async fn encode(self: &Arc<Self>, background: bool, bytes: usize) -> EncodeScratch {
        if background {
            loop {
                let notified = self.encoder_available.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                {
                    let mut busy = lock(&self.encoder_busy);
                    if !*busy {
                        *busy = true;
                        break;
                    }
                }
                notified.await;
            }
        }
        let mut scratch = EncodeScratch {
            resources: Arc::clone(self),
            background,
            bytes: 0,
        };
        scratch.resize(bytes);
        scratch
    }

    pub(crate) fn disk_snapshot(&self) -> OverflowStats {
        self.quota.snapshot()
    }

    pub(super) fn drain(self: &Arc<Self>, bytes: usize) -> DrainScratch {
        let live = self.drain_bytes.fetch_add(bytes, Ordering::AcqRel) + bytes;
        self.drain_peak.fetch_max(live, Ordering::Relaxed);
        DrainScratch {
            resources: Arc::clone(self),
            bytes,
        }
    }
}

pub(super) struct DrainScratch {
    resources: Arc<ContentResources>,
    bytes: usize,
}

impl DrainScratch {
    pub(super) fn shrink_to(&mut self, bytes: usize) {
        let retained = bytes.min(self.bytes);
        self.resources
            .drain_bytes
            .fetch_sub(self.bytes - retained, Ordering::AcqRel);
        self.bytes = retained;
    }
}

impl Drop for DrainScratch {
    fn drop(&mut self) {
        self.resources
            .drain_bytes
            .fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

/// A workspace is independent of queued credit. Oversized legal records keep
/// their actual capacity; the normal 4 MiB workspace is not a rejection limit.
pub(super) struct EncodeScratch {
    resources: Arc<ContentResources>,
    background: bool,
    bytes: usize,
}

impl EncodeScratch {
    pub(super) fn resize(&mut self, bytes: usize) {
        let (live, peak) = if self.background {
            (&self.resources.encode_bytes, &self.resources.encode_peak)
        } else {
            (
                &self.resources.foreground_encode_bytes,
                &self.resources.foreground_encode_peak,
            )
        };
        let total = if bytes >= self.bytes {
            live.fetch_add(bytes - self.bytes, Ordering::AcqRel) + bytes - self.bytes
        } else {
            live.fetch_sub(self.bytes - bytes, Ordering::AcqRel) - (self.bytes - bytes)
        };
        peak.fetch_max(total, Ordering::Relaxed);
        self.bytes = bytes;
    }
}

impl Drop for EncodeScratch {
    fn drop(&mut self) {
        self.resize(0);
        if self.background {
            *lock(&self.resources.encoder_busy) = false;
            self.resources.encoder_available.notify_waiters();
        }
    }
}

#[derive(Default)]
pub(crate) struct ContentSpool {
    factory: OnceLock<OverflowFactory>,
    writer: OnceLock<OverflowWriter>,
    state: Mutex<SpillState>,
}

#[derive(Default)]
struct SpillState {
    spilling: bool,
    outstanding: usize,
}

impl ContentSpool {
    pub(crate) fn bind(&self, factory: OverflowFactory) {
        let _ = self.factory.set(factory);
    }

    pub(super) fn can_spill(&self) -> bool {
        self.factory.get().is_some()
    }

    fn wants_spill(&self, resources: &ContentResources, queued: usize, estimate: usize) -> bool {
        let mut state = lock(&self.state);
        if state.outstanding == 0 && queued < resources.limits.resume {
            state.spilling = false;
        }
        state.spilling || queued.saturating_add(estimate) >= resources.limits.soft
    }

    pub(super) fn writer(&self, resources: &ContentResources) -> Option<&OverflowWriter> {
        let factory = self.factory.get()?;
        Some(
            self.writer.get_or_init(|| {
                factory.writer(Arc::clone(&resources.quota), resources.limits.segment)
            }),
        )
    }

    pub(super) fn retained(self: &Arc<Self>, shared: &Arc<ProducerShared>) -> SpillOwner {
        let mut state = lock(&self.state);
        state.spilling = true;
        state.outstanding += 1;
        SpillOwner {
            spool: Arc::clone(self),
            shared: Arc::clone(shared),
        }
    }
}

pub(super) struct SpillOwner {
    spool: Arc<ContentSpool>,
    shared: Arc<ProducerShared>,
}

impl Drop for SpillOwner {
    fn drop(&mut self) {
        let finished = {
            let mut state = lock(&self.spool.state);
            state.outstanding -= 1;
            if state.outstanding == 0 {
                let content = self.shared.content.as_ref().expect("content ownership");
                if self.shared.queued_bytes.load(Ordering::Acquire) < content.limits.resume {
                    state.spilling = false;
                }
                true
            } else {
                false
            }
        };
        // Release the consumed tail even while another request keeps aggregate
        // RAM above the low watermark. No state lock spans filesystem cleanup.
        if finished && let Some(writer) = self.spool.writer.get() {
            writer.seal();
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum ContentSlotClass {
    Data,
    Control,
    Progress,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ContentPublishFailure {
    Capacity,
    Encoding,
    OverflowIo,
    OverflowIntegrity,
    OverflowCapacity,
    Disconnected,
}

impl ContentPublishFailure {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::Capacity => "content_capture_capacity_exhausted",
            Self::Encoding => "content_capture_encoding_failed",
            Self::OverflowIo => "content_overflow_io_failed",
            Self::OverflowIntegrity => "content_overflow_integrity_failed",
            Self::OverflowCapacity => "content_overflow_capacity_exhausted",
            Self::Disconnected => "content_capture_worker_disconnected",
        }
    }
}

struct SequenceOwner {
    shared: Arc<ProducerShared>,
    sequence: u64,
    armed: bool,
}

impl Drop for SequenceOwner {
    fn drop(&mut self) {
        if self.armed {
            // Also closes an allocated sequence when a builder unwinds.
            restore_worker_loss(
                &self.shared,
                self.sequence,
                ObservationLossReason::SinkFailed,
            );
        }
    }
}

impl ByteBoundedObservationProducer {
    pub(in super::super) fn content_resource_snapshot(&self) -> Option<ContentResourceSnapshot> {
        let resources = self.shared.content.as_ref()?;
        let disk = resources.disk_snapshot();
        Some(ContentResourceSnapshot {
            queued_bytes: self.shared.queued_bytes.load(Ordering::Acquire),
            queue_peak_bytes: self.shared.queue_high_water_bytes.load(Ordering::Relaxed),
            queued_source_bytes: self.shared.work_bytes.load(Ordering::Acquire),
            active_source_bytes: resources.active_source_bytes.load(Ordering::Acquire),
            record_slots: resources.slots.load(Ordering::Acquire),
            encode_bytes: resources.encode_bytes.load(Ordering::Acquire),
            encode_peak_bytes: resources.encode_peak.load(Ordering::Relaxed),
            foreground_encode_bytes: resources.foreground_encode_bytes.load(Ordering::Acquire),
            foreground_encode_peak_bytes: resources.foreground_encode_peak.load(Ordering::Relaxed),
            drain_bytes: resources.drain_bytes.load(Ordering::Acquire),
            drain_peak_bytes: resources.drain_peak.load(Ordering::Relaxed),
            overflow_reserved_bytes: disk.reserved_disk_bytes,
            overflow_file_bytes: disk.retained_file_bytes,
            overflow_written_bytes: disk.written_bytes,
            overflow_read_bytes: disk.read_bytes,
            overflow_peak_bytes: disk.peak_reserved_bytes,
            overflow_live_segments: disk.live_segments,
            overflow_created_segments: disk.created_segments,
            overflow_cleanup_failures: disk.cleanup_failures,
            io_failures: resources.io_failures.load(Ordering::Relaxed),
            integrity_failures: resources.integrity_failures.load(Ordering::Relaxed),
        })
    }

    fn content_ticket(&self, bytes: usize, class: ContentSlotClass) -> Option<RecordCredit> {
        let content = self.shared.content.as_ref()?;
        let _gate = lock(&self.shared.credit_gate);
        let total = self.shared.queued_bytes.load(Ordering::Relaxed);
        let control = content.control_bytes.load(Ordering::Relaxed);
        let progress =
            usize::from(content.progress_busy.load(Ordering::Relaxed)) * content.limits.progress;
        let data = total.saturating_sub(control).saturating_sub(progress);
        let has_slot = content.slots.load(Ordering::Relaxed) < content.limits.slots();
        let available = match class {
            ContentSlotClass::Data => {
                has_slot
                    && data.saturating_add(bytes)
                        <= content.limits.bytes.saturating_sub(content.limits.control)
            }
            // The control reserve is protected from data, not a separate
            // control hard cap. Controls may borrow otherwise unused bytes.
            ContentSlotClass::Control => has_slot,
            ContentSlotClass::Progress => {
                !content.progress_busy.load(Ordering::Relaxed)
                    && bytes == content.limits.progress
                    && bytes >= CONTENT_RECORD_BYTES
            }
        };
        let reserved = available
            && if matches!(class, ContentSlotClass::Progress) {
                reserve_bytes_up_to(&self.shared, bytes, self.shared.capacity_bytes)
            } else {
                reserve_bytes(&self.shared, bytes)
            };
        if !reserved {
            #[cfg(test)]
            {
                self.shared.credit_waits.fetch_add(1, Ordering::Relaxed);
                self.shared.credit_available.notify_all();
            }
            return None;
        }
        match class {
            ContentSlotClass::Data => {
                content.slots.fetch_add(1, Ordering::Relaxed);
            }
            ContentSlotClass::Control => {
                content.slots.fetch_add(1, Ordering::Relaxed);
                content.control_bytes.fetch_add(bytes, Ordering::Relaxed);
            }
            ContentSlotClass::Progress => {
                content.progress_busy.store(true, Ordering::Release);
            }
        }
        Some(RecordCredit {
            shared: Arc::clone(&self.shared),
            bytes,
            content_slot: Some(class),
        })
    }

    pub(in super::super) async fn publish_content<F>(
        &self,
        timing: Option<super::super::super::latency::RequestLatency>,
        estimate: usize,
        background: bool,
        control: bool,
        spool: &Arc<ContentSpool>,
        build: F,
    ) -> Result<(), ContentPublishFailure>
    where
        F: FnOnce(ObservationSequenceStamp) -> Result<Vec<u8>, serde_json::Error>,
    {
        let resources = self.shared.content.as_ref().expect("Content producer");
        let estimate = estimate.max(CONTENT_RECORD_BYTES);
        let timing_bytes = usize::from(timing.is_some()) * 64;
        let descriptor_bytes = CONTENT_RECORD_BYTES + timing_bytes;
        let class = if control {
            ContentSlotClass::Control
        } else {
            ContentSlotClass::Data
        };
        // Every resource is acquired before sequence assignment. If a ticket
        // is unavailable, release scratch before suspending; another request
        // can encode/drain without a later sequence pinning the encoder.
        let (mut scratch, mut credit, mut spill) = loop {
            let notified = self.shared.credit_changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let scratch = resources
                .encode(background, estimate.saturating_mul(2))
                .await;
            let mut spill = spool.can_spill()
                && spool.wants_spill(
                    resources,
                    self.shared.queued_bytes.load(Ordering::Acquire),
                    estimate,
                );
            #[cfg(test)]
            if spill {
                // Test synchronization observes actual RAM pressure even when
                // segmented overflow prevents a producer from blocking.
                self.shared.credit_waits.fetch_add(1, Ordering::Relaxed);
                self.shared.credit_available.notify_all();
            }
            let mut credit =
                self.content_ticket(if spill { descriptor_bytes } else { estimate }, class);
            if credit.is_none() && !spill && spool.can_spill() {
                spill = true;
                credit = self.content_ticket(descriptor_bytes, class);
            }
            if credit.is_none() && background {
                credit = self.content_ticket(resources.limits.progress, ContentSlotClass::Progress);
                if credit.is_some() {
                    spill = false;
                }
            }
            if let Some(credit) = credit {
                break (scratch, credit, spill);
            }
            drop(scratch);
            if !background {
                drop(timing.map(|timing| timing.record()));
                self.declare_work_loss();
                return Err(ContentPublishFailure::Capacity);
            }
            notified.await;
        };
        let sequence = self.shared.next_sequence.fetch_add(1, Ordering::Relaxed);
        let mut sequence_owner = SequenceOwner {
            shared: Arc::clone(&self.shared),
            sequence,
            armed: true,
        };
        let stamp = ObservationSequenceStamp {
            identity: self.identity.clone(),
            sequence,
            loss_watermark: None,
        };
        let payload = build(stamp.clone()).map_err(|_| ContentPublishFailure::Encoding)?;
        scratch.resize(payload.capacity().saturating_mul(2));
        let timing = timing.map(|timing| timing.record());
        let accounted_bytes = payload.len().saturating_add(descriptor_bytes);
        // A conservative estimate is not a new legal-record limit. Unexpected
        // larger envelopes can use the same admitted descriptor and backing.
        if accounted_bytes > credit.bytes
            && spool.can_spill()
            && !matches!(credit.content_slot, Some(ContentSlotClass::Progress))
        {
            spill = true;
        }
        let record = if spill {
            let owner = spool.retained(&self.shared);
            let writer = spool
                .writer(resources)
                .ok_or(ContentPublishFailure::OverflowIo)?;
            let range = writer
                .append(sequence, &payload)
                .map_err(|error| match error {
                    crate::replay::ReplayError::Integrity => {
                        resources.integrity_failures.fetch_add(1, Ordering::Relaxed);
                        ContentPublishFailure::OverflowIntegrity
                    }
                    crate::replay::ReplayError::StructureLimit => {
                        ContentPublishFailure::OverflowCapacity
                    }
                    _ => {
                        resources.io_failures.fetch_add(1, Ordering::Relaxed);
                        ContentPublishFailure::OverflowIo
                    }
                })?;
            credit.shrink_to(descriptor_bytes.min(credit.bytes));
            drop(payload);
            QueuedRecord::spilled(stamp, range, accounted_bytes, timing, credit, owner)
        } else {
            let keep_scratch = matches!(credit.content_slot, Some(ContentSlotClass::Progress))
                || accounted_bytes > credit.bytes;
            if !keep_scratch {
                credit.shrink_to(accounted_bytes);
            }
            let payload: Arc<[u8]> = payload.into();
            scratch.resize(payload.len());
            let record = QueuedRecord::resident(
                ObservationRecord {
                    stamp,
                    payload,
                    accounted_bytes,
                    gap_heartbeat: false,
                    timing,
                },
                credit,
            );
            if keep_scratch {
                record.hold_workspace(scratch)
            } else {
                drop(scratch);
                record
            }
        };
        // Scratch is moved only for the dedicated progress/oversized resident.
        // Spill scratch is released on scope exit after the encoded Vec drops.
        match self
            .sender
            .try_send(Box::new(ProducerMessage::Record(record)))
        {
            Ok(()) => {
                sequence_owner.armed = false;
                self.shared.enqueued.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(error) => {
                drop(error);
                Err(ContentPublishFailure::Disconnected)
            }
        }
    }
}
