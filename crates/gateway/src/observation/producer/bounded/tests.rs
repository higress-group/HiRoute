use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use super::super::super::schema::{
    CONVERSATION_CONTENT_CHANNEL, CONVERSATION_CONTENT_SCHEMA, ContentRefV1,
    ConversationContentEnvelopeV1, CorrelationV1, ObservationBlobAcknowledgementV1,
    ObservationContentAcknowledgementV1, ObservationContentDirectionV1,
};
use super::super::feedback::{
    accounted_acknowledgement, acknowledgement_covers_record, nack_matches_record,
    receiver_unavailable_nack,
};
use super::*;

#[derive(Default)]
struct CapturingSink {
    records: Mutex<Vec<ObservationSequenceStamp>>,
    gaps: Mutex<Vec<ObservationSequenceStamp>>,
    deliveries: Mutex<Vec<(u64, bool)>>,
}

impl ObservationRecordSink for CapturingSink {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        self.deliveries
            .lock()
            .unwrap()
            .push((record.stamp().sequence, record.is_gap_heartbeat()));
        if record.is_gap_heartbeat() {
            self.gaps.lock().unwrap().push(record.stamp().clone());
        } else {
            self.records.lock().unwrap().push(record.stamp().clone());
        }
        Ok(accounted_acknowledgement(record))
    }
}

struct PanicOnceSink {
    panic: AtomicBool,
    records: Mutex<Vec<ObservationSequenceStamp>>,
    gaps: Mutex<Vec<ObservationSequenceStamp>>,
}

impl ObservationRecordSink for PanicOnceSink {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        assert!(!self.panic.swap(false, Ordering::AcqRel), "sink panic");
        if record.is_gap_heartbeat() {
            self.gaps.lock().unwrap().push(record.stamp().clone());
        } else {
            self.records.lock().unwrap().push(record.stamp().clone());
        }
        Ok(accounted_acknowledgement(record))
    }
}

struct RecoveringNackSink {
    nacks_remaining: AtomicUsize,
    gaps: Mutex<Vec<ObservationSequenceStamp>>,
}

#[derive(Default)]
struct WrongIdentityAckSink {
    gaps: Mutex<Vec<ObservationSequenceStamp>>,
}

impl ObservationRecordSink for WrongIdentityAckSink {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        let mut acknowledgement = accounted_acknowledgement(record);
        if record.is_gap_heartbeat() {
            self.gaps.lock().unwrap().push(record.stamp().clone());
        } else {
            acknowledgement.identity.stream_id = "stream:wrong".into();
        }
        Ok(acknowledgement)
    }
}

impl ObservationRecordSink for RecoveringNackSink {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        if self
            .nacks_remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            return Err(receiver_unavailable_nack(record, true));
        }
        if record.is_gap_heartbeat() {
            self.gaps.lock().unwrap().push(record.stamp().clone());
        }
        Ok(accounted_acknowledgement(record))
    }
}

struct SlowOnceSink {
    gate: Arc<(Mutex<bool>, Condvar)>,
    slow: AtomicBool,
    records: Mutex<Vec<ObservationSequenceStamp>>,
    gaps: Mutex<Vec<ObservationSequenceStamp>>,
    deliveries: Mutex<Vec<(u64, bool)>>,
}

#[test]
fn two_long_histories_fit_local_byte_capacity_and_overflow_stays_explicit() {
    // 1,000 canonical parts per history, including roughly 4.5 KiB of text,
    // base64 and envelope metadata per part. The sink is deliberately stopped
    // for the entire two-request burst; generation must never wait for it.
    const RECORDS: usize = 2_000;
    for capacity in [4 * 1024 * 1024, 16 * 1024 * 1024] {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let sink = Arc::new(SlowOnceSink {
            gate: gate.clone(),
            slow: AtomicBool::new(true),
            records: Mutex::new(Vec::new()),
            gaps: Mutex::new(Vec::new()),
            deliveries: Mutex::new(Vec::new()),
        });
        let producer = ByteBoundedObservationProducer::new(
            identity("history-burst"),
            capacity,
            capacity / RECORD_ACCOUNTING_BYTES,
            sink.clone(),
        )
        .unwrap();
        for _ in 0..RECORDS {
            producer.try_publish(|_| vec![b'x'; 4_608]);
        }
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        producer.flush(Duration::from_secs(5)).unwrap();
        let stats = producer.stats();
        assert!(stats.queue_high_water_bytes <= capacity);
        assert_eq!(stats.queued_bytes, 0);
        if capacity == 4 * 1024 * 1024 {
            assert!(stats.dropped > 0);
            assert!(!sink.gaps.lock().unwrap().is_empty());
        } else {
            assert_eq!(stats.dropped, 0);
            assert_eq!(sink.records.lock().unwrap().len(), RECORDS);
            assert!(sink.gaps.lock().unwrap().is_empty());
        }
    }
}

