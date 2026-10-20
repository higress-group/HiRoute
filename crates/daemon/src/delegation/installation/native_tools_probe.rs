//! A test-only deterministic model for real upstream clients, not a substitute ACP adapter.
//! Receipts enter through native resources/tools, never through the submitted task prompt.
#![cfg_attr(not(windows), allow(dead_code))]

use super::*;
use serde_json::json;
use std::path::PathBuf;

pub(super) const CONTINUE_TOOLS: &str =
    "NATIVE-TOOLS-CONTINUE: retain the same native tool history and finish with OK.";
const TARGET_NAME: &str = "tools 中文 % ! ' &.txt";
const CALL_IDS: [&str; 5] = [
    "native_skill_user",
    "native_skill_project",
    "native_read",
    "native_edit",
    "native_shell",
];

#[path = "native_tools_history_probe.rs"]
mod history_probe;
#[path = "../../../tests/support/windows_qoder_native_paths.rs"]
mod qoder_paths;
#[cfg(test)]
#[path = "native_tools_shell_tests.rs"]
mod shell_tests;

pub(super) fn observe_claude_history(path: &Path) {
    history_probe::observe_transcript(path);
}

struct Skill {
    name: String,
    discovery: String,
    contents: String,
    executed: String,
    file: PathBuf,
    script: PathBuf,
    original: Vec<u8>,
    _directory: OwnedSkillDirectory,
}

// These are fresh, exclusively created fixture directories inside a borrowed
// account. Never reuse or clean another run's or the user's existing Skills.
struct OwnedSkillDirectory(PathBuf);

impl Drop for OwnedSkillDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) struct ToolsFixture {
    harness: WorkerHarnessV1,
    skills: Vec<Skill>,
    target: PathBuf,
    shell_receipt: PathBuf,
    before: String,
    after: String,
    receipt: String,
    #[cfg(windows)]
    qoder_bash_inputs: Vec<qoder_paths::BashFile>,
}

impl Drop for ToolsFixture {
    fn drop(&mut self) {
        // Windows resource leases deny deletion, including their ancestors.
        // Release them before the owned Skill directories are dropped.
        #[cfg(windows)]
        self.qoder_bash_inputs.clear();
    }
}

impl ToolsFixture {
    pub(super) fn prepare(
        harness: WorkerHarnessV1,
        home: &Path,
        config: &Path,
        workspace: &Path,
    ) -> Result<Self, DelegationErrorV1> {
        let nonce = String::from_utf8(fresh_challenge()?)
            .map_err(|_| DelegationErrorV1::InvalidArguments)?;
        let project = match harness {
            WorkerHarnessV1::CodexCli => ".agents",
            WorkerHarnessV1::ClaudeCode => ".claude",
            WorkerHarnessV1::QoderCli => ".qoder",
            WorkerHarnessV1::Pi => ".pi",
            WorkerHarnessV1::DeepseekHarness => ".dsh",
            WorkerHarnessV1::OpenCode => ".opencode",
        };
        // Codex's .agents user root is the actual KnownFolder on Windows, not an
        // environment-selected probe HOME. Use its supported private CODEX_HOME user root.
        let user_root = if harness == WorkerHarnessV1::CodexCli && !cfg!(windows) {
            home.join(".agents/skills")
        } else {
            config.join("skills")
        };
        let mut skills = Vec::new();
        for (scope, root) in [
            ("user", user_root),
            ("project", workspace.join(project).join("skills")),
        ] {
            let name = format!("native-tools-{scope}-{}", &nonce[nonce.len() - 24..]);
            let directory = root.join(&name);
            make_directories(&root)?;
            create_private(&directory)?;
            let owned_directory = OwnedSkillDirectory(directory.clone());
            let discovery = format!("discovery-{scope}-{nonce}");
            let contents = format!("contents-{scope}-{nonce}");
            let executed = format!("executed-{scope}-{nonce}");
            let (script_name, script_body) = receipt_script(harness, &executed);
            let script = directory.join(script_name);
            write_new(&script, script_body.as_bytes())?;
            let file = directory.join("SKILL.md");
            let original = format!("---\nname: {name}\ndescription: Native receipt {discovery}\n---\n\nRead this skill and execute its receipt script.\nContent receipt: {contents}\nScript: {}\n", script.display()).into_bytes();
            write_new(&file, &original)?;
            skills.push(Skill {
                name,
                discovery,
                contents,
                executed,
                file,
                script,
                original,
                _directory: owned_directory,
            });
        }
        let before = format!("before-{nonce}");
        let after = format!("after-{nonce}");
        let target = workspace.join(TARGET_NAME);
        write_new(&target, format!("{before}\n").as_bytes())?;
        let shell_receipt = workspace.join("native-shell-receipt.txt");
        #[cfg(windows)]
        if harness == WorkerHarnessV1::QoderCli {
            // Bash truncates this exact new private file. Empty bytes cannot
            // satisfy the original nonce/effect oracle.
            write_new(&shell_receipt, b"")?;
        }
        Ok(Self {
            harness,
            skills,
            target,
            shell_receipt,
            before,
            after,
            receipt: format!("shell-{nonce}"),
            #[cfg(windows)]
            qoder_bash_inputs: Vec::new(),
        })
    }

