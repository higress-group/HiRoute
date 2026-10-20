//! Independent model roles for real native Main and Worker processes. A terminal model
//! answer alone cannot complete this oracle: every receipt comes from a correlated tool
//! result and the Worker must write its own independently checked file.
use super::super::windows_business_http as http;
use super::SECRET;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
#[path = "windows_worker_residual_oracle.rs"]
mod residual;

pub const MODEL: &str = "windows-native-api-model";
pub const DIRECT: &str = "WINDOWS-PUBLIC-WORKER-DIRECT";
pub const DELEGATED: &str = "WINDOWS-PUBLIC-WORKER-DELEGATED";
pub const CONTINUE: &str = "WINDOWS-PUBLIC-WORKER-CONTINUE";
pub const MAIN: &str = "WINDOWS-PUBLIC-MAIN-AGENT";
pub const CANCEL: &str = "WINDOWS-PUBLIC-WORKER-CANCEL";
pub const RESIDUAL: &str = "WINDOWS-PUBLIC-WORKER-RESIDUAL";

#[derive(Clone, Copy)]
pub enum WorkerStage {
    ModelRequest,
    CancelRequest,
    ResidualRequest,
    ContinuationRequest,
    SkillResult,
    WriteResult,
    ReadResult,
}
impl WorkerStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::ModelRequest => "model-request-observed",
            Self::CancelRequest => "cancel-request-observed",
            Self::ResidualRequest => "residual-request-observed",
            Self::ContinuationRequest => "continuation-request-observed",
            Self::SkillResult => "correlated-skill-result-observed",
            Self::WriteResult => "correlated-write-result-observed",
            Self::ReadResult => "correlated-read-result-observed",
        }
    }
}

