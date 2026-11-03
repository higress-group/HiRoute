#![cfg(unix)]

//! Actual CLI/Local Control composition; real Worker execution is separate acceptance.

use std::collections::VecDeque;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hiroute_application::{
    ApplicationService,
    delegation::tasks::{DelegationTaskPort, DelegationTasks},
};
use hiroute_application_api::{
    DelegationResultV1, DelegationRunViewV1, DelegationWaitV1, RunStateV1, WorkerResultRequestV1,
    WorkerWaitRequestV1,
};
use hiroute_daemon::control::{ProductionControlRuntime, start_control};
use hiroute_domain::delegation::DelegationErrorV1;
use serde_json::{Value, json};

struct Observation {
    run: DelegationRunViewV1,
    delay: Duration,
    timed_out: bool,
}

fn observation(state: RunStateV1, revision: u64) -> Observation {
    Observation {
        run: serde_json::from_value(json!({
            "task_id":"task/wait", "run_id":"run/wait", "ordinal":1,
            "continued_from":null, "admission_sequence":1, "accepted_at_ms":1,
            "accepted_time_state":"recorded",
            "executor":{"harness":null,"display_name":null,"basis":"unavailable"},
            "state":state, "state_revision":revision, "cleanup":"complete",
            "result_available":state == RunStateV1::Succeeded,
            "scope":{"canonical_cwd":"/fixture", "permission_policy":"approve_all", "deadline_ms":1000}
        }))
        .unwrap(),
        delay: Duration::ZERO,
        timed_out: false,
    }
}

struct Tasks {
    observations: Mutex<VecDeque<Result<Observation, DelegationErrorV1>>>,
    calls: Mutex<Vec<WorkerWaitRequestV1>>,
    results: Mutex<Vec<String>>,
    result_error: Option<DelegationErrorV1>,
    cancels: AtomicUsize,
}

impl Tasks {
    fn new(observations: impl IntoIterator<Item = Observation>) -> Self {
        Self {
            observations: Mutex::new(observations.into_iter().map(Ok).collect()),
            calls: Mutex::new(Vec::new()),
            results: Mutex::new(Vec::new()),
            result_error: None,
            cancels: AtomicUsize::new(0),
        }
    }

    fn revisions(&self) -> Vec<Option<u64>> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.after_revision)
            .collect()
    }
}

impl DelegationTaskPort for Tasks {
    fn worker_cancel(
        &self,
        _request: &hiroute_application_api::WorkerCancelRequestV1,
    ) -> Result<hiroute_application_api::DelegationCancelV1, DelegationErrorV1> {
        self.cancels.fetch_add(1, Ordering::SeqCst);
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    fn worker_wait(
        &self,
        request: &WorkerWaitRequestV1,
    ) -> Result<DelegationWaitV1, DelegationErrorV1> {
        assert_eq!(request.run_id, "run/wait");
        self.calls.lock().unwrap().push(request.clone());
        let observation = self
            .observations
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected extra observation")?;
        std::thread::sleep(observation.delay);
        Ok(DelegationWaitV1 {
            schema: "hiroute.delegation-wait/v1".into(),
            changed: request
                .after_revision
                .is_none_or(|r| observation.run.state_revision > r),
            run: observation.run,
            timed_out: observation.timed_out,
        })
    }

    fn worker_result(
        &self,
        request: &WorkerResultRequestV1,
    ) -> Result<DelegationResultV1, DelegationErrorV1> {
        self.results.lock().unwrap().push(request.run_id.clone());
        if let Some(error) = self.result_error {
            return Err(error);
        }
        Ok(DelegationResultV1 {
            schema: "hiroute.delegation-result/v1".into(),
            run: observation(RunStateV1::Succeeded, 9).run,
            text: Some("completed result\n".into()),
            next_offset: None,
            incomplete: false,
        })
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    home: std::path::PathBuf,
    runtime: std::path::PathBuf,
}

impl Fixture {
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_hiroute"));
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.home)
            .env("HIROUTE_RUNTIME_DIR", &self.runtime)
            .args(["worker", "wait", "--run", "run/wait", "--output", "json"]);
        command
    }
}

fn with_control(tasks: Tasks, run: impl FnOnce(&Fixture, &Arc<Tasks>)) {
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let fixture = Fixture {
        home: root_path.join("home"),
        runtime: root_path.join("run"),
        _root: root,
    };
    std::fs::create_dir(&fixture.home).unwrap();
    let runtime = ProductionControlRuntime::open(root_path.join("storage")).unwrap();
    let tasks = Arc::new(tasks);
    let ports = runtime
        .application_ports()
        .with_delegation_tasks(Arc::new(DelegationTasks::new(tasks.clone())));
    let mut control = start_control(ApplicationService::new(ports), &fixture.runtime).unwrap();
    run(&fixture, &tasks);
    control.shutdown();
    control.join(Duration::from_secs(5)).unwrap();
}

