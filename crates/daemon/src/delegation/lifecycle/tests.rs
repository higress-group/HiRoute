use super::*;
use crate::delegation::acp::{AcpSessionBinding, AcpSessionStart};
use crate::delegation::profile::{
    CandidateWorkerProfile, ProfileInput, SessionRootUse, TaskSessionRoot,
};
use hiroute_domain::ProtectedSecret;
use hiroute_domain::delegation::*;
use std::sync::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Journal(
    Mutex<Vec<&'static str>>,
    Option<Arc<crate::delegation::finalization::DelegationFinalization>>,
);
impl AcpRunJournal for Journal {
    fn session_bound(&self, _: &AcpSessionBinding) -> Result<(), DelegationErrorV1> {
        self.0.lock().unwrap().push("session");
        Ok(())
    }
    fn before_prompt(&self) -> Result<(), DelegationErrorV1> {
        self.0.lock().unwrap().push("prompt");
        Ok(())
    }
    fn text_update(&self, _: &str) -> Result<(), DelegationErrorV1> {
        Ok(())
    }
    fn allow_permission_once(&self, _: &serde_json::Value) -> bool {
        false
    }
}
impl WorkerRunJournal for Journal {
    fn begin_finalization(
        &self,
    ) -> Option<crate::delegation::finalization::DelegationFinalizationLease> {
        self.1.as_ref().map(|finalization| finalization.acquire())
    }

    fn before_launch(&self) -> Result<(), DelegationErrorV1> {
        self.0.lock().unwrap().push("authorize");
        Ok(())
    }
    fn process_spawned(&self, _: &WorkerProcessIdentity) -> Result<(), DelegationErrorV1> {
        self.0.lock().unwrap().push("spawned");
        Ok(())
    }
    fn stop_intent(&self) -> Result<(), DelegationErrorV1> {
        self.0.lock().unwrap().push("revoke");
        Ok(())
    }
    fn completed(&self, _: &AcpRunOutcome) -> Result<(), DelegationErrorV1> {
        if let Some(finalization) = self.1.as_ref() {
            assert!(
                finalization.try_acquire().is_none(),
                "terminal publication must happen under the finalization lease"
            );
        }
        self.0.lock().unwrap().push("completed");
        Ok(())
    }
    fn execution_failed(&self, _: DelegationErrorV1) -> Result<(), DelegationErrorV1> {
        self.0.lock().unwrap().push("failed");
        Ok(())
    }
    fn spawned_unrecorded(&self) -> Result<(), DelegationErrorV1> {
        self.0.lock().unwrap().push("spawn-unknown");
        Ok(())
    }
}

struct Fixture {
    journal: Arc<Journal>,
    methods: Arc<Mutex<Vec<String>>>,
    disconnect: bool,
    contradictory_stop: bool,
}
#[async_trait::async_trait]
impl WorkerPlatformPort for Fixture {
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
        self.journal.0.lock().unwrap().push("spawn");
        let (client, server) = tokio::io::duplex(8192);
        let methods = self.methods.clone();
        let disconnect = self.disconnect;
        tokio::spawn(async move {
            let (r, mut w) = tokio::io::split(server);
            let mut lines = BufReader::new(r).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let req: serde_json::Value = serde_json::from_str(&line).unwrap();
                let method = req["method"].as_str().unwrap();
                methods.lock().unwrap().push(method.into());
                let result = match method {
                    "initialize" => {
                        serde_json::json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true}})
                    }
                    "session/new" => serde_json::json!({
                        "sessionId":"native-session",
                        "modes":{
                            "currentModeId":"default",
                            "availableModes":[
                                {"id":"default","name":"Manual"},
                                {"id":"agent-full-access","name":"Full access"}
                            ]
                        }
                    }),
                    "session/set_mode" => {
                        assert_eq!(req["params"]["sessionId"], "native-session");
                        assert_eq!(req["params"]["modeId"], "agent-full-access");
                        serde_json::json!({})
                    }
                    "session/prompt" if disconnect => break,
                    "session/prompt" => serde_json::json!({"stopReason":"end_turn"}),
                    "session/cancel" => continue,
                    _ => panic!("unexpected method {method}"),
                };
                let response = serde_json::json!({"jsonrpc":"2.0","id":req["id"],"result":result});
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
                handle_id: "held-child".into(),
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
        Ok(WorkerObservation::Running)
    }
    async fn terminate(
        &self,
        identity: &WorkerProcessIdentity,
        bound: u64,
    ) -> Result<WorkerStopEvidence, DelegationErrorV1> {
        assert_eq!(identity.handle_id, "held-child");
        assert_eq!(bound, STOP_WAIT_MS);
        assert_eq!(self.journal.0.lock().unwrap().last(), Some(&"revoke"));
        self.journal.0.lock().unwrap().push("stop");
        Ok(WorkerStopEvidence {
            scope: WorkerStopScope::Root,
            observation: if self.contradictory_stop {
                WorkerObservation::Running
            } else {
                WorkerObservation::Exited { code: Some(0) }
            },
            scope_stopped: true,
            residual_unknown: false,
        })
    }
}