impl ObservationRecordSink for SlowOnceSink {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        if self.slow.swap(false, Ordering::AcqRel) {
            let (lock, wake) = &*self.gate;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = wake.wait(released).unwrap();
            }
        }
        self.deliveries
            .lock()
            .unwrap()
            .push((record.stamp().sequence, record.is_gap_heartbeat()));
        if record.is_gap_heartbeat() {
            self.gaps.lock().unwrap().push(record.stamp().clone());
        } else {
            self.records.lock().unwrap().push(record.stamp().clone());
        }
        Ok(accounted_acknowledgement(record))
    }
}

#[test]
fn optional_worker_parks_for_credit_without_spinning_and_ack_releases_it() {
    const CAPACITY: usize = 64 * 1024;
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let sink = Arc::new(SlowOnceSink {
        gate: gate.clone(),
        slow: AtomicBool::new(true),
        records: Mutex::new(Vec::new()),
        gaps: Mutex::new(Vec::new()),
        deliveries: Mutex::new(Vec::new()),
    });
    let producer =
        ByteBoundedObservationProducer::new(identity("credit"), CAPACITY, 128, sink.clone())
            .unwrap();
    let work = producer.reserve_work(producer.work_capacity()).unwrap();
    assert!(
        producer.reserve_work(1).is_none(),
        "queued work must leave record headroom"
    );
    assert!(matches!(
        producer.try_publish(|_| vec![0; 8192]),
        ObservationPublishOutcome::Enqueued { .. }
    ));
    let worker = producer.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let result = worker.publish_timed(
            None,
            Some(Instant::now() + Duration::from_secs(1)),
            8192 + RECORD_ACCOUNTING_BYTES,
            |_| vec![1; 8192],
        );
        sender.send(result).unwrap();
    });
    producer.wait_until_credit_blocked();
    assert_eq!(producer.shared.credit_waits.load(Ordering::Acquire), 1);
    assert!(receiver.try_recv().is_err());
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    assert!(matches!(
        receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
        ObservationPublishOutcome::Enqueued { .. }
    ));
    thread.join().unwrap();
    producer.flush(Duration::from_secs(2)).unwrap();
    drop(work);
    assert_eq!(producer.stats().queued_bytes, 0);
    assert_eq!(producer.stats().dropped, 0);
    assert_eq!(sink.records.lock().unwrap().len(), 2);
    assert!(producer.stats().queue_high_water_bytes <= CAPACITY);
}

#[test]
fn background_record_credit_leaves_room_for_a_concurrent_foreground_job() {
    const CAPACITY: usize = 512 * 1024;
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let sink = Arc::new(SlowOnceSink {
        gate: gate.clone(),
        slow: AtomicBool::new(true),
        records: Mutex::new(Vec::new()),
        gaps: Mutex::new(Vec::new()),
        deliveries: Mutex::new(Vec::new()),
    });
    let producer = ByteBoundedObservationProducer::new(
        identity("foreground-credit"),
        CAPACITY,
        128,
        sink.clone(),
    )
    .unwrap();
    let active_work = producer.reserve_work(5 * CAPACITY / 8).unwrap();
    let worker = producer.clone();
    let thread = std::thread::spawn(move || {
        for _ in 0..8 {
            assert!(matches!(
                worker.publish_timed(
                    None,
                    Some(Instant::now() + Duration::from_secs(1)),
                    32 * 1024 + RECORD_ACCOUNTING_BYTES,
                    |_| { vec![b'x'; 32 * 1024] }
                ),
                ObservationPublishOutcome::Enqueued { .. }
            ));
        }
    });
    producer.wait_until_credit_blocked();
    let foreground = producer.reserve_work(64 * 1024);
    // Release the sink even if the assertion below fails, keeping the failed
    // negative-control run bounded instead of stranding a delivery thread.
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    assert!(
        foreground.is_some(),
        "optional encoded bursts displaced an admissible foreground job"
    );
    thread.join().unwrap();
    producer.flush(Duration::from_secs(2)).unwrap();
    drop(foreground);
    drop(active_work);
    assert_eq!(producer.stats().queued_bytes, 0);
    assert_eq!(producer.stats().dropped, 0);
    assert_eq!(sink.records.lock().unwrap().len(), 8);
    assert!(producer.stats().queue_high_water_bytes <= CAPACITY);
}

