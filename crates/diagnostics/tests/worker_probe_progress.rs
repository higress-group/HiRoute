//! The Worker installation step snapshot against real files.
//!
//! A step is entered before its blocking call, the bounded heartbeat repeats only the step that
//! is still current, and an end is recorded once from the real return. The step snapshot is a
//! separate slot from the startup stage, so the two can never clear each other in either
//! direction. This is what makes a paused probe attributable from the log alone.

#![cfg(unix)]

mod support;

use std::path::Path;
use std::time::{Duration, Instant};

use hiroute_diagnostics::event::{
    DiagnosticEvent, HarnessKind, ProcessRole, StageOutcome, StartupStage, WorkerStage,
    WorkerStageKind, WorkerStageOutcome,
};
use hiroute_diagnostics::level::DiagnosticLevel;
use hiroute_diagnostics::record::{Component, DiagnosticRecordV1};
use hiroute_diagnostics::runtime::{DiagnosticRuntime, RuntimeConfig};

fn worker_steps(path: &Path, kind: WorkerStageKind) -> Vec<WorkerStage> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut steps = Vec::new();
    for line in content.lines() {
        let Ok(record) = DiagnosticRecordV1::parse_line(line.as_bytes()) else {
            continue;
        };
        if let DiagnosticEvent::WorkerStage(stage) = record.event
            && stage.stage == kind
        {
            steps.push(stage);
        }
    }
    steps
}

fn stage_ends(path: &Path, stage: StartupStage) -> Vec<u64> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut ends = Vec::new();
    for line in content.lines() {
        let Ok(record) = DiagnosticRecordV1::parse_line(line.as_bytes()) else {
            continue;
        };
        if let DiagnosticEvent::StageEnd(end) = record.event
            && end.stage == stage
        {
            ends.push(end.elapsed_ms);
        }
    }
    ends
}

fn wait_until(deadline: Instant, mut condition: impl FnMut() -> bool) -> bool {
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    condition()
}

#[test]
fn a_running_worker_step_republishes_and_never_clears_the_startup_stage() {
    let temp = support::private_tempdir();
    let runtime = DiagnosticRuntime::start(RuntimeConfig {
        root: temp.path().join("diagnostics"),
        role: ProcessRole::Daemon,
        component: Component::Worker,
        parent_session_id: None,
        level_override: Some(DiagnosticLevel::Debug),
    });
    let log = temp
        .path()
        .join("diagnostics")
        .join("daemon")
        .join("current.jsonl");
    let port = runtime.port();

    runtime.stage_begin(StartupStage::WorkerRevalidate);
    std::thread::sleep(Duration::from_millis(20));
    port.worker_stage_begin(HarnessKind::Codex, WorkerStageKind::ProbeLoad);
    port.worker_stage_note(HarnessKind::Codex, WorkerStageKind::ProbeSession);
    // The outer startup stage ends while the inner probe step is still running; that end must
    // not clear the step snapshot.
    runtime.stage_end(StartupStage::WorkerRevalidate, StageOutcome::Completed);
    let heartbeat_deadline = Instant::now() + Duration::from_secs(12);
    assert!(
        wait_until(heartbeat_deadline, || {
            worker_steps(&log, WorkerStageKind::ProbeLoad)
                .iter()
                .any(|step| step.outcome == WorkerStageOutcome::Entered && step.elapsed_ms >= 250)
        }),
        "the running step must be republished by the bounded heartbeat"
    );
    // Only now does the real end exist.
    port.worker_stage_end(
        HarnessKind::Codex,
        WorkerStageKind::ProbeLoad,
        WorkerStageOutcome::Completed,
    );
    // Shutdown deliberately bounds its wait for the writer; it is not a persistence
    // barrier on a slow filesystem. Observe the real end before checking its ordering.
    assert!(
        wait_until(Instant::now() + Duration::from_secs(12), || {
            worker_steps(&log, WorkerStageKind::ProbeLoad)
                .iter()
                .any(|step| step.outcome == WorkerStageOutcome::Completed)
        }),
        "the real step end must be written before validating the lifecycle"
    );
    runtime.shutdown();

    let steps = worker_steps(&log, WorkerStageKind::ProbeLoad);
    assert_eq!(
        steps
            .iter()
            .filter(|step| step.outcome == WorkerStageOutcome::Completed)
            .count(),
        1,
        "{steps:?}"
    );
    let end = steps.last().expect("the real end is recorded");
    assert_eq!(end.outcome, WorkerStageOutcome::Completed);
    assert!(end.elapsed_ms >= 250, "{steps:?}");
    let entered = steps
        .iter()
        .filter(|step| step.outcome == WorkerStageOutcome::Entered)
        .count();
    assert!(
        (2..=3).contains(&entered),
        "the entry plus at most one heartbeat repeat before the real end: {steps:?}"
    );
    assert_eq!(steps[0].elapsed_ms, 0, "the entry measures no time yet");
    assert_eq!(
        steps
            .iter()
            .filter(|step| matches!(step.outcome, WorkerStageOutcome::Observed))
            .count(),
        0,
        "the session milestone belongs to its own stage kind: {steps:?}"
    );
    let session = worker_steps(&log, WorkerStageKind::ProbeSession);
    assert_eq!(session.len(), 1, "{session:?}");
    assert_eq!(session[0].outcome, WorkerStageOutcome::Observed);
    assert!(
        session[0].elapsed_ms <= end.elapsed_ms,
        "a milestone is measured inside the running step: {session:?} {steps:?}"
    );
    let startup_ends = stage_ends(&log, StartupStage::WorkerRevalidate);
    assert_eq!(startup_ends.len(), 1, "{startup_ends:?}");
    assert!(
        startup_ends[0] >= 20,
        "the outer startup stage kept its own elapsed time: {startup_ends:?}"
    );
}
