//! A fresh test console contains the actual public CLI and GUI daemon only.
//! Signals never target the Cargo/runner console; production uses inherited I/O.
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::thread;
use std::time::{Duration, Instant};

use hiroute_platform::windows::{console, job, lifecycle, security};
use serde_json::Value;
use windows_sys::Win32::System::Console::{
    CTRL_BREAK_EVENT, CTRL_C_EVENT, GenerateConsoleCtrlEvent, GetConsoleProcessList,
    SetConsoleCtrlHandler,
};

use super::{IO_BOUND, InstalledProduct, assert_no_secret};

const MODE: &str = "HIROUTE_NATIVE_FOREGROUND_SIGNAL";
const ENTRY: &str = "windows_foreground_console::foreground_console_host";
const READY: &str = "HiRouteForegroundConsole/v1 stage=ready";
const DONE: &str = "HiRouteForegroundConsole/v1 stage=natural-complete";
const PHASES: [&str; 8] = [
    "host-entered",
    "images-verified",
    "cli-started",
    "daemon-ready",
    "console-members-verified",
    "signal-sent",
    "cli-stopped",
    "control-gateway-released",
];

fn checkpoint(phase: usize) {
    println!("HiRouteForegroundConsole/v1 stage={}", PHASES[phase]);
    std::io::stdout().flush().unwrap();
}

fn subsystem(path: &Path) -> u16 {
    let mut file = security::open_identity(path, false).unwrap();
    let mut dos = [0; 64];
    file.read_exact(&mut dos).unwrap();
    assert_eq!(&dos[..2], b"MZ");
    let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
    assert!(offset <= 1024 * 1024 && offset + 94 <= file.metadata().unwrap().len());
    file.seek(SeekFrom::Start(offset)).unwrap();
    let mut header = [0; 94];
    file.read_exact(&mut header).unwrap();
    assert_eq!(&header[..4], b"PE\0\0");
    assert!(u16::from_le_bytes(header[20..22].try_into().unwrap()) >= 70);
    assert_eq!(u16::from_le_bytes(header[4..6].try_into().unwrap()), 0x8664);
    assert_eq!(
        u16::from_le_bytes(header[24..26].try_into().unwrap()),
        0x20b
    );
    u16::from_le_bytes(header[92..94].try_into().unwrap())
}

fn natural_case(event: &str) {
    let product = InstalledProduct::create();
    assert_eq!(
        subsystem(&product.daemon),
        2,
        "the actual daemon must be a GUI image"
    );
    assert_eq!(
        subsystem(&product.stable),
        3,
        "the actual public CLI keeps its console image"
    );
    let host = std::env::current_exe().unwrap();
    let arguments = [
        "--ignored",
        "--exact",
        ENTRY,
        "--nocapture",
        "--test-threads=1",
    ]
    .map(OsString::from);
    let mut environment: Vec<_> = std::env::vars_os()
        .filter(|(name, _)| {
            !name
                .as_encoded_bytes()
                .eq_ignore_ascii_case(MODE.as_bytes())
        })
        .collect();
    environment.push((MODE.into(), event.into()));
    let mut child = job::Child::spawn_console_host(job::Launch {
        program: &host,
        args: &arguments,
        env: &environment,
        cwd: host.parent(),
        stdin: job::Stream::Piped,
        stdout: job::Stream::Piped,
        stderr: job::Stream::Piped,
    })
    .unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    let phase = Arc::new(AtomicUsize::new(0));
    let observed_phase = phase.clone();
    let output = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut bytes = Vec::new();
        loop {
            let mut line = Vec::new();
            if reader.read_until(b'\n', &mut line).unwrap() == 0 {
                break;
            }
            assert!(bytes.len() + line.len() <= 256 * 1024);
            if let Ok(text) = std::str::from_utf8(&line) {
                for (index, phase) in PHASES.iter().enumerate() {
                    if text.trim_end_matches(['\r', '\n'])
                        == format!("HiRouteForegroundConsole/v1 stage={phase}")
                    {
                        observed_phase.store(index + 1, Ordering::Release);
                    }
                }
            }
            if std::str::from_utf8(&line)
                .ok()
                .is_some_and(|line| line.trim_end_matches(['\r', '\n']) == READY)
            {
                sender.try_send(()).unwrap();
            }
            bytes.extend(line);
        }
        bytes
    });
    let error = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.take(256 * 1024 + 1).read_to_end(&mut bytes).unwrap();
        assert!(bytes.len() <= 256 * 1024);
        bytes
    });
    let ready = receiver.recv_timeout(Duration::from_secs(45));
    if ready.is_err() {
        eprintln!(
            "actual foreground console facts: event={event} ready=false last_phase={} root_exit={:?}",
            phase.load(Ordering::Acquire),
            child.exit_code().ok().flatten()
        );
    }
    ready.expect("actual foreground readiness through preserved inherited pipes");
    let status = product.cli(&["system", "status"]);
    assert_eq!(status["status"], "succeeded");
    child.stdin.take().unwrap().write_all(b"signal\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        if let Some(code) = child.exit_code().unwrap() {
            if code != 0 {
                eprintln!(
                    "actual foreground console facts: event={event} ready=true last_phase={} root_exit={code}",
                    phase.load(Ordering::Acquire)
                );
            }
            assert_eq!(code, 0, "foreground console case must stop naturally");
            if child.empty().unwrap() {
                break;
            }
        }
        if Instant::now() >= deadline {
            eprintln!(
                "actual foreground console facts: event={event} ready=true last_phase={} root_exit={:?} job_empty={:?}",
                phase.load(Ordering::Acquire),
                child.exit_code().ok().flatten(),
                child.empty().ok()
            );
            panic!("the exact held foreground Job must stop naturally");
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = output.join().unwrap();
    let error = error.join().unwrap();
    assert_no_secret(&output);
    assert_no_secret(&error);
    assert!(
        error.is_empty(),
        "isolated foreground fixture cannot hide failure output"
    );
    let output = std::str::from_utf8(&output).unwrap();
    assert_eq!(output.lines().filter(|line| *line == READY).count(), 1);
    assert_eq!(output.lines().filter(|line| *line == DONE).count(), 1);
    assert!(
        lifecycle::query(
            &product.layout.lifecycle_endpoint(),
            &product.daemon,
            IO_BOUND
        )
        .is_err()
    );
}