#[test]
fn background_credit_wait_does_not_claim_the_sequence_needed_to_drain() {
    const CAPACITY: usize = 512 * 1024;
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let sink = Arc::new(SlowOnceSink {
        gate: gate.clone(),
        slow: AtomicBool::new(true),
        records: Mutex::new(Vec::new()),
        gaps: Mutex::new(Vec::new()),
        deliveries: Mutex::new(Vec::new()),
    });
    let producer = ByteBoundedObservationProducer::new(
        identity("sequence-credit"),
        CAPACITY,
        128,
        sink.clone(),
    )
    .unwrap();
    let _work = producer.reserve_work(320 * 1024).unwrap();
    assert!(matches!(
        producer.try_publish(|_| vec![0; 96 * 1024 - RECORD_ACCOUNTING_BYTES]),
        ObservationPublishOutcome::Enqueued { sequence: 1 }
    ));
    let worker = producer.clone();
    let thread = std::thread::spawn(move || {
        worker.publish_timed(
            None,
            Some(Instant::now() + Duration::from_secs(1)),
            90 * 1024 + RECORD_ACCOUNTING_BYTES,
            |_| vec![1; 90 * 1024],
        )
    });
    producer.wait_until_credit_blocked();
    let foreground = producer.try_publish(|_| vec![2; 32 * 1024]);
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    // The waiting background builder must not leave a hole ahead of this
    // foreground record, whose ACK is needed to make that credit available.
    assert_eq!(
        foreground,
        ObservationPublishOutcome::Enqueued { sequence: 2 }
    );
    assert_eq!(
        thread.join().unwrap(),
        ObservationPublishOutcome::Enqueued { sequence: 3 }
    );
    producer.flush(Duration::from_secs(2)).unwrap();
    assert_eq!(
        *sink.deliveries.lock().unwrap(),
        [(1, false), (2, false), (3, false)]
    );
    assert_eq!(producer.stats().dropped, 0);
    assert!(producer.stats().queue_high_water_bytes <= CAPACITY);
}

#[test]
fn exhausted_background_credit_deadline_keeps_explicit_loss_and_releases_work() {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let sink = Arc::new(SlowOnceSink {
        gate: gate.clone(),
        slow: AtomicBool::new(true),
        records: Mutex::new(Vec::new()),
        gaps: Mutex::new(Vec::new()),
        deliveries: Mutex::new(Vec::new()),
    });
    let producer =
        ByteBoundedObservationProducer::new(identity("credit-stalled"), 65536, 128, sink.clone())
            .unwrap();
    let work = producer.reserve_work(producer.work_capacity()).unwrap();
    producer.try_publish(|_| vec![0; 8192]);
    assert!(matches!(
        producer.publish_timed(
            None,
            Some(Instant::now()),
            8192 + RECORD_ACCOUNTING_BYTES,
            |_| vec![1; 8192]
        ),
        ObservationPublishOutcome::Dropped { .. }
    ));
    assert_eq!(producer.shared.credit_waits.load(Ordering::Acquire), 0);
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    producer.flush(Duration::from_secs(2)).unwrap();
    drop(work);
    assert_eq!(producer.stats().queued_bytes, 0);
    assert_eq!(producer.stats().dropped, 1);
    assert_eq!(sink.gaps.lock().unwrap().len(), 1);
}

