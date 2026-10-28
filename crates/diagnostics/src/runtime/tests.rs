use super::*;
use crate::files::{CURRENT_LOG_FILE, SETTINGS_FILE};
use crate::record::DiagnosticRecordV1;

fn write_settings(root: &std::path::Path, revision: u64, level: &str) {
    let dir = PrivateDir::open_or_create(root).expect("root");
    let mut file = dir.create_new(SETTINGS_FILE).expect("create settings");
    file.append(
        format!(
            r#"{{"schema":"hiroute.diagnostic-settings/v1","revision":{revision},"level":"{level}"}}"#
        )
        .as_bytes(),
    )
    .expect("append");
}

fn config(root: &std::path::Path) -> RuntimeConfig {
    RuntimeConfig {
        root: root.to_path_buf(),
        role: ProcessRole::Daemon,
        component: Component::Daemon,
        parent_session_id: None,
        level_override: None,
    }
}

fn role_log_records(root: &std::path::Path) -> Vec<DiagnosticRecordV1> {
    let dir = PrivateDir::open_existing(root)
        .expect("root")
        .child_dir(role_dir_name(ProcessRole::Daemon))
        .expect("role dir");
    let Some(mut file) = dir.open_read(CURRENT_LOG_FILE).expect("open log") else {
        return Vec::new();
    };
    let bytes = file.read_prefix(64 * 1024).expect("read log");
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let mut records = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        match DiagnosticRecordV1::parse_line(line.as_bytes()) {
            Ok(record) => records.push(record),
            // A record still being appended can be read as its own prefix.
            Err(_) if index + 1 == lines.len() && !text.ends_with('\n') => break,
            Err(error) => panic!("unparseable record {index} in the log: {error}"),
        }
    }
    records
}

fn wait_for_kind(root: &std::path::Path, kind: &str) -> Vec<DiagnosticRecordV1> {
    wait_for_count(root, kind, 1)
}

fn wait_for_count(root: &std::path::Path, kind: &str, count: usize) -> Vec<DiagnosticRecordV1> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let records = role_log_records(root);
        let seen = records
            .iter()
            .filter(|record| record.event.kind() == kind)
            .count();
        if seen >= count {
            return records;
        }
        assert!(
            Instant::now() < deadline,
            "only {seen} {kind} records reached the log, wanted {count}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn applied_levels(records: &[DiagnosticRecordV1]) -> Vec<(DiagnosticLevel, u64, LevelSource)> {
    records
        .iter()
        .filter_map(|record| match &record.event {
            DiagnosticEvent::LevelApplied(applied) => {
                Some((applied.level, applied.revision, applied.source))
            }
            _ => None,
        })
        .collect()
}

fn routine_info() -> DiagnosticEvent {
    DiagnosticEvent::StartupEnd(crate::event::StartupEnd {
        outcome: crate::event::StartupOutcome::Success,
        elapsed_ms: 1,
    })
}

fn routine_warning() -> DiagnosticEvent {
    DiagnosticEvent::WriterStats(crate::event::WriterStats {
        rotated: 0,
        flushes: 1,
        bytes_written: 1,
        write_failures: 1,
        lost_at_shutdown: 0,
    })
}

/// R4: a startup that has not read the settings yet must not judge its earliest stages by
/// a guessed default. With Debug saved, the records emitted before the filesystem phase
/// finishes are written, in order, under the persisted level.
#[test]
fn a_saved_debug_level_keeps_the_earliest_startup_records() {
    let temp = crate::private_tempdir();
    let root = temp.path().join("diagnostics");
    write_settings(&root, 1, "debug");
    let (runtime, plan) = DiagnosticRuntime::prepare(config(&root));
    assert_eq!(
        runtime.handle().level(),
        None,
        "no level is known before the first read"
    );
    runtime.stage_begin(StartupStage::ArtifactValidate);
    runtime.stage_end(StartupStage::ArtifactValidate, StageOutcome::Completed);
    runtime.emit(DiagnosticEvent::StageProgress(
        crate::event::StageProgress {
            stage: StartupStage::ReadyWait,
            elapsed_ms: 3,
            budget_ms: Some(10),
        },
    ));
    arm(plan);
    assert_eq!(runtime.handle().level(), Some(DiagnosticLevel::Debug));
    let status = runtime.status();
    assert_eq!(status.revision, 1);
    assert_eq!(status.settings_error, None);
    assert_eq!(status.unavailable, None);

    let records = wait_for_kind(&root, "process_start");
    let kinds: Vec<&str> = records.iter().map(|record| record.event.kind()).collect();
    assert_eq!(
        &kinds[..5],
        [
            "stage_begin",
            "stage_end",
            "stage_progress",
            "level_applied",
            "process_start"
        ],
        "every early record is written in order: {kinds:?}"
    );
    assert_eq!(records[0].level, DiagnosticLevel::Debug);
    let DiagnosticEvent::LevelApplied(applied) = &records[3].event else {
        panic!("expected level_applied");
    };
    assert_eq!(applied.level, DiagnosticLevel::Debug);
    assert_eq!(applied.revision, 1);
    assert_eq!(applied.source, LevelSource::Persisted);
    runtime.shutdown();
}