    pub(super) fn prompt(&self) -> String {
        format!(
            "Use {} and {}. Read the probe file, edit it, execute the skill receipts, and finish with OK.",
            self.skills[0].name, self.skills[1].name
        )
    }

    pub(super) fn assert_effects(&self) -> Result<(), DelegationErrorV1> {
        #[cfg(windows)]
        if self.harness == WorkerHarnessV1::QoderCli
            && (self.qoder_bash_inputs.len() != 2 + 2 * self.skills.len()
                || self
                    .qoder_bash_inputs
                    .iter()
                    .any(|binding| binding.verify().is_err()))
        {
            return failed("native-bash-resource-identity");
        }
        if fs::read_to_string(&self.target).ok().as_deref() != Some(&format!("{}\n", self.after)) {
            return failed("edit-side-effect");
        }
        if fs::read_to_string(&self.shell_receipt)
            .ok()
            .map(|text| text.trim_start_matches('\u{feff}').trim().to_owned())
            .as_deref()
            != Some(self.receipt.as_str())
        {
            return failed("shell-side-effect");
        }
        for skill in &self.skills {
            if fs::read(&skill.file).ok().as_deref() != Some(skill.original.as_slice()) {
                return failed("borrowed-skill-preserved");
            }
        }
        Ok(())
    }

    #[cfg(windows)]
    fn qoder_bash_command(&mut self) -> Result<String, DelegationErrorV1> {
        let retain = |path: &Path| {
            qoder_paths::BashFile::retain(path).map_err(|_| failure("native-bash-resource"))
        };
        let target = retain(&self.target)?;
        let mut parts = vec![format!("cat {}", target.quoted())];
        let output = qoder_paths::BashFile::created_output(&self.shell_receipt, &target)
            .map_err(|_| failure("native-bash-output"))?;
        let mut retained = vec![target];
        for skill in &self.skills {
            let file = retain(&skill.file)?;
            let script = retain(&skill.script)?;
            parts.push(format!("cat {} && bash {}", file.quoted(), script.quoted()));
            retained.extend([file, script]);
        }
        parts.push(format!(
            "printf '%s\\n' {} > {} && cat {}",
            bash_literal(&self.receipt),
            output.quoted(),
            output.quoted()
        ));
        retained.push(output);
        self.qoder_bash_inputs = retained;
        Ok(parts.join(" && "))
    }
}

pub(super) struct ToolsOracle {
    fixture: ToolsFixture,
    next: usize,
    pending: bool,
    pub(super) loaded: bool,
}