fn identity(name: &str) -> ObservationProducerIdentity {
    ObservationProducerIdentity {
        channel: Arc::from(name.to_owned()),
        component: Arc::from(format!("{name}-component")),
        revision: Arc::from("test/v1"),
        producer_id: Arc::from(format!("{name}-producer")),
        producer_epoch: Arc::from(format!("{name}-epoch")),
        stream_id: Arc::from(format!("{name}-stream")),
    }
}

#[test]
fn request_timing_follows_bounded_delivery_without_waiting_on_the_request() {
    use super::super::super::latency::RequestLatency;
    use hiroute_diagnostics::runtime::{DiagnosticRuntime, RuntimeConfig};
    use hiroute_diagnostics::{event::ProcessRole, level::DiagnosticLevel, record::Component};
    let root = std::env::temp_dir().join(format!(
        "hiroute-async-timing-{}-{}",
        std::process::id(),
        super::super::super::unix_nanos()
    ));
    let runtime = DiagnosticRuntime::start(RuntimeConfig {
        root: root.clone(),
        role: ProcessRole::Daemon,
        component: Component::Gateway,
        parent_session_id: None,
        level_override: Some(DiagnosticLevel::Info),
    });
    let latency = RequestLatency::new(runtime.port().handle().context(), None, Instant::now());
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let sink = Arc::new(SlowOnceSink {
        gate: gate.clone(),
        slow: AtomicBool::new(true),
        records: Mutex::new(Vec::new()),
        gaps: Mutex::new(Vec::new()),
        deliveries: Mutex::new(Vec::new()),
    });
    let producer =
        ByteBoundedObservationProducer::new(identity("timed"), 8_192, 64, sink.clone()).unwrap();
    for _ in 0..2 {
        assert!(matches!(
            producer.try_publish_timed(Some(latency.clone()), |_| vec![1; 64]),
            ObservationPublishOutcome::Enqueued { .. }
        ));
    }
    assert!(matches!(
        producer.try_publish_timed(Some(latency.clone()), |_| vec![1; 16_384]),
        ObservationPublishOutcome::Dropped { .. }
    ));
    let work = producer.reserve_work(producer.work_capacity()).unwrap();
    assert!(matches!(
        producer.publish_timed(
            Some(latency.clone()),
            Some(Instant::now()),
            8192,
            |_| panic!("refused credit must not build an envelope"),
        ),
        ObservationPublishOutcome::Dropped { .. }
    ));
    drop(work);
    // The request completes while the sink is still blocked. Neither timing nor
    // delivery requires a request-side flush or another worker.
    latency.finish(
        hiroute_diagnostics::event::RequestOutcome::Completed,
        Some(200),
    );
    drop(latency);
    let deadline = Instant::now() + Duration::from_secs(2);
    while sink.slow.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!sink.slow.load(Ordering::Acquire));
    std::thread::sleep(Duration::from_millis(5));
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    producer.flush(Duration::from_secs(2)).unwrap();
    runtime.shutdown();
    let log = std::fs::read_to_string(root.join("daemon/current.jsonl")).unwrap();
    let summaries = log
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter_map(|record| record["event"].get("request_timing_summary").cloned())
        .collect::<Vec<_>>();
    assert_eq!(summaries.len(), 1, "{log}");
    let summary = &summaries[0];
    assert_eq!(summary["asynchronous"]["records"], 4);
    assert_eq!(summary["asynchronous"]["delivered"], 2);
    assert_eq!(summary["asynchronous"]["dropped"], 2);
    assert!(
        summary["asynchronous"]["queue_wait_max_us"]
            .as_u64()
            .unwrap()
            >= 5_000
    );
    assert!(summary["asynchronous"]["sink_work_us"].as_u64().unwrap() >= 5_000);
    std::fs::remove_dir_all(root).unwrap();
}

