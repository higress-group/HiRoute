//! One bounded maintenance loop per production store. Request producers never
//! wait for this thread; its only inputs are already committed observations.
use crate::LocalObservationStore;
use std::sync::{Arc, Weak, mpsc};
use std::time::{Duration, Instant};

const MAINTENANCE_INTERVAL: Duration = Duration::from_millis(250);
const BLOB_GC_IDLE_INTERVAL: Duration = Duration::from_secs(5);
const BLOB_GC_BATCH: usize = 32;

pub struct ObservationMaintenance {
    stop: mpsc::Sender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
}

/// Optional product-owned work attached to the one maintenance lifecycle.  The hook receives no
/// observation lock or deletion authority; implementations must perform their own bounded work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationMaintenanceHookError;

pub trait ObservationMaintenanceHook: Send + Sync {
    fn cycle(&self, now_ms: i64) -> Result<(), ObservationMaintenanceHookError>;
}

impl ObservationMaintenance {
    pub fn start(store: &Arc<LocalObservationStore>) -> std::io::Result<Self> {
        Self::start_with_hook(store, None)
    }

    pub fn start_with_hook(
        store: &Arc<LocalObservationStore>,
        hook: Option<Arc<dyn ObservationMaintenanceHook>>,
    ) -> std::io::Result<Self> {
        let lease = LocalWorkerGuard::acquire(&store.maintenance_running).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "observation maintenance already running",
            )
        })?;
        let (stop, receiver) = mpsc::channel();
        let weak = Arc::downgrade(store);
        let result = std::thread::Builder::new()
            .name("hiroute-observation-maintenance".into())
            .spawn(move || {
                let _lease = lease;
                run(weak, receiver, hook)
            });
        let worker = result.inspect_err(|_| {
            store
                .maintenance_errors
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        })?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for ObservationMaintenance {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        // Finish the current bounded cycle before releasing the owner. A hook may
        // hold the product stores, and an immediate reopen must not race that lease.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(
    store: Weak<LocalObservationStore>,
    stop: mpsc::Receiver<()>,
    hook: Option<Arc<dyn ObservationMaintenanceHook>>,
) {
    let mut index = store
        .upgrade()
        .and_then(|store| crate::text_index::TextIndexBuilder::new(&store).ok());
    let mut blob_gc = BlobGcSchedule {
        next_due: Instant::now(),
    };
    while matches!(
        stop.recv_timeout(MAINTENANCE_INTERVAL),
        Err(mpsc::RecvTimeoutError::Timeout)
    ) {
        let Some(store) = store.upgrade() else {
            break;
        };
        let backfill_failed = store.backfill_inline_payloads().is_err();
        let settlement_failed = store.settle_pending_valuations(16).is_err();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|time| i64::try_from(time.as_millis()).ok());
        let retention_failed = now_ms
            .map(|now| {
                store.managed_text_expire_pending(now, 16).is_err()
                    | store.managed_text_gc(now, 32).is_err()
                    | store.managed_text_progress_gc(now, 32).is_err()
                    | store.expire_request_details(now, 16).is_err()
            })
            .unwrap_or(true);
        // Only physical blob collection backs off. Expiry, indexing, settlement and hooks
        // retain their existing cadence; explicit user deletion still collects immediately.
        let gc_failed = blob_gc
            .run_if_due(Instant::now, || store.collect_garbage_batch(BLOB_GC_BATCH))
            .is_err();
        let hook_failed = match (hook.as_ref(), now_ms) {
            (Some(hook), Some(now)) => hook.cycle(now).is_err(),
            (Some(_), None) => true,
            (None, _) => false,
        };
        let index_failed = match index.as_mut() {
            Some(index) => index.cycle(&store).is_err(),
            None => {
                index = crate::text_index::TextIndexBuilder::new(&store).ok();
                index.is_none()
            }
        };
        if backfill_failed
            || settlement_failed
            || retention_failed
            || gc_failed
            || hook_failed
            || index_failed
        {
            store
                .maintenance_errors
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// A full batch may leave a backlog. Idle and failed passes wait longer, measured
/// from completion so a slow query cannot immediately schedule another expensive pass.
struct BlobGcSchedule {
    next_due: Instant,
}

impl BlobGcSchedule {
    fn run_if_due(
        &mut self,
        clock: impl Fn() -> Instant,
        collect: impl FnOnce() -> Result<u64, hiroute_domain::ObservationQueryError>,
    ) -> Result<(), hiroute_domain::ObservationQueryError> {
        if clock() < self.next_due {
            return Ok(());
        }
        let result = collect();
        let interval = if matches!(result, Ok(count) if count == BLOB_GC_BATCH as u64) {
            MAINTENANCE_INTERVAL
        } else {
            BLOB_GC_IDLE_INTERVAL
        };
        self.next_due = clock() + interval;
        result.map(|_| ())
    }
}

/// Process-local ownership only: not a distributed lease or durable authority.
pub(crate) struct LocalWorkerGuard(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl LocalWorkerGuard {
    pub(crate) fn acquire(flag: &std::sync::Arc<std::sync::atomic::AtomicBool>) -> Option<Self> {
        flag.compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .ok()
        .map(|_| Self(flag.clone()))
    }
}
impl Drop for LocalWorkerGuard {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}
impl LocalObservationStore {
    pub fn maintenance_status(&self) -> (bool, u64) {
        (
            self.maintenance_running
                .load(std::sync::atomic::Ordering::Acquire),
            self.maintenance_errors
                .load(std::sync::atomic::Ordering::Relaxed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_domain::ObservationQueryError;
    use std::cell::Cell;

    #[test]
    fn blob_gc_idle_partial_and_failed_passes_wait_five_seconds() {
        for outcome in [
            Ok(0),
            Ok(1),
            Ok(31),
            Err(ObservationQueryError::Unavailable),
        ] {
            let start = Instant::now();
            let mut gc = BlobGcSchedule { next_due: start };
            let failed = outcome.is_err();
            assert_eq!(gc.run_if_due(|| start, || outcome).is_err(), failed);
            // Other maintenance may tick twenty times without repeating GC.
            for tick in 1..20 {
                gc.run_if_due(
                    || start + MAINTENANCE_INTERVAL * tick,
                    || panic!("GC ran before its idle/retry deadline"),
                )
                .unwrap();
            }
            let called = Cell::new(false);
            gc.run_if_due(
                || start + BLOB_GC_IDLE_INTERVAL,
                || {
                    called.set(true);
                    Ok(0)
                },
            )
            .unwrap();
            assert!(called.get());
        }
    }

    #[test]
    fn blob_gc_full_batches_continue_and_deadlines_start_at_completion() {
        let start = Instant::now();
        let clock = Cell::new(start);
        let mut gc = BlobGcSchedule { next_due: start };
        gc.run_if_due(
            || clock.get(),
            || {
                clock.set(start + Duration::from_secs(2));
                Ok(BLOB_GC_BATCH as u64)
            },
        )
        .unwrap();
        clock.set(start + Duration::from_secs(2) + Duration::from_millis(249));
        gc.run_if_due(|| clock.get(), || panic!("early backlog retry"))
            .unwrap();
        clock.set(clock.get() + Duration::from_millis(1));
        let called = Cell::new(false);
        gc.run_if_due(
            || clock.get(),
            || {
                called.set(true);
                clock.set(clock.get() + Duration::from_secs(6));
                Err(ObservationQueryError::Unavailable)
            },
        )
        .unwrap_err();
        assert!(called.get());
        clock.set(clock.get() + BLOB_GC_IDLE_INTERVAL - Duration::from_millis(1));
        gc.run_if_due(|| clock.get(), || panic!("retry measured from start"))
            .unwrap();
        clock.set(clock.get() + Duration::from_millis(1));
        gc.run_if_due(|| clock.get(), || Ok(0)).unwrap();
    }

    struct BlockedCycle {
        entered: mpsc::Sender<()>,
        release: std::sync::Mutex<mpsc::Receiver<()>>,
    }

    impl ObservationMaintenanceHook for BlockedCycle {
        fn cycle(&self, _: i64) -> Result<(), ObservationMaintenanceHookError> {
            self.entered.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            Ok(())
        }
    }

    #[test]
    fn dropping_maintenance_finishes_its_current_cycle_and_releases_the_lease() {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(
            LocalObservationStore::open(root.path(), crate::DigestAuthority::new([1; 32])).unwrap(),
        );
        let (entered, did_enter) = mpsc::channel();
        let (release, wait_for_release) = mpsc::channel();
        let worker = ObservationMaintenance::start_with_hook(
            &store,
            Some(Arc::new(BlockedCycle {
                entered,
                release: std::sync::Mutex::new(wait_for_release),
            })),
        )
        .unwrap();
        did_enter.recv_timeout(Duration::from_secs(5)).unwrap();
        let (dropped, did_drop) = mpsc::channel();
        let owner = std::thread::spawn(move || {
            drop(worker);
            dropped.send(()).unwrap();
        });
        assert!(matches!(
            did_drop.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        release.send(()).unwrap();
        did_drop.recv_timeout(Duration::from_secs(5)).unwrap();
        owner.join().unwrap();
        assert!(!store.maintenance_status().0);
        drop(ObservationMaintenance::start(&store).unwrap());
        assert!(!store.maintenance_status().0);
    }
}
