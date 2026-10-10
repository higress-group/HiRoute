//! Writer, rotation, retention and saturation bounds (V5) against real files.
//!
//! These tests exercise the public queue, file and runtime APIs with real temporary
//! directories. They prove that a full queue or an unavailable writer never blocks the
//! emitting thread and that rotation/retention touch only owned files.

#![cfg(unix)]

mod support;

use std::os::unix::fs::FileTypeExt;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hiroute_diagnostics::event::{DiagnosticEvent, ProcessRole, StageBegin, StartupStage};
use hiroute_diagnostics::files::{
    CURRENT_LOG_FILE, PREVIOUS_LOG_FILES, PrivateDir, SETTINGS_FILE, WRITER_LOCK_FILE,
    previous_log_file, role_dir_name,
};
use hiroute_diagnostics::level::DiagnosticLevel;
use hiroute_diagnostics::queue::bounded_queue;
use hiroute_diagnostics::record::{Component, DiagnosticRecordV1, RECORD_SCHEMA_V1, RecordSchema};
use hiroute_diagnostics::runtime::{DiagnosticRuntime, RuntimeConfig};
use hiroute_diagnostics::writer::{CURRENT_MAX_BYTES, WriterCounters, WriterHealth, spawn_writer};
use hiroute_diagnostics::{error::SubsystemReason, identity::BootId};

fn private_root() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = support::private_tempdir();
    let root = temp.path().join("diagnostics");
    PrivateDir::open_or_create(&root).expect("create root");
    (temp, root)
}

fn record_bytes(sequence: u64, timestamp_ms: u64) -> Vec<u8> {
    let record = DiagnosticRecordV1 {
        schema: RecordSchema,
        timestamp_ms,
        monotonic_ms: sequence,
        component: Component::Diagnostics,
        boot_id: BootId::random().expect("boot id"),
        sequence,
        level: DiagnosticLevel::Debug,
        level_revision: 1,
        parent_session_id: None,
        span_id: None,
        parent_span_id: None,
        event: DiagnosticEvent::StageBegin(StageBegin {
            stage: StartupStage::ReadyWait,
        }),
    };
    record.encode_jsonl().expect("encode")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis() as u64
}

fn role_dir(root: &std::path::Path) -> PrivateDir {
    let root = PrivateDir::open_existing(root).expect("open root");
    root.child_dir(role_dir_name(ProcessRole::Daemon))
        .expect("role dir")
}