fn content_record(sequence: u64, chunk_ordinal: u32) -> ObservationRecord {
    let producer_identity = identity(CONVERSATION_CONTENT_CHANNEL);
    let envelope = ConversationContentEnvelopeV1 {
        schema_version: CONVERSATION_CONTENT_SCHEMA.into(),
        schema_digest: "sha256:contract".into(),
        channel: CONVERSATION_CONTENT_CHANNEL.into(),
        producer: ProducerDescriptorV1 {
            component: producer_identity.component.to_string(),
            revision: producer_identity.revision.to_string(),
            producer_id: producer_identity.producer_id.to_string(),
            producer_epoch: producer_identity.producer_epoch.to_string(),
            stream_id: producer_identity.stream_id.to_string(),
        },
        sequence,
        event_id: format!("event:content:{sequence}"),
        correlation: CorrelationV1 {
            workspace_id: "workspace:test".into(),
            conversation_id: "conversation:test".into(),
            session_scope: "request_scoped".into(),
            correlation_provenance: "unproven".into(),
            turn_id: "turn:test".into(),
            request_id: "request:test".into(),
        },
        direction: "response_delivered".into(),
        phase: "append".into(),
        attempt_id: Some("attempt:test".into()),
        fork_id: "fork:test".into(),
        parent_transcript_root: Some("transcript:parent".into()),
        result_transcript_root: None,
        message_instance_id: Some("message:test".into()),
        message_role: Some("assistant".into()),
        content_kind: Some("text_delta".into()),
        content_id: Some("content:test".into()),
        content_blob_digest: Some("blob:test".into()),
        message_ordinal: Some(1),
        part_ordinal: Some(0),
        chunk_ordinal: Some(chunk_ordinal),
        transport_frame_id: Some(format!("frame:test:{chunk_ordinal}")),
        canonical_media_type: Some("text/plain".into()),
        canonical_bytes_base64: Some("dGVzdA==".into()),
        content_ref: Some(ContentRefV1 {
            content_id: "content:test".into(),
            digest: "blob:test".into(),
            byte_count: 131_072,
            media_type: "text/plain".into(),
        }),
        downstream_delivery: Some("full_frame_transport_accepted".into()),
        abort_reason: None,
        occurred_at_unix_nanos: sequence,
        loss_watermark: None,
        completeness_delta: None,
    };
    let payload: Arc<[u8]> = serde_json::to_vec(&envelope).unwrap().into();
    ObservationRecord {
        timing: None,
        stamp: ObservationSequenceStamp {
            identity: producer_identity,
            sequence,
            loss_watermark: None,
        },
        accounted_bytes: payload.len() + RECORD_ACCOUNTING_BYTES,
        payload,
        gap_heartbeat: false,
    }
}

fn content_acknowledgement_for(
    record: &ObservationRecord,
    next_chunk_ordinal: u32,
    acknowledged_blobs: Vec<ObservationBlobAcknowledgementV1>,
) -> ObservationAck {
    let mut acknowledgement = accounted_acknowledgement(record);
    acknowledgement.content_acknowledgement = Some(ObservationContentAcknowledgementV1 {
        request_id: "request:test".into(),
        direction: ObservationContentDirectionV1::ResponseDelivered,
        fork_id: "fork:test".into(),
        next_chunk_ordinal,
        transcript_root: Some("transcript:current".into()),
        delta_parent_transcript_root: Some("transcript:parent".into()),
        acknowledged_blobs,
    });
    acknowledgement
}

#[test]
fn feedback_identity_and_gap_frontier_are_bound_to_the_delivered_record() {
    let producer_identity = identity("feedback");
    let record = gap_heartbeat_record(
        &producer_identity,
        ObservationLossWatermark {
            first_sequence: 2,
            last_sequence: 3,
            reason: ObservationLossReason::SinkNack,
        },
    );
    let acknowledgement = accounted_acknowledgement(&record);
    assert_eq!(acknowledgement.highest_contiguous_sequence, 0);
    assert_eq!(acknowledgement.highest_accounted_sequence, 3);
    assert!(acknowledgement_covers_record(
        &record,
        &acknowledgement,
        None
    ));

    let mut crossed_gap = acknowledgement.clone();
    crossed_gap.highest_contiguous_sequence = 2;
    assert!(crossed_gap.validate().is_ok());
    assert!(!acknowledgement_covers_record(&record, &crossed_gap, None));

    let mut wrong_identity = acknowledgement.clone();
    wrong_identity.identity.stream_id = "stream:other".into();
    assert!(!acknowledgement_covers_record(
        &record,
        &wrong_identity,
        None
    ));

    let nack = receiver_unavailable_nack(&record, true);
    assert!(nack_matches_record(&record, &nack));
    let mut wrong_rejected_sequence = nack;
    wrong_rejected_sequence.rejected_sequence = 2;
    assert!(!nack_matches_record(&record, &wrong_rejected_sequence));
}