#[derive(Default)]
pub struct Ledger {
    pub requests: usize,
    pub completed: BTreeSet<String>,
    pub continued: usize,
    pub main_completed: Option<Value>,
    pub worker_stage: Option<WorkerStage>,
    pub worker_skill_results: usize,
    pub worker_write_results: usize,
    pub worker_read_results: usize,
    pub worker_continuation_requests: usize,
}
pub struct Oracle {
    server: http::Server,
    pub ledger: Arc<Mutex<Ledger>>,
}
impl Oracle {
    pub fn worker(workspace: &Path, skill: PathBuf, contents: String, nonce: &str) -> Self {
        let workspace = workspace.to_owned();
        let nonce = nonce.to_owned();
        Self::start(move |body, ledger| {
            ledger.worker_stage = Some(WorkerStage::ModelRequest);
            let text = native_text(body);
            if [
                "title generator",
                "generate a concise title",
                "generate a short title",
            ]
            .iter()
            .any(|part| text.to_lowercase().contains(part))
            {
                return text_reply(body, "Native task receipt");
            }
            if text.contains(RESIDUAL) {
                ledger.worker_stage = Some(WorkerStage::ResidualRequest);
                println!("Windows public residual checkpoint: stage=oracle-request-observed");
                if let Some((output, is_error)) = residual::returned(body) {
                    println!(
                        "Windows public residual tool facts: {}",
                        residual::facts(&output, is_error)
                    );
                    panic!(
                        "native residual tool returned before its bounded cancellation receipt was confirmed"
                    );
                }
                // The actual native tool supplies its Windows PID and this run's
                // TEMP selector. No model-generated locator or credential is used.
                let binding = workspace.join("residual-started.json");
                let late = workspace.join("residual-late.txt");
                let script = residual::script(&binding, &late, &nonce);
                let powershell = hiroute_platform::windows::launcher::system_executable(Path::new(
                    "WindowsPowerShell/v1.0/powershell.exe",
                ))
                .unwrap();
                use base64::Engine;
                // Bash crosses Windows' native argv parser before PowerShell
                // interprets its program. Encoding preserves quotes/backslashes
                // in JSON and native paths through both command languages.
                let payload: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
                let encoded = base64::engine::general_purpose::STANDARD.encode(payload);
                let bash_literal = |text: &str| format!("'{}'", text.replace('\'', "'\\''"));
                let command = format!(
                    "{} -NoLogo -NoProfile -NonInteractive -EncodedCommand {encoded}",
                    bash_literal(&powershell.to_string_lossy().replace('\\', "/")),
                );
                return function_reply(
                    body,
                    "worker_residual",
                    "bash",
                    json!({"command":command,"timeout":120}),
                );
            }
            if text.contains(CANCEL) {
                ledger.worker_stage = Some(WorkerStage::CancelRequest);
                assert!(
                    correlated(body, "worker_cancel").is_none(),
                    "cancelled native tool must not complete another model turn"
                );
                let quote = |text: &str| format!("'{}'", text.replace('\'', "'\\''"));
                let path =
                    |name: &str| quote(&workspace.join(name).to_string_lossy().replace('\\', "/"));
                let command = if shell_timeout_is_millis(body) {
                    super::cancel::opencode_command(&workspace, &nonce)
                } else {
                    format!(
                        "printf '%s\\n' {} > {} && sleep 60 && printf '%s\\n' {} > {}",
                        quote(&format!("started-{nonce}")),
                        path("cancel-started.txt"),
                        quote(&format!("late-{nonce}")),
                        path("cancel-late.txt")
                    )
                };
                return function_reply(
                    body,
                    "worker_cancel",
                    "bash",
                    json!({"command":command,"timeout":120}),
                );
            }
            let role = if text.contains(DELEGATED) {
                "delegated"
            } else {
                "direct"
            };
            let receipt = format!("worker-{role}-{nonce}");
            let artifact = workspace.join(format!("{role} 中文 % ! ' &.txt"));
            if text.contains(CONTINUE) {
                ledger.worker_stage = Some(WorkerStage::ContinuationRequest);
                ledger.worker_continuation_requests += 1;
                assert_eq!(role, "direct");
                assert!(
                    correlated(body, "worker_read").is_some_and(|output| output.contains(&receipt)),
                    "exact native history must retain the original successful tool receipt"
                );
                assert_eq!(std::fs::read_to_string(&artifact).unwrap().trim(), receipt);
                ledger.continued += 1;
                return text_reply(body, &format!("continued-{receipt}"));
            }
            assert!(
                text.contains(DIRECT) || text.contains(DELEGATED),
                "Main requests must never reach the Worker source"
            );
            if let Some(output) = correlated(body, "worker_read") {
                ledger.worker_stage = Some(WorkerStage::ReadResult);
                ledger.worker_read_results += 1;
                assert!(
                    output.contains(&receipt),
                    "native Worker read did not return its artifact receipt"
                );
                assert_eq!(std::fs::read_to_string(&artifact).unwrap().trim(), receipt);
                ledger.completed.insert(role.into());
                return text_reply(body, &receipt);
            }
            if correlated(body, "worker_write").is_some() {
                ledger.worker_stage = Some(WorkerStage::WriteResult);
                ledger.worker_write_results += 1;
                assert_eq!(std::fs::read_to_string(&artifact).unwrap().trim(), receipt);
                return function_reply(body, "worker_read", "read", json!({"path":artifact}));
            }
            if let Some(output) = correlated(body, "worker_skill") {
                ledger.worker_stage = Some(WorkerStage::SkillResult);
                ledger.worker_skill_results += 1;
                if tool(body, "skill").is_some() {
                    assert!(
                        output.contains(&format!("skill-contents-{nonce}")),
                        "OpenCode must load the actual native project Skill body"
                    );
                } else {
                    assert!(
                        contents
                            .lines()
                            .filter(|line| !line.trim().is_empty())
                            .all(|line| output.contains(line)),
                        "Worker must load the actual project Skill, not a prompt copy"
                    );
                }
                return function_reply(
                    body,
                    "worker_write",
                    "write",
                    json!({"path":artifact,"content":format!("{receipt}\n")}),
                );
            }
            if tool(body, "skill").is_some() {
                // OpenCode 1.19.3 advertises Skills in model context while the
                // tool's description stays static. Match the shared native oracle.
                assert!(
                    text.contains(&format!("skill-discovery-{nonce}")),
                    "native OpenCode Skill discovery receipt is absent"
                );
                function_reply(
                    body,
                    "worker_skill",
                    "skill",
                    json!({"name":"windows-worker-receipt"}),
                )
            } else {
                assert!(
                    text.contains(&format!("skill-discovery-{nonce}")),
                    "native project Skill discovery receipt is absent"
                );
                function_reply(body, "worker_skill", "read", json!({"path":skill}))
            }
        })
    }

