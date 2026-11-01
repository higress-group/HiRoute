//! Resource pressure traverses the real ordered producer/sink, not a mock budget.
use super::*;
fn identity(name: &str) -> ObservationProducerIdentity {
    ObservationProducerIdentity {
        channel: Arc::from(name),
        component: Arc::from("content-resource-tests"),
        revision: Arc::from("test/v1"),
        producer_id: Arc::from(name),
        producer_epoch: Arc::from("epoch"),
        stream_id: Arc::from(name),
    }
}
use crate::replay::{ReplayConfig, ReplayManager};
use hiroute_gateway_core::runtime::body::BudgetTree;
use std::time::Duration;

#[derive(Default)]
struct BytesSink {
    records: Mutex<Vec<(u64, Vec<u8>)>>,
    gate: (Mutex<(bool, bool)>, Condvar),
}
impl BytesSink {
    fn release(&self) {
        lock(&self.gate.0).1 = true;
        self.gate.1.notify_all();
    }
    fn wait(&self) {
        let state = lock(&self.gate.0);
        let (_, timeout) = self
            .gate
            .1
            .wait_timeout_while(state, Duration::from_secs(5), |state| !state.0)
            .unwrap();
        assert!(
            !timeout.timed_out(),
            "sink never received the ordered prefix"
        );
    }
}
impl ObservationRecordSink for BytesSink {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        let mut state = lock(&self.gate.0);
        state.0 = true;
        self.gate.1.notify_all();
        while !state.1 {
            state = self.gate.1.wait(state).unwrap();
        }
        drop(state);
        assert!(
            !record.is_gap_heartbeat(),
            "healthy pressure must not declare a gap"
        );
        lock(&self.records).push((record.stamp().sequence, record.payload.to_vec()));
        Ok(super::super::feedback::accounted_acknowledgement(record))
    }
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
}

#[test]
fn all_source_credit_can_make_progress_without_a_queue_record_or_disk() {
    let sink = Arc::new(BytesSink::default());
    let limits = ContentQueueLimits::for_capacity(64 * 1024);
    let producer =
        ByteBoundedObservationProducer::new_content(identity("progress"), limits, sink.clone())
            .unwrap();
    let source = producer
        .reserve_work(limits.bytes - limits.control)
        .unwrap();
    let spool = Arc::new(ContentSpool::default());
    let first = vec![b'a'; 20 * 1024];
    runtime()
        .block_on(
            producer.publish_content(None, first.len() + 512, true, false, &spool, |_| {
                Ok(first.clone())
            }),
        )
        .unwrap();
    sink.wait();
    assert!(
        producer
            .shared
            .content
            .as_ref()
            .unwrap()
            .progress_busy
            .load(Ordering::Acquire)
    );
    assert_eq!(producer.stats().dropped, 0);
    // A second task waiting on scratch must suspend the runtime, not block it.
    let release = sink.clone();
    runtime().block_on(async {
        let next =
            producer.publish_content(None, 1024, true, false, &spool, |_| Ok(b"next".to_vec()));
        tokio::pin!(next);
        tokio::select! {
            _ = &mut next => panic!("dedicated workspace was released before ACK"),
            _ = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
        release.release();
        tokio::time::timeout(Duration::from_secs(5), next)
            .await
            .unwrap()
            .unwrap();
    });
    drop(source);
    producer.flush(Duration::from_secs(5)).unwrap();
    assert_eq!(
        *lock(&sink.records),
        vec![(1, first), (2, b"next".to_vec())]
    );
    assert_eq!(producer.stats().queued_bytes, 0);
    assert_eq!(producer.stats().dropped, 0);
}

#[test]
fn spilled_large_record_drains_with_full_source_credit_and_reclaims_consumed_tail() {
    let root = std::fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!(
            "hiroute-encoded-pressure-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
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
    let sink = Arc::new(BytesSink::default());
    let mut limits = ContentQueueLimits::for_capacity(128 * 1024);
    limits.soft = 0;
    let producer =
        ByteBoundedObservationProducer::new_content(identity("spill"), limits, sink.clone())
            .unwrap();
    let source = producer.reserve_work(64 * 1024).unwrap();
    // Exceeds both batch byte quantum and segment size; neither is a legal
    // request/content limit. The original exact payload must still reach ACK.
    let bytes = vec![0xa5; 5 * 1024 * 1024];
    runtime()
        .block_on(
            producer.publish_content(None, bytes.len() + 512, true, false, &spool, |_| {
                Ok(bytes.clone())
            }),
        )
        .unwrap();
    sink.wait();
    let resources = producer.shared.content.as_ref().unwrap();
    assert!(resources.disk_snapshot().retained_file_bytes > bytes.len() as u64);
    sink.release();
    producer.flush(Duration::from_secs(5)).unwrap();
    assert_eq!(*lock(&sink.records), vec![(1, bytes)]);
    assert_eq!(resources.disk_snapshot().reserved_disk_bytes, 0);
    assert_eq!(resources.disk_snapshot().live_segments, 0);
    assert_eq!(resources.disk_snapshot().cleanup_failures, 0);
    assert_eq!(resources.drain_bytes.load(Ordering::Acquire), 0);
    assert_eq!(producer.stats().dropped, 0);
    drop(source);
    drop(producer);
    drop(spool);
    drop(manager);
    std::fs::remove_dir_all(root).unwrap();
}
