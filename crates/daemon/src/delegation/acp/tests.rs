use super::*;
use std::sync::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Default)]
pub(super) struct Journal(Mutex<Vec<&'static str>>);
impl AcpRunJournal for Journal {
    fn session_bound(&self, _: &AcpSessionBinding) -> Result<(), DelegationErrorV1> {
        self.0.lock().unwrap().push("session");
        Ok(())
    }
    fn before_prompt(&self) -> Result<(), DelegationErrorV1> {
        self.0.lock().unwrap().push("send_intent");
        Ok(())
    }
    fn text_update(&self, _: &str) -> Result<(), DelegationErrorV1> {
        Ok(())
    }
    fn allow_permission_once(&self, _: &serde_json::Value) -> bool {
        false
    }
}

pub(super) fn input() -> AcpRunInput {
    AcpRunInput {
        cwd: std::env::temp_dir(),
        prompt: "fixture prompt".into(),
        session: AcpSessionStart::New,
        identity_contract: AcpNativeIdentityContract::ExplicitResponseMetadata,
        session_meta: serde_json::Map::new(),
        native_session_mode: None,
        expected_model: None,
        authentication: None,
        deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(2),
        cancellation: tokio_util::sync::CancellationToken::new(),
    }
}

async fn fixture(
    stream: tokio::io::DuplexStream,
    has_load: bool,
    disconnect_on_prompt: bool,
) -> Vec<String> {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    let mut methods = vec![];
    while let Ok(Some(line)) = lines.next_line().await {
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        let method = request["method"].as_str().unwrap().to_owned();
        methods.push(method.clone());
        let result = match method.as_str() {
            "initialize" => {
                assert_eq!(
                    request
                        .pointer("/params/clientCapabilities/_meta/jetbrains/air/capabilities/0"),
                    Some(&serde_json::json!("sessionFailure"))
                );
                serde_json::json!({"protocolVersion":1,"agentCapabilities":{"loadSession":has_load}})
            }
            "session/new" => {
                serde_json::json!({"sessionId":"acp-a","_meta":{"agentSessionId":"native-a"}})
            }
            "session/load" => serde_json::json!({"_meta":{"agentSessionId":"wrong-native"}}),
            "session/prompt" if disconnect_on_prompt => break,
            "session/prompt" => serde_json::json!({"stopReason":"end_turn"}),
            "session/cancel" => continue,
            _ => panic!("unexpected fixture method: {method}"),
        };
        let response = serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":result});
        writer
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();
    }
    methods
}

#[tokio::test]
async fn terminal_session_failure_metadata_fails_the_prompt() {
    let (client, server) = tokio::io::duplex(16_384);
    let server = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut lines = BufReader::new(reader).lines();
        while let Some(line) = lines.next_line().await.unwrap() {
            let request: Value = serde_json::from_str(&line).unwrap();
            let result = match request["method"].as_str().unwrap() {
                "initialize" => {
                    assert_eq!(
                        request.pointer(
                            "/params/clientCapabilities/_meta/jetbrains/air/capabilities/0"
                        ),
                        Some(&json!("sessionFailure"))
                    );
                    json!({"protocolVersion":1,"agentCapabilities":{}})
                }
                "session/new" => {
                    json!({"sessionId":"acp-a","_meta":{"agentSessionId":"native-a"}})
                }
                "session/prompt" => json!({
                    "stopReason":"end_turn",
                    "_meta":{"jetbrains":{"air":{
                        "version":1,
                        "sessionFailure":{
                            "id":"prompt:error",
                            "revision":1,
                            "category":"request",
                            "severity":"error",
                            "title":"request rejected",
                            "actions":[]
                        }
                    }}}
                }),
                other => panic!("unexpected fixture method: {other}"),
            };
            writer
                .write_all(
                    format!(
                        "{}\n",
                        json!({"jsonrpc":"2.0","id":request["id"],"result":result})
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
    });
    let (read, write) = tokio::io::split(client);
    assert!(matches!(
        run_acp(read, write, input(), Arc::new(Journal::default())).await,
        Err(DelegationErrorV1::PromptFailed)
    ));
    server.await.unwrap();
}

#[test]
fn session_failure_metadata_keeps_warnings_non_terminal_and_rejects_malformed_errors() {
    let warning = json!({"jetbrains":{"air":{
        "version":2,
        "sessionFailure":{
            "id":"prompt:notice:1","revision":1,"category":"unknown",
            "severity":"warning","title":"model fallback","actions":[]
        }
    }}});
    assert_eq!(prompt_session_failure(warning.as_object()), Ok(false));
    let malformed = json!({"jetbrains":{"air":{
        "version":1,
        "sessionFailure":{
            "id":"prompt:error","revision":0,"category":"request",
            "severity":"error","title":"request rejected","actions":[]
        }
    }}});
    assert_eq!(
        prompt_session_failure(malformed.as_object()),
        Err(DelegationErrorV1::ProtocolFailed)
    );
}

#[tokio::test]
async fn finite_acp_records_session_and_send_intent_before_prompt() {
    let (client, server) = tokio::io::duplex(16_384);
    let server = tokio::spawn(fixture(server, true, false));
    let (read, write) = tokio::io::split(client);
    let journal = Arc::new(Journal::default());
    let result = run_acp(read, write, input(), journal.clone())
        .await
        .unwrap();
    assert_eq!(
        result.session.native_session_id.as_deref(),
        Some("native-a")
    );
    assert_eq!(result.stop_reason, "end_turn");
    assert_eq!(*journal.0.lock().unwrap(), ["session", "send_intent"]);
    assert_eq!(
        server.await.unwrap(),
        ["initialize", "session/new", "session/prompt"]
    );
}

#[tokio::test]
async fn native_session_mode_is_advertised_and_selected_before_prompt() {
    let (client, server) = tokio::io::duplex(16_384);
    let server = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut lines = BufReader::new(reader).lines();
        let mut methods = vec![];
        while let Some(line) = lines.next_line().await.unwrap() {
            let request: Value = serde_json::from_str(&line).unwrap();
            let method = request["method"].as_str().unwrap().to_owned();
            methods.push(method.clone());
            let result = match method.as_str() {
                "initialize" => json!({"protocolVersion":1,"agentCapabilities":{}}),
                "session/new" => json!({
                    "sessionId":"acp-a",
                    "modes":{
                        "currentModeId":"default",
                        "availableModes":[
                            {"id":"default","name":"Manual"},
                            {"id":"agent-full-access","name":"Full access"}
                        ]
                    },
                    "_meta":{"agentSessionId":"native-a"}
                }),
                "session/set_mode" => {
                    assert_eq!(request["params"]["sessionId"], "acp-a");
                    assert_eq!(request["params"]["modeId"], "agent-full-access");
                    json!({})
                }
                "session/prompt" => json!({"stopReason":"end_turn"}),
                other => panic!("unexpected fixture method: {other}"),
            };
            writer
                .write_all(
                    format!(
                        "{}\n",
                        json!({
                            "jsonrpc":"2.0","id":request["id"],"result":result
                        })
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
        methods
    });
    let (read, write) = tokio::io::split(client);
    let mut request = input();
    request.native_session_mode = Some("agent-full-access".into());
    run_acp(read, write, request, Arc::new(Journal::default()))
        .await
        .unwrap();
    assert_eq!(
        server.await.unwrap(),
        [
            "initialize",
            "session/new",
            "session/set_mode",
            "session/prompt"
        ]
    );
}

#[tokio::test]
async fn unavailable_native_session_mode_fails_before_prompt() {
    let (client, server) = tokio::io::duplex(16_384);
    let server = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut lines = BufReader::new(reader).lines();
        let mut methods = vec![];
        while let Some(line) = lines.next_line().await.unwrap() {
            let request: Value = serde_json::from_str(&line).unwrap();
            let method = request["method"].as_str().unwrap().to_owned();
            methods.push(method.clone());
            let result = match method.as_str() {
                "initialize" => json!({"protocolVersion":1,"agentCapabilities":{}}),
                "session/new" => json!({
                    "sessionId":"acp-a",
                    "modes":{
                        "currentModeId":"default",
                        "availableModes":[{"id":"default","name":"Manual"}]
                    },
                    "_meta":{"agentSessionId":"native-a"}
                }),
                other => panic!("must fail before {other}"),
            };
            writer
                .write_all(
                    format!(
                        "{}\n",
                        json!({
                            "jsonrpc":"2.0","id":request["id"],"result":result
                        })
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
        methods
    });
    let (read, write) = tokio::io::split(client);
    let mut request = input();
    request.native_session_mode = Some("agent-full-access".into());
    assert_eq!(
        run_acp(read, write, request, Arc::new(Journal::default()))
            .await
            .err(),
        Some(DelegationErrorV1::CapabilityUnavailable)
    );
    assert_eq!(server.await.unwrap(), ["initialize", "session/new"]);
}

#[tokio::test]
async fn rejected_native_session_mode_fails_before_prompt() {
    let (client, server) = tokio::io::duplex(16_384);
    let server = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut lines = BufReader::new(reader).lines();
        let mut methods = vec![];
        while let Some(line) = lines.next_line().await.unwrap() {
            let request: Value = serde_json::from_str(&line).unwrap();
            let method = request["method"].as_str().unwrap().to_owned();
            methods.push(method.clone());
            let response = match method.as_str() {
                "initialize" => json!({
                    "jsonrpc":"2.0", "id":request["id"],
                    "result":{"protocolVersion":1,"agentCapabilities":{}}
                }),
                "session/new" => json!({
                    "jsonrpc":"2.0", "id":request["id"],
                    "result":{
                        "sessionId":"acp-a",
                        "modes":{
                            "currentModeId":"default",
                            "availableModes":[
                                {"id":"default","name":"Manual"},
                                {"id":"agent-full-access","name":"Full access"}
                            ]
                        },
                        "_meta":{"agentSessionId":"native-a"}
                    }
                }),
                "session/set_mode" => json!({
                    "jsonrpc":"2.0", "id":request["id"],
                    "error":{"code":-32602,"message":"mode rejected"}
                }),
                other => panic!("must fail before {other}"),
            };
            writer
                .write_all(format!("{response}\n").as_bytes())
                .await
                .unwrap();
        }
        methods
    });
    let (read, write) = tokio::io::split(client);
    let mut request = input();
    request.native_session_mode = Some("agent-full-access".into());
    assert_eq!(
        run_acp(read, write, request, Arc::new(Journal::default()))
            .await
            .err(),
        Some(DelegationErrorV1::CapabilityUnavailable)
    );
    assert_eq!(
        server.await.unwrap(),
        ["initialize", "session/new", "session/set_mode"]
    );
}

#[tokio::test]
async fn loaded_session_selects_advertised_native_mode_before_prompt() {
    let (client, server) = tokio::io::duplex(16_384);
    let server = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut lines = BufReader::new(reader).lines();
        let mut methods = vec![];
        while let Some(line) = lines.next_line().await.unwrap() {
            let request: Value = serde_json::from_str(&line).unwrap();
            let method = request["method"].as_str().unwrap().to_owned();
            methods.push(method.clone());
            let result = match method.as_str() {
                "initialize" => {
                    json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true}})
                }
                "session/load" => json!({
                    "modes":{
                        "currentModeId":"default",
                        "availableModes":[
                            {"id":"default","name":"Manual"},
                            {"id":"bypassPermissions","name":"Bypass permissions"}
                        ]
                    },
                    "_meta":{"agentSessionId":"native-a"}
                }),
                "session/set_mode" => {
                    assert_eq!(request["params"]["sessionId"], "acp-a");
                    assert_eq!(request["params"]["modeId"], "bypassPermissions");
                    json!({})
                }
                "session/prompt" => json!({"stopReason":"end_turn"}),
                other => panic!("unexpected fixture method: {other}"),
            };
            writer
                .write_all(
                    format!(
                        "{}\n",
                        json!({"jsonrpc":"2.0","id":request["id"],"result":result})
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
        methods
    });
    let (read, write) = tokio::io::split(client);
    let mut request = input();
    request.session = AcpSessionStart::Load(AcpSessionBinding {
        acp_session_id: "acp-a".into(),
        native_session_id: Some("native-a".into()),
    });
    request.native_session_mode = Some("bypassPermissions".into());
    run_acp(read, write, request, Arc::new(Journal::default()))
        .await
        .unwrap();
    assert_eq!(
        server.await.unwrap(),
        [
            "initialize",
            "session/load",
            "session/set_mode",
            "session/prompt"
        ]
    );
}

