//! A held, fresh console exercises the real GUI image independently of installation.
use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use hiroute_platform::windows::{console, job};
use serde_json::Value;

const MODE: &str = "HIROUTE_VERSION_CONSOLE_FIXTURE";
const COMPLETE: &str = "HiRouteVersionConsole/v1 complete";

#[test]
fn actual_windows_daemon_version_uses_parent_console_and_preserves_pipes() {
    let host = std::env::current_exe().unwrap();
    let arguments = [
        "--ignored",
        "--exact",
        "windows_version_console::version_console_host",
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
    environment.push((MODE.into(), "1".into()));
    let mut child = job::Child::spawn_console_host(job::Launch {
        program: &host,
        args: &arguments,
        env: &environment,
        cwd: host.parent(),
        stdin: job::Stream::Null,
        stdout: job::Stream::Piped,
        stderr: job::Stream::Piped,
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(code) = child.exit_code().unwrap() {
            assert_eq!(
                code, 0,
                "the isolated version console must finish naturally"
            );
            if child.empty().unwrap() {
                break;
            }
        }
        assert!(
            Instant::now() < deadline,
            "version fixture exceeded its bound"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut stdout = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .take(64 * 1024 + 1)
        .read_to_end(&mut stdout)
        .unwrap();
    assert!(stdout.len() <= 64 * 1024);
    let mut stderr = Vec::new();
    child
        .stderr
        .take()
        .unwrap()
        .take(64 * 1024 + 1)
        .read_to_end(&mut stderr)
        .unwrap();
    assert!(stderr.is_empty());
    assert_eq!(
        String::from_utf8(stdout)
            .unwrap()
            .lines()
            .filter(|line| *line == COMPLETE)
            .count(),
        1
    );
}

fn assert_gui_image(path: &Path) {
    let mut file = std::fs::File::open(path).unwrap();
    let mut dos = [0; 64];
    file.read_exact(&mut dos).unwrap();
    assert_eq!(&dos[..2], b"MZ");
    let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
    assert!(offset <= 1024 * 1024 && offset + 94 <= file.metadata().unwrap().len());
    file.seek(SeekFrom::Start(offset)).unwrap();
    let mut header = [0; 94];
    file.read_exact(&mut header).unwrap();
    assert_eq!(&header[..4], b"PE\0\0");
    assert_eq!(u16::from_le_bytes(header[92..94].try_into().unwrap()), 2);
}

#[test]
#[ignore = "child fixture requiring its own held console"]
fn version_console_host() {
    assert_eq!(std::env::var(MODE).unwrap(), "1");
    assert!(console::is_attached().unwrap());
    let daemon = super::daemon_binary();
    assert_gui_image(&daemon);
    for flag in ["--version", "-V"] {
        let visible = super::windows_version_output::without_inherited_stdio(&daemon, flag);
        assert_eq!(visible["schema"], "hiroute.build-identity/v1");
        assert_eq!(visible["component"], "daemon");
        assert_eq!(visible["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(
            visible["source_revision"],
            serde_json::to_value(option_env!("HIROUTE_BUILD_SOURCE_SHA")).unwrap()
        );
        let redirected = Command::new(&daemon).arg(flag).output().unwrap();
        assert!(redirected.status.success());
        assert!(redirected.stderr.is_empty());
        assert_eq!(
            serde_json::from_slice::<Value>(&redirected.stdout).unwrap(),
            visible
        );
    }
    println!("\n{COMPLETE}");
    std::io::stdout().flush().unwrap();
}