#[test]
fn invalid_ack_is_a_sink_failure_and_becomes_an_explicit_gap() {
    let sink = Arc::new(WrongIdentityAckSink::default());
    let producer =
        ByteBoundedObservationProducer::new(identity("invalid-ack"), 4096, 8, sink.clone())
            .unwrap();
    producer.try_publish(|_| vec![1; 16]);
    wait_until(|| !sink.gaps.lock().unwrap().is_empty());
    producer.flush(Duration::from_secs(1)).unwrap();

    assert_eq!(producer.stats().sink_failures, 1);
    assert_eq!(producer.stats().sink_nacks, 0);
    assert_eq!(
        sink.gaps.lock().unwrap()[0].loss_watermark,
        Some(ObservationLossWatermark {
            first_sequence: 1,
            last_sequence: 1,
            reason: ObservationLossReason::SinkFailed,
        })
    );
}

#[test]
fn content_acknowledgement_is_bound_to_request_direction_fork_ordinal_root_and_blob() {
    let record = content_record(1, 3);
    let mut acknowledgement = content_acknowledgement_for(
        &record,
        4,
        vec![ObservationBlobAcknowledgementV1 {
            content_id: "content:test".into(),
            digest: "blob:test".into(),
        }],
    );
    assert!(acknowledgement_covers_record(
        &record,
        &acknowledgement,
        None
    ));

    acknowledgement
        .content_acknowledgement
        .as_mut()
        .unwrap()
        .next_chunk_ordinal = 3;
    assert!(!acknowledgement_covers_record(
        &record,
        &acknowledgement,
        None
    ));
    acknowledgement
        .content_acknowledgement
        .as_mut()
        .unwrap()
        .next_chunk_ordinal = 4;
    acknowledgement
        .content_acknowledgement
        .as_mut()
        .unwrap()
        .request_id = "request:other".into();
    assert!(!acknowledgement_covers_record(
        &record,
        &acknowledgement,
        None
    ));

    let wrong_blob = content_acknowledgement_for(
        &record,
        4,
        vec![ObservationBlobAcknowledgementV1 {
            content_id: "content:test".into(),
            digest: "blob:other".into(),
        }],
    );
    assert!(!acknowledgement_covers_record(&record, &wrong_blob, None));

    let preceding_blob = content_acknowledgement_for(
        &record,
        4,
        vec![ObservationBlobAcknowledgementV1 {
            content_id: "content:preceding".into(),
            digest: "blob:preceding".into(),
        }],
    );
    assert!(acknowledgement_covers_record(
        &record,
        &preceding_blob,
        None
    ));
}

#[test]
fn multi_chunk_receiver_can_acknowledge_progress_before_blob_installation() {
    for (sequence, chunk_ordinal) in [(1_u64, 0_u32), (2, 1)] {
        let record = content_record(sequence, chunk_ordinal);
        let acknowledgement = content_acknowledgement_for(&record, chunk_ordinal + 1, Vec::new());
        assert!(acknowledgement_covers_record(
            &record,
            &acknowledgement,
            None
        ));
    }
}

#[test]
fn gap_followed_by_content_ack_preserves_authoritative_content_progress() {
    let producer_identity = identity(CONVERSATION_CONTENT_CHANNEL);
    let gap = gap_heartbeat_record(
        &producer_identity,
        ObservationLossWatermark {
            first_sequence: 1,
            last_sequence: 1,
            reason: ObservationLossReason::SinkFailed,
        },
    );
    let gap_acknowledgement = accounted_acknowledgement(&gap);
    assert!(acknowledgement_covers_record(
        &gap,
        &gap_acknowledgement,
        None
    ));

    let record = content_record(2, 0);
    let acknowledgement = content_acknowledgement_for(&record, 1, Vec::new());
    assert!(acknowledgement_covers_record(
        &record,
        &acknowledgement,
        Some(0)
    ));

    let mut crossed_gap = acknowledgement;
    crossed_gap.highest_contiguous_sequence = 1;
    assert!(!acknowledgement_covers_record(
        &record,
        &crossed_gap,
        Some(0)
    ));
}