impl ToolsOracle {
    pub(super) fn new(fixture: ToolsFixture) -> Self {
        Self {
            fixture,
            next: 0,
            pending: false,
            loaded: false,
        }
    }

    /// Called again only after lifecycle has proved the owned process scope stopped.
    /// Correlated HTTP receipts observed while the client was live are not final files.
    pub(super) fn assert_effects(&self) -> Result<(), DelegationErrorV1> {
        self.fixture.assert_effects()
    }

    pub(super) fn respond(&mut self, body: &Value) -> Result<String, DelegationErrorV1> {
        if native_text(body).contains(CONTINUE_TOOLS) {
            if self.next != CALL_IDS.len() || self.pending {
                return failed("continue-before-completion");
            }
            let output = correlated_result(body, CALL_IDS[4]).ok_or_else(|| {
                history_probe::observe_request(body);
                failure("continue-native-tool-history")
            })?;
            self.verify_shell(&output)?;
            self.fixture.assert_effects()?;
            self.loaded = true;
            return Ok(text_stream(body, self.fixture.harness));
        }
        if self.pending {
            let id = CALL_IDS[self.next];
            let output =
                correlated_result(body, id).ok_or_else(|| failure("missing-native-tool-result"))?;
            match self.next {
                0 | 1 => {
                    let skill = &self.fixture.skills[self.next];
                    // Native Skill tools can expand into a following user message. The exact
                    // non-error tool result remains mandatory; prompt text alone never counts.
                    let text = if matches!(
                        self.fixture.harness,
                        WorkerHarnessV1::ClaudeCode | WorkerHarnessV1::QoderCli
                    ) {
                        native_text(body)
                    } else {
                        output
                    };
                    if !text.contains(&skill.contents) {
                        return failed("native-skill-body");
                    }
                }
                2 if !output.contains(&self.fixture.before) => {
                    return failed("native-read-receipt");
                }
                3 => {
                    if fs::read_to_string(&self.fixture.target).ok().as_deref()
                        != Some(&format!("{}\n", self.fixture.after))
                    {
                        return failed("native-edit-receipt");
                    }
                }
                4 => self.verify_shell(&output)?,
                _ => {}
            }
            self.pending = false;
            self.next += 1;
        }
        if self.next == CALL_IDS.len() {
            self.fixture.assert_effects()?;
            return Ok(text_stream(body, self.fixture.harness));
        }
        if self.next == 0
            && self
                .fixture
                .skills
                .iter()
                .any(|skill| !native_text(body).contains(&skill.discovery))
        {
            return failed("native-user-project-skill-discovery");
        }
        let action = self.action(body)?;
        self.pending = true;
        Ok(tool_stream(
            body,
            self.fixture.harness,
            CALL_IDS[self.next],
            action,
        ))
    }

    fn verify_shell(&self, output: &str) -> Result<(), DelegationErrorV1> {
        if !output.contains(&self.fixture.after)
            || !output.contains(&self.fixture.receipt)
            || self
                .fixture
                .skills
                .iter()
                .any(|skill| !output.contains(&skill.contents) || !output.contains(&skill.executed))
        {
            return failed("native-shell-receipts");
        }
        Ok(())
    }

