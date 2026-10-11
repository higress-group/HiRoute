//! The background role writer: one thread per process role owns `writer.lock` and appends
//! encoded records to `current.jsonl`, rotating bounded files and reporting health through
//! counters instead of recursively logging its own failures.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::files::{
    CURRENT_LOG_FILE, FileSafetyError, PREVIOUS_LOG_FILES, PrivateDir, VerifiedFile,
    WRITER_LOCK_FILE, previous_log_file,
};
use crate::queue::QueueReceiver;
use crate::record::DiagnosticRecordV1;

/// Maximum size of the current file before rotation.
pub const CURRENT_MAX_BYTES: u64 = 4 * 1024 * 1024;
/// Flush at most this often.
pub const FLUSH_INTERVAL: Duration = Duration::from_millis(1000);
/// Flush after this many appended bytes.
pub const FLUSH_BYTES: u64 = 64 * 1024;
/// Log files older than this (by last verified event) may be deleted.
pub const RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1000;
/// A failed writer retries the safe target at most this often.
pub const REOPEN_INTERVAL: Duration = Duration::from_secs(5);
/// Normal shutdown gives the writer at most this much time to flush.
pub const SHUTDOWN_FLUSH_BUDGET: Duration = Duration::from_millis(1000);
/// How much of a rotated file's tail is inspected to date its last record.
const RETENTION_TAIL_BYTES: u64 = 64 * 1024;

#[derive(Debug, Default)]
pub struct WriterCounters {
    rotated: AtomicU64,
    flushes: AtomicU64,
    bytes_written: AtomicU64,
    write_failures: AtomicU64,
    lost_at_shutdown: AtomicU64,
}

impl WriterCounters {
    pub fn rotated(&self) -> u64 {
        self.rotated.load(Ordering::Relaxed)
    }
    pub fn flushes(&self) -> u64 {
        self.flushes.load(Ordering::Relaxed)
    }
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written.load(Ordering::Relaxed)
    }
    pub fn write_failures(&self) -> u64 {
        self.write_failures.load(Ordering::Relaxed)
    }
    /// Records the bounded drain could not write when the process exited.
    pub fn lost_at_shutdown(&self) -> u64 {
        self.lost_at_shutdown.load(Ordering::Relaxed)
    }
}

/// The writer's own health: `None` while it can write, the last controlled reason once it
/// cannot. The writer never logs while it is failing; the maintenance thread turns a
/// transition into one bounded `diagnostics_degraded` event.
#[derive(Debug, Clone)]
pub struct WriterHealth {
    inner: Arc<Mutex<Option<crate::error::SubsystemReason>>>,
}

impl WriterHealth {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }

    pub fn snapshot(&self) -> Option<crate::error::SubsystemReason> {
        *self.inner.lock().expect("writer health lock")
    }

    pub(crate) fn set(&self, reason: Option<crate::error::SubsystemReason>) {
        *self.inner.lock().expect("writer health lock") = reason;
    }
}

impl Default for WriterHealth {
    fn default() -> Self {
        Self::new()
    }
}

/// Signals completion so shutdown can wait with a bound instead of joining forever.
#[derive(Debug, Default)]
pub struct WriterCompletion {
    finished: Mutex<bool>,
    signal: Condvar,
}

impl WriterCompletion {
    pub(crate) fn mark_finished(&self) {
        *self.finished.lock().expect("completion lock") = true;
        self.signal.notify_all();
    }

    /// Wait up to `timeout` for the writer thread to finish.
    pub fn wait(&self, timeout: Duration) -> bool {
        let guard = self.finished.lock().expect("completion lock");
        if *guard {
            return true;
        }
        let (guard, _) = self
            .signal
            .wait_timeout(guard, timeout)
            .expect("completion lock");
        *guard
    }

    pub fn is_finished(&self) -> bool {
        *self.finished.lock().expect("completion lock")
    }
}

