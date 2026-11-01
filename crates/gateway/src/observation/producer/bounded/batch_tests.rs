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
