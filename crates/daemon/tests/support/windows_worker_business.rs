//! Real public Worker and installed native Main → user Skill → CLI → Worker journey.
//! The fixture shares the calling gate's one disposable installed Windows product and
//! uses public management throughout. Only the independent model endpoints are controlled.
#[path = "windows_worker_cancel.rs"]
pub(super) mod cancel;
#[path = "windows_worker_dependency_selection.rs"]
mod dependency_selection;
#[path = "windows_worker_failure_diagnostics.rs"]
mod failure_diagnostics;
#[path = "windows_worker_oracle.rs"]
pub(super) mod oracle;

pub(crate) use dependency_selection::discovery_environment;

use super::windows_headless_business::{Business, assert_operation, cli};
use super::windows_routing_business::Plan;
use super::{InstalledProduct, SECRET, assert_no_secret};
use hiroute_platform::windows::{
    job::{Child, Launch, Stream},
    security,
};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const AGENT: &str = "agent_codex_default";
const DIRECT_KEY: &str = "windows-public-worker-direct";
const BOUND: u64 = 1024 * 1024;

pub struct WorkerBusiness {
    workspace: PathBuf,
    direct_prompt: PathBuf,
    continue_prompt: PathBuf,
    receipt: String,
    delegated_receipt: String,
    plan: Plan,
    _worker_source: Business,
    _main_source: Business,
    worker: oracle::Oracle,
    main: oracle::Oracle,
    direct: Value,
    latest: Value,
    delegated: Value,
    context: String,
    connection: String,
    original_config: Option<Vec<u8>>,
    skill: PathBuf,
    cancellation: cancel::CancellationReceipts,
}

