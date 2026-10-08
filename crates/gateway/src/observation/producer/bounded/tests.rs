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