    fn action(&mut self, body: &Value) -> Result<Action, DelegationErrorV1> {
        let harness = self.fixture.harness;
        if self.next < 2 {
            let skill = &self.fixture.skills[self.next];
            return if matches!(
                harness,
                WorkerHarnessV1::ClaudeCode | WorkerHarnessV1::QoderCli
            ) {
                function(body, "Skill", json!({"skill":skill.name}))
            } else {
                self.read(body, &skill.file)
            };
        }
        if self.next == 2 {
            return self.read(body, &self.fixture.target);
        }
        if self.next == 3 {
            let target = self.fixture.target.to_string_lossy();
            return match harness {
                WorkerHarnessV1::CodexCli => {
                    // The current generic Plan catalog deliberately leaves apply_patch_tool_type
                    // absent/null. Codex a956835d/spec_plan.rs:1196 then exposes Shell only.
                    // Preserve that production affordance; assert the edit's actual file effect.
                    if find_tool(body, "apply_patch").is_none() {
                        return self.shell(body, &format!(
                            "$ErrorActionPreference='Stop'; $p={}; $s=Get-Content -Raw -LiteralPath $p; if ($s -cne {}) {{ throw 'private edit precondition' }}; [System.IO.File]::WriteAllText($p, {}, (New-Object System.Text.UTF8Encoding($false))); Get-Content -LiteralPath $p",
                            ps_path(&self.fixture.target), ps_literal(&format!("{}\n", self.fixture.before)), ps_literal(&format!("{}\n", self.fixture.after))));
                    }
                    let patch = format!(
                        "*** Begin Patch\n*** Update File: {TARGET_NAME}\n@@\n-{}\n+{}\n*** End Patch\n",
                        self.fixture.before, self.fixture.after
                    );
                    let tool = declared_tool(body, "apply_patch")?;
                    if tool["type"] == "custom" {
                        Ok(Action {
                            name: "apply_patch".into(),
                            arguments: Value::String(patch),
                            custom: true,
                        })
                    } else {
                        function(body, "apply_patch", json!({"input":patch}))
                    }
                }
                WorkerHarnessV1::Pi => function(
                    body,
                    "edit",
                    // Pi 1.0.2 / cd32f7725fdbddbaecdff5b1e68491563394e0ca declares
                    // edits[] in packages/coding-agent/src/core/tools/edit.ts.
                    json!({"path":target,"edits":[{"oldText":self.fixture.before,"newText":self.fixture.after}]}),
                ),
                _ => function(
                    body,
                    if harness == WorkerHarnessV1::DeepseekHarness {
                        "edit"
                    } else {
                        "Edit"
                    },
                    json!({"file_path":target,"old_string":self.fixture.before,"new_string":self.fixture.after,"replace_all":false}),
                ),
            };
        }
        #[cfg(windows)]
        if harness == WorkerHarnessV1::QoderCli {
            let command = self.fixture.qoder_bash_command()?;
            return self.shell(body, &command);
        }
        let powershell = uses_powershell(harness);
        let command = if powershell {
            let mut parts = vec![format!(
                "$ErrorActionPreference='Stop'; Get-Content -LiteralPath {}",
                ps_path(&self.fixture.target)
            )];
            for skill in &self.fixture.skills {
                parts.push(format!(
                    "Get-Content -LiteralPath {}; & {}",
                    ps_path(&skill.file),
                    ps_path(&skill.script)
                ));
            }
            parts.push(format!(
                "Set-Content -LiteralPath {} -Value {} -Encoding UTF8; Get-Content -LiteralPath {}",
                ps_path(&self.fixture.shell_receipt),
                ps_literal(&self.fixture.receipt),
                ps_path(&self.fixture.shell_receipt)
            ));
            parts.join("; ")
        } else {
            let mut parts = vec![format!("cat {}", bash_path(&self.fixture.target))];
            for skill in &self.fixture.skills {
                parts.push(format!(
                    "cat {} && bash {}",
                    bash_path(&skill.file),
                    bash_path(&skill.script)
                ));
            }
            parts.push(format!(
                "printf '%s\\n' {} > {} && cat {}",
                bash_literal(&self.fixture.receipt),
                bash_path(&self.fixture.shell_receipt),
                bash_path(&self.fixture.shell_receipt)
            ));
            parts.join(" && ")
        };
        self.shell(body, &command)
    }

