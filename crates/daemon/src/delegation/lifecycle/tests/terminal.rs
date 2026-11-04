//! A duplex adapter models a durable tail that requires its input transport to stay live.
//! These are ordering regressions; the real Claude five-tool Continue remains a native gate.
use super::*;
use crate::delegation::acp::{AcpNativeIdentityContract, AcpTerminalCheckpoint, run_acp};
use crate::delegation::finalization::DelegationFinalization;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::{Barrier, Notify};
use tokio::task::JoinHandle;

#[derive(Default)]
struct TerminalJournal {
    events: Mutex<Vec<&'static str>>,
    finalization: Arc<DelegationFinalization>,
    completed: Notify,
    permissions: AtomicUsize,
    text_updates: AtomicUsize,
    fail_publication: AtomicBool,
    fail_revocation: AtomicBool,
}

impl AcpRunJournal for TerminalJournal {
    fn session_bound(&self, _: &AcpSessionBinding) -> Result<(), DelegationErrorV1> {
        Ok(())
    }
    fn before_prompt(&self) -> Result<(), DelegationErrorV1> {
        Ok(())
    }
    fn text_update(&self, _: &str) -> Result<(), DelegationErrorV1> {
        self.text_updates.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn allow_permission_once(&self, _: &Value) -> bool {
        self.permissions.fetch_add(1, Ordering::SeqCst);
        true
    }
}

impl WorkerRunJournal for TerminalJournal {
    fn begin_finalization(&self) -> Option<DelegationFinalizationLease> {
        Some(self.finalization.acquire())
    }
    fn before_launch(&self) -> Result<(), DelegationErrorV1> {
        Ok(())
    }
    fn process_spawned(&self, _: &WorkerProcessIdentity) -> Result<(), DelegationErrorV1> {
        Ok(())
    }
    fn stop_intent(&self) -> Result<(), DelegationErrorV1> {
        self.events.lock().unwrap().push("revoke");
        if self.fail_revocation.load(Ordering::SeqCst) {
            Err(DelegationErrorV1::PermissionDenied)
        } else {
            Ok(())
        }
    }
    fn completed(&self, _: &AcpRunOutcome) -> Result<(), DelegationErrorV1> {
        assert!(self.finalization.try_acquire().is_none());
        self.events.lock().unwrap().push("completed");
        self.completed.notify_one();
        if self.fail_publication.load(Ordering::SeqCst) {
            Err(DelegationErrorV1::Conflict)
        } else {
            Ok(())
        }
    }
    fn execution_failed(&self, _: DelegationErrorV1) -> Result<(), DelegationErrorV1> {
        self.events.lock().unwrap().push("failed");
        Ok(())
    }
    fn spawned_unrecorded(&self) -> Result<(), DelegationErrorV1> {
        panic!("fixture process must have a valid binding")
    }
}

#[derive(Clone, Copy)]
enum Terminal {
    Tail { check_publication: bool },
    Cancelled,
    Error,
    Disconnect,
    WaitForCancel,
    NoTail,
}

struct TailFixture {
    journal: Arc<TerminalJournal>,
    root: PathBuf,
    history: PathBuf,
    terminal: Terminal,
    tail_written: Arc<AtomicBool>,
    prompt_received: Arc<Notify>,
    concurrent_stops: Option<Arc<Barrier>>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl TailFixture {
    fn new(journal: Arc<TerminalJournal>, root: &std::path::Path, terminal: Terminal) -> Self {
        let projects = root.join("projects");
        fixture_private_directory(&projects);
        Self {
            journal,
            root: root.to_owned(),
            history: projects.join("native-session.jsonl"),
            terminal,
            tail_written: Arc::default(),
            prompt_received: Arc::default(),
            concurrent_stops: None,
            task: Mutex::default(),
        }
    }

    async fn joined(&self) {
        let task = self.task.lock().unwrap().take().unwrap();
        timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
    }
}

impl TailFixture {
    /// Sample after the adapter has joined. These closed facts do not participate in
    /// the checkpoint's two-second wait or reveal fixture paths/history content.
    fn history_facts(&self) -> Value {
        let direct = fs::symlink_metadata(&self.history).ok();
        let relative_portable = self
            .history
            .strip_prefix(&self.root)
            .ok()
            .and_then(std::path::Path::to_str)
            .is_some_and(hiroute_domain::delegation::valid_native_history_path);
        let reader = match TaskSessionRoot::native_transcript_path(
            &self.root,
            WorkerHarnessV1::ClaudeCode,
            "native-session",
        ) {
            Ok(_) => "resolved",
            Err(DelegationErrorV1::ResumeUnavailable) => "resume_unavailable",
            Err(_) => "other_error",
        };
        #[cfg(windows)]
        let native = {
            use hiroute_platform::windows::security;
            security::open_identity(&self.history, false).map_or_else(
                |_| json!({"held_open":false,"owner_acl":false,"single_link":false,
                    "same_held_identity":false}),
                |held| json!({
                    "held_open": true,
                    "owner_acl": security::verify_owner_only_handle(&held, false).is_ok(),
                    "single_link": security::file_link_count(&held).ok() == Some(1),
                    "same_held_identity": security::file_identity(&held).ok().is_some_and(|first| {
                        security::open_identity(&self.history, false)
                            .and_then(|second| security::file_identity(&second))
                            .ok()
                            == Some(first)
                    }),
                }),
            )
        };
        #[cfg(not(windows))]
        let native = Value::Null;
        json!({
            "tail_written": self.tail_written.load(Ordering::SeqCst),
            "direct_file": direct
                .as_ref()
                .is_some_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink()),
            "direct_len": direct.as_ref().map(std::fs::Metadata::len),
            "relative_portable": relative_portable,
            "reader": reader,
            "reader_len": claude_history_len(&self.root, "native-session"),
            "native": native,
        })
    }
}

#[async_trait::async_trait]
impl WorkerPlatformPort for TailFixture {
    fn capabilities(
        &self,
        _: &CandidateWorkerProfile,
    ) -> Result<WorkerPlatformCapabilities, DelegationErrorV1> {
        Ok(WorkerPlatformCapabilities {
            can_start: true,
            can_stop: true,
        })
    }

