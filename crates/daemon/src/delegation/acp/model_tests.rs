//! A native model override must never turn a delegated task into a different Plan.
use super::*;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const PLAN_MODEL: &str = "hiroute/frozen-plan";

#[derive(Default)]
struct Journal(Mutex<Vec<&'static str>>);

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

    fn allow_permission_once(&self, _: &Value) -> bool {
        false
    }
}

fn selector(current: &str, options: Value) -> Value {
    json!({
        "id":"model", "name":"Model", "type":"select",
        "currentValue":current, "options":options,
    })
}

fn plan_option() -> Value {
    json!({"value":PLAN_MODEL,"name":"Frozen Plan"})
}

async fn run_fixture(
    load: bool,
    config_options: Value,
    selection_reply: Option<Result<Value, Value>>,
) -> (
    Result<AcpRunOutcome, DelegationErrorV1>,
    Vec<String>,
    Vec<&'static str>,
) {
    let (client, server) = tokio::io::duplex(16_384);
    let server = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut lines = BufReader::new(reader).lines();
        let mut methods = vec![];
        while let Some(line) = lines.next_line().await.unwrap() {
            let request: Value = serde_json::from_str(&line).unwrap();
            let method = request["method"].as_str().unwrap();
            methods.push(method.to_owned());
            let result = match method {
                "initialize" => Ok(json!({
                    "protocolVersion":1,"agentCapabilities":{"loadSession":true},
                })),
                "session/new" | "session/load" => {
                    assert_eq!(method == "session/load", load);
                    if load {
                        assert_eq!(request["params"]["sessionId"], "acp-a");
                    }
                    Ok(json!({
                        "sessionId":"acp-a", "_meta":{"agentSessionId":"native-a"},
                        "configOptions":config_options,
                        // A legacy claim is deliberately insufficient when the current
                        // selector is absent, malformed, or disagrees with this field.
                        "models":{"currentModelId":PLAN_MODEL,"availableModels":[]},
                    }))
                }
                "session/set_config_option" => {
                    assert_eq!(request["params"]["sessionId"], "acp-a");
                    assert_eq!(request["params"]["configId"], "model");
                    assert_eq!(request["params"]["value"], PLAN_MODEL);
                    selection_reply.clone().expect("unexpected model mutation")
                }
                "session/prompt" => Ok(json!({"stopReason":"end_turn"})),
                _ => panic!("unexpected ACP request: {method}"),
            };
            let response = match result {
                Ok(result) => json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
                Err(error) => json!({"jsonrpc":"2.0","id":request["id"],"error":error}),
            };
            writer
                .write_all(format!("{response}\n").as_bytes())
                .await
                .unwrap();
        }
        methods
    });
    let mut input = super::tests::input();
    input.expected_model = Some(PLAN_MODEL.to_owned());
    if load {
        input.session = AcpSessionStart::Load(AcpSessionBinding {
            acp_session_id: "acp-a".into(),
            native_session_id: Some("native-a".into()),
        });
    }
    let journal = Arc::new(Journal::default());
    let (reader, writer) = tokio::io::split(client);
    let result = run_acp(reader, writer, input, journal.clone()).await;
    let methods = server.await.unwrap();
    let events = journal.0.lock().unwrap().clone();
    (result, methods, events)
}

#[tokio::test]
async fn frozen_model_already_selected_needs_no_mutation_for_new_or_continue() {
    for load in [false, true] {
        let (result, methods, events) =
            run_fixture(load, json!([selector(PLAN_MODEL, json!([]))]), None).await;
        assert!(result.is_ok());
        assert_eq!(events, ["session", "send_intent"]);
        assert_eq!(
            methods,
            [
                "initialize",
                if load { "session/load" } else { "session/new" },
                "session/prompt",
            ]
        );
    }
}

#[tokio::test]
async fn user_or_resumed_model_is_corrected_to_frozen_plan_before_prompt() {
    for load in [false, true] {
        // Both current adapters use a flat list; ACP also permits grouped values.
        for options in [
            json!([plan_option()]),
            json!([{"group":"plans","name":"Plans","options":[plan_option()]}]),
        ] {
            let (result, methods, events) = run_fixture(
                load,
                json!([selector("user-model", options)]),
                Some(Ok(json!({
                    "configOptions":[selector(PLAN_MODEL, json!([plan_option()]))],
                }))),
            )
            .await;
            assert!(result.is_ok());
            assert_eq!(events, ["session", "send_intent"]);
            assert_eq!(
                methods,
                [
                    "initialize",
                    if load { "session/load" } else { "session/new" },
                    "session/set_config_option",
                    "session/prompt",
                ]
            );
        }
    }
}

#[tokio::test]
async fn unavailable_or_ambiguous_frozen_model_never_prompts_or_starts_a_replacement_session() {
    for load in [false, true] {
        for options in [
            Value::Null,
            json!([]),
            json!([selector(
                "user-model",
                json!([{"value":"default","name":"Default"}])
            )]),
            json!([{"id":"model","name":"Model","type":"boolean","currentValue":true}]),
            json!([
                selector(PLAN_MODEL, json!([plan_option()])),
                selector("user-model", json!([plan_option()])),
            ]),
        ] {
            let (result, methods, events) = run_fixture(load, options, None).await;
            assert_eq!(result.err(), Some(DelegationErrorV1::CapabilityUnavailable));
            assert_eq!(events, ["session"]);
            assert_eq!(
                methods,
                [
                    "initialize",
                    if load { "session/load" } else { "session/new" }
                ]
            );
        }
    }
}

#[tokio::test]
async fn rejected_or_unconfirmed_model_selection_never_prompts_for_new_or_continue() {
    for load in [false, true] {
        for selection_reply in [
            Err(json!({"code":-32602,"message":"Model pin rejected"})),
            Ok(json!({})),
            Ok(json!({"configOptions":[]})),
            Ok(json!({"configOptions":[selector("user-model", json!([plan_option()]))]})),
        ] {
            let (result, methods, events) = run_fixture(
                load,
                json!([selector("user-model", json!([plan_option()]))]),
                Some(selection_reply),
            )
            .await;
            assert_eq!(result.err(), Some(DelegationErrorV1::CapabilityUnavailable));
            assert_eq!(events, ["session"]);
            assert_eq!(
                methods,
                [
                    "initialize",
                    if load { "session/load" } else { "session/new" },
                    "session/set_config_option",
                ]
            );
        }
    }
}