#[test]
fn sink_panic_isolated_and_gap_heartbeat_needs_no_later_publish() {
    let sink = Arc::new(PanicOnceSink {
        panic: AtomicBool::new(true),
        records: Mutex::new(Vec::new()),
        gaps: Mutex::new(Vec::new()),
    });
    let producer =
        ByteBoundedObservationProducer::new(identity("panic"), 4096, 8, sink.clone()).unwrap();
    producer.try_publish(|_| vec![1; 16]);
    wait_until(|| producer.stats().sink_panics == 1);
    wait_until(|| !sink.gaps.lock().unwrap().is_empty());
    producer.flush(Duration::from_secs(1)).unwrap();

    let records = sink.gaps.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].sequence, 1);
    assert_eq!(
        records[0].loss_watermark,
        Some(ObservationLossWatermark {
            first_sequence: 1,
            last_sequence: 1,
            reason: ObservationLossReason::SinkPanicked,
        })
    );
}

#[test]
fn terminal_nack_retries_gap_heartbeat_without_later_publish() {
    let sink = Arc::new(RecoveringNackSink {
        // The data record and both initial heartbeat deliveries fail. The
        // worker must schedule another bounded-backoff retry while idle.
        nacks_remaining: AtomicUsize::new(3),
        gaps: Mutex::new(Vec::new()),
    });
    let producer =
        ByteBoundedObservationProducer::new(identity("nack"), 4096, 8, sink.clone()).unwrap();
    producer.try_publish(|_| vec![1; 16]);
    wait_until(|| !sink.gaps.lock().unwrap().is_empty());
    producer.flush(Duration::from_secs(1)).unwrap();

    let gaps = sink.gaps.lock().unwrap();
    assert_eq!(gaps.len(), 1);
    assert_eq!(
        gaps[0].loss_watermark,
        Some(ObservationLossWatermark {
            first_sequence: 1,
            last_sequence: 1,
            reason: ObservationLossReason::SinkNack,
        })
    );
    assert_eq!(producer.stats().gap_heartbeat_failures, 2);
}

#[test]
fn terminal_oversize_drop_emits_gap_heartbeat_without_later_publish() {
    let sink = Arc::new(CapturingSink::default());
    let producer =
        ByteBoundedObservationProducer::new(identity("oversize"), 256, 8, sink.clone()).unwrap();

    assert_eq!(
        producer.try_publish(|_| vec![0; 256]),
        ObservationPublishOutcome::Dropped { sequence: 1 }
    );
    wait_until(|| !sink.gaps.lock().unwrap().is_empty());
    producer.flush(Duration::from_secs(1)).unwrap();

    let records = sink.gaps.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].sequence, 1);
    assert_eq!(
        records[0].loss_watermark,
        Some(ObservationLossWatermark {
            first_sequence: 1,
            last_sequence: 1,
            reason: ObservationLossReason::EventTooLarge,
        })
    );
}

#[test]
fn slow_sink_never_blocks_publish_and_byte_overflow_becomes_gap() {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let sink = Arc::new(SlowOnceSink {
        gate: Arc::clone(&gate),
        slow: AtomicBool::new(true),
        records: Mutex::new(Vec::new()),
        gaps: Mutex::new(Vec::new()),
        deliveries: Mutex::new(Vec::new()),
    });
    let producer =
        ByteBoundedObservationProducer::new(identity("slow"), 320, 8, sink.clone()).unwrap();
    assert!(matches!(
        producer.try_publish(|_| vec![1; 64]),
        ObservationPublishOutcome::Enqueued { .. }
    ));
    let started = Instant::now();
    let second = producer.try_publish(|_| vec![2; 64]);
    let third = producer.try_publish(|_| vec![3; 64]);
    assert!(started.elapsed() < Duration::from_millis(50));
    assert!(matches!(second, ObservationPublishOutcome::Dropped { .. }));
    assert!(matches!(third, ObservationPublishOutcome::Dropped { .. }));

    let (released, wake) = &*gate;
    *released.lock().unwrap() = true;
    wake.notify_all();
    wait_until(|| producer.stats().delivered == 1);
    producer.try_publish(|_| vec![4; 32]);
    producer.flush(Duration::from_secs(1)).unwrap();

    let records = sink.records.lock().unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].sequence, 4);
    let gaps = sink.gaps.lock().unwrap();
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].loss_watermark.unwrap().first_sequence, 2);
    assert_eq!(gaps[0].loss_watermark.unwrap().last_sequence, 3);
    assert_eq!(
        *sink.deliveries.lock().unwrap(),
        [(1, false), (3, true), (4, false)]
    );
}

