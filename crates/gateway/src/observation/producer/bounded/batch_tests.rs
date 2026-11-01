use super::*;

struct BatchSink {
    gate: Arc<(Mutex<bool>, Condvar)>,
    first: AtomicBool,
    short: bool,
    reject: bool,
    largest: AtomicUsize,
    captured: CapturingSink,
}

impl ObservationRecordSink for BatchSink {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        if self.first.swap(false, Ordering::AcqRel) {
            let mut open = self.gate.0.lock().unwrap();
            while !*open {
                open = self.gate.1.wait(open).unwrap();
            }
        }
        if self.reject && record.stamp.sequence == 17 && !record.is_gap_heartbeat() {
            return Err(receiver_unavailable_nack(record, true));
        }
        self.captured.deliver(record)
    }

    fn deliver_batch(
        &self,
        records: &[ObservationRecord],
    ) -> Vec<Result<ObservationAck, ObservationNack>> {
        assert!(records.len() <= 32);
        self.largest.fetch_max(records.len(), Ordering::Relaxed);
        let mut results = Vec::new();
        for record in records
            .iter()
            .take(if self.short { 1 } else { records.len() })
        {
            let result = self.deliver(record);
            let rejected = result.is_err();
            results.push(result);
            if rejected {
                break;
            }
        }
        results
    }
}

struct OpenOnDrop(Arc<(Mutex<bool>, Condvar)>);
impl Drop for OpenOnDrop {
    fn drop(&mut self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
    }
}

#[test]
fn valid_short_batch_prefix_keeps_every_unprocessed_record_and_credit() {
    exercise(true, false);
}

#[test]
fn rejected_batch_prefix_sends_its_gap_before_any_later_record() {
    exercise(false, true);
}

fn exercise(short: bool, reject: bool) {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let guard = OpenOnDrop(gate.clone());
    let sink = Arc::new(BatchSink {
        gate,
        first: AtomicBool::new(true),
        short,
        reject,
        largest: AtomicUsize::new(0),
        captured: CapturingSink::default(),
    });
    let producer = ByteBoundedObservationProducer::new(
        identity("batch-prefix"),
        1024 * 1024,
        256,
        sink.clone(),
    )
    .unwrap();
    for _ in 0..200 {
        assert!(matches!(
            producer.try_publish(|_| vec![1; 32]),
            ObservationPublishOutcome::Enqueued { .. }
        ));
    }
    drop(guard);
    producer.flush(Duration::from_secs(5)).unwrap();
    assert!(sink.largest.load(Ordering::Relaxed) > 1);
    let expected: Vec<_> = (1..=200)
        .filter(|sequence| !reject || *sequence != 17)
        .collect();
    assert_eq!(
        sink.captured
            .records
            .lock()
            .unwrap()
            .iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>(),
        expected
    );
    let stats = producer.stats();
    assert_eq!(stats.queued_bytes, 0);
    assert_eq!(stats.delivered, expected.len());
    assert_eq!(stats.sink_nacks, usize::from(reject));
    let deliveries = sink.captured.deliveries.lock().unwrap();
    if reject {
        let gap = deliveries
            .iter()
            .position(|entry| *entry == (17, true))
            .unwrap();
        let next = deliveries
            .iter()
            .position(|entry| *entry == (18, false))
            .unwrap();
        assert!(gap < next);
    } else {
        assert!(sink.captured.gaps.lock().unwrap().is_empty());
    }
}

#[test]
fn spilled_short_prefix_keeps_unprocessed_ranges_until_their_original_ack() {
    exercise_spilled(true, false);
}

#[test]
fn spilled_rejected_prefix_delivers_its_gap_before_the_next_original_range() {
    exercise_spilled(false, true);
}

fn exercise_spilled(short: bool, reject: bool) {
    use crate::replay::{ReplayConfig, ReplayManager};
    use hiroute_gateway_core::runtime::body::BudgetTree;
    let root = std::fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!(
            "hiroute-spilled-prefix-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
    let manager = ReplayManager::open(ReplayConfig {
        root: root.clone(),
        memory_threshold_bytes: 1024,
        record_bytes: 31,
        orphan_ttl: Duration::from_secs(60),
    })
    .unwrap();
    let budget = BudgetTree::new(1024 * 1024, 1024 * 1024).unwrap();
    let replay = manager
        .begin_request(budget.stream(1024 * 1024).unwrap())
        .unwrap();
    let spool = Arc::new(ContentSpool::default());
    spool.bind(replay.overflow_factory());
    drop(replay);
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let guard = OpenOnDrop(gate.clone());
    let sink = Arc::new(BatchSink {
        gate,
        first: AtomicBool::new(true),
        short,
        reject,
        largest: AtomicUsize::new(0),
        captured: CapturingSink::default(),
    });
    let mut limits = ContentQueueLimits::for_capacity(1024 * 1024);
    limits.soft = 0;
    let producer =
        ByteBoundedObservationProducer::new_content(identity("spill-prefix"), limits, sink.clone())
            .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(async {
        for index in 0..200 {
            producer
                .publish_content(None, 1024, true, false, &spool, |_| {
                    Ok(format!("original range {index:04}").into_bytes())
                })
                .await
                .unwrap();
        }
    });
    drop(guard);
    producer.flush(Duration::from_secs(5)).unwrap();
    assert!(sink.largest.load(Ordering::Relaxed) > 1);
    let expected: Vec<_> = (1..=200)
        .filter(|sequence| !reject || *sequence != 17)
        .collect();
    assert_eq!(
        sink.captured
            .records
            .lock()
            .unwrap()
            .iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>(),
        expected
    );
    let snapshot = producer.content_resource_snapshot().unwrap();
    assert!(snapshot.overflow_created_segments > 0);
    assert_eq!(snapshot.overflow_reserved_bytes, 0);
    assert_eq!(snapshot.overflow_live_segments, 0);
    assert_eq!(snapshot.queued_bytes, 0);
    assert_eq!(snapshot.drain_bytes, 0);
    assert_eq!(snapshot.overflow_cleanup_failures, 0);
    assert_eq!(producer.stats().sink_nacks, usize::from(reject));
    let deliveries = sink.captured.deliveries.lock().unwrap();
    if reject {
        let gap = deliveries
            .iter()
            .position(|entry| *entry == (17, true))
            .unwrap();
        let next = deliveries
            .iter()
            .position(|entry| *entry == (18, false))
            .unwrap();
        assert!(gap < next);
    } else {
        assert!(sink.captured.gaps.lock().unwrap().is_empty());
    }
    drop(deliveries);
    drop(producer);
    drop(spool);
    drop(manager);
    std::fs::remove_dir_all(root).unwrap();
}
