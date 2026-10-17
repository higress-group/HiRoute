//! A self-reexecuted native owner process drives actual shipped CLI/Python entries.
//! The fixture accepts no credentials, SIDs, or destructive numeric PID locators.
use std::ffi::OsString;
use std::fs::File;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use hiroute_host_runtime::StandaloneLayout;
use hiroute_platform::windows::{
    background,
    job::{Child, Launch, Stream},
    lifecycle,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::windows_account as account;
use super::windows_execution_host as execution_host;
use super::windows_token_launcher as launcher;

#[path = "windows_background_state.rs"]
mod state;
#[path = "windows_background_transactions.rs"]
mod transactions;

pub const PROBE: &str = "HIROUTE_WINDOWS_BACKGROUND_FIXTURE";
pub const ENTRY: &str = "windows_background_owner_fixture";
const OUTPUT: &str = "HIROUTE_NATIVE_BACKGROUND_RESULT=";
const BOUND: usize = 256 * 1024;

enum Process {
    Product(Child),
    Owner(launcher::Child),
}
impl Process {
    fn pipes(&mut self) -> (Option<File>, File, File) {
        match self {
            Self::Product(child) => (
                child.stdin.take(),
                child.stdout.take().unwrap(),
                child.stderr.take().unwrap(),
            ),
            Self::Owner(child) => (
                child.stdin.take(),
                child.stdout.take().unwrap(),
                child.stderr.take().unwrap(),
            ),
        }
    }
    fn exit_code(&self) -> std::io::Result<Option<i32>> {
        match self {
            Self::Product(child) => child.exit_code(),
            Self::Owner(child) => child.exit_code(),
        }
    }
    fn empty(&self) -> std::io::Result<bool> {
        match self {
            Self::Product(child) => child.empty(),
            Self::Owner(child) => child.empty(),
        }
    }
    fn terminate(&self) -> std::io::Result<()> {
        match self {
            Self::Product(child) => child.terminate(),
            Self::Owner(child) => child.terminate(),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub action: String,
    pub program: Option<PathBuf>,
    pub arguments: Vec<String>,
}

pub fn required(name: &str) -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os(name)
            .unwrap_or_else(|| panic!("required native product gate input missing: {name}")),
    );
    assert!(
        path.is_absolute() && path.is_file(),
        "native gate input must be an existing absolute file: {name}"
    );
    path
}

pub fn prepared_cli() -> PathBuf {
    let bin = required("HIROUTE_WINDOWS_ADMIN_CLI");
    assert_eq!(bin.file_name().unwrap(), "hiroute.exe");
    bin
}

/// Only the Lua fixture demands empty initial state. Upgrade and the pure
/// standard-owner journey retain their existing user-state contracts.
pub fn require_fresh_lua_state(layout: &Value) {
    let path = Path::new(
        layout["state"]
            .as_str()
            .expect("actual owner state path is missing"),
    );
    state::require_absent(path).expect("Lua product gate refuses existing owner state");
}

/// All subprocess waits and capture readers are bounded. Secret stdin callers
/// deliberately omit arbitrary process output from failed-command diagnostics.
pub fn process(
    token: Option<&File>,
    program: &Path,
    args: &[OsString],
    input: Option<&[u8]>,
    secret: bool,
    timeout: Duration,
    fixture: bool,
) -> Value {
    assert!(program.is_absolute());
    if token.is_none() {
        assert!(program.is_file());
    }
    let mut environment = token
        .map(account::environment)
        .unwrap_or_else(|| std::env::vars_os().collect());
    environment.retain(|(key, _)| key != PROBE);
    if let Some(mode) = execution_host::mode() {
        environment.retain(|(key, _)| key != execution_host::VARIABLE);
        environment.push((execution_host::VARIABLE.into(), mode.into()));
    }
    if fixture {
        environment.push((PROBE.into(), "1".into()));
    }
    let mut child = match token {
        Some(token)
            if account::token_sid(token)
                != account::token_sid(&account::process_token(std::process::id())) =>
        {
            Process::Owner(
                launcher::Child::spawn(token, program, args, &environment)
                    .expect("native alternate owner creation failed"),
            )
        }
        _ => {
            let launch = Launch {
                program,
                args,
                env: &environment,
                cwd: program.parent(),
                stdin: if input.is_some() {
                    Stream::Piped
                } else {
                    Stream::Null
                },
                stdout: Stream::Piped,
                stderr: Stream::Piped,
            };
            Process::Product(
                match token {
                    Some(token) => Child::spawn_as_token(launch, token),
                    None => Child::spawn(launch),
                }
                .expect("controlled native test process creation failed"),
            )
        }
    };
    let (stdin, stdout_file, stderr_file) = child.pipes();
    let mut stdout = launcher::Capture::new(stdout_file);
    let mut stderr = launcher::Capture::new(stderr_file);
    // An owner command relays a real public JSON response, including log tails
    // that grow during repeated installed-runtime checks. Give that
    // envelope the existing Control response bound too. Pure fixture metadata
    // and arbitrary stderr keep their smaller test bound.
    let public_command = fixture
        && input.is_some_and(|bytes| {
            serde_json::from_slice::<Request>(bytes)
                .is_ok_and(|request| request.action == "command")
        });
    let stdout_bound = if !fixture || public_command {
        hiroute_application_api::LOCAL_CONTROL_MAX_FRAME_BYTES
    } else {
        BOUND
    };
    if let Some(input) = input {
        stdin
            .unwrap()
            .write_all(input)
            .expect("controlled stdin transfer failed");
    } else {
        drop(stdin);
    }
    let deadline = Instant::now() + timeout;
    let code = loop {
        stdout
            .poll(stdout_bound)
            .expect("bounded native stdout capture failed");
        stderr
            .poll(BOUND)
            .expect("bounded native stderr capture failed");
        if let Some(code) = child.exit_code().unwrap() {
            break code;
        }
        if Instant::now() >= deadline {
            let stage = stderr.fixture_stage();
            let output_bytes = stdout.bytes().len();
            let error_bytes = stderr.bytes().len();
            child
                .terminate()
                .expect("native timeout Job termination failed");
            let cleanup_deadline = Instant::now() + Duration::from_secs(5);
            while (child.exit_code().unwrap().is_none() || !child.empty().unwrap())
                && Instant::now() < cleanup_deadline
            {
                thread::sleep(Duration::from_millis(10));
            }
            panic!(
                "native command exceeded its bounded timeout: fixture_stage={stage}, stdout_bytes={output_bytes}, stderr_bytes={error_bytes}, held_root_exited={}, held_job_empty={}",
                child.exit_code().unwrap().is_some(),
                child.empty().unwrap(),
            );
        }
        thread::sleep(Duration::from_millis(20));
    };
    let empty_deadline = Instant::now() + Duration::from_secs(2);
    while !child.empty().unwrap() && Instant::now() < empty_deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let leaked = !child.empty().unwrap();
    if leaked {
        child.terminate().unwrap();
    }
    assert!(
        child.empty().unwrap(),
        "transient native command leaked descendants"
    );
    assert!(
        !leaked,
        "native command exit left a live descendant beyond its cleanup bound"
    );
    let capture_deadline = Instant::now() + Duration::from_secs(2);
    while (!stdout.eof() || !stderr.eof()) && Instant::now() < capture_deadline {
        stdout
            .poll(stdout_bound)
            .expect("bounded native stdout capture failed");
        stderr
            .poll(BOUND)
            .expect("bounded native stderr capture failed");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        stdout.eof() && stderr.eof(),
        "native stdio did not close after held root/Job exit: stdout_eof={}, stderr_eof={}, fixture_stage={}",
        stdout.eof(),
        stderr.eof(),
        stderr.fixture_stage(),
    );
    let output = stdout.into_bytes();
    let error = stderr.into_bytes();
    if secret && code != 0 {
        return json!({"code":code,"result":null});
    }
    if secret {
        // The credential operation returns a closed no-secret response. Never
        // reflect unknown stdout/stderr when decoding fails.
        assert!(
            error.is_empty(),
            "administrator operation emitted unexpected diagnostic output"
        );
    }
    let parsed = if fixture {
        String::from_utf8_lossy(&output)
            .lines()
            .find_map(|line| line.strip_prefix(OUTPUT))
            .map(|body| {
                serde_json::from_str::<Value>(body).expect("owner fixture result is invalid")
            })
    } else {
        serde_json::from_slice(&output).ok()
    };
    if secret {
        let value = parsed.expect("administrator result is invalid");
        assert_eq!(value["schema"], "hiroute.windows-scm-owner-result/v1");
        assert_eq!(value["completed"], true);
        return json!({"code":code,"result":value});
    }
    json!({"code":code,"result":parsed,"stdout":String::from_utf8_lossy(&output),"stderr":String::from_utf8_lossy(&error)})
}

pub fn owner(token: &File, request: Request, timeout: Duration) -> Value {
    let binary = account::profile_for_token(token)
        .join(format!(
            ".hiroute-native-background-test-{}",
            std::process::id()
        ))
        .join("windows-background-fixture.exe");
    assert!(binary.is_absolute());
    let arguments = [
        OsString::from("--exact"),
        ENTRY.into(),
        "--nocapture".into(),
        "--test-threads=1".into(),
    ];
    let input = serde_json::to_vec(&request).unwrap();
    let result = process(
        Some(token),
        &binary,
        &arguments,
        Some(&input),
        false,
        timeout,
        true,
    );
    assert_eq!(result["code"], 0, "actual owner fixture failed: {result}");
    result["result"].clone()
}

pub fn command(token: &File, program: &Path, args: Vec<String>) -> Value {
    owner(
        token,
        Request {
            action: "command".into(),
            program: Some(program.to_owned()),
            arguments: args,
        },
        Duration::from_secs(90),
    )
}

pub fn accepted(result: &Value) -> &Value {
    assert_eq!(result["code"], 0, "production command failed: {result}");
    assert!(
        result["result"].is_object(),
        "production command returned no JSON result: {result}"
    );
    &result["result"]
}

pub fn cli(token: &File, path: &Path, arguments: &[&str]) -> Value {
    let args = arguments
        .iter()
        .map(|v| (*v).to_owned())
        .chain([
            "--output".into(),
            "json".into(),
            "--timeout".into(),
            "45s".into(),
        ])
        .collect();
    let result = command(token, path, args);
    let envelope = accepted(&result);
    assert_eq!(
        envelope["status"], "succeeded",
        "production CLI request failed: {envelope}"
    );
    envelope["data"].clone()
}

fn gateway_ready(listen: SocketAddr) {
    let mut client = TcpStream::connect_timeout(&listen, Duration::from_secs(5)).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    client
        .write_all(b"GET /_hiroute/ready HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut bytes = Vec::new();
    client.take(65537).read_to_end(&mut bytes).unwrap();
    assert!(bytes.len() <= 65536 && bytes.starts_with(b"HTTP/1.1 200 "));
    let start = bytes.windows(4).position(|v| v == b"\r\n\r\n").unwrap();
    let body: Value = serde_json::from_slice(&bytes[start + 4..]).unwrap();
    assert_eq!(body["schema_version"], "hiroute.gateway.ready/v2");
    assert_eq!(body["status"], "ready");
}

/// Only used in a real token-created owner process. The ordinary test selection
/// validates the closed request shape; it cannot be cited as a product gate.
pub fn entry() {
    if std::env::var_os(PROBE).is_none() {
        assert!(
            serde_json::from_value::<Request>(
                json!({"action":"inspect","program":null,"arguments":[],"sid":"S-1-5-18"})
            )
            .is_err()
        );
        return;
    }
    eprintln!("HIROUTE_NATIVE_BACKGROUND_STAGE=entry");
    launcher::assert_fixture_stdio();
    eprintln!("HIROUTE_NATIVE_BACKGROUND_STAGE=stdio-verified");
    let mut bytes = Vec::new();
    eprintln!("HIROUTE_NATIVE_BACKGROUND_STAGE=request-reading");
    std::io::stdin()
        .lock()
        .take(16385)
        .read_to_end(&mut bytes)
        .unwrap();
    eprintln!("HIROUTE_NATIVE_BACKGROUND_STAGE=request-eof");
    assert!(!bytes.is_empty() && bytes.len() <= 16384);
    let request: Request = serde_json::from_slice(&bytes).unwrap();
    eprintln!("HIROUTE_NATIVE_BACKGROUND_STAGE=request-parsed");
    execution_host::current();
    let current = account::process_token(std::process::id());
    let sid = background::current_sid_string().unwrap();
    account::assert_standard(&current, &sid);
    eprintln!("HIROUTE_NATIVE_BACKGROUND_STAGE=owner-verified");
    let result = match request.action.as_str() {
        "upgrade-business-save" | "upgrade-business-verify" | "upgrade-business-cleanup" => {
            super::windows_background_upgrade_business::run_action(&request)
        }
        "transaction-snapshot"
        | "transaction-verify"
        | "transaction-fail-install"
        | "transaction-upgrade-install" => transactions::run(&request),
        "install" | "uninstall" => transactions::maintenance(&request),
        "command" => {
            let program = request.program.unwrap();
            let args = request
                .arguments
                .iter()
                .map(OsString::from)
                .collect::<Vec<_>>();
            process(
                None,
                &program,
                &args,
                None,
                false,
                Duration::from_secs(75),
                false,
            )
        }
        "layout" => {
            assert!(request.program.is_none() && request.arguments.is_empty());
            let layout = StandaloneLayout::from_environment().unwrap();
            assert!(
                !layout.marker_path.exists(),
                "product gate requires a fresh owner profile"
            );
            json!({"home":layout.home,"state":layout.state_root,"marker":layout.marker_path,"sid":sid})
        }
        "inspect" | "crash" | "forced-stop" => {
            assert!(request.program.is_none() && request.arguments.is_empty());
            let layout = StandaloneLayout::from_environment().unwrap();
            let record =
                hiroute_host_runtime::read_standalone_install_record(&layout.marker_path).unwrap();
            let daemon = hiroute_host_runtime::standalone_daemon_binary(&record.install_root);
            let held = lifecycle::retain_for_stop(
                &layout.lifecycle_endpoint(),
                &daemon,
                Duration::from_secs(5),
            )
            .unwrap();
            let token = account::process_token(held.pid);
            let session = account::assert_standard(&token, &sid);
            let execution = execution_host::daemon(&held);
            let program = hiroute_host_runtime::standalone_cli_binary(&record.install_root);
            let invoke = |args: &[&str]| {
                let args = args
                    .iter()
                    .map(|v| OsString::from(*v))
                    .chain([
                        "--output".into(),
                        "json".into(),
                        "--timeout".into(),
                        "15s".into(),
                    ])
                    .collect::<Vec<_>>();
                let response = process(
                    None,
                    &program,
                    &args,
                    None,
                    false,
                    Duration::from_secs(20),
                    false,
                );
                let envelope = accepted(&response);
                assert_eq!(envelope["status"], "succeeded");
                envelope["data"].clone()
            };
            let control = invoke(&["system", "status"]);
            let gateway = invoke(&["gateway", "show"]);
            assert_eq!(gateway["ready"], true);
            gateway_ready(
                gateway["connect_address"]
                    .as_str()
                    .unwrap()
                    .parse()
                    .unwrap(),
            );
            let result = json!({"pid":held.pid,"creation_time":held.creation_time,"sid":sid,"session":session,"gateway":gateway,"control":control,"execution_host":execution});
            if request.action == "crash" {
                held.terminate().unwrap();
                assert!(
                    held.wait(Duration::from_secs(10)).unwrap(),
                    "actual daemon held handle did not exit after forced crash"
                );
            } else if request.action == "forced-stop" {
                use std::os::windows::io::{AsRawHandle, FromRawHandle};
                use windows_sys::Win32::Foundation::FILETIME;
                use windows_sys::Win32::System::Threading::{
                    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
                };
                // Retain a read-only witness while the lifecycle-authenticated
                // daemon is alive; PID reuse cannot replace this native object.
                let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, held.pid) };
                assert!(!raw.is_null());
                let witness = unsafe { File::from_raw_handle(raw.cast()) };
                let times = || {
                    let (mut creation, mut exit, mut kernel, mut user) = (
                        FILETIME::default(),
                        FILETIME::default(),
                        FILETIME::default(),
                        FILETIME::default(),
                    );
                    assert_ne!(
                        unsafe {
                            GetProcessTimes(
                                witness.as_raw_handle().cast(),
                                &mut creation,
                                &mut exit,
                                &mut kernel,
                                &mut user,
                            )
                        },
                        0
                    );
                    let ticks = |value: FILETIME| {
                        (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
                    };
                    (ticks(creation), ticks(exit))
                };
                assert_eq!(times().0, held.creation_time);
                // Stall the production protected-input reader, which shares the
                // lifecycle poll loop and has a real 2 second I/O deadline.
                // The public 1 second stop must take its actual Task fallback.
                let mut stalled = hiroute_platform::windows::pipe::Stream::connect_timeout(
                    &layout.protected_input_socket(),
                    Duration::from_secs(5),
                )
                .unwrap();
                stalled
                    .set_deadline(Some(Instant::now() + Duration::from_secs(5)))
                    .unwrap();
                let start = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap();
                let latest_forced_exit =
                    (start.as_nanos() / 100) as u64 + 116_444_736_000_000_000 + 20_000_000;
                stalled.write_all(b"HRO").unwrap();
                thread::sleep(Duration::from_millis(150));
                let response = process(
                    None,
                    &program,
                    &[
                        "service".into(),
                        "stop".into(),
                        "--output".into(),
                        "json".into(),
                        "--timeout".into(),
                        "1s".into(),
                    ],
                    None,
                    false,
                    Duration::from_secs(10),
                    false,
                );
                assert!(
                    response["code"] == 0 || response["code"] == 6,
                    "forced stop returned an unexpected public exit: {response}"
                );
                if response["code"] == 0 {
                    assert_eq!(accepted(&response)["status"], "succeeded");
                }
                assert!(
                    held.wait(Duration::from_secs(5)).unwrap(),
                    "forced user stop must terminate the retained actual daemon"
                );
                assert!(
                    times().1 > 0 && times().1 < latest_forced_exit,
                    "native exit timestamp must precede the 2s server stall deadline, proving forced rather than delayed graceful shutdown"
                );
            }
            result
        }
        "cleanup-state" => {
            assert!(request.program.is_none() && request.arguments.is_empty());
            let layout = StandaloneLayout::from_environment().unwrap();
            let disposition = state::after_uninstall(&layout.marker_path)
                .expect("post-uninstall fixture preservation admission failed");
            json!({"removed":disposition.removed,"preserved":disposition.preserved})
        }
        "tamper-daemon" | "restore-daemon" => {
            assert!(request.program.is_none() && request.arguments.is_empty());
            use std::io::{Seek, SeekFrom};
            let layout = StandaloneLayout::from_environment().unwrap();
            assert!(
                hiroute_host_runtime::standalone_maintenance_pending(&layout).unwrap(),
                "tamper fixture requires refused business admission"
            );
            let record =
                hiroute_host_runtime::read_standalone_install_record(&layout.marker_path).unwrap();
            let daemon = hiroute_host_runtime::standalone_daemon_binary(&record.install_root);
            let backup = record.install_root.join("native-test-daemon-first-byte");
            let mut image = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&daemon)
                .unwrap();
            let byte = if request.action == "tamper-daemon" {
                assert!(!backup.exists());
                let mut old = [0];
                image.read_exact(&mut old).unwrap();
                assert_eq!(
                    old[0], b'M',
                    "actual package daemon must be a native PE image"
                );
                let mut saved =
                    hiroute_platform::windows::security::create_owner_only_file(&backup).unwrap();
                saved.write_all(&old).unwrap();
                saved.sync_all().unwrap();
                old[0] ^ 0x20
            } else {
                let mut saved =
                    hiroute_platform::windows::security::open_owner_only(&backup, false).unwrap();
                let mut old = [0];
                saved.read_exact(&mut old).unwrap();
                drop(saved);
                std::fs::remove_file(&backup).unwrap();
                old[0]
            };
            image.seek(SeekFrom::Start(0)).unwrap();
            image.write_all(&[byte]).unwrap();
            image.sync_all().unwrap();
            drop(image);
            let definition = background::Definition::new(background::Manager::Scm, daemon).unwrap();
            json!({"modified":true,"definition":definition})
        }
        _ => panic!("unknown owner fixture action"),
    };
    eprintln!("HIROUTE_NATIVE_BACKGROUND_STAGE=action-completed");
    // Pretty libtest has already written `test <name> ... ` without a newline.
    // Start the closed frame on its own line so the parent can require an exact
    // prefix without searching arbitrary child output for an embedded marker.
    println!("\n{OUTPUT}{}", serde_json::to_string(&result).unwrap());
}

pub fn inspect(token: &File, crash: bool) -> Value {
    owner(
        token,
        Request {
            action: if crash { "crash" } else { "inspect" }.into(),
            program: None,
            arguments: vec![],
        },
        Duration::from_secs(60),
    )
}

pub fn forced_stop(token: &File) -> Value {
    owner(
        token,
        Request {
            action: "forced-stop".into(),
            program: None,
            arguments: vec![],
        },
        Duration::from_secs(60),
    )
}

pub fn transaction_snapshot(token: &File, verify: bool) -> Value {
    owner(
        token,
        Request {
            action: if verify {
                "transaction-verify"
            } else {
                "transaction-snapshot"
            }
            .into(),
            program: None,
            arguments: vec![],
        },
        Duration::from_secs(45),
    )
}
