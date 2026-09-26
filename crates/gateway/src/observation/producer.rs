use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;

use hiroute_diagnostics::event::{
    DiagnosticEvent, GapReason, NackReason, ObservationChannel, ObservationGap, SinkNack,
};
use hiroute_diagnostics::runtime::DiagnosticsPort;

use super::schema::{LossWatermarkV1, ObservationNackDetailV1, ProducerDescriptorV1};

#[path = "producer/bounded.rs"]
mod bounded;
#[path = "producer/feedback.rs"]
mod feedback;

pub(super) use bounded::ObservationProducerIdentity;
use bounded::{
    ByteBoundedObservationProducer, ObservationLossReason, ObservationLossWatermark,
    ObservationSequenceStamp, RECORD_ACCOUNTING_BYTES,
};
pub use bounded::{ObservationAck, ObservationNack, ObservationRecord, ObservationRecordSink};
pub use feedback::accounted_acknowledgement;
use feedback::receiver_unavailable_nack;

const DEFAULT_QUEUE_BYTES: usize = 512 * 1024;
const MAX_QUEUE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct ChannelProducer {
    component: Arc<str>,
    revision: Arc<str>,
    inner: Option<ByteBoundedObservationProducer>,
}

impl ChannelProducer {
    pub(super) fn disabled(component: &'static str, revision: &'static str) -> Self {
        Self {
            component: Arc::from(component),
            revision: Arc::from(revision),
            inner: None,
        }
    }

    pub(super) fn new(
        identity: ObservationProducerIdentity,
        queue_bytes: usize,
        sink: Arc<dyn ObservationRecordSink>,
        diagnostics: Arc<Mutex<DiagnosticsPort>>,
        channel: ObservationChannel,
    ) -> Self {
        let component = Arc::clone(&identity.component);
        let revision = Arc::clone(&identity.revision);
        let sink: Arc<dyn ObservationRecordSink> = Arc::new(ChannelDiagnosticsSink {
            inner: sink,
            diagnostics,
            channel,
            nacks: std::sync::atomic::AtomicU64::new(0),
        });
        // Every record reserves at least the accounting overhead against this
        // channel's byte budget. A second fixed event cap discarded small
        // execution facts before the byte budget was reached, making later
        // request receipts incomplete even though the sink was healthy.
        let queue_events = (queue_bytes / RECORD_ACCOUNTING_BYTES).max(1);
        let inner =
            ByteBoundedObservationProducer::new(identity, queue_bytes, queue_events, sink).ok();
        Self {
            component,
            revision,
            inner,
        }
    }

    pub(super) fn publish<T, F>(&self, build: F)
    where
        T: Serialize,
        F: FnOnce(ProducerDescriptorV1, u64, Option<LossWatermarkV1>) -> T,
    {
        let component = Arc::clone(&self.component);
        let revision = Arc::clone(&self.revision);
        let Some(inner) = &self.inner else {
            return;
        };
        inner.try_publish(move |stamp| {
            let producer = descriptor(&stamp, &component, &revision);
            let loss = stamp.loss_watermark.map(loss_watermark);
            serde_json::to_vec(&build(producer, stamp.sequence, loss)).unwrap_or_else(|_| {
                br#"{"schema_version":"hiroute.observation.serialization-failure/v1"}"#.to_vec()
            })
        });
    }
}

fn descriptor(
    stamp: &ObservationSequenceStamp,
    component: &str,
    revision: &str,
) -> ProducerDescriptorV1 {
    ProducerDescriptorV1 {
        component: component.into(),
        revision: revision.into(),
        producer_id: stamp.identity.producer_id.to_string(),
        producer_epoch: stamp.identity.producer_epoch.to_string(),
        stream_id: stamp.identity.stream_id.to_string(),
    }
}