fn inputs() -> (WorkerLaunchRequest, AcpRunInput, tempfile::TempDir) {
    let fixture = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(fixture.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let session = TaskSessionRoot::prepare(
        fixture.path(),
        &hiroute_domain::WorkspaceId::parse("workspace").unwrap(),
        "root",
        "task-a",
        WorkerHarnessV1::CodexCli,
        SessionRootUse::New,
    )
    .unwrap();
    let cwd = std::env::temp_dir();
    let adapter = cwd.join("adapter");
    let harness = cwd.join("codex");
    let private = fixture.path().join("run");
    let permit = WorkspaceExecutionPermitV1 {
        permit_id: "permit".into(),
        generation: 1,
        root_identity: "root".into(),
        access: WorkspaceAccessV1::TrustedNative,
        tools: vec![WorkerToolV1::Read, WorkerToolV1::Shell],
        network: WorkerNetworkV1::Allowed,
        expires_at_ms: 10000,
        max_run_ms: 1000,
        max_concurrent: 2,
        revoked: false,
    };
    let execution = WorkerExecutionIntentV1 {
        root_identity: "root".into(),
        access: WorkspaceAccessV1::TrustedNative,
        tools: permit.tools.clone(),
        network: WorkerNetworkV1::Allowed,
        duration_ms: 1000,
        delegation_depth: 1,
    };
    let profile = CandidateWorkerProfile::build(ProfileInput {
        harness: WorkerHarnessV1::CodexCli,
        adapter: &adapter,
        harness_binary: &harness,
        node_binary: None,
        private_root: &private,
        session_root: &session,
        workspace: &cwd,
        alias: "fixed-alias",
        codex_catalog: None,
        native_effort: None,
        gateway: "127.0.0.1:10001".parse().unwrap(),
        permit: &permit,
        execution: &execution,
        permission_policy: WorkerPermissionPolicyV1::ApproveAll,
        admitted_at_ms: 1,
        token: ProtectedSecret::new(b"fixture-token".to_vec()).unwrap(),
    })
    .unwrap();
    let input = AcpRunInput {
        cwd,
        prompt: "fixture only".into(),
        session: AcpSessionStart::New,
        identity_contract: profile.identity_contract.clone(),
        session_meta: profile.session_meta.clone(),
        native_session_mode: Some(profile.native_session_mode().to_owned()),
        authentication: None,
        deadline: Instant::now() + Duration::from_secs(1),
        cancellation: CancellationToken::new(),
    };
    (
        WorkerLaunchRequest {
            launch_nonce: "launch".into(),
            profile,
            deadline_unix_ms: 1001,
        },
        input,
        fixture,
    )
}

#[tokio::test]
#[cfg(unix)]
async fn managed_process_reports_real_acp_new_load_failure_and_timeout_without_preprobe() {
    use crate::delegation::local_worker::LocalWorkerPlatform;
    use std::time::{SystemTime, UNIX_EPOCH};
    for case in ["new", "load", "incompatible", "timeout"] {
        let (mut request, mut input, fixture) = inputs();
        let script = fixture.path().join("adapter.py");
        let transcript = fixture.path().join("requests");
        std::fs::write(&script, r#"import json, os, sys, time
assert os.environ['HIROUTE_RUN_TOKEN'] == 'fixture-token'
assert os.environ['HOME'].endswith('/run/home')
case = sys.argv[1]
for line in sys.stdin:
    request = json.loads(line)
    method = request['method']
    with open(sys.argv[2], 'a') as log:
        log.write(method + '\n')
    if case == 'timeout':
        time.sleep(10)
    if case == 'incompatible':
        print('not-json', flush=True)
        break
    modes = {'currentModeId':'agent-full-access','availableModes':[{'id':'agent-full-access','name':'Autonomous'}]}
    if method == 'initialize':
        result = {'protocolVersion':1,'agentCapabilities':{'loadSession':True}}
    elif method == 'session/new':
        assert case == 'new'
        result = {'sessionId':'native-session','modes':modes}
    elif method == 'session/load':
        assert case == 'load'
        assert request['params']['sessionId'] == 'native-session'
        result = {'modes':modes}
    elif method == 'session/set_mode':
        result = {}
    elif method == 'session/prompt':
        result = {'stopReason':'end_turn'}
    else:
        raise AssertionError(method)
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}), flush=True)
"#).unwrap();
        request.profile.executable = std::path::PathBuf::from("/usr/bin/python3");
        request.profile.args = vec![script, case.into(), transcript.clone()];
        request.deadline_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 5_000;
        input.deadline = Instant::now()
            + if case == "timeout" {
                Duration::from_millis(500)
            } else {
                Duration::from_secs(5)
            };
        if case == "load" {
            input.session = AcpSessionStart::Load(AcpSessionBinding {
                acp_session_id: "native-session".into(),
                native_session_id: Some("native-session".into()),
            });
        }
        let private_root = request.profile.private_root.clone();
        let platform = LocalWorkerPlatform::default();
        let started = Instant::now();
        let result = execute(&platform, request, input, Arc::new(Journal::default()))
            .await
            .unwrap();
        match case {
            "new" | "load" => assert_eq!(
                result
                    .execution
                    .unwrap()
                    .session
                    .native_session_id
                    .as_deref(),
                Some("native-session")
            ),
            "timeout" => {
                assert!(matches!(
                    result.execution,
                    Err(DelegationErrorV1::DeadlineExceeded | DelegationErrorV1::Cancelled)
                ));
                assert!(started.elapsed() < Duration::from_secs(7));
            }
            _ => assert!(result.execution.is_err()),
        }
        assert!(result.stop.unwrap().scope_stopped);
        assert!(!private_root.exists());
        let methods = std::fs::read_to_string(transcript).unwrap();
        assert_eq!(
            methods
                .lines()
                .filter(|method| *method == "initialize")
                .count(),
            1
        );
        if case == "load" {
            assert!(!methods.contains("session/new"));
        }
        if case == "incompatible" || case == "timeout" {
            assert!(!methods.contains("session/prompt"));
        }
    }
}