#[test]
fn healthy_sink_preserves_independent_monotonic_streams() {
    let left = Arc::new(CapturingSink::default());
    let right = Arc::new(CapturingSink::default());
    let left_producer =
        ByteBoundedObservationProducer::new(identity("left"), 4096, 8, left.clone()).unwrap();
    let right_producer =
        ByteBoundedObservationProducer::new(identity("right"), 4096, 8, right.clone()).unwrap();
    left_producer.try_publish(|_| vec![1]);
    right_producer.try_publish(|_| vec![2]);
    left_producer.try_publish(|_| vec![3]);
    left_producer.flush(Duration::from_secs(1)).unwrap();
    right_producer.flush(Duration::from_secs(1)).unwrap();
    assert_eq!(
        left.records
            .lock()
            .unwrap()
            .iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(right.records.lock().unwrap()[0].sequence, 1);
}

#[test]
fn concurrent_builders_enqueue_without_waiting_and_preserve_order() {
    let sink = Arc::new(CapturingSink::default());
    let producer =
        ByteBoundedObservationProducer::new(identity("concurrent"), 4096, 8, sink.clone()).unwrap();
    let first_entered = Arc::new(AtomicBool::new(false));
    let first = {
        let producer = producer.clone();
        let first_entered = Arc::clone(&first_entered);
        std::thread::spawn(move || {
            producer.try_publish(|_| {
                first_entered.store(true, Ordering::Release);
                std::thread::sleep(Duration::from_millis(200));
                vec![1]
            })
        })
    };
    wait_until(|| first_entered.load(Ordering::Acquire));
    let started = Instant::now();
    let second = {
        let producer = producer.clone();
        std::thread::spawn(move || producer.try_publish(|_| vec![2]))
    };
    let second = second.join().unwrap();
    assert!(started.elapsed() < Duration::from_millis(100));
    assert!(matches!(
        second,
        ObservationPublishOutcome::Enqueued { sequence: 2 }
    ));
    assert!(matches!(
        first.join().unwrap(),
        ObservationPublishOutcome::Enqueued { sequence: 1 }
    ));
    producer.flush(Duration::from_secs(1)).unwrap();

    let records = sink.records.lock().unwrap();
    assert_eq!(
        records
            .iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(sink.gaps.lock().unwrap().is_empty());
    assert_eq!(*sink.deliveries.lock().unwrap(), [(1, false), (2, false)]);
    assert_eq!(producer.stats().dropped, 0);
}

#[test]
fn repeatedly_restored_loss_ranges_remain_in_sequence_order() {
    let sink = Arc::new(CapturingSink::default());
    let producer =
        ByteBoundedObservationProducer::new(identity("ordered-gap"), 4096, 8, sink.clone())
            .unwrap();
    for watermark in [
        ObservationLossWatermark {
            first_sequence: 2,
            last_sequence: 2,
            reason: ObservationLossReason::QueueBytesExceeded,
        },
        ObservationLossWatermark {
            first_sequence: 1,
            last_sequence: 1,
            reason: ObservationLossReason::SinkPanicked,
        },
        ObservationLossWatermark {
            first_sequence: 3,
            last_sequence: 3,
            reason: ObservationLossReason::EventTooLarge,
        },
    ] {
        record_loss(&producer.shared, watermark);
    }
    producer.shared.next_sequence.store(4, Ordering::Release);
    producer.try_publish(|_| Vec::new());
    producer.flush(Duration::from_secs(1)).unwrap();

    assert_eq!(
        *sink.deliveries.lock().unwrap(),
        [(1, true), (2, true), (3, true), (4, false)]
    );
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while !condition() {
        assert!(Instant::now() < deadline, "condition timed out");
        std::thread::yield_now();
    }
}