    fn read(&self, body: &Value, path: &Path) -> Result<Action, DelegationErrorV1> {
        let target = path.to_string_lossy();
        match self.fixture.harness {
            WorkerHarnessV1::CodexCli => {
                self.shell(body, &format!("Get-Content -LiteralPath {}", ps_path(path)))
            }
            WorkerHarnessV1::Pi => function(body, "read", json!({"path":target})),
            WorkerHarnessV1::DeepseekHarness => function(body, "read", json!({"file_path":target})),
            _ => function(body, "Read", json!({"file_path":target})),
        }
    }

    fn shell(&self, body: &Value, command: &str) -> Result<Action, DelegationErrorV1> {
        match self.fixture.harness {
            WorkerHarnessV1::CodexCli if find_tool(body, "exec_command").is_some() => function(
                body,
                "exec_command",
                json!({"cmd":command,"max_output_tokens":2000,"yield_time_ms":1000}),
            ),
            WorkerHarnessV1::CodexCli => function(
                body,
                "shell_command",
                json!({"command":command,"timeout_ms":10000}),
            ),
            WorkerHarnessV1::Pi => function(body, "bash", json!({"command":command,"timeout":10})),
            WorkerHarnessV1::DeepseekHarness => function(
                body,
                "pwsh",
                json!({"command":command,"description":"Run private native receipt","timeoutMs":10000,"run_in_background":false}),
            ),
            _ => function(body, "Bash", json!({"command":command,"timeout":10000})),
        }
    }
}

fn make_directories(path: &Path) -> Result<(), DelegationErrorV1> {
    if path.exists() {
        return private_directory(path);
    }
    make_directories(path.parent().ok_or(DelegationErrorV1::InvalidArguments)?)?;
    create_private(path)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), DelegationErrorV1> {
    hiroute_platform::durable::write_private_new(path, bytes)
        .map_err(|_| DelegationErrorV1::StorageUnavailable)
}
fn ps_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}
fn ps_path(path: &Path) -> String {
    ps_literal(&path.to_string_lossy())
}
fn bash_literal(text: &str) -> String {
    qoder_paths::literal(text)
}
fn uses_powershell(harness: WorkerHarnessV1) -> bool {
    matches!(
        harness,
        WorkerHarnessV1::CodexCli | WorkerHarnessV1::DeepseekHarness
    )
}
fn receipt_script(harness: WorkerHarnessV1, receipt: &str) -> (&'static str, String) {
    if uses_powershell(harness) {
        (
            "receipt.ps1",
            format!("Write-Output {}\n", ps_literal(receipt)),
        )
    } else {
        (
            "receipt.sh",
            format!("printf '%s\\n' {}\n", bash_literal(receipt)),
        )
    }
}
fn bash_path(path: &Path) -> String {
    bash_literal(&path.to_string_lossy().replace('\\', "/"))
}
fn failure(label: &str) -> DelegationErrorV1 {
    eprintln!("native tools safe failure: {label}");
    DelegationErrorV1::ProtocolFailed
}
fn failed<T>(label: &str) -> Result<T, DelegationErrorV1> {
    Err(failure(label))
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
fn correlated_result(body: &Value, id: &str) -> Option<String> {
    if let Some(items) = body["input"].as_array() {
        return items
            .iter()
            .find(|item| {
                matches!(
                    item["type"].as_str(),
                    Some("function_call_output" | "custom_tool_call_output")
                ) && item["call_id"] == id
            })
            .map(|item| native_text(&item["output"]));
    }
    body["messages"]
        .as_array()?
        .iter()
        .filter(|message| message["role"] == "user")
        .filter_map(|message| message["content"].as_array())
        .flatten()
        .find(|item| {
            item["type"] == "tool_result" && item["tool_use_id"] == id && item["is_error"] != true
        })
        .map(|item| native_text(&item["content"]))
}
fn find_tool<'a>(body: &'a Value, name: &str) -> Option<&'a Value> {
    body["tools"].as_array().and_then(|tools| {
        tools
            .iter()
            .find(|tool| tool["name"] == name || tool["function"]["name"] == name)
    })
}
fn declared_tool<'a>(body: &'a Value, name: &str) -> Result<&'a Value, DelegationErrorV1> {
    find_tool(body, name).ok_or_else(|| failure("native-tool-not-declared"))
}
struct Action {
    name: String,
    arguments: Value,
    custom: bool,
}
fn function(body: &Value, name: &str, arguments: Value) -> Result<Action, DelegationErrorV1> {
    let tool = declared_tool(body, name)?;
    if tool["type"] == "custom" {
        return failed("native-tool-schema-mismatch");
    }
    let schema = tool
        .get("parameters")
        .or_else(|| tool.get("input_schema"))
        .or_else(|| {
            tool.get("function")
                .and_then(|value| value.get("parameters"))
        });
    if schema
        .and_then(|schema| schema.get("required"))
        .and_then(Value::as_array)
        .is_some_and(|required| {
            required
                .iter()
                .filter_map(Value::as_str)
                .any(|key| arguments.get(key).is_none())
        })
    {
        return failed("native-tool-required-argument");
    }
    Ok(Action {
        name: name.into(),
        arguments,
        custom: false,
    })
}

