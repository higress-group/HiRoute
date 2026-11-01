#![cfg(unix)]

//! Real CLI and Local Control boundary. The controlled owner records requests;
//! durable daemon admission and native Main behavior have separate acceptance.
use hiroute_application::{
    ApplicationService,
    delegation::tasks::{DelegationTaskPort, DelegationTasks},
};
use hiroute_application_api::{
    ClientHelloV1, DelegationAcceptedV1, DelegationGetV1, DelegationListV1,
    LocalControlWireRequestV2, RunStateV1, WorkerContinueRequestV1, WorkerExecRequestV1,
    WorkerListRequestV1, WorkerStatusRequestV1, negotiate_hello,
};
use hiroute_daemon::control::{ProductionControlRuntime, start_control};
use hiroute_domain::delegation::DelegationErrorV1;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::{fs::PermissionsExt, net::UnixListener};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Tasks {
    calls: Mutex<Vec<(bool, String, Value)>>,
}

impl Tasks {
    fn submit(
        &self,
        continuation: bool,
        key: &str,
        request: Value,
    ) -> Result<DelegationAcceptedV1, DelegationErrorV1> {
        let mut calls = self.calls.lock().unwrap();
        let previous = calls
            .iter()
            .position(|(op, old, _)| *op == continuation && old == key);
        let index = previous.unwrap_or(calls.len());
        let conflict = previous.is_some_and(|index| calls[index].2 != request);
        calls.push((continuation, key.to_owned(), request));
        if conflict {
            return Err(DelegationErrorV1::Conflict);
        }
        Ok(DelegationAcceptedV1 {
            schema: "hiroute.delegation-accepted/v1".into(),
            task_id: "task/fixture".into(),
            run_id: format!("run/{}", index + 1),
            title: Some("Recovery task".into()),
            state: RunStateV1::Accepted,
            state_revision: 1,
            replayed: previous.is_some(),
        })
    }

    fn task(&self) -> hiroute_application_api::DelegationTaskViewV1 {
        serde_json::from_value(json!({
            "task_id":"task/fixture", "created_at_ms":1, "latest_admission_sequence":1,
            "title":"Recovery task", "brief":"goal", "content_availability":"available",
            "plan_id":"plan/submission-fixture", "plan_revision":1, "latest_run_id":"run/1",
            "run":{
                "task_id":"task/fixture", "run_id":"run/1", "ordinal":1,
                "continued_from":null, "admission_sequence":1, "accepted_at_ms":1,
                "accepted_time_state":"recorded",
                "executor":{"harness":null,"display_name":null,"basis":"unavailable"},
                "state":"succeeded", "state_revision":2, "cleanup":"complete",
                "result_available":true,
                "scope":{"canonical_cwd":"/fixture", "permission_policy":"approve_all", "deadline_ms":1000}
            },
            "resumable_until_ms":null, "session_ids":[], "session_links_complete":true
        }))
        .unwrap()
    }
}

impl DelegationTaskPort for Tasks {
    fn worker_exec(
        &self,
        request: &WorkerExecRequestV1,
    ) -> Result<DelegationAcceptedV1, DelegationErrorV1> {
        self.submit(
            false,
            &request.submission_key,
            serde_json::to_value(request).unwrap(),
        )
    }

    fn worker_continue(
        &self,
        request: &WorkerContinueRequestV1,
    ) -> Result<DelegationAcceptedV1, DelegationErrorV1> {
        self.submit(
            true,
            &request.submission_key,
            serde_json::to_value(request).unwrap(),
        )
    }

    fn worker_status(
        &self,
        request: &WorkerStatusRequestV1,
    ) -> Result<DelegationGetV1, DelegationErrorV1> {
        assert!(
            self.calls
                .lock()
                .unwrap()
                .iter()
                .any(|(_, key, _)| { request.submission_key.as_deref() == Some(key.as_str()) })
        );
        Ok(DelegationGetV1 {
            schema: "hiroute.delegation-get/v1".into(),
            task: self.task(),
        })
    }

