//! Product boundary regressions: shared-root startup, not serialized native tasks.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Semaphore, mpsc};

struct StartupFixture {
    events: mpsc::UnboundedSender<String>,
    initialize: Semaphore,
    sessions: Semaphore,
    stop: Semaphore,
    protocol_version: u32,
    unknown_stop: AtomicBool,
}

impl StartupFixture {
    fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<String>) {
        let (events, receiver) = mpsc::unbounded_channel();
        (
            Arc::new(Self {
                events,
                initialize: Semaphore::new(0),
                sessions: Semaphore::new(0),
                stop: Semaphore::new(0),
                protocol_version: 1,
                unknown_stop: AtomicBool::new(false),
            }),
            receiver,
        )
    }
}

#[async_trait::async_trait]
impl WorkerPlatformPort for Arc<StartupFixture> {
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
        self.events.send("spawn".into()).unwrap();
        let (client, server) = tokio::io::duplex(8192);
        let fixture = self.clone();
        tokio::spawn(async move {
            let (r, mut w) = tokio::io::split(server);
            let mut lines = BufReader::new(r).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                let method = request["method"].as_str().unwrap();
                fixture.events.send(method.into()).unwrap();
                let result = match method {
                    "initialize" => {
                        fixture.initialize.acquire().await.unwrap().forget();
                        serde_json::json!({"protocolVersion":fixture.protocol_version,"agentCapabilities":{"loadSession":true},"_meta":{"jetbrains":{"air":{"version":1,"capabilities":["sessionFailure"]}}}})
                    }
                    "session/new" | "session/load" => {
                        fixture.sessions.acquire().await.unwrap().forget();
                        serde_json::json!({"sessionId":"native-session"})
                    }
                    "session/prompt" => serde_json::json!({"stopReason":"end_turn"}),
                    "session/cancel" => continue,
                    _ => panic!("unexpected fixture method {method}"),
                };
                let response =
                    serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":result});
                if w.write_all(format!("{response}\n").as_bytes())
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let (stdout, stdin) = tokio::io::split(client);
        Ok(ReadyWorker {
            identity: WorkerProcessIdentity {
                launch_nonce: request.launch_nonce,
                handle_id: "held-initializer".into(),
                creation_identity: "creation".into(),
            },
            stdin: Box::pin(stdin),
            stdout: Box::pin(stdout),
        })
    }

    async fn observe(
        &self,
        _: &WorkerProcessIdentity,
    ) -> Result<WorkerObservation, DelegationErrorV1> {
        Ok(WorkerObservation::Unknown)
    }

    async fn terminate(
        &self,
        _: &WorkerProcessIdentity,
        _: u64,
    ) -> Result<WorkerStopEvidence, DelegationErrorV1> {
        self.events.send("stop".into()).unwrap();
        self.stop.acquire().await.unwrap().forget();
        if self.unknown_stop.load(Ordering::SeqCst) {
            return Err(DelegationErrorV1::CapabilityUnavailable);
        }
        Ok(WorkerStopEvidence {
            scope: WorkerStopScope::ProcessGroup,
            observation: WorkerObservation::Exited { code: None },
            scope_stopped: true,
            residual_unknown: false,
        })
    }
}

async fn next(events: &mut mpsc::UnboundedReceiver<String>, expected: &str) {
    assert_eq!(
        timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .as_deref(),
        Some(expected)
    );
}

fn borrowed(
    root: &std::path::Path,
    nonce: &str,
) -> (WorkerLaunchRequest, AcpRunInput, tempfile::TempDir) {
    let (mut request, mut input, fixture) = inputs_in_context(Some(root));
    request.launch_nonce = nonce.into();
    input.deadline = Instant::now() + Duration::from_secs(5);
    input.native_session_mode = None;
    (request, input, fixture)
}

#[tokio::test]
async fn shared_codex_startup_releases_before_new_or_load_and_does_not_serialize_tasks() {
    for load in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (first_request, mut first_input, _first_dir) = borrowed(root.path(), "first");
        let (second_request, mut second_input, _second_dir) = borrowed(root.path(), "second");
        if load {
            let binding = AcpSessionBinding {
                acp_session_id: "native-session".into(),
                native_session_id: Some("native-session".into()),
            };
            first_input.session = AcpSessionStart::Load(binding.clone());
            second_input.session = AcpSessionStart::Load(binding);
        }
        let (first, mut first_events) = StartupFixture::new();
        let (second, mut second_events) = StartupFixture::new();
        let controller = async {
            next(&mut first_events, "spawn").await;
            next(&mut first_events, "initialize").await;
            // Both execute futures are polled, yet the second must not spawn before confirmation.
            assert!(
                timeout(Duration::from_millis(20), second_events.recv())
                    .await
                    .is_err()
            );
            first.initialize.add_permits(1);
            let method = if load { "session/load" } else { "session/new" };
            next(&mut first_events, method).await;
            next(&mut second_events, "spawn").await;
            next(&mut second_events, "initialize").await;
            second.initialize.add_permits(1);
            next(&mut second_events, method).await;
            // Neither native session has answered New/Load; both starts already passed the gate.
            first.sessions.add_permits(1);
            second.sessions.add_permits(1);
            first.stop.add_permits(1);
            second.stop.add_permits(1);
        };
        let (first_result, second_result, ()) = tokio::join!(biased;
            execute(&first, first_request, first_input, Arc::new(Journal::default())),
            execute(&second, second_request, second_input, Arc::new(Journal::default())),
            controller,
        );
        assert!(first_result.unwrap().execution.is_ok());
        assert!(second_result.unwrap().execution.is_ok());
        next(&mut first_events, "session/prompt").await;
        next(&mut second_events, "session/prompt").await;
    }
}

