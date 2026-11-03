use super::tests::{Journal, input};
use super::*;
use crate::delegation::progress::{ProgressBatchWriter, ProgressCapture};
use hiroute_observation::managed_text::{ManagedTextError, ManagedTextProgressWriteOutcome};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Default)]
struct Progress(Mutex<String>);

impl ProgressBatchWriter for Progress {
    fn write_batch(
        &self,
        text: &str,
        _: bool,
        _: i64,
    ) -> Result<ManagedTextProgressWriteOutcome, ManagedTextError> {
        self.0.lock().unwrap().push_str(text);
        Ok(ManagedTextProgressWriteOutcome::Noop)
    }
}

async fn exercise_mode(
    modes: Option<Value>,
    continuation: bool,
    succeeds: bool,
) -> Result<AcpRunOutcome, DelegationErrorV1> {
    let catalog_missing = modes.is_none();
    let (client, server) = tokio::io::duplex(16_384);
    let peer = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut lines = BufReader::new(reader).lines();
        let mut methods = Vec::new();
        while let Some(line) = lines.next_line().await.unwrap() {
            let request: Value = serde_json::from_str(&line).unwrap();
            let method = request["method"].as_str().unwrap();
            methods.push(method.to_owned());
            let result = match method {
                "initialize" => {
                    json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true}})
                }
                "session/new" | "session/load" => {
                    let mut result = json!({"sessionId":"claude-session"});
                    if let Some(modes) = &modes {
                        result["modes"] = modes.clone();
                    }
                    result
                }
                "session/set_mode" if succeeds => {
                    assert_eq!(request["params"]["modeId"], "bypassPermissions");
                    json!({})
                }
                "session/prompt" if succeeds => json!({"stopReason":"end_turn"}),
                _ => panic!("unexpected ACP request: {method}"),
            };
            let response = json!({"jsonrpc":"2.0","id":request["id"],"result":result});
            writer
                .write_all(format!("{response}\n").as_bytes())
                .await
                .unwrap();
        }
        methods
    });
    let (reader, writer) = tokio::io::split(client);
    let journal = Arc::new(Journal::default());
    let mut request = input();
    request.identity_contract = AcpNativeIdentityContract::ClaudeSessionV1;
    request.native_session_mode = Some("bypassPermissions".into());
    if continuation {
        request.session = AcpSessionStart::Load(AcpSessionBinding {
            acp_session_id: "claude-session".into(),
            native_session_id: Some("claude-session".into()),
        });
    }
    let progress = Arc::new(Progress::default());
    let capture = ProgressCapture::start(progress.clone());
    let result = run_acp_with_progress(
        reader,
        writer,
        request,
        journal.clone(),
        Some(capture.sink()),
    )
    .await;
    capture.stop_and_flush().await;
    let text = progress.0.lock().unwrap().clone();
    if !succeeds {
        assert!(
            text.contains(if catalog_missing {
                "returned no session mode catalog"
            } else {
                "session mode catalog does not offer bypassPermissions"
            }),
            "{text}"
        );
    }
    let mut expected = vec![
        "initialize",
        if continuation {
            "session/load"
        } else {
            "session/new"
        },
    ];
    if succeeds {
        expected.extend(["session/set_mode", "session/prompt"]);
    }
    assert_eq!(peer.await.unwrap(), expected);
    assert_eq!(journal.0.lock().unwrap().contains(&"send_intent"), succeeds);
    result
}

#[tokio::test]
async fn missing_claude_autonomous_mode_is_specific_and_never_falls_back_to_manual() {
    for continuation in [false, true] {
        for modes in [
            None,
            Some(json!({
                "currentModeId":"default",
                "availableModes":[{"id":"default","name":"Manual"},{"id":"acceptEdits","name":"Accept edits"}]
            })),
        ] {
            let error = exercise_mode(modes, continuation, false)
                .await
                .err()
                .unwrap();
            let failure = hiroute_domain::delegation::WorkerExecutionFailureV1 {
                stage: Stage::ModeSelection,
                error,
            };
            assert_eq!(
                failure.message_key(),
                "worker.execution.mode_selection.capability_unavailable"
            );
            assert!(failure.next_action_hint().contains("Auto model support"));
        }
    }
}

#[tokio::test]
async fn claude_modes_are_selected_from_new_and_load_without_initialize_modes() {
    for continuation in [false, true] {
        exercise_mode(Some(json!({
            "currentModeId":"default",
            "availableModes":[{"id":"default","name":"Manual"},{"id":"bypassPermissions","name":"Bypass permissions"}]
        })), continuation, true).await.unwrap();
    }
}