/// A cloneable handle to one role's writer. `owns_role` is the sticky outcome of the role
/// lock claim: only the process that won it may write that role's files, and the lock is
/// held by the writer thread for as long as that thread can still write.
#[derive(Debug, Clone)]
pub struct WriterHandle {
    pub health: WriterHealth,
    pub counters: Arc<WriterCounters>,
    pub completion: Arc<WriterCompletion>,
    owns_role: Arc<AtomicBool>,
}

impl WriterHandle {
    pub fn is_healthy(&self) -> bool {
        self.health.snapshot().is_none()
    }

    /// Whether this process won `writer.lock` for the role. A runtime that lost the claim
    /// reports its own memory state but never writes the owner's logs.
    pub fn owns_role(&self) -> bool {
        self.owns_role.load(Ordering::SeqCst)
    }
}

/// What the writer needs to write its own final statistics record on the exit path, while
/// it still holds the role lock: the loss of a bounded exit belongs in the log the process
/// owns, not only in memory.
#[derive(Clone)]
pub struct WriterRecordContext {
    component: crate::record::Component,
    boot_id: crate::identity::BootId,
    parent_session_id: Option<crate::identity::SessionId>,
    started: Instant,
    shared: Arc<crate::shared::Shared>,
}

impl WriterRecordContext {
    pub(crate) fn new(
        component: crate::record::Component,
        boot_id: crate::identity::BootId,
        parent_session_id: Option<crate::identity::SessionId>,
        started: Instant,
        shared: Arc<crate::shared::Shared>,
    ) -> Self {
        Self {
            component,
            boot_id,
            parent_session_id,
            started,
            shared,
        }
    }
}

/// Spawn the writer thread for one role directory. The role lock is claimed by the caller
/// before the thread starts, so ownership is known before any file is written.
pub fn spawn_writer(
    dir: Arc<PrivateDir>,
    receiver: QueueReceiver,
    counters: Arc<WriterCounters>,
    health: WriterHealth,
    record: Option<WriterRecordContext>,
) -> WriterHandle {
    let completion = Arc::new(WriterCompletion::default());
    let owns_role = Arc::new(AtomicBool::new(false));
    let mut writer = Writer::new(dir, receiver, counters.clone(), health.clone(), record);
    if let Err(error) = writer.claim_writer_lock() {
        let reason = if matches!(error, FileSafetyError::Locked) {
            crate::error::SubsystemReason::WriterOwned
        } else {
            crate::error::SubsystemReason::WriterOpenFailed
        };
        health.set(Some(reason));
        // No consumer exists; pushes fail fast once the bounded queue is full.
        completion.mark_finished();
        return WriterHandle {
            health,
            counters,
            completion,
            owns_role,
        };
    }
    owns_role.store(true, Ordering::SeqCst);
    let thread = std::thread::Builder::new()
        .name("hiroute-diagnostics-writer".to_string())
        .spawn({
            let completion = completion.clone();
            move || {
                writer.run();
                // A concurrent fork may briefly inherit the close-on-exec lock
                // descriptor. Explicitly unlocking the shared open-file description
                // prevents that child copy from extending ownership past completion.
                writer.release_writer_lock();
                // Completion means every remaining role-owned resource is released,
                // not merely that the run loop returned. A restart may claim
                // writer.lock immediately after observing this flag.
                drop(writer);
                completion.mark_finished();
            }
        })
        .ok();
    if thread.is_none() {
        owns_role.store(false, Ordering::SeqCst);
        health.set(Some(crate::error::SubsystemReason::WriterOpenFailed));
        completion.mark_finished();
    }
    WriterHandle {
        health,
        counters,
        completion,
        owns_role,
    }
}

struct Writer {
    dir: Arc<PrivateDir>,
    receiver: QueueReceiver,
    counters: Arc<WriterCounters>,
    health: WriterHealth,
    record: Option<WriterRecordContext>,
    writer_lock: Option<VerifiedFile>,
    current: Option<VerifiedFile>,
    current_size: u64,
    bytes_since_flush: u64,
    last_flush: Instant,
    last_reopen_attempt: Option<Instant>,
}

