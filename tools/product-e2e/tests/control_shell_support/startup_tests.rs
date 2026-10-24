use super::*;
use crate::DaemonGuard;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;

const BOOT: &str = "0123456789abcdef0123456789abcdef";
const SECRET: &str = "private-startup-sentinel";

fn spawn(capture: &StartupCapture, script: &str) -> DaemonGuard {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", script]);
    capture.configure(&mut command).unwrap();
    DaemonGuard(command.spawn().unwrap())
}

fn evidence(root: &Path) -> (PathBuf, Value) {
    let paths: Vec<_> = std::fs::read_dir(root).unwrap().collect();
    assert_eq!(paths.len(), 1);
    let path = paths[0].as_ref().unwrap().path();
    let report = serde_json::from_slice(&std::fs::read(path.join("report.json")).unwrap()).unwrap();
    (path, report)
}

#[test]
fn stalled_startup_retains_bounded_private_evidence_after_fixture_cleanup() {
    let fixture = tempfile::tempdir().unwrap();
    let retained = tempfile::tempdir().unwrap();
    let capture = StartupCapture::new(fixture.path()).unwrap();
    let logs = capture.root.join("diagnostics/daemon");
    std::fs::create_dir_all(&logs).unwrap();
    let records = [
        json!({"boot_id": BOOT, "event": {"level_applied": {"level": "debug"}}}),
        json!({"boot_id": BOOT, "event": {"stage_begin": {"stage": "root_validate"}}}),
    ];
    std::fs::write(
        logs.join("current.jsonl"),
        format!("{}\n{}\n", records[0], records[1]),
    )
    .unwrap();
    std::fs::write(fixture.path().join("configuration-secret"), SECRET).unwrap();
    let mut child = spawn(
        &capture,
        &format!(
            "dd if=/dev/zero bs=1024 count=128 2>/dev/null; printf '{SECRET}' >&2; exec sleep 60"
        ),
    );
    let started = Instant::now();
    let failure = capture
        .wait_for_endpoint(
            &fixture.path().join("absent.sock"),
            &mut child.0,
            retained.path(),
        )
        .unwrap_err();
    assert!(started.elapsed() >= READINESS_TIMEOUT);
    assert!(started.elapsed() < READINESS_TIMEOUT + Duration::from_secs(5));
    assert!(failure.starts_with("hirouted readiness timed out"));
    assert!(!failure.contains(SECRET));
    assert!(
        child.0.try_wait().unwrap().is_none(),
        "snapshot must precede guard cleanup"
    );
    drop(child);
    drop(fixture);
    let (path, report) = evidence(retained.path());
    assert_eq!(report["child_status"], "running");
    assert_eq!(report["deadline_ms"], 10_000);
    assert_eq!(report["diagnostics"]["current_boot"], BOOT);
    assert_eq!(
        report["diagnostics"]["last_observed_stage"],
        "root_validate"
    );
    assert_eq!(report["diagnostics"]["state"], "available");
    assert_eq!(report["writer_flush"], "unknown");
    assert_eq!(
        report["files"]["stdout.tail"]["captured_bytes"],
        SNAPSHOT_BYTES
    );
    assert_eq!(report["files"]["stdout.tail"]["truncated"], true);
    assert_eq!(
        std::fs::read_to_string(path.join("stderr.tail")).unwrap(),
        SECRET
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let mut names = Vec::new();
    for entry in std::fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        assert_eq!(
            entry.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    assert_eq!(
        names,
        [
            "daemon-current.tail.jsonl",
            "report.json",
            "stderr.tail",
            "stdout.tail"
        ]
    );
}

#[test]
fn early_exit_retains_status_without_printing_child_output() {
    let fixture = tempfile::tempdir().unwrap();
    let retained = tempfile::tempdir().unwrap();
    let capture = StartupCapture::new(fixture.path()).unwrap();
    let mut child = spawn(&capture, &format!("printf '{SECRET}' >&2; exit 7"));
    // An inherited/duplicated writer remains open. Collection cannot wait for EOF.
    let _open_writer = OpenOptions::new()
        .append(true)
        .open(capture.root.join("stderr"))
        .unwrap();
    let started = Instant::now();
    let failure = capture
        .wait_for_endpoint(
            &fixture.path().join("absent.sock"),
            &mut child.0,
            retained.path(),
        )
        .unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(failure.starts_with("hirouted exited before readiness"));
    assert!(!failure.contains(SECRET));
    assert_eq!(child.0.try_wait().unwrap().unwrap().code(), Some(7));
    drop(child);
    drop(fixture);
    let (path, report) = evidence(retained.path());
    assert!(report["child_status"].as_str().unwrap().contains('7'));
    assert_eq!(report["diagnostics"]["state"], "unavailable");
    assert_eq!(
        report["files"]["daemon-current.tail.jsonl"]["error"],
        "NotFound"
    );
    assert_eq!(
        std::fs::read_to_string(path.join("stderr.tail")).unwrap(),
        SECRET
    );
}

#[test]
fn ready_socket_keeps_the_child_alive_and_creates_no_evidence() {
    let fixture = tempfile::tempdir().unwrap();
    let capture = StartupCapture::new(fixture.path()).unwrap();
    let mut child = spawn(&capture, "exec sleep 60");
    let endpoint = fixture.path().join("ready.sock");
    let _listener = UnixListener::bind(&endpoint).unwrap();
    let retained = fixture.path().join("evidence");
    capture
        .wait_for_endpoint(&endpoint, &mut child.0, &retained)
        .unwrap();
    assert!(child.0.try_wait().unwrap().is_none());
    assert!(!retained.exists());
    drop(child);
    drop(_listener);
    fixture.close().unwrap();
}

#[test]
fn incomplete_or_ambiguous_diagnostics_are_not_claimed_complete() {
    let valid = json!({"boot_id": BOOT, "event": {"stage_begin": {"stage": "control_bind"}}});
    let summary = diagnostic_summary(format!("{valid}\n{{broken").as_bytes(), true);
    assert_eq!(summary["state"], "partial");
    assert_eq!(summary["invalid_record_count"], 1);
    assert_eq!(summary["last_observed_stage"], "control_bind");
    assert_eq!(summary["level_applied_observed"], false);
    let second = json!({"boot_id": "ffffffffffffffffffffffffffffffff", "event": {}});
    let summary = diagnostic_summary(format!("{valid}\n{second}\n").as_bytes(), false);
    assert_eq!(summary["state"], "partial");
    assert!(summary["current_boot"].is_null());
    assert!(summary["last_observed_stage"].is_null());
    assert_eq!(diagnostic_summary(b"", false)["state"], "unavailable");
}

#[test]
fn evidence_failure_preserves_the_startup_failure_and_refuses_special_files() {
    let fixture = tempfile::tempdir().unwrap();
    let capture = StartupCapture::new(fixture.path()).unwrap();
    let mut child = spawn(&capture, "exit 9");
    let invalid_destination = fixture.path().join("not-a-directory");
    std::fs::write(&invalid_destination, "").unwrap();
    let failure = capture
        .wait_for_endpoint(
            &fixture.path().join("absent.sock"),
            &mut child.0,
            &invalid_destination,
        )
        .unwrap_err();
    assert!(failure.starts_with("hirouted exited before readiness"));
    assert!(failure.contains("evidence unavailable"));
    let link = fixture.path().join("link");
    std::os::unix::fs::symlink(&invalid_destination, &link).unwrap();
    assert_eq!(
        read_tail(&link).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}