    async fn launch(&self, request: WorkerLaunchRequest) -> Result<ReadyWorker, DelegationErrorV1> {
        let (client, server) = tokio::io::duplex(8192);
        let journal = self.journal.clone();
        let history = self.history.clone();
        let terminal = self.terminal;
        let tail_written = self.tail_written.clone();
        let prompt_received = self.prompt_received.clone();
        let task = tokio::spawn(async move {
            let (read, mut write) = tokio::io::split(server);
            let mut lines = BufReader::new(read).lines();
            let mut pending_prompt = None;
            while let Some(line) = lines.next_line().await.unwrap() {
                let request: Value = serde_json::from_str(&line).unwrap();
                let method = request["method"].as_str().unwrap();
                let result = match method {
                    "initialize" => json!({"protocolVersion":1,"agentCapabilities":{}}),
                    "session/new" => json!({"sessionId":"native-session"}),
                    "session/prompt" => match terminal {
                        Terminal::WaitForCancel => {
                            pending_prompt = Some(request["id"].clone());
                            prompt_received.notify_one();
                            continue;
                        }
                        Terminal::Cancelled => json!({"stopReason":"cancelled"}),
                        Terminal::Error => {
                            write
                                .write_all(
                                    format!(
                                        "{}\n",
                                        json!({"jsonrpc":"2.0","id":request["id"],
                                "error":{"code":-32603,"message":"fixture prompt error"}})
                                    )
                                    .as_bytes(),
                                )
                                .await
                                .unwrap();
                            break;
                        }
                        _ => json!({"stopReason":"end_turn"}),
                    },
                    "session/cancel" => {
                        let id = pending_prompt.take().unwrap();
                        write.write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":id,"result":{"stopReason":"cancelled"}})).as_bytes()).await.unwrap();
                        continue;
                    }
                    other => panic!("unexpected fixture method {other}"),
                };
                write
                    .write_all(
                        format!(
                            "{}\n",
                            json!({"jsonrpc":"2.0","id":request["id"],"result":result})
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                if method != "session/prompt" {
                    continue;
                }
                let Terminal::Tail { check_publication } = terminal else {
                    if !matches!(terminal, Terminal::Disconnect) {
                        assert!(lines.next_line().await.unwrap().is_none());
                    }
                    break;
                };
                if check_publication {
                    // Completion wakes this adapter while its input is still open. A permission
                    // request arriving afterwards must not re-enter the journal's admission.
                    journal.completed.notified().await;
                    assert_eq!(*journal.events.lock().unwrap(), ["completed", "revoke"]);
                    assert!(journal.finalization.try_acquire().is_none());
                    write.write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":"late-permission",
                        "method":"session/request_permission", "params":{"sessionId":"native-session",
                            "toolCall":{"toolCallId":"late-tool"},"options":[{"optionId":"once","name":"Once","kind":"allow_once"}]}})).as_bytes()).await.unwrap();
                    let response = timeout(Duration::from_millis(250), lines.next_line())
                        .await
                        .unwrap()
                        .unwrap()
                        .unwrap();
                    let response: Value = serde_json::from_str(&response).unwrap();
                    assert_eq!(response["id"], "late-permission");
                    assert_eq!(response["result"]["outcome"]["outcome"], "cancelled");
                    write.write_all(format!("{}\n", json!({"jsonrpc":"2.0","method":"session/update",
                        "params":{"sessionId":"native-session","update":{"sessionUpdate":"agent_message_chunk",
                            "content":{"type":"text","text":"late ignored text"}}}})).as_bytes()).await.unwrap();
                }
                tokio::select! {
                    line = lines.next_line() => assert!(line.unwrap().is_none()),
                    _ = tokio::time::sleep(Duration::from_millis(50)) => {
                        hiroute_platform::durable::write_private_new(&history, b"final-tail\n").unwrap();
                        tail_written.store(true, Ordering::SeqCst);
                        journal.events.lock().unwrap().push("tail");
                        assert!(lines.next_line().await.unwrap().is_none());
                    }
                }
                break;
            }
        });
        *self.task.lock().unwrap() = Some(task);
        let (stdout, stdin) = tokio::io::split(client);
        Ok(ReadyWorker {
            identity: WorkerProcessIdentity {
                launch_nonce: request.launch_nonce,
                handle_id: "held-tail-fixture".into(),
                creation_identity: "tail-fixture-creation".into(),
            },
            stdin: Box::pin(stdin),
            stdout: Box::pin(stdout),
        })
    }

    async fn observe(
        &self,
        _: &WorkerProcessIdentity,
    ) -> Result<WorkerObservation, DelegationErrorV1> {
        Ok(WorkerObservation::Running)
    }

    async fn terminate(
        &self,
        identity: &WorkerProcessIdentity,
        bound: u64,
    ) -> Result<WorkerStopEvidence, DelegationErrorV1> {
        assert_eq!(identity.handle_id, "held-tail-fixture");
        assert_eq!(bound, STOP_WAIT_MS);
        if let Some(stops) = &self.concurrent_stops {
            timeout(Duration::from_secs(2), stops.wait())
                .await
                .expect("both interrupted Workers must start stopping before either finishes");
        } else {
            assert!(self.journal.finalization.try_acquire().is_none());
        }
        self.journal.events.lock().unwrap().push("stop");
        Ok(WorkerStopEvidence {
            scope: WorkerStopScope::Root,
            observation: WorkerObservation::Exited { code: Some(0) },
            scope_stopped: true,
            residual_unknown: false,
        })
    }
}