impl Writer {
    fn new(
        dir: Arc<PrivateDir>,
        receiver: QueueReceiver,
        counters: Arc<WriterCounters>,
        health: WriterHealth,
        record: Option<WriterRecordContext>,
    ) -> Self {
        Self {
            dir,
            receiver,
            counters,
            health,
            record,
            writer_lock: None,
            current: None,
            current_size: 0,
            bytes_since_flush: 0,
            last_flush: Instant::now(),
            last_reopen_attempt: None,
        }
    }

    /// The role lock is claimed before this thread starts; the writer assumes it holds it.
    fn run(&mut self) {
        if let Err(error) = self.open_initial_current() {
            self.fail_soft(error);
        }
        loop {
            if self.receiver.is_closed() {
                self.shutdown();
                return;
            }
            if self.current.is_none() {
                self.try_reopen();
                continue;
            }
            match self.receiver.pop_timeout(Duration::from_millis(50)) {
                Some(bytes) => {
                    self.append(&bytes);
                }
                None => {
                    self.flush_if_due(true);
                }
            }
        }
    }

    fn claim_writer_lock(&mut self) -> Result<(), FileSafetyError> {
        let lock = self.dir.open_lock(WRITER_LOCK_FILE)?;
        lock.try_lock_exclusive()?;
        self.writer_lock = Some(lock);
        Ok(())
    }

    fn release_writer_lock(&mut self) {
        if let Some(lock) = self.writer_lock.take()
            && lock.unlock().is_err()
        {
            self.health
                .set(Some(crate::error::SubsystemReason::WriterWriteFailed));
        }
    }

    /// Archive a previous `current.jsonl` and start a fresh file.
    fn open_initial_current(&mut self) -> Result<(), FileSafetyError> {
        // A file left behind by an earlier process run is archived first, even when this
        // writer has recovered from a failure.
        self.current = None;
        if self.dir.open_read(CURRENT_LOG_FILE)?.is_some() {
            self.rotate_files()?;
        }
        self.create_current()
    }

    fn create_current(&mut self) -> Result<(), FileSafetyError> {
        let file = self.dir.create_new(CURRENT_LOG_FILE)?;
        self.current_size = file.len();
        self.bytes_since_flush = 0;
        self.current = Some(file);
        Ok(())
    }

    /// Append one record. Rotation is decided before the write from the record length, so
    /// the current file never exceeds the cap by even one record; returns whether the
    /// record was written.
    fn append(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() as u64 > CURRENT_MAX_BYTES {
            // Bounded records cannot reach this; count instead of writing an oversized file.
            self.fail_soft(FileSafetyError::InvalidData);
            return false;
        }
        if self.current.is_none() {
            return false;
        }
        if self.current_size + bytes.len() as u64 > CURRENT_MAX_BYTES {
            if let Err(error) = self.rotate_files() {
                self.fail_soft(error);
                return false;
            }
            if let Err(error) = self.create_current() {
                self.fail_soft(error);
                return false;
            }
        }
        let Some(current) = self.current.as_mut() else {
            return false;
        };
        match current.append(bytes) {
            Ok(()) => {
                self.current_size += bytes.len() as u64;
                self.bytes_since_flush += bytes.len() as u64;
                self.counters
                    .bytes_written
                    .fetch_add(bytes.len() as u64, Ordering::Relaxed);
                self.flush_if_due(false);
                true
            }
            Err(error) => {
                self.fail_soft(error);
                false
            }
        }
    }

    /// Flush when the byte or time budget is reached. Returns whether anything flushed.
    fn flush_if_due(&mut self, time_only: bool) -> bool {
        let byte_due = !time_only && self.bytes_since_flush >= FLUSH_BYTES;
        let time_due = self.last_flush.elapsed() >= FLUSH_INTERVAL;
        if !byte_due && !time_due {
            return false;
        }
        let Some(current) = self.current.as_mut() else {
            return false;
        };
        match current.sync() {
            Ok(()) => {
                self.counters.flushes.fetch_add(1, Ordering::Relaxed);
                self.bytes_since_flush = 0;
                self.last_flush = Instant::now();
                true
            }
            Err(error) => {
                self.fail_soft(error);
                false
            }
        }
    }