/// R4/2a: with Error saved, the routine Debug/Info records admitted while no level was
/// known are dropped before the writer starts, so the log never presents them as if they
/// followed the saved level. An early failure still survives, and the applied level is
/// stated even though its own record is `info`.
#[test]
fn a_saved_error_level_discards_the_earliest_routine_records() {
    let temp = crate::private_tempdir();
    let root = temp.path().join("diagnostics");
    write_settings(&root, 5, "error");
    let (runtime, plan) = DiagnosticRuntime::prepare(config(&root));
    runtime.emit(DiagnosticEvent::PanicObserved(
        crate::event::PanicObserved {
            source_file: crate::identity::SourceFileRef::parse("crates/daemon/src/control/bin.rs")
                .expect("path"),
            line: 11,
        },
    ));
    runtime.stage_begin(StartupStage::ReadyWait);
    arm(plan);
    assert_eq!(runtime.handle().level(), Some(DiagnosticLevel::Error));
    assert_eq!(runtime.status().revision, 5);

    let records = wait_for_kind(&root, "level_applied");
    let kinds: Vec<&str> = records.iter().map(|record| record.event.kind()).collect();
    assert_eq!(
        kinds,
        ["panic_observed", "level_applied"],
        "routine records stay filtered under a saved Error level; only the applied-level \
         meta record survives"
    );
    let meta = records.last().expect("the level_applied record");
    assert_eq!(
        meta.level,
        DiagnosticLevel::Info,
        "the meta record keeps its info severity"
    );
    assert_eq!(
        applied_levels(&records),
        [(DiagnosticLevel::Error, 5, LevelSource::Persisted)],
        "the process states the level it actually runs at"
    );
    assert!(
        runtime.handle().counters().expect("counters").filtered() >= 1,
        "the discarded early record is counted, not silently dropped"
    );
    runtime.shutdown();
}

/// 2a: every supported level is verifiable from the role's own log at startup, and the
/// saved value is the one reported — the meta record is written even at `warn`/`error`,
/// where its own `info` severity would otherwise be filtered.
#[test]
fn every_supported_level_is_verifiable_from_the_log() {
    for (level, saved) in [
        (DiagnosticLevel::Debug, "debug"),
        (DiagnosticLevel::Info, "info"),
        (DiagnosticLevel::Warn, "warn"),
        (DiagnosticLevel::Error, "error"),
    ] {
        let temp = crate::private_tempdir();
        let root = temp.path().join("diagnostics");
        write_settings(&root, 3, saved);
        let (runtime, plan) = DiagnosticRuntime::prepare(config(&root));
        arm(plan);
        assert_eq!(runtime.handle().level(), Some(level));

        let records = wait_for_kind(&root, "level_applied");
        assert_eq!(
            applied_levels(&records),
            [(level, 3, LevelSource::Persisted)],
            "the log states the saved level {saved}"
        );
        assert_eq!(
            records.last().expect("the meta record").level,
            DiagnosticLevel::Info,
            "the meta record keeps its info severity at {saved}"
        );
        runtime.shutdown();
    }
}

/// 2a: a running process states every level change it applies, and the records of the
/// previous threshold do not follow it: `info` records admitted at `info` are filtered
/// once the process runs at `warn`, and `warn` records once it runs at `error`.
#[test]
fn a_level_change_stays_verifiable_when_it_filters_its_own_severity() {
    let temp = crate::private_tempdir();
    let root = temp.path().join("diagnostics");
    write_settings(&root, 1, "info");
    let (runtime, plan) = DiagnosticRuntime::prepare(config(&root));
    arm(plan);
    runtime.emit(routine_info());

    runtime.apply_saved_level(DiagnosticLevel::Warn, 2);
    runtime.emit(routine_info());
    runtime.emit(routine_warning());

    runtime.apply_saved_level(DiagnosticLevel::Error, 3);
    runtime.emit(routine_info());
    runtime.emit(routine_warning());

    let records = wait_for_count(&root, "level_applied", 3);
    assert_eq!(
        applied_levels(&records),
        [
            (DiagnosticLevel::Info, 1, LevelSource::Persisted),
            (DiagnosticLevel::Warn, 2, LevelSource::Persisted),
            (DiagnosticLevel::Error, 3, LevelSource::Persisted),
        ],
        "each applied level is readable from the log"
    );
    let kinds: Vec<&str> = records.iter().map(|record| record.event.kind()).collect();
    assert_eq!(
        kinds.iter().filter(|kind| **kind == "startup_end").count(),
        1,
        "only the info record admitted while the process ran at info is written: {kinds:?}"
    );
    assert_eq!(
        kinds.iter().filter(|kind| **kind == "writer_stats").count(),
        1,
        "the warn record admitted at warn is written, the one emitted at error is not: {kinds:?}"
    );
    assert_eq!(
        runtime.handle().counters().expect("counters").filtered(),
        3,
        "the two info records and the warn record below the threshold are counted"
    );
    runtime.shutdown();
}