fn loss_watermark(loss: ObservationLossWatermark) -> LossWatermarkV1 {
    LossWatermarkV1 {
        first_sequence: loss.first_sequence,
        last_sequence: loss.last_sequence,
        reason: match loss.reason {
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
        .into(),
    }
}

/// Projects one observation channel's own delivery feedback into the local diagnostic log.
/// Only the channel, the sequence range and the typed reason leave this wrapper; payload
/// bytes, envelopes and content are never inspected.
struct ChannelDiagnosticsSink {
    inner: Arc<dyn ObservationRecordSink>,
    diagnostics: Arc<Mutex<DiagnosticsPort>>,
    channel: ObservationChannel,
    nacks: std::sync::atomic::AtomicU64,
}

impl ChannelDiagnosticsSink {
    fn port(&self) -> DiagnosticsPort {
        self.diagnostics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl ObservationRecordSink for ChannelDiagnosticsSink {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        let result = self.inner.deliver(record);
        match &result {
            Ok(_) => {
                if record.is_gap_heartbeat()
                    && let Some(loss) = record.stamp().loss_watermark
                {
                    // The worker retries an unacknowledged gap heartbeat, so only a
                    // delivered heartbeat reports its loss range exactly once.
                    self.port()
                        .handle()
                        .try_emit(DiagnosticEvent::ObservationGap(ObservationGap {
                            channel: self.channel,
                            first_seq: loss.first_sequence,
                            last_seq: loss.last_sequence,
                            missing: loss
                                .last_sequence
                                .saturating_sub(loss.first_sequence)
                                .saturating_add(1),
                            reason: gap_reason(loss.reason),
                        }));
                }
            }
            Err(nack) => {
                let seen = self
                    .nacks
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                // A persistent sink outage stays visible without letting one channel fill
                // the bounded diagnostic queue: report the first failure, then a sample.
                if seen == 0 || seen.is_multiple_of(256) {
                    self.port()
                        .handle()
                        .try_emit(DiagnosticEvent::SinkNack(SinkNack {
                            channel: self.channel,
                            reason: nack_reason(&nack.detail),
                            retryable: nack.retryable,
                        }));
                }
            }
        }
        result
    }
}

fn gap_reason(reason: ObservationLossReason) -> GapReason {
    match reason {
        ObservationLossReason::PublishContended => GapReason::Backpressure,
        ObservationLossReason::QueueBytesExceeded
        | ObservationLossReason::QueueEventsExceeded
        | ObservationLossReason::EventTooLarge
        | ObservationLossReason::Compacted => GapReason::QueueOverflow,
        ObservationLossReason::SinkFailed | ObservationLossReason::SinkPanicked => {
            GapReason::SinkUnavailable
        }
        ObservationLossReason::SinkNack => GapReason::SinkUnavailable,
        ObservationLossReason::WorkerDisconnected => GapReason::Closed,
    }
}

fn nack_reason(detail: &ObservationNackDetailV1) -> NackReason {
    match detail {
        ObservationNackDetailV1::ReceiverUnavailable { .. } => NackReason::Unavailable,
        ObservationNackDetailV1::UnsupportedSchema { .. }
        | ObservationNackDetailV1::InvalidEnvelope { .. }
        | ObservationNackDetailV1::SequenceEventConflict { .. } => NackReason::SerializationFailed,
        ObservationNackDetailV1::DigestMismatch { .. } => NackReason::IntegrityFailed,
        ObservationNackDetailV1::MissingSequenceRanges { .. }
        | ObservationNackDetailV1::MissingPrerequisite { .. }
        | ObservationNackDetailV1::UnknownTranscriptRoot { .. }
        | ObservationNackDetailV1::MissingBlob { .. }
        | ObservationNackDetailV1::ChunkOrdinalConflict { .. }
        | ObservationNackDetailV1::ContentStateConflict { .. }
        | ObservationNackDetailV1::ImmutableProjectionConflict { .. } => NackReason::StoreRejected,
    }
}

pub struct GatewayObservationSinks {
    pub lifecycle: Arc<dyn ObservationRecordSink>,
    pub execution_fact: Arc<dyn ObservationRecordSink>,
    pub conversation_content: Arc<dyn ObservationRecordSink>,
    pub run_relation: Arc<dyn ObservationRecordSink>,
    pub otel: Arc<dyn ObservationRecordSink>,
}

impl GatewayObservationSinks {
    pub fn discard() -> Self {
        let sink: Arc<dyn ObservationRecordSink> = Arc::new(DiscardSink);
        Self {
            lifecycle: Arc::clone(&sink),
            execution_fact: Arc::clone(&sink),
            conversation_content: Arc::clone(&sink),
            run_relation: Arc::clone(&sink),
            otel: sink,
        }
    }

    fn files(directory: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        Ok(Self {
            lifecycle: faulting_file(
                directory.join("lifecycle.jsonl"),
                "HIROUTE_OBSERVATION_LIFECYCLE_SINK",
            )?,
            execution_fact: faulting_file(
                directory.join("execution-fact.jsonl"),
                "HIROUTE_OBSERVATION_EXECUTION_SINK",
            )?,
            conversation_content: faulting_file(
                directory.join("conversation-content.jsonl"),
                "HIROUTE_OBSERVATION_CONTENT_SINK",
            )?,
            run_relation: faulting_file(
                directory.join("run-relation.jsonl"),
                "HIROUTE_OBSERVATION_RUN_RELATION_SINK",
            )?,
            otel: faulting_file(
                directory.join("otel.jsonl"),
                "HIROUTE_OBSERVATION_OTEL_SINK",
            )?,
        })
    }
}

pub(super) struct EnvironmentObservation {
    pub(super) enabled: bool,
    pub(super) queue_bytes: usize,
    pub(super) sinks: GatewayObservationSinks,
}

impl EnvironmentObservation {
    pub(super) fn load() -> Self {
        // The file collector exists only so a real hirouted process can be
        // exercised by the hermetic E2E harness. Product adapters inject
        // sinks through `GatewayObservation::with_sinks`; production must not
        // acquire a plaintext content-store mode from ambient environment.
        if std::env::var("HIROUTE_E2E_OBSERVATION_CAPTURE").as_deref() != Ok("1") {
            return Self {
                enabled: false,
                queue_bytes: DEFAULT_QUEUE_BYTES,
                sinks: GatewayObservationSinks::discard(),
            };
        }
        let Some(directory) = std::env::var_os("HIROUTE_OBSERVATION_DIRECTORY") else {
            return Self {
                enabled: false,
                queue_bytes: DEFAULT_QUEUE_BYTES,
                sinks: GatewayObservationSinks::discard(),
            };
        };
        let queue_bytes = std::env::var("HIROUTE_OBSERVATION_QUEUE_BYTES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0 && *value <= MAX_QUEUE_BYTES)
            .unwrap_or(DEFAULT_QUEUE_BYTES);
        match GatewayObservationSinks::files(&PathBuf::from(directory)) {
            Ok(sinks) => Self {
                enabled: true,
                queue_bytes,
                sinks,
            },
            Err(_) => Self {
                enabled: false,
                queue_bytes,
                sinks: GatewayObservationSinks::discard(),
            },
        }
    }
}

struct DiscardSink;

impl ObservationRecordSink for DiscardSink {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        Ok(accounted_acknowledgement(record))
    }
}

struct FileSink {
    file: Mutex<File>,
    behavior: SinkBehavior,
}

enum SinkBehavior {
    Healthy,
    FailOnce(AtomicBool),
    PanicOnce(AtomicBool),
    SlowOnce {
        pending: AtomicBool,
        duration: Duration,
    },
}

impl ObservationRecordSink for FileSink {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        match &self.behavior {
            SinkBehavior::Healthy => {}
            SinkBehavior::FailOnce(pending) if pending.swap(false, Ordering::AcqRel) => {
                return Err(receiver_unavailable_nack(record, false));
            }
            SinkBehavior::PanicOnce(pending) if pending.swap(false, Ordering::AcqRel) => {
                panic!("injected observation sink panic");
            }
            SinkBehavior::SlowOnce { pending, duration }
                if pending.swap(false, Ordering::AcqRel) =>
            {
                std::thread::sleep(*duration);
            }
            SinkBehavior::FailOnce(_)
            | SinkBehavior::PanicOnce(_)
            | SinkBehavior::SlowOnce { .. } => {}
        }
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        file.write_all(record.payload())
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.flush())
            .map_err(|_| receiver_unavailable_nack(record, true))?;
        Ok(accounted_acknowledgement(record))
    }
}

fn faulting_file(
    path: PathBuf,
    behavior_variable: &'static str,
) -> std::io::Result<Arc<dyn ObservationRecordSink>> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    let behavior =
        parse_behavior(&std::env::var(behavior_variable).unwrap_or_else(|_| "healthy".into()));
    Ok(Arc::new(FileSink {
        file: Mutex::new(file),
        behavior,
    }))
}

fn parse_behavior(value: &str) -> SinkBehavior {
    match value {
        "fail_once" => SinkBehavior::FailOnce(AtomicBool::new(true)),
        "panic_once" => SinkBehavior::PanicOnce(AtomicBool::new(true)),
        value if value.starts_with("slow_once:") => {
            let millis = value
                .strip_prefix("slow_once:")
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| *value <= 30_000)
                .unwrap_or(1_000);
            SinkBehavior::SlowOnce {
                pending: AtomicBool::new(true),
                duration: Duration::from_millis(millis),
            }
        }
        _ => SinkBehavior::Healthy,
    }
}
