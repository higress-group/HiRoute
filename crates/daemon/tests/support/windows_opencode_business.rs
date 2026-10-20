//! OpenCode's native Windows leaves reuse public routing, Worker receipts and Job checks.
use super::InstalledProduct;
use super::windows_headless_business::{Business, cli};
use super::windows_routing_business::Plan;
use super::windows_worker_business::{
    cancel, command, exec_args, oracle, select_native, selected, wait_result, write_new,
};
use hiroute_application_api::WorkerHarnessV1;
use hiroute_platform::windows::security;
use serde_json::{Value, json};
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

pub struct Context {
    variables: Vec<(OsString, OsString)>,
    configuration: PathBuf,
    original: Vec<u8>,
}
impl Context {
    pub fn prepare(product: &InstalledProduct) -> Self {
        let root = product.root.join("OpenCode native 中文 roots");
        security::create_owner_only_directory(&root).unwrap();
        let mut variables = Vec::new();
        let mut config = PathBuf::new();
        for (variable, directory) in [
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_STATE_HOME", "state"),
        ] {
            let parent = root.join(directory);
            security::create_owner_only_directory(&parent).unwrap();
            let native = parent.join("opencode");
            security::create_owner_only_directory(&native).unwrap();
            if variable == "XDG_CONFIG_HOME" {
                config = native;
            }
            variables.push((variable.into(), parent.into_os_string()));
        }
        let configuration = config.join("opencode.json");
        let original = br#"{"$schema":"https://opencode.ai/config.json","instructions":[],"autoupdate":false,"share":"disabled"}"#.to_vec();
        write_new(&configuration, &original);
        Self {
            variables,
            configuration,
            original,
        }
    }
    pub fn variables(&self) -> &[(OsString, OsString)] {
        &self.variables
    }
    pub fn preserved(&self) {
        assert_eq!(std::fs::read(&self.configuration).unwrap(), self.original);
    }
}

pub struct Journey {
    workspace: PathBuf,
    prompt: PathBuf,
    continued_prompt: PathBuf,
    receipt: String,
    plan: Plan,
    _source: Business,
    worker: oracle::Oracle,
    original: Value,
}
impl Journey {
    pub fn run(product: &InstalledProduct) -> Self {
        let binary = selected("HIROUTE_WORKER_OPENCODE_BINARY");
        let workspace = product.root.join("OpenCode public Worker 中文 % ! ' &");
        security::create_owner_only_directory(&workspace).unwrap();
        let mut entropy = [0u8; 16];
        getrandom::fill(&mut entropy).unwrap();
        let nonce: String = entropy.iter().map(|b| format!("{b:02x}")).collect();
        let receipt = format!("worker-direct-{nonce}");
        let project = workspace.join(".opencode");
        let skills = project.join("skills");
        let skill = skills.join("windows-worker-receipt");
        for path in [&project, &skills, &skill] {
            security::create_owner_only_directory(path).unwrap();
        }
        let skill = skill.join("SKILL.md");
        let contents = format!(
            "---\nname: windows-worker-receipt\ndescription: Complete native receipt skill-discovery-{nonce}\n---\n\nRead this actual Skill and use native write/read tools.\nContent receipt: skill-contents-{nonce}\n"
        );
        write_new(&skill, contents.as_bytes());
        let worker = oracle::Oracle::worker(&workspace, skill.clone(), contents.clone(), &nonce);
        let source = Business::save_on(
            product,
            "candidate/native/windows-opencode",
            "windows-opencode-source",
            worker.address(),
        );
        let editor = json!({"schema":"hiroute.plan-editor/v2","display_name":"Windows OpenCode Worker",
            "purpose":"Use actual native Skills, tools and exact history","mode":"fixed_model",
            "candidates":[{"binding_id":source.binding_id()}],
            "smart":{"economy":[],"primary":[],"judgment":super::windows_routing_business::judgment(),
                "reselect_on_user_message":false,"classifier":{"kind":"local_rules"},"complex_keywords":[]},
            "free":{"candidates":[],"primary":[],"primary_fallback":false},
            "delegation_enabled":true,"work":{"harness":"opencode","protocol":"responses"},"requirements":{},
            "limits":{"maximum_attempts":1,"request_timeout_ms":30000,"attempt_timeout_ms":30000}});
        let plan = Plan::create_with_editor(product, editor, "windows-opencode-plan");
        let discovered = cli(
            product,
            &[
                "worker",
                "dependencies",
                "discover",
                "--harness",
                "opencode",
            ],
            None,
        );
        select_native(product, &discovered, WorkerHarnessV1::OpenCode, &binary);
        let prompt = workspace.join("direct.txt");
        let continued_prompt = workspace.join("continue.txt");
        write_new(&prompt, format!("{}: Load the windows-worker-receipt Skill and complete its bounded tool receipt.\n", oracle::DIRECT).as_bytes());
        write_new(
            &continued_prompt,
            format!(
                "{} {}: Continue the original task with its original successful read receipt.\n",
                oracle::CONTINUE,
                oracle::DIRECT
            )
            .as_bytes(),
        );
        let accepted = command(
            product,
            &exec_args(
                &plan.plan_id,
                &workspace,
                &prompt,
                "windows-opencode-direct",
            ),
            None,
            Duration::from_secs(180),
        );
        let result = wait_result(product, accepted, &worker);
        assert!(result["result"].as_str().unwrap().contains(&receipt));
        assert_eq!(
            std::fs::read_to_string(workspace.join("direct 中文 % ! ' &.txt"))
                .unwrap()
                .trim(),
            receipt
        );
        assert_eq!(std::fs::read(&skill).unwrap(), contents.as_bytes());
        cancel::verify_opencode(product, &workspace, &plan.plan_id, &nonce);
        Self {
            workspace,
            prompt,
            continued_prompt,
            receipt,
            plan,
            _source: source,
            worker,
            original: result,
        }
    }

    pub fn recovered(&self, product: &InstalledProduct) {
        let replay = command(
            product,
            &exec_args(
                &self.plan.plan_id,
                &self.workspace,
                &self.prompt,
                "windows-opencode-direct",
            ),
            None,
            Duration::from_secs(180),
        );
        assert_eq!(replay["data"]["task_id"], self.original["task_id"]);
        assert_eq!(replay["data"]["run_id"], self.original["run_id"]);
        assert_eq!(replay["data"]["replayed"], true);
        let accepted = command(
            product,
            &[
                "worker",
                "continue",
                "--task",
                self.original["task_id"].as_str().unwrap(),
                "--expected-latest-run",
                self.original["run_id"].as_str().unwrap(),
                "--file",
                self.continued_prompt.to_str().unwrap(),
                "--no-wait",
                "--run-timeout",
                "120",
                "--submission-key",
                "windows-opencode-continue",
            ],
            None,
            Duration::from_secs(180),
        );
        let result = wait_result(product, accepted, &self.worker);
        assert_eq!(result["task_id"], self.original["task_id"]);
        assert_ne!(result["run_id"], self.original["run_id"]);
        assert!(
            result["result"]
                .as_str()
                .unwrap()
                .contains(&format!("continued-{}", self.receipt))
        );
        assert!(self.worker.ledger.lock().unwrap().continued > 0);
    }
}
