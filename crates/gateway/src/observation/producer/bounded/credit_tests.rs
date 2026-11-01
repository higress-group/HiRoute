use super::*;

#[test]
fn encoded_records_preserve_the_allowance_for_later_foreground_work() {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let sink = Arc::new(SlowOnceSink {
        gate: gate.clone(),
        slow: AtomicBool::new(true),
        records: Mutex::new(Vec::new()),
        gaps: Mutex::new(Vec::new()),
        deliveries: Mutex::new(Vec::new()),
    });
    let producer = ByteBoundedObservationProducer::new(
        identity("future-work-credit"),
        512 * 1024,
        1024,
        sink.clone(),
    )
    .unwrap();
    let encoder = producer.clone();
    let worker = std::thread::spawn(move || {
        for _ in 0..160 {
            assert!(matches!(
                encoder.publish_timed(
                    None,
                    Some(Instant::now() + Duration::from_secs(5)),
                    5_000,
                    |_| vec![b'x'; 4096],
                ),
                ObservationPublishOutcome::Enqueued { .. }
            ));
        }
    });
    producer.wait_until_credit_blocked();
    // This source fits the existing 13/16 work allowance. Earlier encoded
    // records must not borrow that space and force the next valid source down
    // the nonblocking, lossy fallback. The sink stays stopped until admission.
    let later_source = producer.reserve_work(400 * 1024);
    let admitted = later_source.is_some();
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    worker.join().unwrap();
    drop(later_source);
    producer.flush(Duration::from_secs(5)).unwrap();
    assert!(
        admitted,
        "encoded records consumed the future-work allowance"
    );
    assert_eq!(sink.records.lock().unwrap().len(), 160);
    assert!(sink.gaps.lock().unwrap().is_empty());
    assert_eq!(producer.stats().queued_bytes, 0);
}
