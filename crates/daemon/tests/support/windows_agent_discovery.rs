//! Public discovery through an isolated production Control daemon and measured CLI.
use super::{security, windows_user_models};
use hiroute_integrations::agents::NativeProbeProcess;
use hiroute_platform::windows::{
    job::{Child, Launch, Stream},
    launcher::system_executable,
};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

const CODEX: &str = "agent_codex_default";
const CODEX_DESKTOP: &str = "agent_codex_desktop";
const DSH: &str = "agent_dsh_default";

#[test]
#[ignore = "requires official DSH npm and Node artifacts on Windows"]
fn actual_windows_dsh_scan_accepts_a_standalone_shim_and_skips_invalid_path_candidates() {
    let node = input("HIROUTE_WORKER_NODE");
    let entry = input("HIROUTE_WORKER_DSH_BINARY");
    let npm = windows_user_models::declared_shim_directory(&entry, "@deepseek-ai/dsh", "dsh");
    let root = tempfile::tempdir().unwrap();
    let bare = root.path().join("standalone bin");
    let invalid_directory = root.path().join("directory candidate");
    let invalid_files = root.path().join("file candidates");
    for path in [&bare, &invalid_directory, &invalid_files] {
        fs::create_dir(path).unwrap();
    }
    fs::create_dir(invalid_directory.join("dsh.exe")).unwrap();
    fs::write(invalid_files.join("dsh.exe"), b"invalid Windows image").unwrap();
    fs::write(invalid_files.join("dsh.cmd"), b"@exit /b 9\r\n").unwrap();
    fs::write(
        bare.join("dsh.cmd"),
        format!(
            "@echo off\r\n\"{}\" \"{}\" %*\r\n",
            node.display(),
            entry.display()
        ),
    )
    .unwrap();
    assert!(!bare.join("node_modules").exists());
    let cmd = system_executable(Path::new("cmd.exe")).unwrap();
    let cases = [
        ("npm", vec![npm.as_path()]),
        ("standalone", vec![bare.as_path()]),
        (
            "earlier-directory",
            vec![invalid_directory.as_path(), bare.as_path()],
        ),
        (
            "earlier-files",
            vec![invalid_files.as_path(), bare.as_path()],
        ),
    ];
    for (case, mut search) in cases {
        search.push(node.parent().unwrap());
        let case_root = root.path().join(case);
        let scope = ScanScope::new(&case_root, search);
        let version_args = ["/d", "/c", "dsh --version"].map(OsString::from);
        // A malformed regular PE can stop cmd's own resolution. Those files
        // still must not hide the later admitted DSH from HiRoute.
        let version = if case == "earlier-files" {
            let (exit, bytes) = scope.run(
                &node,
                &[entry.as_os_str().to_owned(), "--version".into()],
                None,
            );
            assert_eq!(exit, 0, "{case}: the pinned DSH CLI must be runnable");
            bytes
        } else {
            let (exit, bytes) = scope.run(&cmd, &version_args, None);
            assert_eq!(exit, 0, "{case}: the shell must resolve the real DSH");
            bytes
        };
        let version = std::str::from_utf8(&version).unwrap().trim();
        assert!(!version.is_empty());
        let mut daemon = scope.start();
        let all = scope.scan(None);
        let dsh = row(&all, DSH);
        assert_eq!(dsh["version"], version, "{case}");
        assert_registered(dsh, "dsh_cli");
        let targeted = scope.scan(dsh["context_id"].as_str());
        assert_eq!(targeted.len(), 1);
        assert_registered(row(&targeted, DSH), "dsh_cli");
        daemon.stop().unwrap();
        println!(
            "Windows DSH discovery: case={case}, version={version}, full and targeted scans registered"
        );
    }
}

#[test]
#[ignore = "requires the official Codex Desktop MSIX registered for a standard Windows user"]
fn actual_windows_codex_desktop_only_registers_both_agent_entries() {
    let root = tempfile::tempdir().unwrap();
    // The test daemon only sees System32 and a new, empty HOME/.local/bin.
    // MSIX registration still belongs to the actual user running the test.
    let case = root.path().join("desktop-only");
    let scope = ScanScope::new(&case, vec![]);
    let mut daemon = scope.start();
    let all = scope.scan(None);
    for agent in [CODEX, CODEX_DESKTOP] {
        let discovered = row(&all, agent);
        assert_registered(discovered, "codex_desktop");
        let targeted = scope.scan(discovered["context_id"].as_str());
        assert_eq!(targeted.len(), 1);
        assert_registered(row(&targeted, agent), "codex_desktop");
    }
    assert_ne!(
        row(&all, CODEX)["context_id"],
        row(&all, CODEX_DESKTOP)["context_id"]
    );
    let repeated = scope.scan(None);
    for agent in [CODEX, CODEX_DESKTOP] {
        assert_registered(row(&repeated, agent), "codex_desktop");
    }
    daemon.stop().unwrap();
    println!(
        "Windows Codex Desktop discovery: both entries and both targeted scans advertise only codex_desktop"
    );
}

