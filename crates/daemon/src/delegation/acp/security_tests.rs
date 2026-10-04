use super::*;
use crate::delegation::progress::{
    ObservationProgressWriter, ProgressBatchWriter, ProgressCapture,
};
use hiroute_domain::WorkspaceId;
use hiroute_observation::managed_text::{
    ManagedTextProgressRead, ManagedTextProgressTarget, ManagedTextScope,
};
use hiroute_observation::{DigestAuthority, LocalObservationStore};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

struct Journal;
impl AcpRunJournal for Journal {
    fn session_bound(&self, _: &AcpSessionBinding) -> Result<(), DelegationErrorV1> {
        Ok(())
    }
    fn before_prompt(&self) -> Result<(), DelegationErrorV1> {
        Ok(())
    }
    fn text_update(&self, _: &str) -> Result<(), DelegationErrorV1> {
        Ok(())
    }
    fn allow_permission_once(&self, _: &Value) -> bool {
        true
    }
}

fn input() -> AcpRunInput {
    AcpRunInput {
        cwd: std::env::temp_dir(),
        prompt: "fixture".into(),
        session: AcpSessionStart::New,
        identity_contract: AcpNativeIdentityContract::ExplicitResponseMetadata,
        session_meta: Map::new(),
        native_session_mode: None,
        expected_model: None,
        authentication: None,
        deadline: Instant::now() + Duration::from_secs(3),
        cancellation: CancellationToken::new(),
    }
}

async fn send(writer: &mut (impl AsyncWrite + Unpin), value: Value) {
    writer
        .write_all(format!("{value}\n").as_bytes())
        .await
        .unwrap();
}

#[tokio::test]
async fn cancellation_interrupts_stalled_initialization_without_waiting_for_run_deadline() {
    let (client, server) = tokio::io::duplex(8192);
    let request = input();
    let cancel = request.cancellation.clone();
    let server = tokio::spawn(async move {
        let mut reader = BufReader::new(server);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&line).unwrap()["method"],
            "initialize"
        );
        cancel.cancel();
        line.clear();
        assert_eq!(reader.read_line(&mut line).await.unwrap(), 0);
    });
    let (read, write) = tokio::io::split(client);
    let result = timeout(
        Duration::from_secs(1),
        run_acp(read, write, request, Arc::new(Journal)),
    )
    .await
    .unwrap();
    assert_eq!(result.err(), Some(DelegationErrorV1::Cancelled));
    server.await.unwrap();
}

#[tokio::test]
async fn permission_never_selects_allow_always_even_when_current_tool_is_allowed() {
    for once in [false, true] {
        let (client, server) = tokio::io::duplex(8192);
        let server = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut lines = BufReader::new(reader).lines();
            let mut prompt_id = Value::Null;
            while let Some(line) = lines.next_line().await.unwrap() {
                let request: Value = serde_json::from_str(&line).unwrap();
                if request["id"] == "permission" {
                    let outcome = &request["result"]["outcome"];
                    if once {
                        assert_eq!(outcome["outcome"], "selected");
                        assert_eq!(outcome["optionId"], "once");
                    } else {
                        assert_eq!(outcome["outcome"], "cancelled");
                    }
                    send(
                        &mut writer,
                        json!({"jsonrpc":"2.0","id":prompt_id,"result":{"stopReason":"end_turn"}}),
                    )
                    .await;
                    continue;
                }
                let result = match request["method"].as_str().unwrap() {
                    "initialize" => json!({"protocolVersion":1,"agentCapabilities":{}}),
                    "session/new" => json!({"sessionId":"s"}),
                    "session/prompt" => {
                        prompt_id = request["id"].clone();
                        let mut options = vec![
                            json!({"optionId":"always","name":"Always","kind":"allow_always"}),
                        ];
                        if once {
                            options
                                .push(json!({"optionId":"once","name":"Once","kind":"allow_once"}));
                        }
                        send(&mut writer, json!({"jsonrpc":"2.0","id":"permission","method":"session/request_permission",
                            "params":{"sessionId":"s","toolCall":{"toolCallId":"read"},"options":options}})).await;
                        continue;
                    }
                    method => panic!("unexpected method {method}"),
                };
                send(
                    &mut writer,
                    json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
                )
                .await;
            }
        });
        let (read, write) = tokio::io::split(client);
        run_acp(read, write, input(), Arc::new(Journal))
            .await
            .unwrap();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn exact_load_history_is_not_reimported_as_current_run_output() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(
        LocalObservationStore::open(directory.path(), DigestAuthority::new([41; 32])).unwrap(),
    );
    let created_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap();
    let target = ManagedTextProgressTarget {
        scope: ManagedTextScope {
            workspace_id: WorkspaceId::default(),
            task_id: "task-acp-progress".into(),
            run_id: "run-acp-progress".into(),
        },
        created_at_ms,
    };
    let writer: Arc<dyn ProgressBatchWriter> = Arc::new(ObservationProgressWriter::new(
        Arc::clone(&store),
        target.clone(),
    ));
    let capture = ProgressCapture::start(writer);
    let (client, server) = tokio::io::duplex(8192);
    let server = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut lines = BufReader::new(reader).lines();
        while let Some(line) = lines.next_line().await.unwrap() {
            let request: Value = serde_json::from_str(&line).unwrap();
            let method = request["method"].as_str().unwrap();
            let result = match method {
                "initialize" => {
                    json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true}})
                }
                "session/load" | "session/prompt" => {
                    let text = if method == "session/load" {
                        "old history"
                    } else {
                        "new result"
                    };
                    send(&mut writer, json!({"jsonrpc":"2.0","method":"session/update","params":{
                        "sessionId":"s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":text}}
                    }})).await;
                    if method == "session/load" {
                        json!({"_meta":{"agentSessionId":"native"}})
                    } else {
                        json!({"stopReason":"end_turn"})
                    }
                }
                other => panic!("unexpected method {other}"),
            };
            send(
                &mut writer,
                json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
            )
            .await;
        }
    });
    let mut request = input();
    request.session = AcpSessionStart::Load(AcpSessionBinding {
        acp_session_id: "s".into(),
        native_session_id: Some("native".into()),
    });
    let (read, write) = tokio::io::split(client);
    let result = run_acp_with_progress(
        read,
        write,
        request,
        Arc::new(Journal),
        Some(capture.sink()),
    )
    .await
    .unwrap();
    assert_eq!(result.text, "new result");
    capture.stop_and_flush().await;
    let ManagedTextProgressRead::Available(page) =
        store.managed_text_progress_read(&target, None, 32).unwrap()
    else {
        panic!("expected saved progress");
    };
    assert_eq!(page.text, "new result");
    server.await.unwrap();
}