fn response(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn completion_crosses_progress_and_bounded_timeouts_and_reads_the_result_once() {
    let mut timeout = observation(RunStateV1::Running, 2);
    timeout.timed_out = true;
    with_control(
        Tasks::new([
            observation(RunStateV1::Accepted, 1),
            observation(RunStateV1::Running, 2),
            timeout,
            observation(RunStateV1::Succeeded, 3),
        ]),
        |fixture, tasks| {
            let output = fixture.command().output().unwrap();
            assert!(output.status.success(), "{output:?}");
            let json = response(&output);
            assert_eq!(json["data"]["operation"], "wait");
            assert_eq!(json["data"]["run_state"], "succeeded");
            assert_eq!(json["data"]["result"], "completed result\n");
            assert_eq!(json["data"]["timed_out"], false);
            assert_eq!(tasks.revisions(), [None, Some(1), Some(2), Some(2)]);
            assert!(
                tasks
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|r| r.wait_timeout_secs == 30)
            );
            assert_eq!(*tasks.results.lock().unwrap(), ["run/wait"]);
        },
    );
}

#[test]
fn an_already_succeeded_run_returns_its_result_immediately() {
    with_control(
        Tasks::new([observation(RunStateV1::Succeeded, 9)]),
        |fixture, tasks| {
            let output = fixture.command().output().unwrap();
            assert!(output.status.success());
            assert_eq!(response(&output)["data"]["result"], "completed result\n");
            assert_eq!(tasks.revisions(), [None]);
        },
    );
}

#[test]
fn completion_does_not_wait_for_unavailable_result_content() {
    let mut success = observation(RunStateV1::Succeeded, 9);
    success.run.result_available = false;
    with_control(Tasks::new([success]), |fixture, tasks| {
        let output = fixture.command().output().unwrap();
        assert!(output.status.success());
        let json = response(&output);
        assert_eq!(json["data"]["run_state"], "succeeded");
        assert!(json["data"]["result"].is_null());
        assert!(tasks.results.lock().unwrap().is_empty());
        assert_eq!(tasks.cancels.load(Ordering::SeqCst), 0);
    });
}

#[test]
fn all_unsuccessful_terminal_states_end_completion_with_their_failure_classification() {
    for (state, reason) in [
        (RunStateV1::Failed, "worker.execution.failure_not_recorded"),
        (RunStateV1::Cancelled, "worker.execution.cancelled"),
        (RunStateV1::Unknown, "worker.execution.outcome_unknown"),
    ] {
        with_control(
            Tasks::new([observation(RunStateV1::Running, 1), observation(state, 2)]),
            |fixture, tasks| {
                let output = fixture.command().output().unwrap();
                assert_eq!(output.status.code(), Some(1));
                let json = response(&output);
                assert_eq!(json["error"]["code"], "WORKER_EXECUTION_FAILED");
                assert_eq!(json["error"]["message_key"], reason);
                assert_eq!(json["data"]["task_id"], "task/wait");
                assert_eq!(json["data"]["run_id"], "run/wait");
                assert!(tasks.results.lock().unwrap().is_empty());
            },
        );
    }
}

#[test]
fn recorded_terminal_failure_keeps_its_stage_and_reason() {
    let mut failure = observation(RunStateV1::Failed, 2);
    failure.run.failure = Some(hiroute_application_api::WorkerExecutionFailureV1 {
        stage: hiroute_application_api::WorkerExecutionStageV1::Initialize,
        error: DelegationErrorV1::ProtocolFailed,
    });
    with_control(Tasks::new([failure]), |fixture, _| {
        let output = fixture.command().output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        let json = response(&output);
        assert_eq!(
            json["error"]["message_key"],
            "worker.execution.initialize.protocol_failed"
        );
        assert_eq!(json["data"]["failure"]["stage"], "initialize");
    });
}