fn text_stream(body: &Value, harness: WorkerHarnessV1) -> String {
    tool_stream(
        body,
        harness,
        "native_done",
        Action {
            name: String::new(),
            arguments: Value::String(PROBE_TEXT.into()),
            custom: false,
        },
    )
}
fn tool_stream(body: &Value, harness: WorkerHarnessV1, id: &str, action: Action) -> String {
    let model = body["model"].as_str().unwrap_or(PROBE_MODEL);
    let terminal = action.name.is_empty();
    let mut events = if harness == WorkerHarnessV1::ClaudeCode {
        let block = if terminal {
            json!({"type":"text","text":PROBE_TEXT})
        } else {
            json!({"type":"tool_use","id":id,"name":action.name,"input":action.arguments})
        };
        let initial = if terminal {
            json!({"type":"text","text":""})
        } else {
            json!({"type":"tool_use","id":id,"name":action.name,"input":{}})
        };
        vec![
            (
                "message_start",
                json!({"message":{"id":format!("msg_{id}"),"type":"message","role":"assistant","model":model,"content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":0}}}),
            ),
            (
                "content_block_start",
                json!({"index":0,"content_block":initial}),
            ),
            (
                "content_block_delta",
                json!({"index":0,"delta": if terminal { json!({"type":"text_delta","text":PROBE_TEXT}) } else { json!({"type":"input_json_delta","partial_json":block["input"].to_string()}) }}),
            ),
            ("content_block_stop", json!({"index":0})),
            (
                "message_delta",
                json!({"delta":{"stop_reason":if terminal {"end_turn"} else {"tool_use"},"stop_sequence":null},"usage":{"output_tokens":1}}),
            ),
            ("message_stop", json!({})),
        ]
    } else {
        let item_id = format!("item_{id}");
        let item = if terminal {
            json!({"type":"message","id":item_id,"role":"assistant","status":"completed","content":[{"type":"output_text","text":PROBE_TEXT,"annotations":[]}]})
        } else if action.custom {
            json!({"type":"custom_tool_call","id":item_id,"call_id":id,"name":action.name,"input":action.arguments,"status":"completed"})
        } else {
            json!({"type":"function_call","id":item_id,"call_id":id,"name":action.name,"arguments":action.arguments.to_string(),"status":"completed"})
        };
        let mut initial = item.clone();
        initial["status"] = json!("in_progress");
        initial[if terminal {
            "content"
        } else if action.custom {
            "input"
        } else {
            "arguments"
        }] = if terminal { json!([]) } else { json!("") };
        let response = json!({"id":format!("resp_{id}"),"object":"response","created_at":1,"model":model,"status":"completed","output":[item.clone()],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}});
        let mut created = response.clone();
        created["status"] = json!("in_progress");
        created["output"] = json!([]);
        let mut result = vec![
            ("response.created", json!({"response":created})),
            (
                "response.output_item.added",
                json!({"output_index":0,"item":initial}),
            ),
        ];
        if terminal {
            result.extend([
                ("response.content_part.added", json!({"item_id":item_id,"output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}})),
                ("response.output_text.delta", json!({"item_id":item_id,"output_index":0,"content_index":0,"delta":PROBE_TEXT})),
                ("response.output_text.done", json!({"item_id":item_id,"output_index":0,"content_index":0,"text":PROBE_TEXT})),
                ("response.content_part.done", json!({"item_id":item_id,"output_index":0,"content_index":0,"part":{"type":"output_text","text":PROBE_TEXT,"annotations":[]}})),
            ]);
        } else {
            let field = if action.custom { "input" } else { "arguments" };
            result.push((
                if action.custom {
                    "response.custom_tool_call_input.delta"
                } else {
                    "response.function_call_arguments.delta"
                },
                json!({"item_id":item_id,"call_id":id,"output_index":0,"delta":item[field]}),
            ));
            result.push((
                if action.custom {
                    "response.custom_tool_call_input.done"
                } else {
                    "response.function_call_arguments.done"
                },
                json!({"item_id":item_id,"call_id":id,"output_index":0,field:item[field]}),
            ));
        }
        result.extend([
            (
                "response.output_item.done",
                json!({"output_index":0,"item":item}),
            ),
            ("response.completed", json!({"response":response})),
        ]);
        result
    };
    for (index, (event, value)) in events.iter_mut().enumerate() {
        value["type"] = json!(event);
        if harness != WorkerHarnessV1::ClaudeCode {
            value["sequence_number"] = json!(index);
        }
    }
    events
        .into_iter()
        .map(|(event, data)| format!("event: {event}\ndata: {data}\n\n"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipts_require_correlated_successful_native_tool_results() {
        let mut body = json!({"messages":[{"role":"user","content":[{"type":"text","text":"native receipt"},{"type":"tool_result","tool_use_id":"wrong","content":"receipt"}]}]});
        assert_eq!(correlated_result(&body, "expected"), None);
        body["messages"][0]["content"][1]["tool_use_id"] = json!("expected");
        body["messages"][0]["content"][1]["is_error"] = json!(true);
        assert_eq!(correlated_result(&body, "expected"), None);
        body["messages"][0]["content"][1]["is_error"] = json!(false);
        assert_eq!(
            correlated_result(&body, "expected").as_deref(),
            Some("receipt")
        );
        assert_eq!(correlated_result(&json!({"input":[{"type":"custom_tool_call_output","call_id":"expected","output":"patch receipt"}]}), "expected").as_deref(), Some("patch receipt"));
    }
    #[test]
    fn custom_patch_and_messages_tool_streams_keep_the_declared_native_wire_shape() {
        let body = json!({"model":PROBE_MODEL});
        let stream = tool_stream(
            &body,
            WorkerHarnessV1::CodexCli,
            "patch",
            Action {
                name: "apply_patch".into(),
                arguments: json!("*** Begin Patch\n*** End Patch\n"),
                custom: true,
            },
        );
        let values = stream
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let item = &values
            .iter()
            .find(|value| value["type"] == "response.output_item.done")
            .unwrap()["item"];
        assert_eq!(item["type"], "custom_tool_call");
        assert!(
            item["input"]
                .as_str()
                .unwrap()
                .starts_with("*** Begin Patch")
        );
        assert_eq!(item["call_id"], "patch");
        let stream = tool_stream(
            &body,
            WorkerHarnessV1::ClaudeCode,
            "read",
            Action {
                name: "Read".into(),
                arguments: json!({"file_path":"C:/private/file"}),
                custom: false,
            },
        );
        assert!(stream.contains("\"stop_reason\":\"tool_use\""));
        assert!(stream.contains("input_json_delta"));
    }
}