impl WorkerBusiness {
    pub fn run(product: &InstalledProduct, allowed_main_plan_ids: &[&str]) -> Self {
        let workspace = product.root.join("public Worker 中文 % ! ' &");
        security::create_owner_only_directory(&workspace).unwrap();
        let mut entropy = [0u8; 16];
        getrandom::fill(&mut entropy).unwrap();
        let nonce: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
        let receipt = format!("worker-direct-{nonce}");
        let delegated_receipt = format!("worker-delegated-{nonce}");
        let project = workspace.join(".pi");
        let skills = project.join("skills");
        let skill_root = skills.join("windows-worker-receipt");
        for path in [&project, &skills, &skill_root] {
            security::create_owner_only_directory(path).unwrap();
        }
        let project_skill = skill_root.join("SKILL.md");
        let contents = format!(
            "---\nname: windows-worker-receipt\ndescription: Complete native receipt skill-discovery-{nonce}\n---\n\nRead this actual project Skill and use the native write/read tools to complete the delegated receipt task.\nContent receipt: skill-contents-{nonce}\n"
        );
        write_new(&project_skill, contents.as_bytes());
        let direct_prompt = workspace.join("direct-task.txt");
        let delegated_prompt = workspace.join("delegated-task.txt");
        let continue_prompt = workspace.join("continue-task.txt");
        write_new(&direct_prompt, format!("{}: Use the windows-worker-receipt project skill and native tools to complete the bounded local receipt task.\n", oracle::DIRECT).as_bytes());
        write_new(&delegated_prompt, format!("{}: Use the windows-worker-receipt project skill and native tools to complete the bounded local receipt task.\n", oracle::DELEGATED).as_bytes());
        write_new(&continue_prompt, format!("{} {}: Continue the exact existing native conversation. Return its original successful native read receipt, prefixed continued-.\n", oracle::CONTINUE, oracle::DIRECT).as_bytes());
        let worker =
            oracle::Oracle::worker(&workspace, project_skill.clone(), contents.clone(), &nonce);
        let worker_source = Business::save_on(
            product,
            "candidate/native/windows-worker",
            "windows-worker-source",
            worker.address(),
        );
        // Publish through the current typed editor producer. The Worker protocol is
        // declared independently from the Main route and then frozen by admission.
        let editor = json!({"schema":"hiroute.plan-editor/v2","display_name":"Windows real Pi Worker",
            "purpose":"Read the native project Skill and create a local tool receipt","mode":"fixed_model",
            "candidates":[{"binding_id":worker_source.binding_id()}],
            "smart":{"economy":[],"primary":[],"judgment":super::windows_routing_business::judgment(),
                "reselect_on_user_message":false,"classifier":{"kind":"local_rules"},"complex_keywords":[]},
            "free":{"candidates":[],"primary":[],"primary_fallback":false},
            "delegation_enabled":true,"work":{"harness":"pi","protocol":"responses"},"requirements":{},
            "limits":{"maximum_attempts":1,"request_timeout_ms":30000,"attempt_timeout_ms":30000}});
        let plan = Plan::create_with_editor(product, editor, "windows-worker-plan");
        select_pi(product);
        let plans = cli(product, &["worker", "plans"], None);
        let selected_plans: Vec<_> = plans["data"]["plans"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["agent_plan_id"] == plan.plan_id)
            .collect();
        assert_eq!(selected_plans.len(), 1);
        assert_eq!(selected_plans[0]["harness"], "pi");
        assert_eq!(selected_plans[0]["availability"], "ready");
        let direct_args = exec_args(&plan.plan_id, &workspace, &direct_prompt, DIRECT_KEY);
        let direct = command(product, &direct_args, None, Duration::from_secs(180));
        let replay = command(product, &direct_args, None, Duration::from_secs(180));
        same_identity(&replay["data"], &direct["data"]);
        assert_eq!(replay["data"]["replayed"], true);
        let result = wait_result(product, direct.clone(), &worker);
        assert!(result["result"].as_str().unwrap().contains(&receipt));
        assert_eq!(
            std::fs::read_to_string(workspace.join("direct 中文 % ! ' &.txt"))
                .unwrap()
                .trim(),
            receipt
        );
        assert!(worker.ledger.lock().unwrap().completed.contains("direct"));
        assert_eq!(std::fs::read(&project_skill).unwrap(), contents.as_bytes());
        let listed = cli(product, &["worker", "list"], None);
        assert!(
            listed["data"]["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|task| task["task_id"] == result["task_id"])
        );
        let read = cli(
            product,
            &[
                "worker",
                "read",
                "--run",
                result["run_id"].as_str().unwrap(),
                "--max-bytes",
                "16384",
            ],
            None,
        );
        assert_eq!(read["data"]["content_state"], "available");
        assert!(read["data"]["text"].as_str().unwrap().contains(&receipt));
        let cancellation = cancel::verify(product, &workspace, &plan.plan_id, &nonce);

        let scanned = cli(product, &["agents", "scan"], None);
        let codex = scanned["data"]["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|agent| agent["agent_id"] == AGENT)
            .expect("fixed actual Codex must be discoverable by the production scanner");
        let context = codex["codex_access"]["root_context_id"]
            .as_str()
            .or_else(|| codex["context_id"].as_str())
            .unwrap()
            .to_owned();
        command(
            product,
            &["agents", "check", AGENT, "--scope", "native-authentication"],
            None,
            Duration::from_secs(60),
        );
        let skill = product
            .layout
            .home
            .join(".agents/skills/hiroute-collaboration/SKILL.md");
        assert!(
            !skill.exists(),
            "disposable product fixture must not replace a borrowed user Skill"
        );
        command(
            product,
            &[
                "agents",
                "collaboration",
                "configure",
                AGENT,
                "--trigger",
                "explicit",
                "--idempotency-key",
                "windows-public-main-collaboration",
            ],
            None,
            Duration::from_secs(90),
        );
        let skill_contents = std::fs::read_to_string(&skill)
            .expect("public Configure must install the actual user Skill");
        assert!(
            skill_contents.contains("hiroute worker plans")
                && skill_contents.contains("hiroute worker exec")
        );
        security::verify_owner_only(&skill, false).unwrap();
        command(
            product,
            &["agents", "check", AGENT, "--scope", "collaboration"],
            None,
            Duration::from_secs(90),
        );
        let main = oracle::Oracle::main(
            skill.clone(),
            skill_contents,
            product.stable.clone(),
            workspace.clone(),
            delegated_prompt,
            plan.plan_id.clone(),
            delegated_receipt.clone(),
            worker.ledger.clone(),
        );
        let main_source = Business::save_on(
            product,
            "candidate/native/windows-main",
            "windows-main-source",
            main.address(),
        );
        let main_plan = Plan::create(product, main_source.binding_id(), "windows-main-plan");
        let config = codex_config(product);
        let original_config = read_optional(&config);
        let mut allowed: Vec<_> = allowed_main_plan_ids
            .iter()
            .map(|id| (*id).to_owned())
            .collect();
        allowed.extend([main_plan.plan_id.clone(), plan.plan_id.clone()]);
        allowed.sort();
        allowed.dedup();
        let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":context,
            "model":{"intent":"configure","settings":{"mode":"codex_default","native_model_mode":"hiroute_only",
                "fixed_models":[],"allowed_plan_ids":allowed,"default_selection":{"kind":"plan","plan_id":main_plan.plan_id}}}});
        apply_settings(product, "connect", &spec, "windows-public-main-model");
        let status = cli(product, &["agents", "connect", "status", &context], None);
        assert_eq!(status["data"]["state"], "configured");
        assert_eq!(status["data"]["collaboration"]["state"], "configured");
        assert!(status["data"]["restore_point_ref"].is_string());
        assert!(status["data"]["collaboration"]["restore_point_ref"].is_string());
        let connection = format!("agent-connection/{context}");
        let binary = selected("HIROUTE_WORKER_CODEX_BINARY");
        let arguments = [
            "-a",
            "never",
            "exec",
            "--ephemeral",
            "--skip-git-repo-check",
            "--ignore-rules",
            "--sandbox",
            "danger-full-access",
            "--color",
            "never",
            "--json",
            "WINDOWS-PUBLIC-MAIN-AGENT: Use $hiroute-collaboration to delegate the bounded local receipt task through the published Windows real Pi Worker Plan. Observe its actual public result before reporting completion.",
        ];
        let (exit, output, _error) = run(
            &binary,
            &arguments.iter().map(OsString::from).collect::<Vec<_>>(),
            Some(&workspace),
            None,
            Duration::from_secs(360),
        );
        assert_eq!(
            exit, 0,
            "actual fixed native Main process failed before completing the public Worker journey"
        );
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains(&format!("main-completed-{delegated_receipt}"))
        );
        let delegated = main
            .ledger
            .lock()
            .unwrap()
            .main_completed
            .clone()
            .expect("Main must observe its own real public Worker identity and tool result");
        assert_ne!(delegated["task_id"], result["task_id"]);
        assert_ne!(delegated["task_id"], cancellation.residual_task);
        assert_ne!(delegated["run_id"], result["run_id"]);
        assert!(main.ledger.lock().unwrap().requests >= 5);
        assert!(worker.ledger.lock().unwrap().requests >= 8);
        Self {
            workspace,
            direct_prompt,
            continue_prompt,
            receipt,
            delegated_receipt,
            plan,
            _worker_source: worker_source,
            _main_source: main_source,
            worker,
            main,
            direct: result.clone(),
            latest: result,
            delegated,
            context,
            connection,
            original_config,
            skill,
            cancellation,
        }
    }

    pub fn connection_id(&self) -> &str {
        &self.connection
    }

    /// Called only after the shared fixture has actually restarted its daemon. Durable
    /// public locators must survive, then Continue must consume the exact native history.
    pub fn recovered(&mut self, product: &InstalledProduct) {
        for expected in [&self.direct, &self.delegated] {
            let result = cli(
                product,
                &[
                    "worker",
                    "result",
                    "--run",
                    expected["run_id"].as_str().unwrap(),
                ],
                None,
            );
            same_identity(&result["data"], expected);
            assert_eq!(result["data"]["run_state"], "succeeded");
            assert_eq!(result["data"]["result"], expected["result"]);
        }
        let replay = command(
            product,
            &exec_args(
                &self.plan.plan_id,
                &self.workspace,
                &self.direct_prompt,
                DIRECT_KEY,
            ),
            None,
            Duration::from_secs(180),
        );
        same_identity(&replay["data"], &self.direct);
        assert_eq!(replay["data"]["replayed"], true);
        let continued = command(
            product,
            &[
                "worker",
                "continue",
                "--task",
                self.latest["task_id"].as_str().unwrap(),
                "--expected-latest-run",
                self.latest["run_id"].as_str().unwrap(),
                "--file",
                self.continue_prompt.to_str().unwrap(),
                "--no-wait",
                "--run-timeout",
                "120",
                "--submission-key",
                "windows-public-worker-recovered-continue",
            ],
            None,
            Duration::from_secs(180),
        );
        assert_eq!(continued["data"]["task_id"], self.direct["task_id"]);
        assert_ne!(continued["data"]["run_id"], self.direct["run_id"]);
        let result = wait_result(product, continued, &self.worker);
        assert!(
            result["result"]
                .as_str()
                .unwrap()
                .contains(&format!("continued-{}", self.receipt))
        );
        assert!(self.worker.ledger.lock().unwrap().continued > 0);
        self.latest = result;
        for (role, receipt) in [
            ("direct", &self.receipt),
            ("delegated", &self.delegated_receipt),
        ] {
            assert_eq!(
                std::fs::read_to_string(self.workspace.join(format!("{role} 中文 % ! ' &.txt")))
                    .unwrap()
                    .trim(),
                receipt
            );
        }
        assert!(self.main.ledger.lock().unwrap().main_completed.is_some());
        assert!(
            self.cancellation.late.iter().all(|path| !path.exists()),
            "a cancelled native tool survived the actual daemon restart"
        );
    }

    /// Keep the actual connection alive for the caller's Gateway/observation phase.
    /// Restore only after that phase; do not delete sources referenced by Plan history.
    pub fn restore(self, product: &InstalledProduct) {
        assert!(
            self.cancellation.late.iter().all(|path| !path.exists()),
            "a cancelled native tool survived the complete business journey"
        );
        let status = cli(
            product,
            &["agents", "connect", "status", &self.context],
            None,
        )["data"]
            .clone();
        let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":self.context,
            "model":{"intent":"restore","restore_point_ref":status["restore_point_ref"]},
            "collaboration":{"intent":"restore","restore_point_ref":status["collaboration"]["restore_point_ref"]}});
        apply_settings(product, "restore", &spec, "windows-public-main-restore");
        let status = cli(
            product,
            &["agents", "connect", "status", &self.context],
            None,
        );
        // Model restore withdraws the active grant/artifact. The current model
        // status returns to its unconfigured baseline; collaboration retains
        // its separate restore-operation history.
        assert_eq!(status["data"]["state"], "not_configured");
        assert_eq!(status["data"]["collaboration"]["state"], "restored");
        assert!(status["data"]["current_selection"].is_null());
        assert!(status["data"]["restore_point_ref"].is_null());
        assert!(
            !self.skill.exists(),
            "Restore must remove only its installed user Skill"
        );
        assert_eq!(read_optional(&codex_config(product)), self.original_config);
    }
}

