use super::*;
use crate::replay::{ReplayConfig, ReplayManager};
use hiroute_gateway_core::runtime::body::BudgetTree;
use std::time::Duration;

struct TestRoot(std::path::PathBuf);

impl TestRoot {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Self(
            std::fs::canonicalize(std::env::temp_dir())
                .unwrap()
                .join(format!(
                    "hiroute-overflow-test-{}-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos(),
                    NEXT.fetch_add(1, Ordering::Relaxed),
                )),
        )
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn factory(root: &std::path::Path) -> OverflowFactory {
    let root = root.join("overflow");
    let manager = ReplayManager::open(ReplayConfig {
        root,
        memory_threshold_bytes: 1024,
        record_bytes: 31,
        orphan_ttl: Duration::from_secs(60),
    })
    .unwrap();
    let tree = BudgetTree::new(1024 * 1024, 1024 * 1024).unwrap();
    let request = manager
        .begin_request(tree.stream(1024 * 1024).unwrap())
        .unwrap();
    // The factory must survive without retaining the original business request.
    request.overflow_factory()
}

#[test]
fn consumed_segments_release_without_waiting_for_the_long_request() {
    let root = TestRoot::new();
    let factory = factory(root.path());
    let quota = OverflowQuota::new(4096);
    let writer = factory.writer(Arc::clone(&quota), 128);
    let first = writer.append(1, &[1; 64]).unwrap();
    let second = writer.append(2, &[2; 64]).unwrap();
    let third = writer.append(3, &[3; 64]).unwrap();
    assert_eq!(quota.snapshot().live_segments, 3);
    let pin = first.clone();
    drop(first);
    assert_eq!(quota.snapshot().live_segments, 3);
    assert_eq!(pin.read().unwrap(), vec![1; 64]);
    drop(pin);
    assert_eq!(quota.snapshot().live_segments, 2);
    assert_eq!(second.read().unwrap(), vec![2; 64]);
    drop(second);
    assert_eq!(quota.snapshot().live_segments, 1);
    assert_eq!(third.read().unwrap(), vec![3; 64]);
    writer.seal();
    drop(third);
    let stats = quota.snapshot();
    assert_eq!(stats.live_segments, 0);
    assert_eq!(stats.reserved_disk_bytes, 0);
    assert_eq!(stats.retained_file_bytes, 0);
    assert_eq!(stats.cleanup_failures, 0);
    assert_eq!(stats.created_segments, 3);
    assert!(stats.written_bytes > 3 * 64);
    assert_eq!(stats.read_bytes, stats.written_bytes);
}

#[test]
fn quota_exhaustion_does_not_acknowledge_or_prevent_later_recovery() {
    let root = TestRoot::new();
    let quota = OverflowQuota::new(192);
    let writer = factory(root.path()).writer(Arc::clone(&quota), 128);
    let first = writer.append(41, &[4; 64]).unwrap();
    assert!(matches!(
        writer.append(42, &[5; 64]),
        Err(ReplayError::StructureLimit)
    ));
    assert_eq!(quota.snapshot().reserved_disk_bytes, 120);
    drop(first);
    assert_eq!(quota.snapshot().reserved_disk_bytes, 0);
    let recovered = writer.append(43, &[6; 64]).unwrap();
    assert_eq!(recovered.read().unwrap(), vec![6; 64]);
    writer.seal();
    drop(recovered);
    assert_eq!(quota.snapshot().reserved_disk_bytes, 0);
}

#[test]
fn corrupt_and_truncated_ranges_fail_closed_through_the_held_handle() {
    let root = TestRoot::new();
    let quota = OverflowQuota::new(4096);
    let writer = factory(root.path()).writer(Arc::clone(&quota), 1024);
    let corrupt = writer.append(1, b"unchanged accepted content").unwrap();
    corrupt
        .segment
        .write_at(corrupt.offset + HEADER_BYTES as u64, b"!")
        .unwrap();
    assert!(matches!(corrupt.read(), Err(ReplayError::Integrity)));
    let truncated = writer.append(2, b"another accepted content").unwrap();
    {
        let handle = lock(&truncated.segment.file);
        handle
            .as_ref()
            .unwrap()
            .set_len(truncated.offset + HEADER_BYTES as u64 + 2)
            .unwrap();
    }
    assert!(
        matches!(truncated.read(), Err(ReplayError::Io(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof)
    );
    writer.seal();
    drop((corrupt, truncated));
    assert_eq!(quota.snapshot().reserved_disk_bytes, 0);
}

#[test]
fn a_legal_record_larger_than_the_segment_target_is_read_exactly() {
    let root = TestRoot::new();
    let quota = OverflowQuota::new(16 * 1024 * 1024);
    let writer = factory(root.path()).writer(Arc::clone(&quota), 4 * 1024 * 1024);
    let bytes = vec![b'x'; 5 * 1024 * 1024];
    let record = writer.append(99, &bytes).unwrap();
    assert_eq!(record.len(), bytes.len());
    assert_eq!(record.read().unwrap(), bytes);
    writer.seal();
    drop(record);
    assert_eq!(quota.snapshot().retained_file_bytes, 0);
}