#[test]
fn actual_windows_service_run_ctrl_c_drains_the_gui_daemon_and_preserves_pipes() {
    natural_case("ctrl-c");
}

#[test]
fn actual_windows_service_run_ctrl_break_drains_the_gui_daemon_and_preserves_pipes() {
    natural_case("ctrl-break");
}

#[test]
#[ignore = "a child fixture used only inside a fresh held native test console"]
fn foreground_console_host() {
    let event = match std::env::var(MODE).unwrap().as_str() {
        "ctrl-c" => CTRL_C_EVENT,
        "ctrl-break" => CTRL_BREAK_EVENT,
        _ => panic!("closed foreground signal fixture mode required"),
    };
    assert!(
        console::is_attached().unwrap(),
        "the fixture must own its new console"
    );
    checkpoint(0);
    let wait = console::ForegroundWaitGuard::install().unwrap();
    let layout = hiroute_host_runtime::StandaloneLayout::from_environment().unwrap();
    let record = hiroute_host_runtime::read_standalone_install_record(&layout.marker_path).unwrap();
    let cli = hiroute_host_runtime::standalone_cli_binary(&record.install_root);
    let daemon = hiroute_host_runtime::standalone_daemon_binary(&record.install_root);
    assert_eq!(subsystem(&cli), 3);
    assert_eq!(subsystem(&daemon), 2);
    checkpoint(1);
    // Shells and process supervisors may pass this inheritable attribute to a
    // console child. The actual GUI daemon must restore foreground Ctrl-C after
    // attaching; this fixture's own handler still protects Ctrl-Break broadcasts.
    assert_ne!(unsafe { SetConsoleCtrlHandler(None, 1) }, 0);
    // Ordinary CreateProcess inherits this host's already assigned non-breakaway
    // Job and existing console before execution. Do not create a second console.
    let mut child = Command::new(&cli)
        .args(["service", "run"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    checkpoint(2);
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    let output = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut bytes = Vec::new();
        loop {
            let mut line = Vec::new();
            if reader.read_until(b'\n', &mut line).unwrap() == 0 {
                break;
            }
            assert!(bytes.len() + line.len() <= 256 * 1024);
            if let Ok(value) = serde_json::from_slice::<Value>(&line)
                && value["schema"] == "hiroute.daemon-ready/v1"
            {
                sender.try_send(value).unwrap();
            }
            bytes.extend(line);
        }
        bytes
    });
    let error = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.take(256 * 1024 + 1).read_to_end(&mut bytes).unwrap();
        assert!(bytes.len() <= 256 * 1024);
        bytes
    });
    let ready = receiver.recv_timeout(Duration::from_secs(45)).unwrap();
    checkpoint(3);
    let held = lifecycle::query(&layout.lifecycle_endpoint(), &daemon, IO_BOUND).unwrap();
    assert_eq!(ready["process_id"].as_u64(), Some(u64::from(held.pid)));
    assert!(!held.exited().unwrap());
    let gateway: SocketAddr = ready["gateway_listen"].as_str().unwrap().parse().unwrap();
    assert!(TcpStream::connect_timeout(&gateway, IO_BOUND).is_ok());
    let mut members = [0u32; 8];
    // SAFETY: only observe the fixture's current console. It was newly created
    // for this held Job, and the expected processes are independently retained.
    let count =
        unsafe { GetConsoleProcessList(members.as_mut_ptr(), members.len() as u32) } as usize;
    assert!(count <= members.len() && count == 3);
    members[..count].sort_unstable();
    let mut expected = [std::process::id(), child.id(), held.pid];
    expected.sort_unstable();
    assert_eq!(
        members[..count],
        expected,
        "the actual GUI daemon attached only to this isolated host console"
    );
    checkpoint(4);
    println!("{READY}");
    std::io::stdout().flush().unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        std::io::stdin().take(65).read_to_end(&mut bytes).unwrap();
        sender.send(bytes).unwrap();
    });
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(45)).unwrap(),
        b"signal\n"
    );
    // SAFETY: the caller has just verified all members of its fresh console.
    // Group zero broadcasts only within that console, never the external runner.
    assert_ne!(unsafe { GenerateConsoleCtrlEvent(event, 0) }, 0);
    checkpoint(5);
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the public CLI must wait for natural daemon drainage"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(held.wait(IO_BOUND).unwrap());
    checkpoint(6);
    assert!(lifecycle::query(&layout.lifecycle_endpoint(), &daemon, IO_BOUND).is_err());
    assert!(
        TcpListener::bind(gateway).is_ok(),
        "actual Gateway port released after console shutdown"
    );
    assert!(
        hiroute_platform::windows::pipe::Stream::connect(
            &layout.runtime_root.join("hiroute/control.sock")
        )
        .is_err(),
        "actual Local Control pipe released after console shutdown"
    );
    checkpoint(7);
    assert_no_secret(&output.join().unwrap());
    assert_no_secret(&error.join().unwrap());
    wait.finish().unwrap();
    println!("{DONE}");
    std::io::stdout().flush().unwrap();
}