#[tokio::test]
async fn incompatible_initialize_holds_the_root_until_owned_stop_then_allows_another_start() {
    let root = tempfile::tempdir().unwrap();
    let (first_request, first_input, _first_dir) = borrowed(root.path(), "incompatible");
    let (second_request, second_input, _second_dir) = borrowed(root.path(), "next");
    let (mut first, mut first_events) = StartupFixture::new();
    Arc::get_mut(&mut first).unwrap().protocol_version = 2;
    let (second, mut second_events) = StartupFixture::new();
    let controller = async {
        next(&mut first_events, "spawn").await;
        next(&mut first_events, "initialize").await;
        first.initialize.add_permits(1);
        next(&mut first_events, "stop").await;
        assert!(
            timeout(Duration::from_millis(20), second_events.recv())
                .await
                .is_err()
        );
        first.stop.add_permits(1);
        next(&mut second_events, "spawn").await;
        next(&mut second_events, "initialize").await;
        second.initialize.add_permits(1);
        second.sessions.add_permits(1);
        second.stop.add_permits(1);
    };
    let (first_result, second_result, ()) = tokio::join!(biased;
        execute(&first, first_request, first_input, Arc::new(Journal::default())),
        execute(&second, second_request, second_input, Arc::new(Journal::default())),
        controller,
    );
    assert!(matches!(
        first_result.unwrap().execution,
        Err(DelegationErrorV1::CapabilityUnavailable)
    ));
    assert!(second_result.unwrap().execution.is_ok());
}

#[tokio::test]
async fn failed_initializer_with_unknown_stop_blocks_until_the_existing_stop_path_verifies_it() {
    let root = tempfile::tempdir().unwrap();
    let (request, input, _dir) = borrowed(root.path(), "uncertain");
    let (mut platform, mut events) = StartupFixture::new();
    Arc::get_mut(&mut platform).unwrap().protocol_version = 2;
    platform.unknown_stop.store(true, Ordering::SeqCst);
    platform.initialize.add_permits(1);
    platform.stop.add_permits(1);
    let failed = execute(&platform, request, input, Arc::new(Journal::default()))
        .await
        .unwrap();
    assert!(failed.stop.is_none());
    for expected in ["spawn", "initialize", "stop"] {
        next(&mut events, expected).await;
    }
    let (request, input, _dir) = borrowed(root.path(), "blocked");
    assert!(matches!(
        execute(&platform, request, input, Arc::new(Journal::default())).await,
        Err(DelegationErrorV1::Busy)
    ));
    assert!(
        events.try_recv().is_err(),
        "a blocked initializer must not spawn"
    );
    platform.unknown_stop.store(false, Ordering::SeqCst);
    platform.stop.add_permits(1);
    assert!(
        stop_owned(&platform, &failed.identity)
            .await
            .unwrap()
            .scope_stopped
    );
    let (request, input, _dir) = borrowed(root.path(), "recovered");
    let (platform, _events) = StartupFixture::new();
    platform.initialize.add_permits(1);
    platform.sessions.add_permits(1);
    platform.stop.add_permits(1);
    assert!(
        execute(&platform, request, input, Arc::new(Journal::default()))
            .await
            .unwrap()
            .execution
            .is_ok()
    );
}