    // Each input freezes a distinct native resource, route or correlated receipt.
    #[allow(clippy::too_many_arguments)]
    pub fn main(
        skill: PathBuf,
        contents: String,
        cli: PathBuf,
        workspace: PathBuf,
        prompt: PathBuf,
        plan: String,
        receipt: String,
        worker_ledger: Arc<Mutex<Ledger>>,
    ) -> Self {
        Self::start(move |body, ledger| {
            assert!(
                native_text(body).contains(MAIN),
                "Worker requests must never reach the Main source"
            );
            // Unified exec can return a retained native shell session before a long
            // public wait finishes. Poll that exact session through the native tool,
            // rather than treating a partial stdout receipt as a completed command.
            let mut outputs = std::collections::BTreeMap::new();
            let mut calls = vec![
                "main_skill".to_owned(),
                "main_plans".to_owned(),
                "main_exec".to_owned(),
            ];
            calls.extend((0..10).map(|index| format!("main_wait_{index}")));
            calls.push("main_result".into());
            for call in calls {
                match complete_shell(body, &call) {
                    ShellResult::Missing => {}
                    ShellResult::Complete(output) => {
                        outputs.insert(call, output);
                    }
                    ShellResult::Running { session, poll } => {
                        return function_reply(
                            body,
                            &format!("{call}_poll_{poll}"),
                            "write_stdin",
                            json!({"session_id":session,"chars":"","yield_time_ms":1000,"max_output_tokens":12000}),
                        );
                    }
                }
            }
            if let Some(output) = outputs.get("main_result") {
                let result = envelope(output);
                assert_eq!(result["data"]["run_state"], "succeeded");
                assert!(
                    result["data"]["result"]
                        .as_str()
                        .unwrap()
                        .contains(&receipt)
                );
                let accepted = envelope(outputs.get("main_exec").unwrap());
                same_run(&result, &accepted);
                assert!(
                    worker_ledger
                        .lock()
                        .unwrap()
                        .completed
                        .contains("delegated"),
                    "a real Worker tool turn must precede Main completion"
                );
                assert_eq!(
                    std::fs::read_to_string(workspace.join("delegated 中文 % ! ' &.txt"))
                        .unwrap()
                        .trim(),
                    receipt
                );
                ledger.main_completed = Some(result["data"].clone());
                return text_reply(body, &format!("main-completed-{receipt}"));
            }
            let accepted = outputs.get("main_exec").map(|output| envelope(output));
            if let Some(accepted) = accepted {
                let mut latest = accepted.clone();
                let mut index = 0;
                while let Some(output) = outputs.get(&format!("main_wait_{index}")) {
                    latest = envelope(output);
                    same_run(&latest, &accepted);
                    index += 1;
                }
                let run = accepted["data"]["run_id"].as_str().unwrap();
                if latest["data"]["run_state"] == "succeeded" {
                    return shell(
                        body,
                        "main_result",
                        &cli_command(
                            &cli,
                            &["worker", "result", "--run", run, "--output", "json"],
                        ),
                    );
                }
                assert!(
                    !matches!(
                        latest["data"]["run_state"].as_str(),
                        Some("failed" | "cancelled" | "unknown")
                    ),
                    "Main must observe a successful public Worker run"
                );
                assert!(
                    index < 10,
                    "Main public wait observation exceeded its bound"
                );
                let revision = wait_revision(&latest, run);
                shell(
                    body,
                    &format!("main_wait_{index}"),
                    &cli_command(
                        &cli,
                        &[
                            "worker",
                            "wait",
                            "--run",
                            run,
                            "--after-revision",
                            &revision.to_string(),
                            "--wait-timeout",
                            "20",
                            "--output",
                            "json",
                        ],
                    ),
                )
            } else if let Some(output) = outputs.get("main_plans") {
                let directory = envelope(output);
                let candidates: Vec<_> = directory["data"]["plans"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|candidate| candidate["agent_plan_id"] == plan)
                    .collect();
                assert_eq!(candidates.len(), 1);
                assert_eq!(candidates[0]["harness"], "pi");
                assert_eq!(candidates[0]["availability"], "ready");
                shell(
                    body,
                    "main_exec",
                    &cli_command(
                        &cli,
                        &[
                            "worker",
                            "exec",
                            "--plan",
                            &plan,
                            "--cwd",
                            workspace.to_str().unwrap(),
                            "--file",
                            prompt.to_str().unwrap(),
                            "--no-wait",
                            "--run-timeout",
                            "120",
                            "--submission-key",
                            "windows-main-public-worker",
                            "--output",
                            "json",
                        ],
                    ),
                )
            } else if let Some(output) = outputs.get("main_skill") {
                assert!(
                    contents
                        .lines()
                        .filter(|line| !line.trim().is_empty())
                        .all(|line| output.contains(line)),
                    "Main must read the exact installed user collaboration Skill"
                );
                shell(
                    body,
                    "main_plans",
                    &cli_command(&cli, &["worker", "plans", "--output", "json"]),
                )
            } else {
                assert!(
                    native_text(body).contains("hiroute-collaboration"),
                    "installed user Skill was not discovered"
                );
                shell(
                    body,
                    "main_skill",
                    &format!(
                        "Get-Content -Raw -LiteralPath {}",
                        ps(skill.to_str().unwrap())
                    ),
                )
            }
        })
    }