    /// Rotate the current file into `previous-1`, shifting older files. The caller creates
    /// the next current file. Capacity (not age) bounds the file set.
    fn rotate_files(&mut self) -> Result<(), FileSafetyError> {
        // Close the current handle first so its rename is a plain rename.
        self.current = None;
        let current_exists = self.dir.open_read(CURRENT_LOG_FILE)?.is_some();
        if current_exists {
            let last = previous_log_file(PREVIOUS_LOG_FILES);
            if let Some(file) = self.dir.open_read(&last)? {
                self.dir.remove_verified(&last, file.identity())?;
            }
            for index in (1..PREVIOUS_LOG_FILES).rev() {
                let from = previous_log_file(index);
                let Some(file) = self.dir.open_read(&from)? else {
                    continue;
                };
                let to = previous_log_file(index + 1);
                self.dir.rename_verified(&from, &to, file.identity())?;
            }
            let file = self
                .dir
                .open_read(CURRENT_LOG_FILE)?
                .ok_or(FileSafetyError::NotFound)?;
            self.dir
                .rename_verified(CURRENT_LOG_FILE, &previous_log_file(1), file.identity())?;
        }
        self.counters.rotated.fetch_add(1, Ordering::Relaxed);
        self.purge_aged();
        Ok(())
    }