fn claude_inputs() -> (
    WorkerLaunchRequest,
    AcpRunInput,
    crate::test_support::PrivateTempDir,
) {
    let (request, mut input, fixture) = inputs_for_harness(None, WorkerHarnessV1::ClaudeCode);
    assert_eq!(
        request.profile.identity_contract,
        AcpNativeIdentityContract::ClaudeSessionV1
    );
    input.native_session_mode = None;
    input.deadline = Instant::now() + Duration::from_secs(3);
    (request, input, fixture)
}

#[tokio::test]
async fn claude_terminal_checkpoint_keeps_transport_live_until_final_tail() {
    let (request, input, _directory) = claude_inputs();
    let journal = Arc::new(TerminalJournal::default());
    let fixture = TailFixture::new(
        journal.clone(),
        &request.profile.session_root,
        Terminal::Tail {
            check_publication: true,
        },
    );
    let result = execute(&fixture, request, input, journal.clone())
        .await
        .unwrap();
    fixture.joined().await;
    assert!(result.execution.as_ref().unwrap().text.is_empty());
    let facts = fixture.history_facts();
    assert!(
        result.native_history_settled,
        "closed history facts: {facts}"
    );
    assert!(result.stop.as_ref().unwrap().scope_stopped);
    assert!(fixture.tail_written.load(Ordering::SeqCst));
    assert_eq!(fs::read(&fixture.history).unwrap(), b"final-tail\n");
    assert_eq!(
        *journal.events.lock().unwrap(),
        ["completed", "revoke", "tail", "stop"]
    );
    assert_eq!(journal.permissions.load(Ordering::SeqCst), 0);
    assert_eq!(journal.text_updates.load(Ordering::SeqCst), 0);
    assert!(journal.finalization.try_acquire().is_none());
    drop(result);
    assert!(journal.finalization.try_acquire().is_some());
}

