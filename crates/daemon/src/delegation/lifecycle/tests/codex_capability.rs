use super::*;
use crate::delegation::local_worker::LocalWorkerPlatform;
use crate::delegation::progress::ObservationProgressWriter;
use hiroute_domain::WorkspaceId;
use hiroute_observation::managed_text::{
    ManagedTextProgressRead, ManagedTextProgressTarget, ManagedTextScope,
};
use hiroute_observation::{DigestAuthority, LocalObservationStore};
use std::time::{SystemTime, UNIX_EPOCH};

#[tokio::test]
async fn spawned_codex_capability_failure_survives_cleanup_and_progress_reopen() {
    for continued in [false, true] {
        for supported in [false, true] {
            let (mut request, mut input, fixture) = inputs();
            let script = fixture.path().join("adapter.py");
            let transcript = fixture.path().join("requests");
            std::fs::write(&script, r#"import json, sys
supported = sys.argv[1] == 'supported'
for line in sys.stdin:
    request = json.loads(line)
    method = request['method']
    with open(sys.argv[2], 'a') as log:
        log.write(method + '\n')
    modes = {'currentModeId':'agent-full-access','availableModes':[{'id':'agent-full-access','name':'Autonomous'}]}
    if method == 'initialize':
        result = {'protocolVersion':1,'agentCapabilities':{'loadSession':True},'agentInfo':{'name':'PRIVATE_ADAPTER_OUTPUT','version':'PRIVATE_VERSION'}}
        if supported:
            result['_meta'] = {'jetbrains':{'air':{'version':1,'capabilities':['sessionFailure']}}}
    elif method == 'session/new':
        result = {'sessionId':'native-session','modes':modes}
    elif method == 'session/load':
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
            request.profile.executable = "/usr/bin/python3".into();
            request.profile.args = vec![
                script,
                if supported {
                    "supported"
                } else {
                    "unsupported"
                }
                .into(),
                transcript.clone(),
            ];
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64;
            request.deadline_unix_ms = now_ms as u64 + 5_000;
            input.deadline = Instant::now() + Duration::from_secs(5);
            if continued {
                input.session = AcpSessionStart::Load(AcpSessionBinding {
                    acp_session_id: "native-session".into(),
                    native_session_id: Some("native-session".into()),
                });
            }
            let observation = tempfile::tempdir().unwrap();
            let store = Arc::new(
                LocalObservationStore::open(observation.path(), DigestAuthority::new([9; 32]))
                    .unwrap(),
            );
            let target = ManagedTextProgressTarget {
                scope: ManagedTextScope {
                    workspace_id: WorkspaceId::default(),
                    task_id: "capability-task".into(),
                    run_id: "capability-run".into(),
                },
                created_at_ms: now_ms,
            };
            let journal = Arc::new(Journal(
                Mutex::default(),
                None,
                Default::default(),
                Some(WorkerProgressWriter::new(Arc::new(
                    ObservationProgressWriter::new(Arc::clone(&store), target.clone()),
                ))),
            ));
            let private_root = request.profile.private_root.clone();
            let result = execute(
                &LocalWorkerPlatform::default(),
                request,
                input,
                journal.clone(),
            )
            .await
            .unwrap();
            assert!(result.stop.unwrap().scope_stopped);
            assert!(result.stop_intent_error.is_none());
            assert!(!private_root.exists());
            let methods = std::fs::read_to_string(transcript).unwrap();
            let events = journal.0.lock().unwrap().clone();
            assert!(events.contains(&"spawned"));
            assert!(events.contains(&"revoke"));
            drop(journal);
            drop(store);
            let reopened =
                LocalObservationStore::open(observation.path(), DigestAuthority::new([9; 32]))
                    .unwrap();
            let progress = reopened
                .managed_text_progress_read(&target, None, 4096)
                .unwrap();
            if supported {
                assert!(result.execution.unwrap().text.is_empty());
                assert!(methods.contains("session/prompt"));
                assert!(methods.contains(if continued {
                    "session/load"
                } else {
                    "session/new"
                }));
                assert!(matches!(progress, ManagedTextProgressRead::Missing { .. }));
            } else {
                assert!(matches!(
                    result.execution,
                    Err(DelegationErrorV1::CapabilityUnavailable)
                ));
                assert_eq!(methods, "initialize\n");
                assert!(events.contains(&"failed"));
                assert!(!events.contains(&"prompt") && !events.contains(&"completed"));
                let ManagedTextProgressRead::Available(page) = progress else {
                    panic!(
                        "missing adapter capability must leave readable Worker progress after cleanup"
                    );
                };
                assert_eq!(page.text.matches("[HiRoute]").count(), 1);
                assert!(page.text.contains("codex-acp"));
                assert!(page.text.contains("sessionFailure"));
                assert!(page.text.contains("Update") && page.text.contains("select"));
                assert!(!page.text.contains("PRIVATE_"));
                assert!(!page.text.contains("fixture-token"));
            }
        }
    }
}