/// An unconfigured runtime must actually write early Debug events in development,
/// filter them in release, and leave the settings file absent in both profiles.
#[test]
fn unconfigured_runtime_uses_build_default_without_persisting() {
    let temp = crate::private_tempdir();
    let root = temp.path().join("diagnostics");
    let (runtime, plan) = DiagnosticRuntime::prepare(config(&root));
    runtime.stage_begin(StartupStage::ArtifactValidate);
    arm(plan);
    runtime.emit(routine_info());
    let records = wait_for_kind(&root, "startup_end");
    let expected = if cfg!(debug_assertions) {
        DiagnosticLevel::Debug
    } else {
        DiagnosticLevel::Info
    };
    assert_eq!(runtime.status().level, expected);
    assert_eq!(
        applied_levels(&records),
        [(expected, 0, LevelSource::Default)]
    );
    assert_eq!(
        records
            .iter()
            .any(|record| record.event.kind() == "stage_begin"),
        cfg!(debug_assertions)
    );
    assert!(!root.join(SETTINGS_FILE).exists());
    runtime.shutdown();
}

#[test]
fn publication_timings_preserve_results_and_only_emit_at_debug() {
    use crate::publication::{PublicationStage, measure};
    for level in ["debug", "info"] {
        let temp = crate::private_tempdir();
        let root = temp.path().join("diagnostics");
        write_settings(&root, 1, level);
        let (runtime, plan) = DiagnosticRuntime::prepare(config(&root));
        arm(plan);
        let port = runtime.port();
        let result: Result<u32, &str> = measure(
            &port,
            PublicationStage::OperationDecode,
            Some("private-operation-id"),
            Some(830_000),
            || Err("private-error-payload"),
        );
        assert_eq!(result, Err("private-error-payload"));
        let result: Result<u32, &str> =
            measure(&port, PublicationStage::OperationRead, None, None, || {
                Ok(17)
            });
        assert_eq!(result, Ok(17));
        runtime.shutdown();
        let records = role_log_records(&root);
        let timings: Vec<_> = records
            .iter()
            .filter_map(|r| match &r.event {
                DiagnosticEvent::PublicationTiming(t) => Some(t),
                _ => None,
            })
            .collect();
        if level == "debug" {
            assert_eq!(timings.len(), 2);
            assert!(!timings[0].ok);
            assert_eq!(timings[0].operation_bytes, Some(830_000));
            assert!(timings[0].operation_token.is_some());
            assert!(timings[1].ok);
        } else {
            assert!(timings.is_empty());
        }
        let encoded = serde_json::to_string(&records).unwrap();
        assert!(!encoded.contains("private-operation-id"));
        assert!(!encoded.contains("private-error-payload"));
    }
}

#[test]
fn agent_read_timings_are_bounded_counts_and_debug_only() {
    use crate::agent_read::{AgentReadCategory, AgentReadStage, Timer};
    for level in ["debug", "info"] {
        let temp = crate::private_tempdir();
        let root = temp.path().join("diagnostics");
        write_settings(&root, 1, level);
        let (runtime, plan) = DiagnosticRuntime::prepare(config(&root));
        arm(plan);
        {
            let mut timer = Timer::start(
                &runtime.port(),
                AgentReadCategory::Model,
                AgentReadStage::GrantAuthentication,
            );
            timer.counts.operations = 1;
            timer.counts.operation_bytes = 1024;
            timer.counts.payloads = 2;
            timer.counts.payload_bytes = 4096;
            timer.counts.grant_authentications = 1;
        }
        runtime.shutdown();
        let records = role_log_records(&root);
        let samples: Vec<_> = records
            .iter()
            .filter_map(|record| match &record.event {
                DiagnosticEvent::AgentReadTiming(timing) => Some(timing),
                _ => None,
            })
            .collect();
        if level == "debug" {
            assert_eq!(samples.len(), 1);
            assert_eq!(samples[0].counts.operations, 1);
            assert_eq!(samples[0].counts.payloads, 2);
            assert_eq!(samples[0].counts.payload_bytes, 4096);
            assert_eq!(samples[0].counts.grant_authentications, 1);
            let encoded = serde_json::to_value(samples[0]).unwrap();
            assert_eq!(encoded.as_object().unwrap().len(), 4);
            assert!(serde_json::from_value::<crate::agent_read::AgentReadTiming>(serde_json::json!({
                "category":"model", "stage":"native_read", "elapsed_us":0,
                "counts":{"operations":0,"operation_bytes":0,"payloads":0,"payload_bytes":0,"grant_authentications":0},
                "path":"sensitive"
            })).is_err());
        } else {
            assert!(samples.is_empty());
        }
    }
}
