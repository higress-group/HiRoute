//! Exercise the actual probe launch without changing the test runner's environment.
use super::*;
use std::io::Read;
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Stdio;

const TEST: &str = "agents::native_claude_ingress_probe::environment_tests::claude_probe_owns_native_temporary_roots";
const ROLE: &str = "HIROUTE_CLAUDE_PROBE_TEST_ROLE";
const CASE: &str = "HIROUTE_CLAUDE_PROBE_TEST_CASE";
const REPORT: &str = "HIROUTE_CLAUDE_PROBE_TEST_REPORT";
const AMBIENT: &str = "HIROUTE_CLAUDE_PROBE_AMBIENT_SENTINEL";

#[test]
fn claude_probe_owns_native_temporary_roots() {
    match std::env::var(ROLE).ok().as_deref() {
        Some("probe") => exercise_probe(),
        Some("native") => native_fixture(),
        None => isolated_test_runner(),
        _ => panic!("unexpected fixture role"),
    }
}

fn isolated_test_runner() {
    let root = tempfile::tempdir().unwrap();
    let runner = std::env::current_exe().unwrap();
    for case in ["complete", "no_message", "wrong_auth", "failed_exit"] {
        let report = root.path().join(format!("{case}.json"));
        let binary = root.path().join(format!("claude-{case}"));
        fs::write(
            &binary,
            format!(
                "#!/bin/sh\n{ROLE}=native {CASE}={} {REPORT}={} exec {} --exact {} --nocapture --test-threads=1\n",
                shell_quote(case),
                shell_quote(report.to_str().unwrap()),
                shell_quote(runner.to_str().unwrap()),
                shell_quote(TEST),
            ),
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let outside = root.path().join("ambient-roots");
        fs::create_dir_all(&outside).unwrap();
        let mut command = Command::new(&runner);
        command
            .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
            .env(ROLE, "probe")
            .env(CASE, case)
            .env(REPORT, &report)
            .env("HIROUTE_CLAUDE_PROBE_TEST_BINARY", &binary)
            .env(AMBIENT, "must-not-reach-native-child")
            .stdin(Stdio::null());
        for key in ["HOME", "CLAUDE_CONFIG_DIR", "TMPDIR", "CLAUDE_CODE_TMPDIR"] {
            command.env(key, &outside);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{case}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    }
}

fn exercise_probe() {
    let binary = std::env::var_os("HIROUTE_CLAUDE_PROBE_TEST_BINARY").unwrap();
    let case = std::env::var(CASE).unwrap();
    let result = ClaudeNativeIngressProbe::run(Path::new(&binary));
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(std::env::var_os(REPORT).unwrap()).unwrap()).unwrap();
    let root = Path::new(report["root"].as_str().unwrap());
    assert!(
        !root.exists(),
        "probe roots must be removed after owned stop"
    );
    assert_eq!(
        report["roots_owned"], true,
        "all native roots must be probe-owned"
    );
    assert_eq!(report["ambient_absent"], true);
    assert_eq!(report["stdin_closed"], true);
    match case.as_str() {
        "complete" => {
            result.unwrap_or_else(|error| panic!("{error}"));
        }
        "wrong_auth" => assert_eq!(
            result.err().unwrap().to_string(),
            "native ingress probe could not establish wrong authentication"
        ),
        _ => assert_eq!(
            result.err().unwrap().to_string(),
            "native ingress probe could not establish authenticated native completion"
        ),
    }
}

fn native_fixture() {
    let workspace = fs::canonicalize(".").unwrap();
    let root = workspace.parent().unwrap();
    let home = root.join("home");
    let roots_owned = [
        ("HOME", home.clone()),
        ("CLAUDE_CONFIG_DIR", home.join(".claude")),
        ("TMPDIR", workspace.clone()),
        ("CLAUDE_CODE_TMPDIR", workspace.clone()),
    ]
    .into_iter()
    .all(|(key, expected)| std::env::var_os(key).map(PathBuf::from) == Some(expected));
    for directory in [root, home.as_path(), &home.join(".claude"), &workspace] {
        assert_eq!(
            fs::metadata(directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    let ambient_absent = std::env::var_os(AMBIENT).is_none();
    let stdin_closed = std::io::stdin().read(&mut [0]).unwrap() == 0;
    fs::write(
        std::env::var_os(REPORT).unwrap(),
        serde_json::to_vec(&json!({
            "root": root,
            "roots_owned": roots_owned,
            "ambient_absent": ambient_absent,
            "stdin_closed": stdin_closed,
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(roots_owned);
    assert!(ambient_absent);
    assert!(stdin_closed);

    let case = std::env::var(CASE).unwrap();
    if case == "no_message" {
        return;
    }
    let settings: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("settings.json")).unwrap()).unwrap();
    let endpoint = settings["env"]["ANTHROPIC_BASE_URL"].as_str().unwrap();
    let address = endpoint.strip_prefix("http://127.0.0.1:").unwrap();
    let port: u16 = address.parse().unwrap();
    let mut stream = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let body =
        json!({"model": MODEL, "stream": true, "messages": [{"role": "user", "content": "OK"}]})
            .to_string();
    let marker = if case == "wrong_auth" {
        "wrong-public-marker"
    } else {
        MARKER
    };
    write!(
        stream,
        "POST /v1/messages HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {marker}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(response.contains("event: message_stop\n"));
    if case == "failed_exit" {
        std::process::exit(7);
    }
}
