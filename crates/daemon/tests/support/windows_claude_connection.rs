//! Real Claude discovery, public model settings, native ingress proof and owned restore.
use super::windows_headless_business::{assert_operation, cli};
use super::{InstalledProduct, assert_no_secret};
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

pub struct ClaudeConnection {
    context: String,
    restore: String,
    config: PathBuf,
    before: Value,
}
impl ClaudeConnection {
    pub fn configure(product: &InstalledProduct, plan_id: &str) -> Self {
        let selected = std::env::var_os("HIROUTE_WORKER_CLAUDE_BINARY")
            .map(PathBuf::from)
            .expect("Messages business gate requires the exact real Claude native artifact");
        assert!(selected.is_absolute() && selected.is_file());
        super::windows_execution_host::image(&selected);
        let discovered = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|directory| directory.join("claude.exe"))
            .find(|path| path.is_file())
            .expect("daemon startup PATH must expose the selected Claude native PE");
        assert_eq!(
            security::file_identity(&security::open_identity(&selected, false).unwrap()).unwrap(),
            security::file_identity(&security::open_identity(&discovered, false).unwrap()).unwrap(),
            "native discovery and supplied Claude artifact must be one object"
        );
        let home = std::env::var_os("CLAUDE_CONFIG_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| product.layout.home.join(".claude"));
        assert!(home.is_absolute());
        security::ensure_private_directory(&home).unwrap();
        let config = home.join("settings.json");
        assert!(
            !config.exists(),
            "disposable Windows Claude gate requires a fresh user settings file"
        );
        let before = b"{\"windows_headless_preserved\":true}\n".to_vec();
        let mut file = security::create_owner_only_file(&config).unwrap();
        file.write_all(&before).unwrap();
        file.sync_all().unwrap();
        drop(file);
        let scan = cli(product, &["agents", "scan"], None);
        let agent = scan["data"]["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|agent| agent["agent_id"] == "agent_claude_default")
            .unwrap();
        assert_eq!(
            agent["supported"], true,
            "real Claude scanner must support this private native settings source"
        );
        let version = version(&selected);
        assert!(
            version.contains(agent["version"].as_str().unwrap()),
            "public scanner version must come from the selected native binary"
        );
        let context = agent["context_id"].as_str().unwrap().to_owned();
        let listed = cli(product, &["agents", "list"], None);
        assert!(
            listed["data"]["agents"]
                .as_array()
                .unwrap()
                .iter()
                .any(|agent| agent["agent_id"] == "agent_claude_default"
                    && agent["context_id"] == context)
        );
        let checked = cli(
            product,
            &[
                "agents",
                "check",
                "agent_claude_default",
                "--scope",
                "native-authentication",
            ],
            None,
        );
        assert_eq!(
            checked["data"]["native_authentication"], "proven",
            "a configuration write alone does not prove native Claude ingress"
        );
        let mapping = json!({"kind":"plan","plan_id":plan_id});
        let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":context,
            "model":{"intent":"configure","settings":{"mode":"claude_launcher","surfaces":["claude_cli"],"fixed_models":[],
                "preset_mappings":{"opus":mapping,"sonnet":mapping,"haiku":mapping}}}});
        settings(product, &spec, "connect", "windows-public-claude-connect");
        let status = cli(product, &["agents", "connect", "status", &context], None);
        assert_eq!(status["data"]["state"], "configured");
        assert_eq!(
            status["data"]["current_selection"]["preset_mappings"]["sonnet"]["plan_id"],
            plan_id
        );
        let restore = status["data"]["restore_point_ref"]
            .as_str()
            .unwrap()
            .to_owned();
        let configured: Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
        assert_eq!(configured["windows_headless_preserved"], true);
        assert!(
            !configured["apiKeyHelper"].is_null(),
            "Claude must resolve its grant with the installed native helper"
        );
        Self {
            context,
            restore,
            config,
            before: serde_json::from_slice(&before).unwrap(),
        }
    }
    pub fn connection_id(&self) -> String {
        format!("agent-connection/{}", self.context)
    }
    /// Invoke the released managed launcher itself, not the native client directly.
    pub fn launch(&self, product: &InstalledProduct, alias: &str) {
        let args = [
            "agent",
            "launch",
            "--agent",
            "claude-code",
            "--context",
            &self.context,
            "--",
            "--print",
            "--model",
            alias,
            "--",
            "windows-routing-managed-launch",
        ]
        .map(OsString::from);
        let environment = std::env::vars_os().collect::<Vec<_>>();
        let mut child = Child::spawn(Launch {
            program: &product.stable,
            args: &args,
            env: &environment,
            cwd: Some(&product.root),
            stdin: Stream::Null,
            stdout: Stream::Piped,
            stderr: Stream::Piped,
        })
        .unwrap();
        let capture = |file: std::fs::File| {
            thread::spawn(move || {
                let mut bytes = Vec::new();
                file.take(1024 * 1024 + 1).read_to_end(&mut bytes).unwrap();
                assert!(bytes.len() <= 1024 * 1024);
                assert_no_secret(&bytes);
                bytes
            })
        };
        let output = capture(child.stdout.take().unwrap());
        let error = capture(child.stderr.take().unwrap());
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            if let Some(exit) = child.exit_code().unwrap() {
                assert_eq!(exit, 0, "released managed Claude launch failed");
                break;
            }
            if Instant::now() >= deadline {
                child.terminate().unwrap();
                panic!("released managed Claude launch exceeded bound");
            }
            thread::sleep(Duration::from_millis(20));
        }
        while !child.empty().unwrap() {
            assert!(
                Instant::now() < deadline,
                "managed launch leaked its native tree"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let output = output.join().unwrap();
        let error = error.join().unwrap();
        assert_no_secret(&error);
        assert!(
            String::from_utf8_lossy(&output).contains("windows native route answer"),
            "native Claude must consume its real Gateway answer"
        );
    }

    pub fn recovered(&self, product: &InstalledProduct) {
        let status = cli(
            product,
            &["agents", "connect", "status", &self.context],
            None,
        );
        assert_eq!(status["data"]["state"], "configured");
        assert_eq!(status["data"]["restore_point_ref"], self.restore);
        assert_eq!(
            serde_json::from_slice::<Value>(&std::fs::read(&self.config).unwrap()).unwrap()["windows_headless_preserved"],
            true
        );
    }
    pub fn restore(self, product: &InstalledProduct) {
        let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":self.context,
            "model":{"intent":"restore","restore_point_ref":self.restore}});
        settings(product, &spec, "restore", "windows-public-claude-restore");
        let status = cli(
            product,
            &["agents", "connect", "status", &self.context],
            None,
        );
        assert!(matches!(
            status["data"]["state"].as_str(),
            Some("restored" | "not_configured")
        ));
        assert_eq!(
            serde_json::from_slice::<Value>(&std::fs::read(&self.config).unwrap()).unwrap(),
            self.before,
            "public restore must preserve all prior native JSON values and remove owned settings"
        );
        security::verify_owner_only(&self.config, false).unwrap();
        std::fs::remove_file(&self.config).unwrap();
    }
}
fn settings(product: &InstalledProduct, spec: &Value, verb: &str, key: &str) {
    let preview = cli(
        product,
        &["agents", verb, "preview"],
        Some(&json!({"spec":spec})),
    )["data"]
        .clone();
    assert_eq!(
        preview["applicable"], true,
        "native public settings must be applicable"
    );
    assert_ne!(preview["resident_service"]["login_item_required"], true);
    assert_ne!(
        preview["resident_service"]["login_item_removal_required"],
        true
    );
    let request = json!({"spec":preview["spec"],"accept_digest":preview["accept_digest"],"dependency_digest":preview["dependency_digest"],"expected_revisions":preview["expected_revisions"],"idempotency_key":key});
    let applied = cli(product, &["agents", verb, "apply"], Some(&request));
    let operation = applied["operation"]["operation_id"].as_str().unwrap();
    assert_operation(product, operation);
    assert_eq!(
        cli(product, &["agents", verb, "apply"], Some(&request))["operation"]["operation_id"],
        operation
    );
}
fn version(binary: &Path) -> String {
    let args = [OsString::from("--version")];
    let environment = std::env::vars_os().collect::<Vec<_>>();
    let mut child = Child::spawn(Launch {
        program: binary,
        args: &args,
        env: &environment,
        cwd: binary.parent(),
        stdin: Stream::Null,
        stdout: Stream::Piped,
        stderr: Stream::Piped,
    })
    .unwrap();
    let capture = |file: std::fs::File| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            file.take(65537).read_to_end(&mut bytes).unwrap();
            assert!(bytes.len() <= 65536);
            assert_no_secret(&bytes);
            bytes
        })
    };
    let out = capture(child.stdout.take().unwrap());
    let err = capture(child.stderr.take().unwrap());
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(exit) = child.exit_code().unwrap() {
            assert_eq!(exit, 0, "selected native Claude version invocation failed");
            break;
        }
        if Instant::now() >= deadline {
            child.terminate().unwrap();
            panic!("selected Claude version exceeded bound");
        }
        thread::sleep(Duration::from_millis(10));
    }
    while !child.empty().unwrap() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    assert!(err.join().unwrap().is_empty());
    String::from_utf8(out.join().unwrap()).unwrap()
}