    fn start(mut handler: impl FnMut(&Value, &mut Ledger) -> http::Reply + Send + 'static) -> Self {
        let ledger = Arc::new(Mutex::new(Ledger::default()));
        let observations = ledger.clone();
        let server = http::Server::start(move |request| {
            assert!(
                request.authenticated(&format!("Bearer {SECRET}")),
                "source must consume the protected saved credential"
            );
            if request.method == "GET" && request.path == "/v1/models" {
                return http::Reply::json(
                    200,
                    json!({"object":"list","data":[{"id":MODEL,"object":"model"}]}),
                );
            }
            assert_eq!(request.method, "POST");
            assert_eq!(request.path, "/v1/responses");
            assert_eq!(request.body["model"], MODEL);
            if tool(&request.body, "hiroute_probe").is_some() {
                assert_eq!(request.body["stream"], false);
                return http::Reply::json(
                    200,
                    response(
                        &request.body,
                        "compute_check",
                        json!({"type":"function_call","id":"compute_check",
                    "call_id":"compute_check","name":"hiroute_probe","arguments":"{\"value\":\"ok\"}","status":"completed"}),
                    ),
                );
            }
            assert_eq!(request.body["stream"], true);
            let mut ledger = observations.lock().unwrap();
            ledger.requests += 1;
            assert!(
                ledger.requests <= 64,
                "native model requests exceeded the bounded acceptance budget"
            );
            handler(&request.body, &mut ledger)
        });
        Self { server, ledger }
    }
    pub fn address(&self) -> std::net::SocketAddr {
        self.server.address
    }
}