    fn worker_list(
        &self,
        request: &WorkerListRequestV1,
    ) -> Result<DelegationListV1, DelegationErrorV1> {
        assert_eq!(request.title.as_deref(), Some("Recovery task"));
        Ok(DelegationListV1 {
            schema: "hiroute.delegation-list/v1".into(),
            tasks: vec![self.task()],
            next_cursor: None,
        })
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    _home: tempfile::TempDir,
    home: PathBuf,
    runtime: PathBuf,
    caller: PathBuf,
    worker: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        std::fs::create_dir_all(env!("CARGO_TARGET_TMPDIR")).unwrap();
        let home = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
        let path = root.path().canonicalize().unwrap();
        let home_path = home.path().canonicalize().unwrap();
        let caller = path.join("caller");
        let worker = path.join("worker");
        std::fs::create_dir(&caller).unwrap();
        std::fs::create_dir(&worker).unwrap();
        Self {
            _root: root,
            _home: home,
            home: home_path,
            runtime: path.join("run"),
            caller,
            worker,
        }
    }

    fn command(&self) -> Command {
        let binary = std::env::var_os("HIROUTE_WORKER_SUBMISSION_REGRESSION_CLI")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_hiroute").into());
        let mut command = Command::new(binary);
        self.configure(&mut command);
        command
    }

    fn configure(&self, command: &mut Command) {
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.home)
            .env("HIROUTE_RUNTIME_DIR", &self.runtime)
            .current_dir(&self.caller);
    }

    fn exec_args(&self, key: Option<&str>, goal: &str) -> Vec<String> {
        let mut args = vec![
            "worker",
            "exec",
            "--plan",
            "plan/submission-fixture",
            "--cwd",
            self.worker.to_str().unwrap(),
            "--title",
            "Recovery task",
            "--no-wait",
            "--output",
            "json",
        ];
        if let Some(key) = key {
            args.extend(["--submission-key", key]);
        }
        args.extend(["--", goal]);
        args.into_iter().map(str::to_owned).collect()
    }

    fn assert_no_workspace_receipt(&self) {
        assert!(!self.caller.join(".hiroute").exists());
        assert!(!self.worker.join(".hiroute").exists());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.caller, std::fs::Permissions::from_mode(0o700));
    }
}