#[tokio::test]
async fn queued_starts_obey_cancel_deadline_and_recheck_authority_before_spawn() {
    use std::future::{Future, poll_fn};
    use std::task::Poll;
    for case in ["cancel", "deadline", "revoke"] {
        let root = tempfile::tempdir().unwrap();
        let cancel = CancellationToken::new();
        let held = super::super::startup::StartupPermit::acquire(
            Some(root.path()),
            Instant::now() + Duration::from_secs(5),
            &cancel,
        )
        .await
        .unwrap()
        .unwrap();
        let (request, mut input, _dir) = borrowed(root.path(), case);
        if case == "deadline" {
            input.deadline = Instant::now() + Duration::from_millis(50);
        }
        let cancellation = input.cancellation.clone();
        let journal = Arc::new(Journal::default());
        let (platform, mut events) = StartupFixture::new();
        let mut execution = Box::pin(execute(&platform, request, input, journal.clone()));
        poll_fn(|cx| match execution.as_mut().poll(cx) {
            Poll::Pending => Poll::Ready(()),
            Poll::Ready(_) => panic!("queued start unexpectedly completed"),
        })
        .await;
        assert!(
            journal.0.lock().unwrap().is_empty(),
            "authority must be checked after waiting"
        );
        let expected = match case {
            "cancel" => {
                cancellation.cancel();
                DelegationErrorV1::Cancelled
            }
            "deadline" => DelegationErrorV1::DeadlineExceeded,
            "revoke" => {
                journal.2.store(true, Ordering::SeqCst);
                drop(held);
                DelegationErrorV1::PermissionDenied
            }
            _ => unreachable!(),
        };
        assert!(matches!(execution.await, Err(error) if error == expected));
        assert!(events.try_recv().is_err(), "rejected start must not spawn");
        if case == "revoke" {
            assert_eq!(*journal.0.lock().unwrap(), ["authorize"]);
        }
    }
}

#[tokio::test]
async fn repeated_durable_cancel_recovers_the_failed_initializers_root_without_respawning() {
    use crate::delegation::authorization_tests::fixture::sample;
    use crate::delegation::dispatcher::{
        CancellationDispatchState, DelegationCancellationDispatcher,
    };
    use hiroute_domain::OperationId;
    use hiroute_local_storage::LocalStorageSet;
    let root = tempfile::tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(root.path().join("store")).unwrap();
    let run = stores
        .runtime()
        .accept(&sample("task", "run", "workspace-root"))
        .unwrap();
    let run = stores
        .runtime()
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "preparing",
            &DelegationCheckpointV1::Progress {
                event: RunEventV1::Preparing,
            },
        )
        .unwrap();
    let process = WorkerProcessIdentity {
        launch_nonce: run.launch_nonce.clone(),
        handle_id: "held-initializer".into(),
        creation_identity: "creation".into(),
    };
    stores
        .runtime()
        .checkpoint(
            &run.workspace_id,
            &run.run_id,
            run.progress.revision,
            "bound",
            &DelegationCheckpointV1::ProcessSpawned {
                binding: process.clone(),
            },
        )
        .unwrap();
    let cancel = CancellationToken::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    let owner = super::super::startup::StartupPermit::acquire(Some(root.path()), deadline, &cancel)
        .await
        .unwrap()
        .unwrap();
    owner.bind_process(&process);
    drop(owner);
    let operation = OperationId::parse("op_00112233445566778899aabbccddeeff").unwrap();
    let dispatcher = DelegationCancellationDispatcher::default();
    let (platform, mut events) = StartupFixture::new();
    for unknown in [true, false] {
        // Public worker_cancel also wakes this dispatcher on an idempotent receipt replay.
        stores
            .runtime()
            .request_cancel(&run.workspace_id, &run.run_id, &operation, "user")
            .unwrap();
        platform.unknown_stop.store(unknown, Ordering::SeqCst);
        platform.stop.add_permits(1);
        let result = dispatcher
            .dispatch_pending(stores.runtime(), &run.workspace_id, &platform)
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].state,
            if unknown {
                CancellationDispatchState::ResidualUnknown
            } else {
                CancellationDispatchState::Stopped
            }
        );
        next(&mut events, "stop").await;
        assert!(
            events.try_recv().is_err(),
            "recovery must never spawn, initialize or prompt"
        );
        let acquired =
            super::super::startup::StartupPermit::acquire(Some(root.path()), deadline, &cancel)
                .await;
        if unknown {
            assert!(matches!(acquired, Err(DelegationErrorV1::Busy)));
        } else {
            assert!(acquired.is_ok());
        }
    }
}