pub fn ps(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
fn cli_command(cli: &Path, args: &[&str]) -> String {
    format!(
        "& {} {}",
        ps(cli.to_str().unwrap()),
        args.iter().map(|arg| ps(arg)).collect::<Vec<_>>().join(" ")
    )
}
fn shell(body: &Value, id: &str, command: &str) -> http::Reply {
    if tool(body, "exec_command").is_some() {
        function_reply(
            body,
            id,
            "exec_command",
            json!({"cmd":command,"yield_time_ms":30000,"max_output_tokens":12000}),
        )
    } else {
        function_reply(
            body,
            id,
            "shell_command",
            json!({"command":command,"timeout_ms":30000,"max_output_tokens":12000}),
        )
    }
}
enum ShellResult {
    Missing,
    Complete(String),
    Running { session: u64, poll: usize },
}
fn complete_shell(body: &Value, call: &str) -> ShellResult {
    let Some(mut output) = correlated(body, call) else {
        return ShellResult::Missing;
    };
    let session = output
        .split("Process running with session ID ")
        .nth(1)
        .and_then(|text| text.split_whitespace().next())
        .map(|number| {
            number
                .parse::<u64>()
                .expect("actual native unified-exec session must have an integer identity")
        });
    let Some(session) = session else {
        return ShellResult::Complete(output);
    };
    for poll in 0..40 {
        let Some(next) = correlated(body, &format!("{call}_poll_{poll}")) else {
            return ShellResult::Running { session, poll };
        };
        let exited = next.contains("Process exited with code");
        output.push('\n');
        output.push_str(&next);
        if exited {
            assert!(
                next.contains("Process exited with code 0"),
                "native public CLI tool session failed"
            );
            return ShellResult::Complete(output);
        }
    }
    panic!("exact native shell session exceeded its bounded public observation budget");
}
pub fn wait_revision(envelope: &Value, run: &str) -> u64 {
    let actions: Vec<_> = envelope["next_actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["command_id"] == "worker.wait" && action["input"]["run_id"] == run)
        .collect();
    assert_eq!(
        actions.len(),
        1,
        "active Worker response must publish an exact wait cursor"
    );
    actions[0]["input"]["after_revision"].as_u64().unwrap()
}
fn same_run(current: &Value, accepted: &Value) {
    for field in ["task_id", "run_id"] {
        assert_eq!(current["data"][field], accepted["data"][field]);
    }
}
fn envelope(text: &str) -> Value {
    let values: Vec<Value> = text
        .match_indices('{')
        .filter_map(|(offset, _)| {
            serde_json::Deserializer::from_str(&text[offset..])
                .into_iter::<Value>()
                .next()?
                .ok()
        })
        .filter(|value| value.get("schema_version").is_some() && value.get("status").is_some())
        .collect();
    assert_eq!(
        values.len(),
        1,
        "native tool must return exactly one real public CLI envelope"
    );
    assert_eq!(values[0]["status"], "succeeded");
    values.into_iter().next().unwrap()
}
fn native_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(native_text).collect::<Vec<_>>().join("\n"),
        Value::Object(object) => object
            .iter()
            .filter(|(key, _)| {
                matches!(
                    key.as_str(),
                    "text"
                        | "content"
                        | "input"
                        | "instructions"
                        | "output"
                        | "messages"
                        | "system"
                )
            })
            .map(|(_, value)| native_text(value))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}
fn correlated(body: &Value, id: &str) -> Option<String> {
    body["input"]
        .as_array()?
        .iter()
        .find(|item| {
            matches!(
                item["type"].as_str(),
                Some("function_call_output" | "custom_tool_call_output")
            ) && item["call_id"] == id
                && item["is_error"] != true
        })
        .map(|item| native_text(&item["output"]))
}
fn tool<'a>(body: &'a Value, name: &str) -> Option<&'a Value> {
    body["tools"]
        .as_array()?
        .iter()
        .find(|tool| tool["name"] == name || tool["function"]["name"] == name)
}
fn shell_timeout_is_millis(body: &Value) -> bool {
    let Some(declaration) = tool(body, "bash") else {
        return false;
    };
    declaration
        .get("parameters")
        .or_else(|| declaration["function"].get("parameters"))
        .and_then(|schema| schema["properties"]["timeout"]["description"].as_str())
        .is_some_and(|description| description.to_ascii_lowercase().contains("milliseconds"))
}
fn function_reply(body: &Value, id: &str, name: &str, mut arguments: Value) -> http::Reply {
    let declaration =
        tool(body, name).expect("actual native client must declare the selected tool");
    assert_ne!(
        declaration["type"], "custom",
        "tool must use its declared native function schema"
    );
    let parameters = declaration
        .get("parameters")
        .or_else(|| declaration["function"].get("parameters"));
    // Follow the actual native declaration; Pi uses path, OpenCode uses filePath.
    if parameters.is_some_and(|schema| schema["properties"].get("filePath").is_some())
        && let Some(path) = arguments.as_object_mut().unwrap().remove("path")
    {
        arguments["filePath"] = path;
    }
    if name == "bash"
        && parameters.is_some_and(|schema| schema["properties"].get("description").is_some())
    {
        arguments["description"] = json!("Produce the bounded native acceptance receipt");
    }
    // OpenCode 1.19.3 no longer declares a description argument; its timeout
    // still uses milliseconds. Pi's timeout remains in seconds.
    if name == "bash" && shell_timeout_is_millis(body) {
        arguments["timeout"] = json!(120000);
    }
    if let Some(required) = parameters.and_then(|schema| schema["required"].as_array()) {
        assert!(
            required
                .iter()
                .filter_map(Value::as_str)
                .all(|field| arguments.get(field).is_some()),
            "tool argument schema mismatch"
        );
    }
    stream(
        body,
        id,
        json!({"type":"function_call","id":format!("item_{id}"),"call_id":id,"name":name,"arguments":arguments.to_string(),"status":"completed"}),
    )
}
fn text_reply(body: &Value, text: &str) -> http::Reply {
    stream(
        body,
        "done",
        json!({"type":"message","id":"item_done","role":"assistant","status":"completed",
        "content":[{"type":"output_text","text":text,"annotations":[]}]}),
    )
}
fn response(body: &Value, id: &str, item: Value) -> Value {
    json!({"id":format!("resp_{id}"),"object":"response","created_at":1,"model":body["model"],"status":"completed","output":[item],
        "usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}})
}
fn stream(body: &Value, id: &str, item: Value) -> http::Reply {
    let response = response(body, id, item.clone());
    let item_id = item["id"].as_str().unwrap();
    let mut created = response.clone();
    created["status"] = json!("in_progress");
    created["output"] = json!([]);
    let mut initial = item.clone();
    initial["status"] = json!("in_progress");
    let terminal = item["type"] == "message";
    initial[if terminal { "content" } else { "arguments" }] =
        if terminal { json!([]) } else { json!("") };
    let mut events = vec![
        ("response.created", json!({"response":created})),
        (
            "response.output_item.added",
            json!({"output_index":0,"item":initial}),
        ),
    ];
    if terminal {
        let text = &item["content"][0]["text"];
        events.extend([
            ("response.content_part.added", json!({"item_id":item_id,"output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}})),
            ("response.output_text.delta", json!({"item_id":item_id,"output_index":0,"content_index":0,"delta":text})),
            ("response.output_text.done", json!({"item_id":item_id,"output_index":0,"content_index":0,"text":text})),
            ("response.content_part.done", json!({"item_id":item_id,"output_index":0,"content_index":0,"part":item["content"][0]})),
        ]);
    } else {
        events.extend([
            ("response.function_call_arguments.delta", json!({"item_id":item_id,"call_id":id,"output_index":0,"delta":item["arguments"]})),
            ("response.function_call_arguments.done", json!({"item_id":item_id,"call_id":id,"output_index":0,"arguments":item["arguments"]})),
        ]);
    }
    events.extend([
        (
            "response.output_item.done",
            json!({"output_index":0,"item":item}),
        ),
        ("response.completed", json!({"response":response})),
    ]);
    let output = events
        .into_iter()
        .enumerate()
        .map(|(index, (event, mut value))| {
            value["type"] = json!(event);
            value["sequence_number"] = json!(index);
            format!("event: {event}\ndata: {value}\n\n")
        })
        .collect();
    http::Reply::sse(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_timeout_follows_declared_units_without_a_description_argument() {
        for (description, expected) in [
            ("Optional timeout in milliseconds", 120000),
            ("Optional timeout in seconds", 120),
        ] {
            let body = json!({"tools":[{"type":"function","name":"bash","parameters":{
                "type":"object","properties":{"command":{"type":"string"},
                    "timeout":{"type":"number","description":description}},
                "required":["command"]}}]});
            let reply = function_reply(
                &body,
                "worker_cancel",
                "bash",
                json!({"command":"native-receipt","timeout":120}),
            );
            let event: Value = String::from_utf8(reply.body)
                .unwrap()
                .lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .find(|event| event["type"] == "response.output_item.done")
                .unwrap();
            let arguments: Value =
                serde_json::from_str(event["item"]["arguments"].as_str().unwrap()).unwrap();
            assert_eq!(arguments["timeout"], expected);
            assert!(arguments.get("description").is_none());
        }
    }

    #[test]
    fn a_prompt_or_unrelated_or_error_tool_result_cannot_supply_a_native_receipt() {
        let mut body = json!({"input":[{"type":"message","content":"native-receipt"},
            {"type":"function_call_output","call_id":"unrelated","output":"native-receipt"}]});
        assert_eq!(correlated(&body, "actual"), None);
        body["input"][1]["call_id"] = json!("actual");
        body["input"][1]["is_error"] = json!(true);
        assert_eq!(correlated(&body, "actual"), None);
        body["input"][1]["is_error"] = json!(false);
        assert_eq!(
            correlated(&body, "actual").as_deref(),
            Some("native-receipt")
        );
    }

    #[test]
    fn a_running_native_shell_does_not_complete_even_after_printing_json() {
        let mut body = json!({"input":[{"type":"function_call_output","call_id":"main_exec",
            "output":"Process running with session ID 17\nOutput:\n{\"schema_version\":{\"major\":2},\"status\":\"succeeded\"}"}]});
        assert!(matches!(
            complete_shell(&body, "main_exec"),
            ShellResult::Running {
                session: 17,
                poll: 0
            }
        ));
        body["input"].as_array_mut().unwrap().push(
            json!({"type":"function_call_output","call_id":"main_exec_poll_0",
            "output":"Process exited with code 0\nOutput:\nfinal"}),
        );
        assert!(
            matches!(complete_shell(&body, "main_exec"), ShellResult::Complete(output) if output.contains("final"))
        );
    }

    #[test]
    #[should_panic(expected = "exactly one real public CLI envelope")]
    fn ambiguous_native_cli_stdout_is_rejected() {
        envelope(
            "{\"schema_version\":{},\"status\":\"succeeded\"}\n{\"schema_version\":{},\"status\":\"succeeded\"}",
        );
    }
}