fn with_control(run: impl FnOnce(&Fixture, &Tasks)) {
    let fixture = Fixture::new();
    let runtime = ProductionControlRuntime::open(fixture._root.path().join("storage")).unwrap();
    let tasks = Arc::new(Tasks::default());
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
fn obsolete_receipt_configuration_and_readonly_cwd_do_not_block_exec_replay_or_continue() {
    with_control(|fixture, tasks| {
        let legacy = fixture.home.join("legacy-receipts");
        std::fs::write(&legacy, b"existing-private-receipt-sentinel").unwrap();
        let original = std::fs::metadata(&legacy).unwrap().modified().unwrap();
        std::fs::set_permissions(&fixture.caller, std::fs::Permissions::from_mode(0o500)).unwrap();
        let invoke = |args: Vec<String>| {
            response(
                &fixture
                    .command()
                    .env("HIROUTE_WORKER_RECEIPT_DIR", &legacy)
                    .args(args)
                    .output()
                    .unwrap(),
            )
        };
        let first = invoke(fixture.exec_args(Some("submission/original"), "goal"));
        assert_eq!(first["status"], "succeeded", "{first}");
        assert_eq!(first["data"]["submission_key"], "submission/original");
        let replay = invoke(fixture.exec_args(Some("submission/original"), "goal"));
        assert_eq!(replay["data"]["replayed"], true);
        assert_eq!(replay["data"]["run_id"], first["data"]["run_id"]);
        let changed = invoke(fixture.exec_args(Some("submission/original"), "different goal"));
        assert_eq!(changed["error"]["code"], "REVISION_CONFLICT");
        assert_eq!(
            tasks.calls.lock().unwrap().len(),
            3,
            "conflict is owned by the server"
        );
        let continued = invoke(
            [
                "worker",
                "continue",
                "--task",
                "task/fixture",
                "--expected-latest-run",
                "run/1",
                "--submission-key",
                "submission/continued",
                "--no-wait",
                "--output",
                "json",
                "--",
                "continue goal",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        );
        assert_eq!(continued["status"], "succeeded", "{continued}");
        assert_eq!(continued["data"]["submission_key"], "submission/continued");
        for (_, _, request) in tasks.calls.lock().unwrap().iter() {
            assert_eq!(request["permission_policy"], "approve_all");
        }
        assert_eq!(
            std::fs::read(&legacy).unwrap(),
            b"existing-private-receipt-sentinel"
        );
        assert_eq!(
            std::fs::metadata(&legacy).unwrap().modified().unwrap(),
            original
        );
        fixture.assert_no_workspace_receipt();
        assert_eq!(std::fs::read_dir(&fixture.home).unwrap().count(), 1);
    });
}

#[test]
fn explicit_permission_options_fail_actionably_before_daemon_access() {
    let fixture = Fixture::new();
    for command in ["exec", "continue"] {
        for output in ["json", "text", "quiet"] {
            for option in [
                vec!["--permission-policy", "approve-all"],
                vec!["--permission-policy", "approve-reads"],
                vec!["--permission-policy", "deny-all"],
                vec!["--permission-policy", "invalid"],
                vec!["--permission-policy=approve-reads"],
                vec!["--permission-policy"],
            ] {
                let result = fixture
                    .command()
                    .args(["worker", command, "--output", output])
                    .args(&option)
                    .output()
                    .unwrap();
                assert_eq!(result.status.code(), Some(2), "{command}: {option:?}");
                let stderr = String::from_utf8_lossy(&result.stderr);
                assert!(stderr.contains("temporarily unsupported"), "{stderr}");
                assert!(stderr.contains("Remove this argument"), "{stderr}");
                assert!(stderr.contains("approve-all by default"), "{stderr}");
                if output == "json" {
                    let envelope = response(&result);
                    assert_eq!(envelope["status"], "usage_error");
                    assert_eq!(envelope["error"]["code"], "INVALID_ARGUMENTS");
                    assert_eq!(
                        envelope["error"]["message_key"],
                        "worker.error.permission_policy_unsupported_remove_argument"
                    );
                }
                assert!(
                    !fixture.runtime.exists(),
                    "must reject before daemon access"
                );
                fixture.assert_no_workspace_receipt();
            }
        }
    }
}

#[test]
fn permission_option_text_in_a_prompt_or_title_is_preserved_as_content() {
    with_control(|fixture, tasks| {
        let mut args = fixture.exec_args(Some("submission/literal-option"), "--permission-policy");
        let title = args.iter().position(|arg| arg == "--title").unwrap() + 1;
        args[title] = "--permission-policy".into();
        let result = fixture.command().args(args).output().unwrap();
        assert!(result.status.success(), "{result:?}");
        let calls = tasks.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].2["permission_policy"], "approve_all");
        assert_eq!(calls[0].2["input"]["goal"], "--permission-policy");
        assert_eq!(calls[0].2["title"], "--permission-policy");
    });
}

#[test]
fn generated_key_is_returned_and_completed_task_can_be_queried_by_key_or_title() {
    with_control(|fixture, tasks| {
        let first = response(
            &fixture
                .command()
                .args(fixture.exec_args(None, "goal"))
                .output()
                .unwrap(),
        );
        let key = first["data"]["submission_key"].as_str().unwrap();
        let status = response(
            &fixture
                .command()
                .args([
                    "worker",
                    "status",
                    "--submission-key",
                    key,
                    "--operation",
                    "start",
                    "--output",
                    "json",
                ])
                .output()
                .unwrap(),
        );
        assert_eq!(status["data"]["task_id"], first["data"]["task_id"]);
        assert_eq!(status["data"]["run_state"], "succeeded");
        let listed = response(
            &fixture
                .command()
                .args([
                    "worker",
                    "list",
                    "--title",
                    "Recovery task",
                    "--output",
                    "json",
                ])
                .output()
                .unwrap(),
        );
        assert_eq!(
            listed["data"]["tasks"][0]["task_id"],
            first["data"]["task_id"]
        );
        assert_eq!(
            tasks.calls.lock().unwrap().len(),
            1,
            "queries never resubmit work"
        );
        let second = response(
            &fixture
                .command()
                .args(fixture.exec_args(None, "goal"))
                .output()
                .unwrap(),
        );
        assert_ne!(second["data"]["submission_key"], key);
        assert_eq!(std::fs::read_dir(&fixture.home).unwrap().count(), 0);
        fixture.assert_no_workspace_receipt();
    });
}

