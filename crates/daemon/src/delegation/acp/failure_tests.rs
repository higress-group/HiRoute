use super::tests::{Journal, input};
use super::*;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

async fn native_cancel_fixture(
    stream: tokio::io::DuplexStream,
    unsolicited: bool,
    stop_reason: &'static str,
) {
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();
    let mut prompt_id = None;
    while let Some(line) = lines.next_line().await.unwrap() {
        let request: Value = serde_json::from_str(&line).unwrap();
        let (id, result) = match request["method"].as_str().unwrap() {
            "initialize" => (
                request["id"].clone(),
                json!({"protocolVersion":1,"agentCapabilities":{}}),
            ),
            "session/new" => (
                request["id"].clone(),
                json!({"sessionId":"acp-a","_meta":{"agentSessionId":"native-a"}}),
            ),
            "session/prompt" if !unsolicited => {
                prompt_id = Some(request["id"].clone());
                continue;
            }
            "session/prompt" => {
                if stop_reason == "rpc_500" {
                    write.write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":500,"message":"native model request failed"}})).as_bytes()).await.unwrap();
                    continue;
                }
                write.write_all(format!("{}\n", json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"acp-a","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"[Error] You do not have access to this model service"}}}})).as_bytes()).await.unwrap();
                (request["id"].clone(), json!({"stopReason":stop_reason}))
            }
            "session/cancel" => (prompt_id.take().unwrap(), json!({"stopReason":"cancelled"})),
            method => panic!("unexpected method {method}"),
        };
        write
            .write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":id,"result":result})).as_bytes())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn native_unsolicited_cancel_is_failed_not_a_user_cancel() {
    let (client, server) = tokio::io::duplex(8192);
    let server = tokio::spawn(native_cancel_fixture(server, true, "cancelled"));
    let (read, write) = tokio::io::split(client);
    let result = run_acp(read, write, input(), Arc::new(Journal::default())).await;
    assert!(matches!(result, Err(DelegationErrorV1::PromptFailed)));
    server.await.unwrap();
}

#[tokio::test]
async fn deadline_keeps_its_cause_after_native_cancel_acknowledgement() {
    let (client, server) = tokio::io::duplex(8192);
    let server = tokio::spawn(native_cancel_fixture(server, false, "cancelled"));
    let (read, write) = tokio::io::split(client);
    let mut request = input();
    request.deadline = Instant::now() + Duration::from_millis(250);
    let result = run_acp(read, write, request, Arc::new(Journal::default())).await;
    assert!(matches!(result, Err(DelegationErrorV1::DeadlineExceeded)));
    server.await.unwrap();
}

#[tokio::test]
async fn assistant_error_like_text_does_not_authorize_a_host_failure_classification() {
    let (client, server) = tokio::io::duplex(8192);
    let server = tokio::spawn(native_cancel_fixture(server, true, "end_turn"));
    let (read, write) = tokio::io::split(client);
    let result = run_acp(read, write, input(), Arc::new(Journal::default()))
        .await
        .unwrap();
    assert_eq!(result.stop_reason, "end_turn");
    assert!(result.text.contains("You do not have access"));
    server.await.unwrap();
}

#[tokio::test]
async fn qoder_terminal_rpc_500_is_a_known_failure_without_guessing_from_text() {
    for (identity, expected) in [
        (
            AcpNativeIdentityContract::QoderSessionV1,
            DelegationErrorV1::PromptFailed,
        ),
        (
            AcpNativeIdentityContract::ExplicitResponseMetadata,
            DelegationErrorV1::ProtocolFailed,
        ),
    ] {
        let (client, server) = tokio::io::duplex(8192);
        let server = tokio::spawn(native_cancel_fixture(server, true, "rpc_500"));
        let (read, write) = tokio::io::split(client);
        let mut request = input();
        request.identity_contract = identity;
        assert!(
            matches!(run_acp(read, write, request, Arc::new(Journal::default())).await, Err(error) if error == expected)
        );
        server.await.unwrap();
    }
}