fn codex_config(product: &InstalledProduct) -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| product.layout.home.join(".codex"))
        .join("config.toml")
}
fn read_optional(path: &Path) -> Option<Vec<u8>> {
    match security::open_identity(path, false) {
        Ok(file) => {
            let mut bytes = Vec::new();
            file.take(BOUND + 1).read_to_end(&mut bytes).unwrap();
            assert!(bytes.len() as u64 <= BOUND);
            Some(bytes)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!(
            "native config snapshot failed: kind={:?}, os={:?}",
            error.kind(),
            error.raw_os_error()
        ),
    }
}

pub(super) fn write_new(path: &Path, bytes: &[u8]) {
    hiroute_platform::durable::write_private_new(path, bytes).unwrap();
    security::verify_owner_only(path, false).unwrap();
}
pub(super) fn selected(variable: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())
        .unwrap_or_else(|| panic!("{variable} must select the fixed actual native client"))
}
pub(super) fn select_native(
    product: &InstalledProduct,
    discovered: &Value,
    harness: hiroute_application_api::WorkerHarnessV1,
    binary: &Path,
) {
    let selection = dependency_selection::select_native(discovered, harness, binary);
    command(
        product,
        &["worker", "dependencies", "select"],
        Some(&serde_json::to_value(&selection.request).unwrap()),
        Duration::from_secs(120),
    );
}