#[tokio::test]
async fn spawn_stdio_acp_result_and_owned_cleanup_need_no_helper_handshake() {
    let journal = Arc::new(Journal::default());
    let backend = Fixture {
        journal: journal.clone(),
        methods: Arc::default(),
        disconnect: false,
        contradictory_stop: false,
    };
    let (request, input, _fixture) = inputs();
    let result = execute(&backend, request, input, journal.clone())
        .await
        .unwrap();
    assert!(result.native_history_settled);
    assert_eq!(
        result
            .execution
            .unwrap()
            .session
            .native_session_id
            .as_deref(),
        Some("native-session")
    );
    assert!(result.stop.unwrap().scope_stopped);
    assert_eq!(
        *journal.0.lock().unwrap(),
        [
            "authorize",
            "spawn",
            "spawned",
            "session",
            "prompt",
            "completed",
            "revoke",
            "stop"
        ]
    );
}

#[tokio::test]
async fn terminal_publication_lease_survives_until_the_executor_receives_the_result() {
    let finalization = Arc::new(crate::delegation::finalization::DelegationFinalization::default());
    let journal = Arc::new(Journal(Mutex::default(), Some(Arc::clone(&finalization))));
    let backend = Fixture {
        journal: Arc::clone(&journal),
        methods: Arc::default(),
        disconnect: false,
        contradictory_stop: false,
    };
    let (request, input, _fixture) = inputs();

    let result = execute(&backend, request, input, journal).await.unwrap();

    assert!(finalization.try_acquire().is_none());
    drop(result);
    assert!(finalization.try_acquire().is_some());
}

#[tokio::test]
async fn claude_history_checkpoint_observes_new_and_appended_stable_transcripts() {
    for existing in [false, true] {
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let session_id = format!("native-{existing}");
        let history = root
            .path()
            .join("projects/workspace")
            .join(format!("{session_id}.jsonl"));
        std::fs::create_dir_all(history.parent().unwrap()).unwrap();
        let baseline = if existing {
            std::fs::write(&history, b"old\n").unwrap();
            HistoryBaseline::Existing(4)
        } else {
            HistoryBaseline::New
        };
        let checkpoint = ClaudeHistoryCheckpoint {
            root: root.path().to_owned(),
            baseline,
        };
        let history_for_write = history.clone();
        let writer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(history_for_write)
                .unwrap();
            file.write_all(b"new\n").unwrap();
            file.sync_all().unwrap();
        });
        assert!(
            checkpoint
                .wait_until_settled(Some(session_id.as_str()))
                .await
        );
        writer.await.unwrap();
    }
}

#[tokio::test]
async fn prompt_disconnect_never_respawns_or_replays_and_cleanup_is_not_completion() {
    let journal = Arc::new(Journal::default());
    let backend = Fixture {
        journal: journal.clone(),
        methods: Arc::default(),
        disconnect: true,
        contradictory_stop: false,
    };
    let (request, input, _fixture) = inputs();
    let result = execute(&backend, request, input, journal).await.unwrap();
    assert!(result.execution.is_err());
    assert!(result.stop.unwrap().scope_stopped);
    assert_eq!(
        backend
            .methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.as_str() == "session/prompt")
            .count(),
        1
    );
}

#[tokio::test]
async fn contradictory_stop_is_unknown_and_pre_cancel_does_not_spawn() {
    let journal = Arc::new(Journal::default());
    let backend = Fixture {
        journal: journal.clone(),
        methods: Arc::default(),
        disconnect: false,
        contradictory_stop: true,
    };
    let (request, input, _fixture) = inputs();
    let result = execute(&backend, request, input, journal.clone())
        .await
        .unwrap();
    assert!(result.stop.is_none());
    journal.0.lock().unwrap().clear();
    let (request, input, _fixture) = inputs();
    input.cancellation.cancel();
    assert_eq!(
        execute(&backend, request, input, journal.clone())
            .await
            .err(),
        Some(DelegationErrorV1::Cancelled)
    );
    assert_eq!(*journal.0.lock().unwrap(), ["failed"]);
}