#[test]
fn explicit_timeout_is_one_total_budget_even_with_intermediate_progress() {
    let mut progress = observation(RunStateV1::Running, 2);
    progress.delay = Duration::from_millis(650);
    let mut success = observation(RunStateV1::Succeeded, 3);
    success.delay = Duration::from_secs(2);
    with_control(
        Tasks::new([observation(RunStateV1::Running, 1), progress, success]),
        |fixture, tasks| {
            let start = Instant::now();
            let output = fixture
                .command()
                .args(["--wait-timeout", "1"])
                .output()
                .unwrap();
            let elapsed = start.elapsed();
            assert!(output.status.success(), "{output:?}");
            let json = response(&output);
            assert_eq!(json["data"]["run_state"], "running");
            assert_eq!(json["data"]["timed_out"], true);
            assert!(elapsed >= Duration::from_millis(900), "{elapsed:?}");
            assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
            assert_eq!(tasks.revisions(), [None, Some(1), Some(2)]);
            assert!(tasks.results.lock().unwrap().is_empty());
        },
    );
}

#[test]
fn revision_mode_preserves_one_observation_and_the_thirty_second_default() {
    with_control(
        Tasks::new([observation(RunStateV1::Running, 8)]),
        |fixture, tasks| {
            let output = fixture
                .command()
                .args(["--after-revision", "7"])
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(response(&output)["data"]["run_state"], "running");
            assert_eq!(tasks.revisions(), [Some(7)]);
            assert_eq!(tasks.calls.lock().unwrap()[0].wait_timeout_secs, 30);
        },
    );
}

#[test]
fn revision_mode_preserves_terminal_output_and_explicit_timeout_alias() {
    for state in [RunStateV1::Succeeded, RunStateV1::Failed] {
        with_control(Tasks::new([observation(state, 8)]), |fixture, tasks| {
            let output = fixture
                .command()
                .args(["--after-revision", "7", "--wait-timeout-secs", "2"])
                .output()
                .unwrap();
            assert!(output.status.success());
            assert!(response(&output)["data"]["result"].is_null());
            assert_eq!(tasks.calls.lock().unwrap()[0].wait_timeout_secs, 2);
            assert!(tasks.results.lock().unwrap().is_empty());
        });
    }
}

#[test]
fn daemon_errors_during_wait_or_result_are_reported_without_retry() {
    let tasks = Tasks::new([observation(RunStateV1::Running, 1)]);
    tasks
        .observations
        .lock()
        .unwrap()
        .push_back(Err(DelegationErrorV1::StorageUnavailable));
    with_control(tasks, |fixture, tasks| {
        let output = fixture.command().output().unwrap();
        assert!(!output.status.success());
        assert!(response(&output)["error"].is_object());
        assert_eq!(tasks.revisions(), [None, Some(1)]);
    });
    let mut tasks = Tasks::new([observation(RunStateV1::Succeeded, 9)]);
    tasks.result_error = Some(DelegationErrorV1::StorageUnavailable);
    with_control(tasks, |fixture, tasks| {
        let output = fixture.command().output().unwrap();
        assert!(!output.status.success());
        assert!(response(&output)["error"].is_object());
        assert_eq!(*tasks.results.lock().unwrap(), ["run/wait"]);
    });
}

#[test]
fn transport_deadline_is_a_failure_and_is_not_retried() {
    let mut delayed = observation(RunStateV1::Running, 2);
    delayed.delay = Duration::from_secs(2);
    with_control(
        Tasks::new([observation(RunStateV1::Running, 1), delayed]),
        |fixture, tasks| {
            let output = fixture
                .command()
                .args(["--timeout", "1s"])
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert_eq!(response(&output)["error"]["code"], "DAEMON_UNAVAILABLE");
            assert_eq!(tasks.revisions(), [None, Some(1)]);
        },
    );
}

#[test]
fn stopping_the_cli_does_not_issue_worker_cancellation() {
    let mut delayed = observation(RunStateV1::Running, 2);
    delayed.delay = Duration::from_secs(2);
    with_control(
        Tasks::new([observation(RunStateV1::Running, 1), delayed]),
        |fixture, tasks| {
            let mut child = fixture
                .command()
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            while tasks.calls.lock().unwrap().len() < 2 && Instant::now() < deadline {
                assert!(
                    child.try_wait().unwrap().is_none(),
                    "wait exited before the run ended"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(child.id() as i32),
                nix::sys::signal::Signal::SIGINT,
            )
            .unwrap();
            assert_eq!(child.wait().unwrap().signal(), Some(2));
            assert_eq!(tasks.revisions(), [None, Some(1)]);
            assert_eq!(tasks.cancels.load(Ordering::SeqCst), 0);
        },
    );
}

#[test]
fn wait_timeout_keeps_its_existing_range() {
    with_control(Tasks::new([]), |fixture, tasks| {
        for value in ["0", "31"] {
            let output = fixture
                .command()
                .args(["--wait-timeout", value])
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert_eq!(response(&output)["error"]["code"], "INVALID_ARGUMENTS");
        }
        assert!(tasks.calls.lock().unwrap().is_empty());
    });
}
