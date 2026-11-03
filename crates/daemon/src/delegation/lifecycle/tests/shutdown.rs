use super::*;
use crate::delegation::local_worker::LocalWorkerPlatform;
use std::time::SystemTime;

#[tokio::test]
async fn daemon_shutdown_stops_the_owned_process_group_after_prompt_without_user_cancel() {
    let (mut request, mut input, root) = inputs();
    let script = root.path().join("shutdown-adapter.py");
    let heartbeat = root.path().join("heartbeat");
    let methods = root.path().join("methods");
    fs::write(&script, r#"import json, subprocess, sys
pending = None
for line in sys.stdin:
    request = json.loads(line)
    method = request['method']
    with open(sys.argv[2], 'a') as log:
        log.write(method + '\n')
    if method == 'initialize':
        result = {'protocolVersion':1,'agentCapabilities':{'loadSession':True},'_meta':{'jetbrains':{'air':{'version':1,'capabilities':['sessionFailure']}}}}
    elif method == 'session/new':
        result = {'sessionId':'native-session','modes':{'currentModeId':'agent-full-access','availableModes':[{'id':'agent-full-access','name':'Autonomous'}]}}
    elif method == 'session/set_mode':
        result = {}
    elif method == 'session/prompt':
        pending = request['id']
        subprocess.Popen([sys.executable, '-c', "import sys,time\nwhile True:\n with open(sys.argv[1],'a') as f: f.write('x')\n time.sleep(.02)", sys.argv[1]])
        continue
    elif method == 'session/cancel':
        print(json.dumps({'jsonrpc':'2.0','id':pending,'result':{'stopReason':'cancelled'}}), flush=True)
        continue
    else:
        raise AssertionError(method)
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}), flush=True)
"#).unwrap();
    request.profile.executable = PathBuf::from("/usr/bin/python3");
    request.profile.args = vec![script, heartbeat.clone(), methods.clone()];
    request.deadline_unix_ms = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 30_000;
    input.deadline = Instant::now() + Duration::from_secs(30);
    let shutdown = CancellationToken::new();
    let trigger = shutdown.clone();
    let observed = heartbeat.clone();
    let stop = tokio::spawn(async move {
        timeout(Duration::from_secs(5), async {
            while fs::metadata(&observed).map_or(true, |file| file.len() == 0) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        trigger.cancel();
    });
    let private_root = request.profile.private_root.clone();
    let journal = Arc::new(Journal::default());
    let platform = LocalWorkerPlatform::default();
    let result = execute_with_shutdown(&platform, request, input, journal.clone(), shutdown)
        .await
        .unwrap();
    stop.await.unwrap();
    assert!(matches!(
        result.execution,
        Err(DelegationErrorV1::ProtocolFailed)
    ));
    let evidence = result.stop.unwrap();
    assert_eq!(evidence.scope, WorkerStopScope::ProcessGroup);
    assert!(evidence.scope_stopped && !evidence.residual_unknown);
    assert!(matches!(
        evidence.observation,
        WorkerObservation::Exited { .. }
    ));
    let events = journal.0.lock().unwrap().clone();
    assert!(
        events.iter().position(|event| *event == "failed").unwrap()
            < events.iter().position(|event| *event == "revoke").unwrap()
    );
    assert!(!events.contains(&"completed"));
    let length = fs::metadata(&heartbeat).unwrap().len();
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(
        fs::metadata(&heartbeat).unwrap().len(),
        length,
        "owned child side effects continued after stop proof"
    );
    assert!(!private_root.exists());
    let methods = fs::read_to_string(methods).unwrap();
    assert_eq!(
        methods
            .lines()
            .filter(|method| *method == "session/prompt")
            .count(),
        1
    );
    assert!(methods.lines().any(|method| method == "session/cancel"));
}

#[tokio::test]
async fn shutdown_before_preparation_does_not_spawn() {
    let (request, input, _root) = inputs();
    let journal = Arc::new(Journal::default());
    let platform = Fixture {
        journal: journal.clone(),
        methods: Arc::default(),
        disconnect: false,
        contradictory_stop: false,
    };
    let shutdown = CancellationToken::new();
    shutdown.cancel();
    assert!(matches!(
        execute_with_shutdown(&platform, request, input, journal.clone(), shutdown).await,
        Err(DelegationErrorV1::ProtocolFailed)
    ));
    assert_eq!(*journal.0.lock().unwrap(), ["failed"]);
    assert!(platform.methods.lock().unwrap().is_empty());
}