const PLAN: &str = "hiroute/frozen-auto-plan";

fn auto_options(mode: &str, model: &str, offer_auto: bool) -> Value {
    let mut modes = vec![json!({"value":"default","name":"Manual"})];
    if offer_auto {
        modes.push(json!({"value":"auto","name":"Auto"}));
    }
    json!([
        {"id":"mode","name":"Mode","type":"select","currentValue":mode,"options":modes},
        {"id":"model","name":"Model","type":"select","currentValue":model,
         "options":[{"value":PLAN,"name":"Frozen Plan"}]}
    ])
}

async fn exercise_auto(
    continuation: bool,
    returned_mode: &'static str,
    returned_model: &'static str,
    offer_auto: bool,
) -> (
    Result<AcpRunOutcome, DelegationErrorV1>,
    Vec<String>,
    Vec<&'static str>,
) {
    let (client, server) = tokio::io::duplex(16_384);
    let peer = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut lines = BufReader::new(reader).lines();
        let mut methods = Vec::new();
        while let Some(line) = lines.next_line().await.unwrap() {
            let request: Value = serde_json::from_str(&line).unwrap();
            let method = request["method"].as_str().unwrap();
            let result = match method {
                "initialize" => {
                    json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true}})
                }
                "session/new" | "session/load" => json!({
                    "sessionId":"auto-session",
                    "modes":{"currentModeId":"default","availableModes":[{"id":"auto","name":"Auto"}]},
                    "configOptions":auto_options("default", "ambient-model", false),
                }),
                "session/set_config_option" => {
                    let id = request["params"]["configId"].as_str().unwrap();
                    methods.push(format!("set:{id}"));
                    let options = if id == "model" {
                        assert_eq!(request["params"]["value"], PLAN);
                        auto_options("default", PLAN, offer_auto)
                    } else {
                        assert_eq!(id, "mode");
                        assert_eq!(request["params"]["value"], "auto");
                        auto_options(returned_mode, returned_model, offer_auto)
                    };
                    json!({"configOptions":options})
                }
                "session/set_mode" => json!({}),
                "session/prompt" => json!({"stopReason":"end_turn"}),
                _ => panic!("unexpected ACP method: {method}"),
            };
            if method != "session/set_config_option" {
                methods.push(method.to_owned());
            }
            let response = json!({"jsonrpc":"2.0","id":request["id"],"result":result});
            writer
                .write_all(format!("{response}\n").as_bytes())
                .await
                .unwrap();
        }
        methods
    });
    let journal = Arc::new(Journal::default());
    let mut request = input();
    request.identity_contract = AcpNativeIdentityContract::ClaudeSessionV1;
    request.native_session_mode = Some("auto".into());
    request.expected_model = Some(PLAN.into());
    if continuation {
        request.session = AcpSessionStart::Load(AcpSessionBinding {
            acp_session_id: "auto-session".into(),
            native_session_id: Some("auto-session".into()),
        });
    }
    let (reader, writer) = tokio::io::split(client);
    let result = run_acp(reader, writer, request, journal.clone()).await;
    let methods = peer.await.unwrap();
    let events = journal.0.lock().unwrap().clone();
    (result, methods, events)
}

#[tokio::test]
async fn root_auto_requires_confirmed_mode_and_frozen_model_before_new_or_loaded_prompt() {
    for continuation in [false, true] {
        for (mode, model, offer_auto) in [
            ("auto", PLAN, true),
            ("acceptEdits", PLAN, true),
            ("default", PLAN, true),
            ("auto", "another-model", true),
            ("auto", PLAN, false),
        ] {
            let (result, methods, events) =
                exercise_auto(continuation, mode, model, offer_auto).await;
            let success = mode == "auto" && model == PLAN && offer_auto;
            if success {
                assert!(result.is_ok());
            } else {
                assert_eq!(result.err(), Some(DelegationErrorV1::CapabilityUnavailable));
            }
            let mut expected = vec![
                "initialize",
                if continuation {
                    "session/load"
                } else {
                    "session/new"
                },
                "set:model",
            ];
            if offer_auto {
                expected.push("set:mode");
            }
            if success {
                expected.push("session/prompt");
            }
            assert_eq!(methods, expected);
            assert_eq!(events.contains(&"send_intent"), success);
        }
    }
}
