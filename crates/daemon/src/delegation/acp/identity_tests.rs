use super::*;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn contracts() -> [AcpNativeIdentityContract; 3] {
    [
        AcpNativeIdentityContract::CodexThreadV1,
        AcpNativeIdentityContract::ClaudeSessionV1,
        AcpNativeIdentityContract::QoderSessionV1,
    ]
}

async fn fixture(stream: tokio::io::DuplexStream, expected_setting: &'static str) -> Vec<String> {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    let mut methods = vec![];
    while let Some(line) = lines.next_line().await.unwrap() {
        let request: Value = serde_json::from_str(&line).unwrap();
        let method = request["method"].as_str().unwrap();
        methods.push(method.to_owned());
        let response = match method {
            "initialize" => json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true},
                "_meta":{"jetbrains":{"air":{"version":1,"capabilities":["sessionFailure"]}}}}),
            "session/new" => {
                assert_eq!(
                    request["params"]["_meta"]["fixtureRunSetting"],
                    expected_setting
                );
                json!({"sessionId":"native-session-a"})
            }
            "session/load" => {
                assert_eq!(request["params"]["sessionId"], "native-session-a");
                assert_eq!(
                    request["params"]["_meta"]["fixtureRunSetting"],
                    expected_setting
                );
                json!({}) // Official response need not carry a custom native-id extension.
            }
            "session/prompt" => json!({"stopReason":"end_turn"}),
            other => panic!("unexpected method {other}"),
        };
        writer
            .write_all(
                format!(
                    "{}\n",
                    json!({"jsonrpc":"2.0","id":request["id"],"result":response})
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    }
    methods
}

async fn execute(
    request: AcpRunInput,
    setting: &'static str,
) -> (Result<AcpRunOutcome, DelegationErrorV1>, Vec<String>) {
    let (client, server) = tokio::io::duplex(8192);
    let server = tokio::spawn(fixture(server, setting));
    let (read, write) = tokio::io::split(client);
    let result = run_acp(read, write, request, Arc::new(tests::Journal::default())).await;
    (result, server.await.unwrap())
}

#[tokio::test]
async fn declared_native_profile_mappings_resume_exact_native_id_without_generic_metadata() {
    for contract in contracts() {
        let mut request = tests::input();
        request.identity_contract = contract.clone();
        request
            .session_meta
            .insert("fixtureRunSetting".into(), json!("first"));
        let first = execute(request, "first").await.0.unwrap();
        assert_eq!(
            first.session.native_session_id.as_deref(),
            Some("native-session-a")
        );
        let serialized = serde_json::to_vec(&first.session).unwrap();
        let binding = serde_json::from_slice(&serialized).unwrap();
        let mut request = tests::input();
        request.identity_contract = contract;
        request.session = AcpSessionStart::Load(binding);
        request
            .session_meta
            .insert("fixtureRunSetting".into(), json!("next"));
        let (next, methods) = execute(request, "next").await;
        assert_eq!(next.unwrap().session, first.session);
        assert_eq!(methods, ["initialize", "session/load", "session/prompt"]);
    }
}

#[test]
fn opaque_metadata_does_not_override_the_declared_native_mapping() {
    for contract in contracts() {
        let response = json!({"_meta":{"sessionId":"unrelated-extension"}});
        assert_eq!(
            contract
                .new_native("native-session-a", &response)
                .unwrap()
                .as_deref(),
            Some("native-session-a")
        );
        let binding = AcpSessionBinding {
            acp_session_id: "native-session-a".into(),
            native_session_id: Some("native-session-a".into()),
        };
        assert!(contract.verify_load(&binding, &response));
    }
}