#[tokio::test]
async fn prompt_cancellation_is_sent_once_and_late_success_does_not_win() {
    let (client, server) = tokio::io::duplex(8192);
    let request = input();
    let cancellation = request.cancellation.clone();
    let server = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut lines = BufReader::new(reader).lines();
        let mut prompt_id = Value::Null;
        let mut methods = vec![];
        while let Some(line) = lines.next_line().await.unwrap() {
            let request: Value = serde_json::from_str(&line).unwrap();
            let method = request["method"].as_str().unwrap();
            methods.push(method.to_owned());
            let result = match method {
                "initialize" => json!({"protocolVersion":1,"agentCapabilities":{}}),
                "session/new" => json!({"sessionId":"s"}),
                "session/prompt" => {
                    prompt_id = request["id"].clone();
                    cancellation.cancel();
                    continue;
                }
                "session/cancel" => {
                    send(
                        &mut writer,
                        json!({"jsonrpc":"2.0","id":prompt_id,"result":{"stopReason":"end_turn"}}),
                    )
                    .await;
                    continue;
                }
                other => panic!("unexpected method {other}"),
            };
            send(
                &mut writer,
                json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
            )
            .await;
        }
        methods
    });
    let (read, write) = tokio::io::split(client);
    let result = run_acp(read, write, request, Arc::new(Journal))
        .await
        .unwrap();
    assert_eq!(result.stop_reason, "cancelled");
    assert_eq!(
        server.await.unwrap(),
        [
            "initialize",
            "session/new",
            "session/prompt",
            "session/cancel"
        ]
    );
}

#[tokio::test]
async fn cancellation_finitely_closes_an_agent_that_never_resolves_the_prompt() {
    let (client, server) = tokio::io::duplex(8192);
    let mut request = input();
    request.deadline = Instant::now() + Duration::from_secs(30);
    let cancellation = request.cancellation.clone();
    let server = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut lines = BufReader::new(reader).lines();
        let mut methods = vec![];
        while let Some(line) = lines.next_line().await.unwrap() {
            let request: Value = serde_json::from_str(&line).unwrap();
            let method = request["method"].as_str().unwrap();
            methods.push(method.to_owned());
            let result = match method {
                "initialize" => json!({"protocolVersion":1,"agentCapabilities":{}}),
                "session/new" => json!({"sessionId":"s"}),
                "session/prompt" => {
                    cancellation.cancel();
                    continue;
                }
                "session/cancel" => continue,
                other => panic!("unexpected method {other}"),
            };
            send(
                &mut writer,
                json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
            )
            .await;
        }
        methods
    });
    let (read, write) = tokio::io::split(client);
    let result = timeout(
        Duration::from_secs(8),
        run_acp(read, write, request, Arc::new(Journal)),
    )
    .await
    .expect("cancellation must not wait for the run deadline");
    assert_eq!(result.err(), Some(DelegationErrorV1::Cancelled));
    assert_eq!(
        server.await.unwrap(),
        [
            "initialize",
            "session/new",
            "session/prompt",
            "session/cancel"
        ]
    );
}

#[tokio::test]
async fn overlong_wire_frame_fails_before_unbounded_json_allocation() {
    let (client, server) = tokio::io::duplex(8192);
    let server = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut lines = BufReader::new(reader).lines();
        assert!(lines.next_line().await.unwrap().is_some());
        let bytes = vec![b'x'; MAX_ACP_FRAME_BYTES + 1];
        // Reader may close as soon as it exceeds the bound; BrokenPipe is expected.
        let _ = writer.write_all(&bytes).await;
    });
    let (read, write) = tokio::io::split(client);
    let result = timeout(
        Duration::from_secs(2),
        run_acp(read, write, input(), Arc::new(Journal)),
    )
    .await
    .unwrap();
    assert_eq!(result.err(), Some(DelegationErrorV1::ProtocolFailed));
    server.await.unwrap();
}