    /// Delete rotated files whose last verified event is older than the retention window.
    /// Unparseable or future-dated files are kept; the hard file cap still bounds usage.
    fn purge_aged(&mut self) {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_millis() as u64)
            .unwrap_or(0);
        for index in 1..=PREVIOUS_LOG_FILES {
            let name = previous_log_file(index);
            let Some(mut file) = self.dir.open_read(&name).ok().flatten() else {
                continue;
            };
            let last_event_ms = match file.read_tail(RETENTION_TAIL_BYTES) {
                Ok(tail) => last_record_timestamp(&tail),
                Err(_) => None,
            };
            let Some(last_event_ms) = last_event_ms else {
                continue;
            };
            if last_event_ms > now_ms {
                // Clock anomaly: never delete early.
                continue;
            }
            if now_ms - last_event_ms > RETENTION_MS {
                let identity = file.identity();
                if self.dir.remove_verified(&name, identity).is_err() {
                    self.health
                        .set(Some(crate::error::SubsystemReason::RetentionFailed));
                }
            }
        }
    }

    /// A write-side failure drops the current handle and schedules a bounded reopen attempt.
    /// The writer never retries in a tight loop and never blocks the emitting threads.
    fn fail_soft(&mut self, error: FileSafetyError) {
        self.current = None;
        self.bytes_since_flush = 0;
        self.counters.write_failures.fetch_add(1, Ordering::Relaxed);
        let reason = match error {
            FileSafetyError::UnsafeDirectory | FileSafetyError::UnsafeFile => {
                crate::error::SubsystemReason::PathUnsafe
            }
            FileSafetyError::UnsupportedPlatform => {
                crate::error::SubsystemReason::UnsupportedPlatform
            }
            _ => crate::error::SubsystemReason::WriterWriteFailed,
        };
        self.health.set(Some(reason));
    }

    fn try_reopen(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last_reopen_attempt
            && now.duration_since(last) < REOPEN_INTERVAL
        {
            // Sleep in bounded slices so shutdown is not delayed.
            std::thread::sleep(Duration::from_millis(50));
            return;
        }
        self.last_reopen_attempt = Some(now);
        match self.open_initial_current() {
            Ok(()) => self.health.set(None),
            Err(error) => self.fail_soft(error),
        }
    }

    /// Drain inside the flush budget. Every record goes through the same accepted write
    /// path (rotation included), and whatever the bounded drain cannot write is counted
    /// instead of being reported as flushed.
    fn shutdown(&mut self) {
        let deadline = Instant::now() + SHUTDOWN_FLUSH_BUDGET;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let Some(bytes) = self
                .receiver
                .pop_timeout(remaining.min(Duration::from_millis(50)))
            else {
                if self.receiver.is_closed() {
                    break;
                }
                continue;
            };
            if !self.append(&bytes) {
                self.counters
                    .lost_at_shutdown
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        let unwritten = self.receiver.drain().len() as u64;
        if unwritten > 0 {
            self.counters
                .lost_at_shutdown
                .fetch_add(unwritten, Ordering::Relaxed);
        }
        // The loss is written as one real record before the lock is released, so an
        // operator reading the log sees the incomplete drain instead of a silent end.
        self.write_final_stats_record();
        if let Some(current) = self.current.as_mut() {
            match current.sync() {
                Ok(()) => {
                    self.counters.flushes.fetch_add(1, Ordering::Relaxed);
                }
                Err(error) => self.fail_soft(error),
            }
        }
        self.current = None;
        if self.counters.lost_at_shutdown() > 0 {
            // A bounded exit that could not write everything is not a clean flush.
            self.health
                .set(Some(crate::error::SubsystemReason::WriterWriteFailed));
        }
    }

    /// One final statistics record when this writer failed or lost records. Written on this
    /// thread with the role lock still held; the record carries the same identity fields as
    /// every other record of this process. It is `warn` severity and is written even under a
    /// stricter level: a bounded exit that could not write everything must not end the log
    /// silently.
    fn write_final_stats_record(&mut self) {
        if self.counters.write_failures() == 0 && self.counters.lost_at_shutdown() == 0 {
            return;
        }
        let Some(context) = self.record.clone() else {
            return;
        };
        let event = crate::event::DiagnosticEvent::WriterStats(crate::event::WriterStats {
            rotated: self.counters.rotated(),
            flushes: self.counters.flushes(),
            bytes_written: self.counters.bytes_written(),
            write_failures: self.counters.write_failures(),
            lost_at_shutdown: self.counters.lost_at_shutdown(),
        });
        let level = event.level();
        let (_, _, revision) = context.shared.level_state();
        let record = DiagnosticRecordV1 {
            schema: crate::record::RecordSchema,
            timestamp_ms: crate::context::now_epoch_ms(),
            monotonic_ms: context.started.elapsed().as_millis() as u64,
            component: context.component,
            boot_id: context.boot_id,
            sequence: context.shared.next_sequence(),
            level,
            level_revision: revision,
            parent_session_id: context.parent_session_id,
            span_id: None,
            parent_span_id: None,
            event,
        };
        if let Ok(bytes) = record.encode_jsonl() {
            self.append(&bytes);
        }
    }
}