#[tokio::test]
async fn finite_acp_never_creates_new_session_when_exact_load_is_unavailable() {
    let (client, server) = tokio::io::duplex(16_384);
    let server = tokio::spawn(fixture(server, false, false));
    let (read, write) = tokio::io::split(client);
    let mut request = input();
    request.session = AcpSessionStart::Load(AcpSessionBinding {
        acp_session_id: "acp-a".into(),
        native_session_id: Some("native-a".into()),
    });
    assert_eq!(
        run_acp(read, write, request, Arc::new(Journal::default()))
            .await
            .err(),
        Some(DelegationErrorV1::ResumeUnavailable)
    );
    assert_eq!(server.await.unwrap(), ["initialize"]);
}

#[tokio::test]
async fn finite_acp_rejects_native_identity_change_before_prompt() {
    let (client, server) = tokio::io::duplex(16_384);
    let server = tokio::spawn(fixture(server, true, false));
    let (read, write) = tokio::io::split(client);
    let mut request = input();
    request.session = AcpSessionStart::Load(AcpSessionBinding {
        acp_session_id: "acp-a".into(),
        native_session_id: Some("native-a".into()),
    });
    assert_eq!(
        run_acp(read, write, request, Arc::new(Journal::default()))
            .await
            .err(),
        Some(DelegationErrorV1::ResumeUnavailable)
    );
    assert_eq!(server.await.unwrap(), ["initialize", "session/load"]);
}

#[tokio::test]
async fn finite_acp_does_not_replay_after_prompt_disconnect() {
    let (client, server) = tokio::io::duplex(16_384);
    let server = tokio::spawn(fixture(server, true, true));
    let (read, write) = tokio::io::split(client);
    let journal = Arc::new(Journal::default());
    assert!(
        run_acp(read, write, input(), journal.clone())
            .await
            .is_err()
    );
    assert_eq!(
        server.await.unwrap(),
        ["initialize", "session/new", "session/prompt"]
    );
    assert_eq!(*journal.0.lock().unwrap(), ["session", "send_intent"]);
}