#[test]
fn lost_response_keeps_the_key_and_recovery_action_without_resubmitting() {
    for key in [None, Some("submission/uncertain")] {
        let fixture = Fixture::new();
        let directory = fixture.runtime.join("hiroute");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let endpoint = directory.join("control.sock");
        let listener = UnixListener::bind(&endpoint).unwrap();
        std::fs::set_permissions(endpoint, std::fs::Permissions::from_mode(0o600)).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut stream = BufReader::new(stream);
            let mut line = String::new();
            stream.read_line(&mut line).unwrap();
            let hello: ClientHelloV1 = serde_json::from_str(&line).unwrap();
            let mut reply = serde_json::to_vec(&negotiate_hello(&hello).unwrap()).unwrap();
            reply.push(b'\n');
            stream.get_mut().write_all(&reply).unwrap();
            line.clear();
            stream.read_line(&mut line).unwrap();
            let request: LocalControlWireRequestV2 = serde_json::from_str(&line).unwrap();
            assert_eq!(request.operation_id, "WorkerExec");
            request.payload["submission_key"]
                .as_str()
                .unwrap()
                .to_owned()
            // The recorded request may have been accepted; the connection closes before reply.
        });
        let result = response(
            &fixture
                .command()
                .args(fixture.exec_args(key, "goal"))
                .output()
                .unwrap(),
        );
        let observed = server.join().unwrap();
        assert_eq!(result["data"]["submission_state"], "unknown");
        assert_eq!(result["data"]["submission_key"], observed);
        assert!(result["data"]["task_id"].is_null());
        assert_eq!(result["next_actions"][0]["command_id"], "worker.status");
        assert_eq!(
            result["next_actions"][0]["input"]["submission_key"],
            observed
        );
        assert_eq!(std::fs::read_dir(&fixture.home).unwrap().count(), 0);
        fixture.assert_no_workspace_receipt();
    }
}

#[cfg(target_os = "macos")]
#[test]
fn seatbelt_denies_local_file_writes_but_cli_can_submit_and_continue() {
    with_control(|fixture, tasks| {
        assert!(!fixture.home.starts_with("/private/tmp"));
        let policy = "(version 1)(allow default)(deny file-write*)";
        let binary = std::env::var_os("HIROUTE_WORKER_SUBMISSION_REGRESSION_CLI")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_hiroute").into());
        let invoke = |args: Vec<String>| {
            let mut command = Command::new("/usr/bin/sandbox-exec");
            command.args(["-p", policy]).arg(&binary);
            fixture.configure(&mut command);
            response(&command.args(args).output().unwrap())
        };
        let result = invoke(fixture.exec_args(None, "goal"));
        assert_eq!(result["status"], "succeeded", "{result}");
        let result = invoke(
            [
                "worker",
                "continue",
                "--task",
                "task/fixture",
                "--expected-latest-run",
                "run/1",
                "--submission-key",
                "submission/continued",
                "--no-wait",
                "--output",
                "json",
                "--",
                "continue goal",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        );
        assert_eq!(result["status"], "succeeded", "{result}");
        assert_eq!(tasks.calls.lock().unwrap().len(), 2);
        assert_eq!(std::fs::read_dir(&fixture.home).unwrap().count(), 0);
        fixture.assert_no_workspace_receipt();
    });
}