fn last_record_timestamp(tail: &[u8]) -> Option<u64> {
    let text = std::str::from_utf8(tail).ok()?;
    let line = text.lines().next_back()?;
    if line.is_empty() {
        return None;
    }
    let record = DiagnosticRecordV1::parse_line(line.as_bytes()).ok()?;
    Some(record.timestamp_ms)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::files::PrivateDir;
    use crate::identity::BootId;
    use crate::level::DiagnosticLevel;
    use crate::queue::bounded_queue;
    use crate::record::Component;
    use crate::shared::Shared;

    fn record_bytes() -> Vec<u8> {
        DiagnosticRecordV1 {
            schema: crate::record::RecordSchema,
            timestamp_ms: 1,
            monotonic_ms: 0,
            component: Component::Diagnostics,
            boot_id: BootId::random().expect("boot id"),
            sequence: 1,
            level: DiagnosticLevel::Info,
            level_revision: 0,
            parent_session_id: None,
            span_id: None,
            parent_span_id: None,
            event: crate::event::DiagnosticEvent::StageBegin(crate::event::StageBegin {
                stage: crate::event::StartupStage::ReadyWait,
            }),
        }
        .encode_jsonl()
        .expect("encode")
    }

    #[test]
    fn explicit_unlock_releases_an_inherited_descriptor() {
        let temp = crate::private_tempdir();
        let root = temp.path().join("diagnostics");
        let dir = Arc::new(PrivateDir::open_or_create(&root).expect("root"));
        let (_sender, receiver) = bounded_queue();
        let mut predecessor = Writer::new(
            dir.clone(),
            receiver,
            Arc::new(WriterCounters::default()),
            WriterHealth::new(),
            None,
        );
        predecessor.claim_writer_lock().expect("claim lock");
        let inherited = predecessor
            .writer_lock
            .as_ref()
            .expect("lock")
            .duplicate_for_test()
            .expect("duplicate inherited descriptor");

        predecessor.release_writer_lock();
        drop(predecessor);

        let (_sender, receiver) = bounded_queue();
        let mut successor = Writer::new(
            dir,
            receiver,
            Arc::new(WriterCounters::default()),
            WriterHealth::new(),
            None,
        );
        successor
            .claim_writer_lock()
            .expect("the successor claims while the unlocked duplicate remains open");
        drop(inherited);
    }

    /// A write failure that is later repaired still ends with one real `writer_stats` record
    /// under the process's own identity, written by the thread that holds the role lock.
    #[test]
    fn a_failed_writer_records_its_own_exit_statistics() {
        let temp = crate::private_tempdir();
        let root = temp.path().join("diagnostics");
        PrivateDir::open_or_create(&root).expect("root");
        let dir = PrivateDir::open_existing(&root).expect("reopen");
        let (sender, receiver) = bounded_queue();
        let counters = Arc::new(WriterCounters::default());
        let health = WriterHealth::new();
        let boot_id = BootId::random().expect("boot id");
        let context = WriterRecordContext::new(
            Component::Diagnostics,
            boot_id,
            None,
            Instant::now(),
            Arc::new(Shared::new()),
        );
        let writer = spawn_writer(
            Arc::new(dir),
            receiver,
            counters.clone(),
            health.clone(),
            Some(context),
        );
        assert!(
            sender
                .try_push(DiagnosticLevel::Debug, record_bytes())
                .is_ok(),
            "the first record must be accepted"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while counters.bytes_written() == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(counters.bytes_written() > 0);

        let log_path = root.join(CURRENT_LOG_FILE);
        std::fs::hard_link(&log_path, root.join("foreign-link")).expect("hardlink");
        assert!(
            sender
                .try_push(DiagnosticLevel::Debug, record_bytes())
                .is_ok(),
            "the failing record must be accepted by the queue"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while health.snapshot().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(health.snapshot().is_some(), "the refusal must be observed");
        assert!(counters.write_failures() >= 1);

        // The environment is repaired; the writer reopens and its exit record becomes
        // writable again.
        std::fs::remove_file(root.join("foreign-link")).expect("restore single link");
        let deadline = Instant::now() + Duration::from_secs(10);
        while health.snapshot().is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(health.snapshot().is_none(), "the writer must recover");
        sender.close();
        assert!(writer.completion.wait(Duration::from_secs(5)));

        let mut current = PrivateDir::open_existing(&root)
            .expect("reopen")
            .open_read(CURRENT_LOG_FILE)
            .expect("open")
            .expect("present");
        let text = String::from_utf8(current.read_prefix(4096).expect("read")).expect("utf8");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines.len(),
            1,
            "the recovered file holds exactly the final record: {text}"
        );
        let record =
            DiagnosticRecordV1::parse_line(lines[0].as_bytes()).expect("the record must parse");
        assert_eq!(record.event.kind(), "writer_stats");
        assert_eq!(record.level, DiagnosticLevel::Warn);
        assert_eq!(record.boot_id, boot_id, "the same process identity");
        assert_eq!(record.component, Component::Diagnostics);
        let crate::event::DiagnosticEvent::WriterStats(stats) = record.event else {
            panic!("expected writer_stats");
        };
        assert!(stats.write_failures >= 1);
        assert_eq!(stats.lost_at_shutdown, 0);
    }
}