fn assert_registered(agent: &Value, surface: &str) {
    assert_eq!(agent["supported"], true, "{}", agent["agent_id"]);
    assert_eq!(agent["configuration_state"], "registered");
    assert_eq!(agent["available_surfaces"], json!([surface]));
    assert!(
        agent["context_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
}

fn row<'a>(rows: &'a [Value], id: &str) -> &'a Value {
    rows.iter()
        .find(|entry| entry["agent_id"] == id)
        .expect("expected Agent row")
}

fn input(name: &str) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(name).expect("required validation input"));
    security::open_identity(&path, false).expect("validation requires a regular absolute file");
    path
}

struct ScanScope<'a> {
    root: &'a Path,
    env: Vec<(OsString, OsString)>,
}

impl<'a> ScanScope<'a> {
    fn new(root: &'a Path, search: Vec<&Path>) -> Self {
        let cmd = system_executable(Path::new("cmd.exe")).unwrap();
        let system = cmd.parent().unwrap();
        let search = std::env::join_paths(search.into_iter().chain([system])).unwrap();
        fs::create_dir(root).unwrap();
        for dir in ["home", "runtime", "storage", "diagnostics", "codex", "dsh"] {
            security::create_owner_only_directory(&root.join(dir)).unwrap();
        }
        let mut env: Vec<_> = [
            "SystemRoot",
            "WINDIR",
            "SystemDrive",
            "TEMP",
            "TMP",
            "USERPROFILE",
            "LOCALAPPDATA",
            "APPDATA",
        ]
        .into_iter()
        .filter_map(|key| std::env::var_os(key).map(|value| (key.into(), value)))
        .collect();
        env.extend([
            ("PATH".into(), search),
            ("HOME".into(), root.join("home").into_os_string()),
            ("CODEX_HOME".into(), root.join("codex").into_os_string()),
            ("DSH_HOME".into(), root.join("dsh").into_os_string()),
            (
                "HIROUTE_RUNTIME_DIR".into(),
                root.join("runtime").into_os_string(),
            ),
            ("PATHEXT".into(), OsString::from(".COM;.EXE;.BAT;.CMD")),
        ]);
        Self { root, env }
    }

    fn start(&self) -> NativeProbeProcess {
        let args = [
            "--role".into(),
            "control".into(),
            "--storage-root".into(),
            self.root.join("storage").into_os_string(),
            "--runtime-root".into(),
            self.root.join("runtime").into_os_string(),
            "--diagnostics-root".into(),
            self.root.join("diagnostics").into_os_string(),
        ];
        let child = Child::spawn(Launch {
            program: &input("HIROUTE_WORKER_DAEMON_BINARY"),
            args: &args,
            env: &self.env,
            cwd: Some(self.root),
            stdin: Stream::Null,
            stdout: Stream::Null,
            stderr: Stream::Null,
        })
        .unwrap();
        let endpoint =
            hiroute_daemon::ControlEndpoint::from_runtime_root(self.root.join("runtime"));
        let deadline = Instant::now() + Duration::from_secs(20);
        while hiroute_platform::windows::pipe::Stream::connect_timeout(
            endpoint.path(),
            Duration::from_millis(200),
        )
        .is_err()
        {
            assert_eq!(
                child.exit_code().unwrap(),
                None,
                "Control daemon exited before readiness"
            );
            assert!(
                Instant::now() < deadline,
                "Control daemon readiness deadline"
            );
            thread::sleep(Duration::from_millis(50));
        }
        NativeProbeProcess::new(child)
    }

    fn run(&self, program: &Path, args: &[OsString], input: Option<&[u8]>) -> (i32, Vec<u8>) {
        windows_user_models::run_native(
            program,
            args,
            &self.env,
            self.root,
            input,
            Duration::from_secs(30),
        )
    }

    fn scan(&self, context: Option<&str>) -> Vec<Value> {
        let mut args = ["agents", "scan", "--non-interactive", "--output", "json"]
            .map(OsString::from)
            .to_vec();
        let input = context.map(|context_id| {
            args.push("--request-stdin".into());
            serde_json::to_vec(&json!({"context_id": context_id})).unwrap()
        });
        let (exit, bytes) = self.run(
            &self::input("HIROUTE_WORKER_CLI_BINARY"),
            &args,
            input.as_deref(),
        );
        let result: Value = serde_json::from_slice(&bytes).expect("public CLI machine envelope");
        assert_eq!(exit, 0, "scan failed: {:?}", result["error"]["message_key"]);
        result["data"]["agents"]
            .as_array()
            .expect("scan result rows")
            .clone()
    }
}