fn select_pi(product: &InstalledProduct) {
    let discovered = cli(
        product,
        &["worker", "dependencies", "discover", "--harness", "pi"],
        None,
    );
    let selection = dependency_selection::select(
        &discovered,
        &selected("HIROUTE_WORKER_PI_BINARY"),
        &selected("HIROUTE_WORKER_NODE"),
    );
    let revision = selection.request.expected_selection_revision;
    // Direct LocalControl still binds the caller's exact tuple. Only the CLI
    // ordinary input boundary projects actual identities before transport.
    let client = hiroute_client_core::Client::new(
        "hiroute-cli",
        hiroute_client_core::LocalEndpoint::from_runtime_root(&product.layout.runtime_root),
    )
    .with_timeout(Duration::from_secs(120));
    let rejected =
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(client.select_worker_dependencies(
                "windows-direct-canonical-contract",
                &selection.ordinary,
            ))
            .unwrap_or_else(|_| panic!("direct LocalControl canonical contract transport failed"));
    let rejected = serde_json::to_value(rejected).unwrap();
    assert_eq!(
        dependency_selection::error_code(&rejected),
        "INVALID_ARGUMENTS"
    );
    assert_eq!(
        dependency_selection::message_key(&rejected),
        "cli.error.invalid_arguments"
    );
    assert!(
        rejected["operation"].is_null(),
        "rejected tuple must not start an Operation"
    );
    let unchanged = cli(
        product,
        &["worker", "dependencies", "discover", "--harness", "pi"],
        None,
    );
    assert_eq!(
        unchanged["data"]["selection_revisions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["harness"] == "pi")
            .unwrap()["revision"]
            .as_u64(),
        Some(revision),
        "direct noncanonical tuple rejection must not consume the selection revision"
    );
    let selected = command(
        product,
        &["worker", "dependencies", "select"],
        Some(&serde_json::to_value(&selection.ordinary).unwrap()),
        Duration::from_secs(120),
    );
    assert_eq!(selected["operation"]["state"], "succeeded");
    assert_eq!(
        selected["data"]["selection_revisions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["harness"] == "pi")
            .unwrap()["revision"]
            .as_u64()
            .unwrap(),
        revision + 1
    );
    let replay = command(
        product,
        &["worker", "dependencies", "select"],
        Some(&serde_json::to_value(&selection.request).unwrap()),
        Duration::from_secs(120),
    );
    assert_eq!(
        replay["operation"], selected["operation"],
        "ordinary and canonical CLI input must reuse one Operation"
    );
    assert_eq!(
        replay["data"]["selection_revisions"],
        selected["data"]["selection_revisions"]
    );
    assert_eq!(replay["data"]["selected"], selected["data"]["selected"]);
}
pub(crate) fn exec_args<'a>(
    plan: &'a str,
    workspace: &'a Path,
    prompt: &'a Path,
    key: &'a str,
) -> Vec<&'a str> {
    vec![
        "worker",
        "exec",
        "--plan",
        plan,
        "--cwd",
        workspace.to_str().unwrap(),
        "--file",
        prompt.to_str().unwrap(),
        "--run-timeout",
        "120",
        "--no-wait",
        "--submission-key",
        key,
    ]
}
fn same_identity(current: &Value, expected: &Value) {
    for field in ["task_id", "run_id"] {
        assert_eq!(current[field], expected[field]);
    }
}
pub(super) fn wait_result(
    product: &InstalledProduct,
    accepted: Value,
    worker: &oracle::Oracle,
) -> Value {
    let task = accepted["data"]["task_id"].as_str().unwrap();
    let run = accepted["data"]["run_id"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut current = accepted.clone();
    while !matches!(
        current["data"]["run_state"].as_str(),
        Some("succeeded" | "failed" | "cancelled" | "unknown")
    ) {
        assert!(
            Instant::now() < deadline,
            "actual public Worker did not finish within the admitted task deadline"
        );
        let revision = oracle::wait_revision(&current, run).to_string();
        current = command(
            product,
            &[
                "worker",
                "wait",
                "--run",
                run,
                "--after-revision",
                &revision,
                "--wait-timeout",
                "20",
            ],
            None,
            Duration::from_secs(30),
        );
        same_identity(&current["data"], &accepted["data"]);
    }
    if current["data"]["run_state"] != "succeeded" {
        // Query the real public result before the unchanged success assertion.
        // Arbitrary native/model text remains only in memory, never diagnostics.
        let (exit, result) = command_outcome(
            product,
            &["worker", "result", "--run", run],
            None,
            Duration::from_secs(30),
        );
        let (ledger, poisoned) = match worker.ledger.lock() {
            Ok(ledger) => (ledger, false),
            Err(error) => (error.into_inner(), true),
        };
        let receipt =
            failure_diagnostics::receipt(&current, &result, task, run, exit, &ledger, poisoned);
        println!("Windows public Worker terminal facts: {receipt}");
    }
    assert_eq!(
        current["data"]["run_state"], "succeeded",
        "actual native public Worker run must succeed"
    );
    let result = cli(product, &["worker", "result", "--run", run], None);
    assert_eq!(result["data"]["task_id"], task);
    assert_eq!(result["data"]["run_id"], run);
    assert_eq!(result["data"]["run_state"], "succeeded");
    result["data"].clone()
}
fn apply_settings(product: &InstalledProduct, verb: &str, spec: &Value, key: &str) {
    let preview = cli(
        product,
        &["agents", verb, "preview"],
        Some(&json!({"spec":spec})),
    )["data"]
        .clone();
    if preview["applicable"] != true {
        for blocker in preview["blockers"].as_array().into_iter().flatten() {
            let reason = match blocker["reason"].as_str() {
                Some("capability_unavailable") => "capability-unavailable",
                Some("model_plan_unavailable") => "model-plan-unavailable",
                Some("native_model_coverage_unavailable") => "native-model-coverage",
                Some("codex_context_override") => "codex-context-override",
                _ => "other-blocker",
            };
            let capabilities: Vec<_> = blocker["capabilities"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|block| {
                    let capability = serde_json::from_value::<hiroute_domain::AgentCapability>(
                        block["capability"].clone(),
                    )
                    .ok();
                    let reason = match block["reason"].as_str() {
                        Some("unknown") => "unknown",
                        Some("unavailable") => "unavailable",
                        Some("stale") => "stale",
                        _ => "unclassified",
                    };
                    (capability, reason)
                })
                .collect();
            eprintln!(
                "Windows Agent preview failure: reason={reason}, capabilities={capabilities:?}"
            );
        }
    }
    assert_eq!(
        preview["applicable"], true,
        "public Agent preview must admit this actual native context"
    );
    assert_ne!(
        preview["resident_service"]["login_item_required"], true,
        "foreground standalone gate must not fabricate a background registration declaration"
    );
    let request = json!({"spec":preview["spec"],"accept_digest":preview["accept_digest"],
        "dependency_digest":preview["dependency_digest"],"expected_revisions":preview["expected_revisions"],"idempotency_key":key});
    let applied = cli(product, &["agents", verb, "apply"], Some(&request));
    let operation = applied["operation"]["operation_id"]
        .as_str()
        .or_else(|| applied["data"]["operation_id"].as_str())
        .unwrap();
    assert_operation(product, operation);
    let replay = cli(product, &["agents", verb, "apply"], Some(&request));
    assert_eq!(
        replay["operation"]["operation_id"]
            .as_str()
            .or_else(|| replay["data"]["operation_id"].as_str())
            .unwrap(),
        operation
    );
}
pub(super) fn command(
    product: &InstalledProduct,
    args: &[&str],
    payload: Option<&Value>,
    deadline: Duration,
) -> Value {
    let (exit, value) = command_outcome(product, args, payload, deadline);
    if exit != 0 && args.starts_with(&["agents", "collaboration", "configure"]) {
        // Read-only failure localization: transport health and the same intent's
        // preview distinguish daemon loss from the CLI's frozen receipt boundary.
        for (stage, probe) in [
            ("service", vec!["system", "status"]),
            (
                "preview",
                vec![
                    "agents",
                    "collaboration",
                    "configure",
                    args[3],
                    "--trigger",
                    "explicit",
                    "--preview",
                ],
            ),
        ] {
            let (probe_exit, observed) =
                command_outcome(product, &probe, None, Duration::from_secs(30));
            println!(
                "Windows collaboration failure facts: stage={stage}, exit={probe_exit}, code={}, applicable={:?}",
                dependency_selection::error_code(&observed),
                observed["data"]["applicable"].as_bool()
            );
        }
    }
    assert_eq!(
        exit,
        0,
        "actual public Worker business CLI failed: command={}, code={}, key={}",
        dependency_selection::command_label(args),
        dependency_selection::error_code(&value),
        dependency_selection::message_key(&value)
    );
    assert!(matches!(
        value["status"].as_str(),
        Some("succeeded" | "accepted")
    ));
    value
}

fn command_outcome(
    product: &InstalledProduct,
    args: &[&str],
    payload: Option<&Value>,
    deadline: Duration,
) -> (i32, Value) {
    let label = dependency_selection::command_label(args);
    println!("Windows public Worker checkpoint: command={label}");
    let mut args: Vec<OsString> = args.iter().map(OsString::from).collect();
    if payload.is_some() {
        args.push("--request-stdin".into());
    }
    args.extend(["--output", "json"].map(OsString::from));
    let input = payload.map(|value| serde_json::to_vec(value).unwrap());
    let (exit, output, _error) = run(
        &product.stable,
        &args,
        product.stable.parent(),
        input.as_deref(),
        deadline.max(Duration::from_secs(30)) + Duration::from_secs(5),
    );
    let value: Value = serde_json::from_slice(&output)
        .expect("actual public Worker business CLI must return its JSON envelope");
    if exit != 0 {
        println!(
            "Windows public Worker failure: command={label}, code={}, key={}",
            dependency_selection::error_code(&value),
            dependency_selection::message_key(&value)
        );
    }
    (exit, value)
}
fn run(
    program: &Path,
    args: &[OsString],
    cwd: Option<&Path>,
    input: Option<&[u8]>,
    bound: Duration,
) -> (i32, Vec<u8>, Vec<u8>) {
    let environment: Vec<_> = std::env::vars_os().collect();
    let mut child = Child::spawn(Launch {
        program,
        args,
        env: &environment,
        cwd,
        stdin: if input.is_some() {
            Stream::Piped
        } else {
            Stream::Null
        },
        stdout: Stream::Piped,
        stderr: Stream::Piped,
    })
    .unwrap();
    let capture = |file: std::fs::File| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            file.take(BOUND + 1).read_to_end(&mut bytes).unwrap();
            assert!(
                bytes.len() as u64 <= BOUND,
                "native business output exceeds bound"
            );
            assert_no_secret(&bytes);
            bytes
        })
    };
    let stdout = capture(child.stdout.take().unwrap());
    let stderr = capture(child.stderr.take().unwrap());
    if let Some(bytes) = input {
        assert_no_secret(bytes);
        assert!(bytes.len() <= 65536);
        child.stdin.take().unwrap().write_all(bytes).unwrap();
    }
    let deadline = Instant::now() + bound;
    let exit = loop {
        if let Some(exit) = child.exit_code().unwrap() {
            break exit;
        }
        if Instant::now() >= deadline {
            child.terminate().unwrap();
            panic!("real native business process exceeded its deadline");
        }
        thread::sleep(Duration::from_millis(10));
    };
    while !child.empty().unwrap() {
        if Instant::now() >= deadline {
            child.terminate().unwrap();
            panic!("native business process left owned descendants");
        }
        thread::sleep(Duration::from_millis(10));
    }
    (exit, stdout.join().unwrap(), stderr.join().unwrap())
}
