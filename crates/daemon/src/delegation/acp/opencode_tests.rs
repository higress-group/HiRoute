use super::tests::{Journal, input};
use super::*;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn options(mode: &str, model: &str) -> Vec<Value> {
    vec![
        json!({"id":"mode","name":"Mode","type":"select","currentValue":mode,"options":[{"value":"owned","name":"Owned"}]}),
        json!({"id":"model","name":"Model","type":"select","currentValue":model,"options":[{"value":"provider/route","name":"Route"}]}),
    ]
}

async fn fixture(
    start: AcpSessionStart,
    invalid: Option<&str>,
) -> (Result<AcpRunOutcome, DelegationErrorV1>, Vec<String>) {
    let invalid = invalid.map(str::to_owned);
    let (client, server) = tokio::io::duplex(16_384);
    let server = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut lines = BufReader::new(reader).lines();
        let mut calls = vec![];
        while let Some(line) = lines.next_line().await.unwrap() {
            let request: Value = serde_json::from_str(&line).unwrap();
            let method = request["method"].as_str().unwrap();
            calls.push(if method == "session/set_config_option" {
                format!("set:{}", request["params"]["configId"].as_str().unwrap())
            } else {
                method.into()
            });
            let result = match method {
                "initialize" => {
                    json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true}})
                }
                "session/new" | "session/load" => {
                    let mut opts = options("daily", "daily/model");
                    match invalid.as_deref() {
                        Some("missing") => {
                            opts.remove(0);
                        }
                        Some("duplicate") => opts.push(opts[0].clone()),
                        Some("wrong_type") => opts[0]["type"] = json!("boolean"),
                        _ => {}
                    }
                    if method == "session/new" {
                        json!({"sessionId":"native-a","configOptions":opts})
                    } else {
                        json!({"configOptions":opts})
                    }
                }
                "session/set_config_option" if request["params"]["configId"] == "mode" => {
                    assert_eq!(request["params"]["value"], "owned");
                    json!({"configOptions":options(if invalid.as_deref()==Some("mismatch") {"daily"} else {"owned"},"changed-by-mode")})
                }
                "session/set_config_option" => {
                    assert_eq!(request["params"]["configId"], "model");
                    assert_eq!(request["params"]["value"], "provider/route");
                    json!({"configOptions":options("owned","provider/route")})
                }
                "session/prompt" => json!({"stopReason":"end_turn"}),
                other => panic!("unexpected {other}"),
            };
            if writer
                .write_all(
                    format!(
                        "{}\n",
                        json!({"jsonrpc":"2.0","id":request["id"],"result":result})
                    )
                    .as_bytes(),
                )
                .await
                .is_err()
            {
                break;
            }
        }
        calls
    });
    let (read, write) = tokio::io::split(client);
    let mut request = input();
    request.session = start;
    request.identity_contract = AcpNativeIdentityContract::OpenCodeSessionV1;
    request.native_session_mode = Some("owned".into());
    request.expected_model = Some("provider/route".into());
    let result = run_acp(read, write, request, Arc::new(Journal::default())).await;
    (result, server.await.unwrap())
}

#[tokio::test]
async fn opencode_typed_mode_then_final_model_before_new_or_load_prompt() {
    for start in [
        AcpSessionStart::New,
        AcpSessionStart::Load(AcpSessionBinding {
            acp_session_id: "native-a".into(),
            native_session_id: Some("native-a".into()),
        }),
    ] {
        let (result, calls) = fixture(start, None).await;
        assert!(result.is_ok(), "{:?}", result.err());
        assert_eq!(&calls[2..], ["set:mode", "set:model", "session/prompt"]);
    }
}

#[tokio::test]
async fn invalid_opencode_mode_never_sends_prompt() {
    for failure in ["missing", "duplicate", "wrong_type", "mismatch"] {
        let (result, calls) = fixture(AcpSessionStart::New, Some(failure)).await;
        assert!(result.is_err(), "{failure}");
        assert!(!calls.iter().any(|c| c == "session/prompt"), "{failure}");
    }
}