#[tokio::test]
async fn eof_before_claude_checkpoint_prevents_final_tail() {
    let (request, input, _directory) = claude_inputs();
    let journal = Arc::new(TerminalJournal::default());
    let checkpoint = ClaudeHistoryCheckpoint::capture(&request.profile, &input).unwrap();
    let fixture = TailFixture::new(
        journal.clone(),
        &request.profile.session_root,
        Terminal::Tail {
            check_publication: false,
        },
    );
    let worker = fixture.launch(request).await.unwrap();
    // The previous ordering awaited this foreground result (and dropped the transport)
    // before invoking the exact same history checkpoint below.
    let outcome = run_acp(worker.stdout, worker.stdin, input, journal)
        .await
        .unwrap();
    fixture.joined().await;
    assert!(
        !checkpoint
            .wait_until_settled(outcome.session.native_session_id.as_deref())
            .await
    );
    assert!(!fixture.tail_written.load(Ordering::SeqCst));
    assert!(!fixture.history.exists());
}

#[tokio::test]
async fn unsolicited_native_cancel_and_protocol_error_do_not_wait_for_history() {
    for terminal in [Terminal::Cancelled, Terminal::Error] {
        let (request, input, _directory) = claude_inputs();
        let journal = Arc::new(TerminalJournal::default());
        let fixture = TailFixture::new(journal.clone(), &request.profile.session_root, terminal);
        let result = timeout(
            Duration::from_millis(500),
            execute(&fixture, request, input, journal.clone()),
        )
        .await
        .unwrap()
        .unwrap();
        fixture.joined().await;
        match terminal {
            Terminal::Cancelled => {
                assert!(matches!(
                    result.execution,
                    Err(DelegationErrorV1::PromptFailed)
                ));
                assert_eq!(
                    *journal.events.lock().unwrap(),
                    ["failed", "revoke", "stop"]
                );
            }
            Terminal::Error => {
                assert!(result.execution.is_err());
                assert_eq!(
                    *journal.events.lock().unwrap(),
                    ["failed", "revoke", "stop"]
                );
            }
            _ => unreachable!(),
        }
        assert!(!result.native_history_settled);
        assert!(!fixture.tail_written.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn execution_deadline_is_not_replaced_by_its_acp_shutdown_cancellation() {
    let (request, mut input, _directory) = claude_inputs();
    input.deadline = Instant::now() + Duration::from_millis(250);
    let journal = Arc::new(TerminalJournal::default());
    let fixture = TailFixture::new(
        journal.clone(),
        &request.profile.session_root,
        Terminal::WaitForCancel,
    );
    let result = execute(&fixture, request, input, journal.clone())
        .await
        .unwrap();
    fixture.joined().await;
    assert!(matches!(
        result.execution,
        Err(DelegationErrorV1::DeadlineExceeded)
    ));
    let events = journal.events.lock().unwrap();
    assert_eq!(events.first(), Some(&"failed"));
    assert!(!events.contains(&"completed"));
    assert_eq!(events.last(), Some(&"stop"));
    assert!(result.stop.unwrap().scope_stopped);
}

#[tokio::test]
async fn claude_disconnect_after_terminal_keeps_completion_without_history_ready() {
    let (request, input, _directory) = claude_inputs();
    let journal = Arc::new(TerminalJournal::default());
    let fixture = TailFixture::new(
        journal.clone(),
        &request.profile.session_root,
        Terminal::Disconnect,
    );
    let result = execute(&fixture, request, input, journal.clone())
        .await
        .unwrap();
    fixture.joined().await;
    assert_eq!(result.execution.unwrap().stop_reason, "end_turn");
    assert!(!result.native_history_settled);
    assert_eq!(
        *journal.events.lock().unwrap(),
        ["completed", "revoke", "stop"]
    );
}

#[tokio::test]
async fn terminal_publication_failure_does_not_wait_or_double_write() {
    let (request, input, _directory) = claude_inputs();
    let journal = Arc::new(TerminalJournal::default());
    journal.fail_publication.store(true, Ordering::SeqCst);
    journal.fail_revocation.store(true, Ordering::SeqCst);
    let fixture = TailFixture::new(
        journal.clone(),
        &request.profile.session_root,
        Terminal::NoTail,
    );
    let result = timeout(
        Duration::from_millis(500),
        execute(&fixture, request, input, journal.clone()),
    )
    .await
    .unwrap()
    .unwrap();
    fixture.joined().await;
    assert!(matches!(result.execution, Err(DelegationErrorV1::Conflict)));
    assert_eq!(
        result.stop_intent_error,
        Some(DelegationErrorV1::PermissionDenied)
    );
    assert!(result.stop.unwrap().scope_stopped);
    assert!(!result.native_history_settled);
    assert_eq!(
        *journal.events.lock().unwrap(),
        ["completed", "revoke", "stop"]
    );
}

#[tokio::test]
async fn interrupted_terminal_checkpoint_retains_lease_completion_and_false_readiness() {
    let (request, input, _directory) = claude_inputs();
    let journal = Arc::new(TerminalJournal::default());
    let checkpoint = LiveTerminalCheckpoint::new(
        journal.clone(),
        ClaudeHistoryCheckpoint::capture(&request.profile, &input).unwrap(),
    );
    let outcome = AcpRunOutcome {
        session: AcpSessionBinding {
            acp_session_id: "native-session".into(),
            native_session_id: Some("native-session".into()),
        },
        stop_reason: "end_turn".into(),
        text: "completed body".into(),
        content_incomplete: false,
    };
    {
        let callback = checkpoint.before_transport_close(&outcome);
        tokio::pin!(callback);
        tokio::select! {
            biased;
            _ = &mut callback => panic!("missing history cannot settle"),
            _ = journal.completed.notified() => {}
        }
    }
    // Dropping a live checkpoint future models the ACP hard bound interrupting it.
    // Terminal publication remains authoritative, with no invented Continue readiness.
    assert_eq!(*journal.events.lock().unwrap(), ["completed", "revoke"]);
    assert_eq!(checkpoint.revoke(), None);
    let receipt = checkpoint.take_receipt().unwrap();
    assert_eq!(receipt.execution.unwrap().text, "completed body");
    assert!(!receipt.history_settled);
    assert!(journal.finalization.try_acquire().is_none());
    drop(receipt.finalization);
    assert!(journal.finalization.try_acquire().is_some());
    assert_eq!(*journal.events.lock().unwrap(), ["completed", "revoke"]);
}

#[tokio::test]
async fn execute_outer_interruptions_after_completion_preserve_terminal_receipt() {
    let mut exercised = Vec::new();
    for interruption in ["outer-cancel", "deadline", "daemon-shutdown"] {
        let (request, mut input, _directory) = claude_inputs();
        if interruption == "deadline" {
            // The terminal response precedes this bound; the missing native tail
            // still keeps the two-second checkpoint active when it expires.
            input.deadline = Instant::now() + Duration::from_secs(1);
        }
        let deadline = input.deadline;
        let cancellation = input.cancellation.clone();
        let shutdown = CancellationToken::new();
        let journal = Arc::new(TerminalJournal::default());
        let fixture = TailFixture::new(
            journal.clone(),
            &request.profile.session_root,
            Terminal::NoTail,
        );
        let interrupt = async {
            timeout(Duration::from_secs(1), journal.completed.notified())
                .await
                .unwrap();
            assert!(
                Instant::now() < deadline,
                "completion must precede interruption"
            );
            assert_eq!(*journal.events.lock().unwrap(), ["completed", "revoke"]);
            assert!(journal.finalization.try_acquire().is_none());
            assert!(!fixture.task.lock().unwrap().as_ref().unwrap().is_finished());
            assert!(!fixture.history.exists());
            if interruption == "outer-cancel" {
                cancellation.cancel();
                assert!(cancellation.is_cancelled());
            } else if interruption == "daemon-shutdown" {
                shutdown.cancel();
                assert!(shutdown.is_cancelled());
            } else {
                tokio::time::sleep_until(deadline).await;
                assert!(Instant::now() >= deadline);
            }
            // No interruption may publish another outcome, revoke twice or
            // release the finalization lease while the live transport is draining.
            assert_eq!(*journal.events.lock().unwrap(), ["completed", "revoke"]);
            assert!(journal.finalization.try_acquire().is_none());
            assert!(!fixture.task.lock().unwrap().as_ref().unwrap().is_finished());
        };
        let (result, ()) = timeout(Duration::from_secs(4), async {
            tokio::join!(biased; execute_with_shutdown(&fixture, request, input, journal.clone(), shutdown.clone()), interrupt)
        })
        .await
        .unwrap();
        let result = result.unwrap();
        fixture.joined().await;
        assert_eq!(result.execution.as_ref().unwrap().stop_reason, "end_turn");
        assert!(!result.native_history_settled);
        assert!(!fixture.history.exists());
        assert!(result.stop.as_ref().unwrap().scope_stopped);
        assert!(result.stop_intent_error.is_none());
        assert_eq!(
            *journal.events.lock().unwrap(),
            ["completed", "revoke", "stop"]
        );
        assert!(result.finalization.is_some());
        assert!(journal.finalization.try_acquire().is_none());
        drop(result);
        assert!(journal.finalization.try_acquire().is_some());
        exercised.push(interruption);
    }
    assert_eq!(exercised, ["outer-cancel", "deadline", "daemon-shutdown"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn daemon_shutdown_stops_independent_workers_concurrently() {
    let finalization = Arc::new(DelegationFinalization::default());
    let stops = Arc::new(Barrier::new(2));
    let shutdown = CancellationToken::new();
    let mut workers = Vec::new();
    for _ in 0..2 {
        let (request, mut input, directory) = claude_inputs();
        input.deadline = Instant::now() + Duration::from_secs(15);
        let journal = Arc::new(TerminalJournal {
            finalization: finalization.clone(),
            ..Default::default()
        });
        let mut fixture = TailFixture::new(
            journal.clone(),
            &request.profile.session_root,
            Terminal::WaitForCancel,
        );
        fixture.concurrent_stops = Some(stops.clone());
        let ready = fixture.prompt_received.clone();
        let shutdown = shutdown.clone();
        let worker = tokio::spawn(async move {
            let _directory = directory;
            let result = execute_with_shutdown(&fixture, request, input, journal, shutdown)
                .await
                .unwrap();
            assert!(matches!(
                result.execution,
                Err(DelegationErrorV1::ProtocolFailed)
            ));
            assert!(result.stop.as_ref().unwrap().scope_stopped);
            assert!(result.finalization.is_some());
            drop(result);
            fixture.joined().await;
        });
        workers.push((worker, ready));
    }
    for (_, ready) in &workers {
        timeout(Duration::from_secs(5), ready.notified())
            .await
            .expect("Worker prompt must be active before shutdown");
    }
    shutdown.cancel();
    for (worker, _) in workers {
        timeout(Duration::from_secs(6), worker)
            .await
            .expect("Worker did not finish shutdown")
            .unwrap();
    }
}

/// ACP EOF belongs to the transport. This independently retained platform object
/// remains Running when the exact terminate call fails; no stop evidence is forged.
struct FailingTerminalStop {
    adapter: TailFixture,
    held: Mutex<Option<WorkerProcessIdentity>>,
    stop_attempts: AtomicUsize,
}

#[async_trait::async_trait]
impl WorkerPlatformPort for FailingTerminalStop {
    fn capabilities(
        &self,
        profile: &CandidateWorkerProfile,
    ) -> Result<WorkerPlatformCapabilities, DelegationErrorV1> {
        self.adapter.capabilities(profile)
    }

    async fn launch(&self, request: WorkerLaunchRequest) -> Result<ReadyWorker, DelegationErrorV1> {
        let worker = self.adapter.launch(request).await?;
        assert!(
            self.held
                .lock()
                .unwrap()
                .replace(worker.identity.clone())
                .is_none()
        );
        Ok(worker)
    }

    async fn observe(
        &self,
        identity: &WorkerProcessIdentity,
    ) -> Result<WorkerObservation, DelegationErrorV1> {
        assert_eq!(self.held.lock().unwrap().as_ref(), Some(identity));
        Ok(WorkerObservation::Running)
    }

    async fn terminate(
        &self,
        identity: &WorkerProcessIdentity,
        bound: u64,
    ) -> Result<WorkerStopEvidence, DelegationErrorV1> {
        assert_eq!(self.held.lock().unwrap().as_ref(), Some(identity));
        assert_eq!(bound, STOP_WAIT_MS);
        assert!(self.adapter.journal.finalization.try_acquire().is_none());
        self.stop_attempts.fetch_add(1, Ordering::SeqCst);
        self.adapter
            .journal
            .events
            .lock()
            .unwrap()
            .push("stop-failed");
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
}

#[tokio::test]
async fn claude_completed_settled_history_does_not_replace_failed_owned_stop_evidence() {
    let (request, input, _directory) = claude_inputs();
    let journal = Arc::new(TerminalJournal::default());
    let fixture = FailingTerminalStop {
        adapter: TailFixture::new(
            journal.clone(),
            &request.profile.session_root,
            Terminal::Tail {
                check_publication: true,
            },
        ),
        held: Mutex::default(),
        stop_attempts: AtomicUsize::default(),
    };
    let result = timeout(
        Duration::from_secs(3),
        execute(&fixture, request, input, journal.clone()),
    )
    .await
    .unwrap()
    .unwrap();
    fixture.adapter.joined().await;
    assert_eq!(result.execution.as_ref().unwrap().stop_reason, "end_turn");
    assert!(result.native_history_settled);
    assert_eq!(fs::read(&fixture.adapter.history).unwrap(), b"final-tail\n");
    assert!(
        result.stop.is_none(),
        "completion/history is not owned stop evidence"
    );
    assert_eq!(fixture.stop_attempts.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.observe(&result.identity).await.unwrap(),
        WorkerObservation::Running
    );
    assert_eq!(
        *journal.events.lock().unwrap(),
        ["completed", "revoke", "tail", "stop-failed"]
    );
    assert!(result.finalization.is_some());
    assert!(journal.finalization.try_acquire().is_none());
    // The executor must receive this lease together with the missing stop proof;
    // dropping its result is distinct from proving the retained object stopped.
    let identity = result.identity.clone();
    drop(result);
    assert!(journal.finalization.try_acquire().is_some());
    assert_eq!(
        fixture.observe(&identity).await.unwrap(),
        WorkerObservation::Running
    );
}