#[test]
fn rotation_shifts_files_and_keeps_capacity_bounded() {
    let (_temp, root) = private_root();
    let dir = role_dir(&root);
    let (sender, receiver) = bounded_queue();
    let counters = Arc::new(WriterCounters::default());
    let health = WriterHealth::new();
    let writer = spawn_writer(
        Arc::new(PrivateDir::open_existing(dir.path()).expect("reopen")),
        receiver,
        counters.clone(),
        health,
        None,
    );

    // Push enough records to force at least one rotation, then close and drain.
    let mut pushed = 0u64;
    let deadline = Instant::now() + Duration::from_secs(30);
    while counters.rotated() == 0 && Instant::now() < deadline {
        let batch: Vec<Vec<u8>> = (0..64)
            .map(|index| record_bytes(pushed + index, now_ms()))
            .collect();
        for bytes in batch {
            let mut attempts = 0;
            loop {
                match sender.try_push(DiagnosticLevel::Debug, bytes.clone()) {
                    Ok(()) => break,
                    Err(_) => {
                        attempts += 1;
                        assert!(attempts < 10_000, "writer stopped draining");
                        std::thread::sleep(Duration::from_millis(2));
                    }
                }
            }
            pushed += 1;
        }
        if pushed > 4 * CURRENT_MAX_BYTES / 200 {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    assert!(
        counters.rotated() >= 1,
        "rotation must happen at the byte cap"
    );
    sender.close();
    assert!(writer.completion.wait(Duration::from_secs(5)));

    let names = dir.list_names().expect("list");
    let log_files: Vec<&String> = names
        .iter()
        .filter(|name| name.ends_with(".jsonl") && !name.contains("attacker"))
        .collect();
    assert!(log_files.len() <= PREVIOUS_LOG_FILES + 1, "{names:?}");
    assert!(names.contains(&CURRENT_LOG_FILE.to_string()));
    assert!(names.contains(&previous_log_file(1)));
    let current = dir
        .open_read(CURRENT_LOG_FILE)
        .expect("open")
        .expect("present");
    assert!(
        current.len() <= CURRENT_MAX_BYTES,
        "rotation is decided before the write, so the open file never exceeds the cap"
    );
    let previous = dir
        .open_read(&previous_log_file(1))
        .expect("open")
        .expect("present");
    assert!(!previous.is_empty());
    assert!(
        previous.len() <= CURRENT_MAX_BYTES,
        "an archived file must keep the same bound, got {}",
        previous.len()
    );
}

#[test]
fn restart_archives_the_previous_current_file() {
    let (_temp, root) = private_root();
    let dir_path = role_dir(&root).path().to_path_buf();

    for round in 0..2 {
        let dir = PrivateDir::open_existing(&dir_path).expect("open role dir");
        let (sender, receiver) = bounded_queue();
        let writer = spawn_writer(
            Arc::new(PrivateDir::open_existing(dir.path()).expect("reopen")),
            receiver,
            Arc::new(WriterCounters::default()),
            WriterHealth::new(),
            None,
        );
        assert!(
            writer.owns_role(),
            "a completed predecessor must release writer.lock before restart (round {round}, health {:?})",
            writer.health.snapshot()
        );
        let mut attempts = 0;
        loop {
            match sender.try_push(DiagnosticLevel::Debug, record_bytes(round, now_ms())) {
                Ok(()) => break,
                Err(_) => {
                    attempts += 1;
                    assert!(attempts < 1000);
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
        sender.close();
        assert!(writer.completion.wait(Duration::from_secs(5)));
        if round == 0 {
            let dir = PrivateDir::open_existing(&dir_path).expect("open role dir");
            assert!(dir.open_read(CURRENT_LOG_FILE).expect("open").is_some());
        }
    }

    let dir = PrivateDir::open_existing(&dir_path).expect("open role dir");
    let previous = dir
        .open_read(&previous_log_file(1))
        .expect("open")
        .expect("previous-1 exists after restart");
    assert!(
        !previous.is_empty(),
        "the old current file must be archived"
    );
    let current = dir
        .open_read(CURRENT_LOG_FILE)
        .expect("open")
        .expect("present");
    assert!(!current.is_empty());
}

#[test]
fn aged_files_are_removed_but_future_dated_and_invalid_are_kept() {
    let (_temp, root) = private_root();
    let dir = role_dir(&root);
    let day = 24 * 60 * 60 * 1000u64;
    // previous-1: last event 8 days ago -> removable.
    let mut aged = dir.create_new(&previous_log_file(1)).expect("create");
    aged.append(&record_bytes(1, now_ms() - 8 * day))
        .expect("append");
    drop(aged);
    // previous-2: last event in the future -> clock anomaly, kept.
    let mut future = dir.create_new(&previous_log_file(2)).expect("create");
    future
        .append(&record_bytes(2, now_ms() + day))
        .expect("append");
    drop(future);
    // previous-3: not parseable -> kept.
    let mut garbage = dir.create_new(&previous_log_file(3)).expect("create");
    garbage.append(b"not-json\n").expect("append");
    drop(garbage);
    // A second, unknown file -> never touched.
    let mut unknown = dir.create_new("attacker.jsonl").expect("create");
    unknown.append(b"unknown").expect("append");
    drop(unknown);

    let (sender, receiver) = bounded_queue();
    let counters = Arc::new(WriterCounters::default());
    let writer = spawn_writer(
        Arc::new(PrivateDir::open_existing(dir.path()).expect("reopen")),
        receiver,
        counters.clone(),
        WriterHealth::new(),
        None,
    );
    // Force a rotation by writing past the cap.
    let mut index = 0u64;
    let deadline = Instant::now() + Duration::from_secs(30);
    while counters.rotated() == 0 && Instant::now() < deadline {
        for _ in 0..64 {
            let bytes = record_bytes(index, now_ms());
            index += 1;
            let mut attempts = 0;
            loop {
                match sender.try_push(DiagnosticLevel::Debug, bytes.clone()) {
                    Ok(()) => break,
                    Err(_) => {
                        attempts += 1;
                        assert!(attempts < 10_000);
                        std::thread::sleep(Duration::from_millis(2));
                    }
                }
            }
        }
    }
    assert!(counters.rotated() >= 1);
    sender.close();
    assert!(writer.completion.wait(Duration::from_secs(5)));

    assert!(
        dir.open_read(&previous_log_file(2))
            .expect("open")
            .is_none(),
        "a rotated file older than the retention window must be removed"
    );
    assert!(
        dir.open_read(&previous_log_file(3))
            .expect("open")
            .is_some(),
        "future-dated files must not be deleted on a clock anomaly"
    );
    assert!(
        dir.open_read(&previous_log_file(4))
            .expect("open")
            .is_some(),
        "unparseable files must be kept"
    );
    assert!(
        dir_path_has(&dir, "attacker.jsonl"),
        "unknown files must never be touched"
    );
}

fn dir_path_has(dir: &PrivateDir, name: &str) -> bool {
    dir.list_names()
        .expect("list")
        .iter()
        .any(|entry| entry == name)
}

#[test]
fn second_writer_reports_writer_owned_and_does_not_take_over() {
    let (_temp, root) = private_root();
    let root_dir = PrivateDir::open_existing(&root).expect("open root");
    let role = root_dir
        .child_dir(role_dir_name(ProcessRole::Daemon))
        .expect("role dir");
    let held = role.open_lock(WRITER_LOCK_FILE).expect("lock file");
    held.try_lock_exclusive().expect("hold writer lock");

    let config = RuntimeConfig {
        root: root.clone(),
        role: ProcessRole::Daemon,
        component: Component::Daemon,
        parent_session_id: None,
        level_override: None,
    };
    let runtime = DiagnosticRuntime::start(config);
    // The runtime must never take the lock from the live writer, and it must say so.
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut status = runtime.status();
    while status.unavailable.is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        status = runtime.status();
    }
    assert_eq!(
        status.unavailable,
        Some(SubsystemReason::WriterOwned),
        "the losing writer must report writer_owned"
    );
    assert!(status.armed);
    // The live owner's role directory is never touched by the loser.
    assert!(
        !dir_path_has(&role, CURRENT_LOG_FILE),
        "the loser must not create the owner's log file"
    );
    runtime.shutdown();
}

#[test]
fn saturated_queue_drops_without_blocking_the_emitter() {
    let (_temp, root) = private_root();
    let root_dir = PrivateDir::open_existing(&root).expect("open root");
    let role = root_dir
        .child_dir(role_dir_name(ProcessRole::Daemon))
        .expect("role dir");
    // Keep the writer lock so the runtime's writer never drains the queue: the emitter
    // path must stay non-blocking even with a fully saturated queue.
    let held = role.open_lock(WRITER_LOCK_FILE).expect("lock file");
    held.try_lock_exclusive().expect("hold writer lock");

    let runtime = DiagnosticRuntime::start(RuntimeConfig {
        root: root.clone(),
        role: ProcessRole::Daemon,
        component: Component::Daemon,
        parent_session_id: None,
        level_override: Some(DiagnosticLevel::Debug),
    });
    let handle = runtime.handle().clone();
    let mut worst = Duration::ZERO;
    for index in 0..4096u64 {
        let started = Instant::now();
        handle.try_emit(DiagnosticEvent::StageBegin(StageBegin {
            stage: StartupStage::ReadyWait,
        }));
        worst = worst.max(started.elapsed());
        let _ = index;
    }
    assert!(
        worst < Duration::from_millis(20),
        "emit must never wait for I/O, worst was {worst:?}"
    );
    let counters = handle.counters().expect("counters");
    assert!(counters.dropped() > 0, "a saturated queue must count drops");
    let queue = runtime.queue_snapshot().expect("queue snapshot");
    assert!(queue.bytes <= hiroute_diagnostics::queue::MAX_QUEUE_BYTES);
    assert!(queue.events <= hiroute_diagnostics::queue::MAX_QUEUE_EVENTS);
    runtime.shutdown();
}

#[test]
fn writer_enforces_single_record_size_limit() {
    let record = DiagnosticRecordV1 {
        schema: RecordSchema,
        timestamp_ms: now_ms(),
        monotonic_ms: 0,
        component: Component::Diagnostics,
        boot_id: BootId::random().expect("boot id"),
        sequence: 1,
        level: DiagnosticLevel::Info,
        level_revision: 1,
        parent_session_id: None,
        span_id: None,
        parent_span_id: None,
        event: DiagnosticEvent::PanicObserved(hiroute_diagnostics::event::PanicObserved {
            source_file: hiroute_diagnostics::identity::SourceFileRef::parse(
                "crates/daemon/src/control/bin.rs",
            )
            .expect("path"),
            line: 42,
        }),
    };
    let bytes = record.encode_jsonl().expect("encode");
    assert!(bytes.len() <= hiroute_diagnostics::record::MAX_RECORD_BYTES);
    assert!(bytes.ends_with(b"\n"));
    let parsed = DiagnosticRecordV1::parse_line(&bytes).expect("parse");
    assert_eq!(parsed.schema, RecordSchema);
    assert_eq!(parsed.event.kind(), "panic_observed");

    // Unknown fields are rejected rather than imported.
    let tampered = br#"{"schema":"hiroute.diagnostic-event/v1","timestamp_ms":1,"monotonic_ms":0,"component":"daemon","boot_id":"00000000000000000000000000000000","sequence":0,"level":"info","level_revision":0,"event":{"stage_begin":{"stage":"ready_wait"}},"extra":"sentinel"}"#;
    assert_eq!(
        DiagnosticRecordV1::parse_line(tampered),
        Err(hiroute_diagnostics::record::RecordError::Invalid)
    );
    let wrong_schema = br#"{"schema":"hiroute.diagnostic-event/v2","timestamp_ms":1,"monotonic_ms":0,"component":"daemon","boot_id":"00000000000000000000000000000000","sequence":0,"level":"info","level_revision":0,"event":{"stage_begin":{"stage":"ready_wait"}}}"#;
    assert_eq!(
        DiagnosticRecordV1::parse_line(wrong_schema),
        Err(hiroute_diagnostics::record::RecordError::Invalid)
    );
    assert!(RECORD_SCHEMA_V1.ends_with("/v1"));
}

#[test]
fn symlinked_role_directory_is_reported_instead_of_blocking_startup() {
    let temp = support::private_tempdir();
    let root = temp.path().join("diagnostics");
    let outside = temp.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, &root).unwrap();
    let runtime = DiagnosticRuntime::start(RuntimeConfig {
        root,
        role: ProcessRole::Desktop,
        component: Component::Desktop,
        parent_session_id: None,
        // Debug keeps the level filter out of the way: the event below must reach the
        // queue boundary so the drop (not a filter) is what the assertion measures.
        level_override: Some(DiagnosticLevel::Debug),
    });
    let status = runtime.status();
    assert_eq!(status.unavailable, Some(SubsystemReason::PathUnsafe));
    assert!(status.armed);
    // Call sites keep working: every event is dropped and counted instead of piling up in
    // an unread queue or being claimed as written.
    assert_eq!(
        runtime.emit(DiagnosticEvent::StageBegin(StageBegin {
            stage: StartupStage::ReadyWait,
        })),
        hiroute_diagnostics::EmitOutcome::Dropped
    );
    assert!(runtime.handle().counters().expect("counters").dropped() > 0);
    runtime.shutdown();
}

#[test]
fn writer_lock_file_is_not_left_readable_by_others() {
    let (_temp, root) = private_root();
    let dir = role_dir(&root);
    let (sender, receiver) = bounded_queue();
    let writer = spawn_writer(
        Arc::new(PrivateDir::open_existing(dir.path()).expect("reopen")),
        receiver,
        Arc::new(WriterCounters::default()),
        WriterHealth::new(),
        None,
    );
    sender.close();
    assert!(writer.completion.wait(Duration::from_secs(5)));
    let lock_path = dir.path().join(WRITER_LOCK_FILE);
    let mode = std::fs::metadata(&lock_path)
        .expect("stat")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

/// A real first start: the application root does not exist yet, so the whole parent chain is
/// created. The runtime must arm, stay healthy and write its first record into a directory
/// nobody had to chmod.
#[test]
fn first_start_creates_a_missing_root_chain_and_writes() {
    let temp = support::private_tempdir();
    let root = temp
        .path()
        .join("app-data")
        .join("hiroute")
        .join("diagnostics");
    assert!(!root.exists(), "the chain must be missing before the start");

    let runtime = DiagnosticRuntime::start(RuntimeConfig {
        root: root.clone(),
        role: ProcessRole::Desktop,
        component: Component::Desktop,
        parent_session_id: None,
        level_override: Some(DiagnosticLevel::Debug),
    });
    assert!(
        runtime.armed(),
        "a missing parent chain must not leave the report unarmed"
    );
    let status = runtime.status();
    assert_eq!(status.unavailable, None, "{status:?}");
    assert_eq!(status.settings_error, None);
    assert!(runtime.owns_role());

    runtime.emit(DiagnosticEvent::StageBegin(StageBegin {
        stage: StartupStage::ReadyWait,
    }));
    let dir = PrivateDir::open_existing(&root)
        .expect("open root")
        .child_dir(role_dir_name(ProcessRole::Desktop))
        .expect("role dir");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut written = 0;
    while Instant::now() < deadline {
        if let Some(file) = dir.open_read(CURRENT_LOG_FILE).expect("open") {
            written = file.len();
        }
        if written > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(written > 0, "the first record must reach the new role dir");

    let mode = std::fs::metadata(&root)
        .expect("stat root")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700, "a level this process created is owner-private");
    runtime.shutdown();
}

/// Two runtimes on one role: the loser reports its own state in memory and never touches the
/// owner's role directory at all — no log record, no level revision, no file.
#[test]
fn losing_role_runtime_never_publishes_over_the_owner() {
    let (_temp, root) = private_root();
    let winner = DiagnosticRuntime::start(RuntimeConfig {
        root: root.clone(),
        role: ProcessRole::Daemon,
        component: Component::Daemon,
        parent_session_id: None,
        level_override: None,
    });
    assert!(winner.owns_role());
    winner.emit(DiagnosticEvent::StageBegin(StageBegin {
        stage: StartupStage::ReadyWait,
    }));
    let dir = role_dir(&root);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !dir_path_has(&dir, CURRENT_LOG_FILE) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    let before_names = dir.list_names().expect("names");
    let before_log = std::fs::read(dir.path().join(CURRENT_LOG_FILE)).expect("read log");
    let owner_boot_id = before_log
        .split(|byte| *byte == b'\n')
        .find(|line| !line.is_empty())
        .map(DiagnosticRecordV1::parse_line)
        .transpose()
        .expect("parse owner log")
        .expect("owner log record")
        .boot_id;

    let loser = DiagnosticRuntime::start(RuntimeConfig {
        root: root.clone(),
        role: ProcessRole::Daemon,
        component: Component::Daemon,
        parent_session_id: None,
        level_override: None,
    });
    assert!(
        !loser.owns_role(),
        "the second runtime must not take the lock"
    );
    let status = loser.status();
    assert_eq!(status.unavailable, Some(SubsystemReason::WriterOwned));
    assert!(status.armed, "the loser still settles its own report");
    // The loser keeps working in memory: its own view and queue accept the change...
    loser.apply_saved_level(DiagnosticLevel::Debug, 7);
    assert_eq!(loser.status().revision, 7);
    assert_eq!(loser.status().level, DiagnosticLevel::Debug);
    loser.emit(DiagnosticEvent::StageBegin(StageBegin {
        stage: StartupStage::ReadyWait,
    }));
    loser.shutdown();

    // ...but the owner's role directory never learned about it: no new file and no record
    // carrying the loser's distinct boot identity. The live owner may append its own queued
    // startup records between these snapshots, so byte-for-byte equality would be racy.
    let after_names = dir.list_names().expect("names");
    assert_eq!(
        after_names, before_names,
        "the loser must not create or replace the owner's files"
    );
    let after_log = std::fs::read(dir.path().join(CURRENT_LOG_FILE)).expect("read log");
    for line in after_log
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let record = DiagnosticRecordV1::parse_line(line).expect("parse owner log");
        assert_eq!(
            record.boot_id, owner_boot_id,
            "the loser's events must never reach the owner's log"
        );
    }
    winner.shutdown();
}

/// The bounded exit drain uses the accepted write path: a record it cannot write is counted
/// as lost and degrades the report instead of being reported as a clean flush.
#[test]
fn shutdown_drain_counts_records_it_cannot_write() {
    let (_temp, root) = private_root();
    let dir = role_dir(&root);
    let counters = Arc::new(WriterCounters::default());
    let (sender, receiver) = bounded_queue();
    let writer = spawn_writer(
        Arc::new(PrivateDir::open_existing(dir.path()).expect("reopen")),
        receiver,
        counters.clone(),
        WriterHealth::new(),
        None,
    );
    assert!(
        sender
            .try_push(DiagnosticLevel::Debug, record_bytes(1, now_ms()))
            .is_ok()
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while counters.bytes_written() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(counters.bytes_written() > 0, "the first record is written");

    // A hardlink violates stable single-file ownership even though broad modes are allowed.
    std::fs::hard_link(
        dir.path().join(CURRENT_LOG_FILE),
        dir.path().join("foreign-link"),
    )
    .unwrap();
    for sequence in 10..14 {
        let _ = sender.try_push(DiagnosticLevel::Debug, record_bytes(sequence, now_ms()));
    }
    sender.close();
    assert!(writer.completion.wait(Duration::from_secs(5)));
    assert!(
        counters.lost_at_shutdown() >= 1,
        "an unwritable record must be counted, not claimed as flushed"
    );
    assert_eq!(
        writer.health.snapshot(),
        Some(SubsystemReason::WriterWriteFailed),
        "a lossy exit is degraded, never silently successful"
    );
}

/// A writer that can never open its file degrades in memory instead of blocking, and the exit
/// budget still holds: the emitting caller never performs file I/O.
#[test]
fn shutdown_stays_bounded_when_the_writer_cannot_open_its_file() {
    let (_temp, root) = private_root();
    let role = root.join(role_dir_name(ProcessRole::Daemon));
    std::fs::create_dir_all(&role).expect("role dir");
    std::fs::set_permissions(&role, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    let fifo = role.join(CURRENT_LOG_FILE);
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo")
            .success()
    );

    let runtime = DiagnosticRuntime::start(RuntimeConfig {
        root: root.clone(),
        role: ProcessRole::Daemon,
        component: Component::Daemon,
        parent_session_id: None,
        level_override: Some(DiagnosticLevel::Debug),
    });
    assert!(runtime.armed(), "an unusable log file still settles");
    assert_eq!(
        runtime.status().settings_error,
        None,
        "a log-side fault is not a settings fault"
    );
    for _ in 0..64 {
        runtime.emit(DiagnosticEvent::StageBegin(StageBegin {
            stage: StartupStage::ReadyWait,
        }));
    }
    let started = Instant::now();
    runtime.shutdown();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "a writer that cannot open its file must not extend the exit budget"
    );
    assert!(
        std::fs::metadata(&fifo)
            .expect("stat")
            .file_type()
            .is_fifo(),
        "the refused target must be left exactly as it was"
    );
}

fn write_settings(root: &std::path::Path, revision: u64, level: &str) {
    let path = root.join(SETTINGS_FILE);
    std::fs::write(
        &path,
        format!(
            r#"{{"schema":"hiroute.diagnostic-settings/v1","revision":{revision},"level":"{level}"}}"#
        ),
    )
    .expect("write settings");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
}

/// R7: a damaged settings file that follows a good read keeps the last known revision and
/// level, reports the error instead of a silent reset, and is never rewritten.
#[test]
fn a_damaged_settings_file_keeps_the_last_known_values_and_is_reported() {
    let (_temp, root) = private_root();
    write_settings(&root, 3, "error");
    let runtime = DiagnosticRuntime::start(RuntimeConfig {
        root: root.clone(),
        role: ProcessRole::Desktop,
        component: Component::Desktop,
        parent_session_id: None,
        level_override: None,
    });
    let status = runtime.status();
    assert_eq!(status.revision, 3);
    assert_eq!(status.level, DiagnosticLevel::Error);
    assert_eq!(status.settings_error, None);

    std::fs::write(
        root.join(SETTINGS_FILE),
        br#"{"schema":"hiroute.diagnostic-settings/v1","revision":9,"extra":true}"#,
    )
    .expect("damage");
    let deadline = Instant::now() + Duration::from_secs(5);
    while runtime.status().settings_error != Some(SubsystemReason::SettingsInvalid) {
        assert!(
            Instant::now() < deadline,
            "a damaged settings file must be reported: {:?}",
            runtime.status()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    let status = runtime.status();
    assert_eq!(status.revision, 3, "the last known revision is kept");
    assert_eq!(
        status.level,
        DiagnosticLevel::Error,
        "the known level is kept"
    );
    assert!(
        std::fs::read(root.join(SETTINGS_FILE))
            .expect("read")
            .ends_with(b"\"extra\":true}"),
        "the damaged file is reported, never rewritten"
    );
    runtime.shutdown();
}

/// R7: the first read has no known value, so a damaged file reports the safe default plus the
/// error; once the file is restored the error clears and the process converges without a
/// restart.
#[test]
fn settings_recovery_clears_the_error_and_converges() {
    let (_temp, root) = private_root();
    std::fs::write(root.join(SETTINGS_FILE), b"not-json").expect("damage");
    std::fs::set_permissions(
        root.join(SETTINGS_FILE),
        std::fs::Permissions::from_mode(0o600),
    )
    .expect("chmod");
    let runtime = DiagnosticRuntime::start(RuntimeConfig {
        root: root.clone(),
        role: ProcessRole::Daemon,
        component: Component::Daemon,
        parent_session_id: None,
        level_override: None,
    });
    let status = runtime.status();
    assert_eq!(
        status.settings_error,
        Some(SubsystemReason::SettingsInvalid)
    );
    assert_eq!(status.revision, 0);
    assert_eq!(status.level, DiagnosticLevel::runtime_default());

    write_settings(&root, 5, "debug");
    let deadline = Instant::now() + Duration::from_secs(5);
    while runtime.status().settings_error.is_some() {
        assert!(
            Instant::now() < deadline,
            "a restored file must clear the error: {:?}",
            runtime.status()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    let status = runtime.status();
    assert_eq!(status.revision, 5);
    assert_eq!(status.level, DiagnosticLevel::Debug);
    assert_eq!(
        runtime.handle().level(),
        Some(DiagnosticLevel::Debug),
        "the recovered level is applied to this process without a restart"
    );
    runtime.shutdown();
}

/// Eventual consistency without any cross-process confirmation: a running process uses the
/// level it read at start and adopts a newer revision written by another process in its next
/// watch cycle, without a confirmation call, an event bus or a process-state aggregate.
#[test]
fn a_running_process_converges_to_a_newer_saved_revision() {
    let (_temp, root) = private_root();
    write_settings(&root, 2, "error");
    let runtime = DiagnosticRuntime::start(RuntimeConfig {
        root: root.clone(),
        role: ProcessRole::Daemon,
        component: Component::Daemon,
        parent_session_id: None,
        level_override: None,
    });
    assert_eq!(runtime.handle().level(), Some(DiagnosticLevel::Error));
    assert_eq!(
        runtime.emit(DiagnosticEvent::StageBegin(StageBegin {
            stage: StartupStage::ReadyWait,
        })),
        hiroute_diagnostics::EmitOutcome::Filtered,
        "a Debug event is below the level read at start"
    );
    assert_eq!(
        runtime.emit(DiagnosticEvent::PanicObserved(
            hiroute_diagnostics::event::PanicObserved {
                source_file: hiroute_diagnostics::identity::SourceFileRef::parse(
                    "crates/daemon/src/control/bin.rs",
                )
                .expect("path"),
                line: 7,
            }
        )),
        hiroute_diagnostics::EmitOutcome::Queued,
        "a failure passes every threshold"
    );

    write_settings(&root, 4, "debug");
    let deadline = Instant::now() + Duration::from_secs(5);
    while runtime.status().revision != 4 {
        assert!(
            Instant::now() < deadline,
            "the watch must pick up the new revision: {:?}",
            runtime.status()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(runtime.status().settings_error, None);
    assert_eq!(runtime.handle().level(), Some(DiagnosticLevel::Debug));
    assert_eq!(
        runtime.emit(DiagnosticEvent::StageBegin(StageBegin {
            stage: StartupStage::ReadyWait,
        })),
        hiroute_diagnostics::EmitOutcome::Queued,
        "the converged level admits Debug events"
    );
    assert!(
        std::fs::read(root.join(SETTINGS_FILE))
            .expect("read")
            .ends_with(b"\"revision\":4,\"level\":\"debug\"}"),
        "the reader never rewrites the settings file it reads"
    );
    runtime.shutdown();
}
