use super::*;

#[test]
fn upgrade_prepare_status_cancel_exchange_complete_correlated_frames() {
    use hiroute_host_runtime::{LauncherUpgradeRequest, LauncherUpgradeStatus, UpgradeAction};
    let directory = tempfile::tempdir().unwrap();
    let (shutdown_reader, shutdown_writer) = pipe().unwrap();
    let (mut requests, capability) = pipe().unwrap();
    let (ack, mut replies) = pipe().unwrap();
    nonblocking(&requests).unwrap();
    nonblocking(&ack).unwrap();
    let child = Command::new("/bin/cat")
        .stdin(Stdio::from(shutdown_reader))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut resident = Resident {
        client: Client::new(
            "hiroute-desktop",
            LocalEndpoint::from_runtime_root(directory.path().join("run")),
        ),
        owned: Some(OwnedChild {
            child,
            shutdown: Some(shutdown_writer),
            capability,
            ack,
            authority_healthy: true,
            manual_input_candidates: BTreeSet::new(),
            diagnostics: None,
        }),
        _lock: acquire_host_lock(directory.path()).unwrap(),
    };
    std::thread::scope(|scope| {
        let peer = scope.spawn(move || {
            let mut registrations = BTreeSet::new();
            for expected in ["prepare", "status", "cancel"] {
                let bytes = read_frame(&mut requests, 4096, Duration::from_secs(2), None).unwrap();
                let request: LauncherUpgradeRequest = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(request.schema, "hiroute.launcher-upgrade/v1");
                assert_eq!(serde_json::to_value(request.action).unwrap(), expected);
                assert!(registrations.insert(request.registration_id.clone()));
                let mut reply = serde_json::to_vec(&LauncherUpgradeStatus {
                    schema: "hiroute.launcher-upgrade-status/v1".into(),
                    registration_id: request.registration_id,
                    paused: expected != "cancel",
                    active_calls: 0,
                    active_tasks: 0,
                })
                .unwrap();
                reply.push(b'\n');
                write_frame(&mut replies, &reply).unwrap();
            }
        });
        for (action, paused) in [
            (UpgradeAction::Prepare, true),
            (UpgradeAction::Status, true),
            (UpgradeAction::Cancel, false),
        ] {
            let response = resident.upgrade_status(action).unwrap();
            assert_eq!(response.paused, paused);
            assert_eq!(response.drained(), paused);
        }
        peer.join().unwrap();
    });
    assert!(resident.owned.as_ref().unwrap().authority_healthy);
}

#[test]
fn shutdown_interrupts_a_pending_daemon_ready_read() {
    let (mut reader, _writer) = std::os::unix::net::UnixStream::pair().unwrap();
    reader.set_nonblocking(true).unwrap();
    let cancelled = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(20));
            cancelled.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        assert_eq!(
            read_frame(&mut reader, 4096, DAEMON_READY_TIMEOUT, Some(&cancelled)).unwrap_err(),
            "DAEMON_START_CANCELLED"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    });
}

#[test]
fn startup_failure_frame_maps_storage_without_accepting_extra_payload() {
    assert_eq!(
        decode_startup_failure(&serde_json::json!({
            "schema": "hiroute.daemon-startup-failure/v1",
            "code": "storage_unavailable"
        })),
        Ok(Some("DAEMON_STORAGE_UNREADABLE"))
    );
    assert_eq!(
        decode_startup_failure(&serde_json::json!({
            "schema": "hiroute.daemon-startup-failure/v1",
            "code": "storage_unavailable",
            "message": "/private/data/path"
        })),
        Err("DAEMON_READY_INVALID")
    );
    assert_eq!(
        decode_startup_failure(&serde_json::json!({
            "schema": "hiroute.daemon-ready/v1"
        })),
        Ok(None)
    );
}

#[test]
fn cancelled_startup_does_not_create_state_or_launch_a_child() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("not-created");
    let result = Resident::open_managed(
        &root,
        &root.join("hirouted"),
        &AtomicBool::new(true),
        &NativeDiagnostics::disabled(),
    );
    assert_eq!(result.err().as_deref(), Some("DAEMON_START_CANCELLED"));
    assert!(!root.exists());
}

#[test]
fn a_second_descriptor_of_a_held_host_lock_reports_the_duplicate() {
    // Parallel process tests may fork while this test holds a file lock. A child
    // retains the same open-file description until exec, even with CLOEXEC. Exercise
    // exact close/reclaim ordering in one isolated test process instead.
    const CASE: &str =
        "bootstrap::tests::a_second_descriptor_of_a_held_host_lock_reports_the_duplicate";
    const CHILD: &str = "HIROUTE_ISOLATED_LOCK_TEST";
    if std::env::var(CHILD).as_deref() != Ok(CASE) {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CASE])
            .env(CHILD, CASE)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success()
                && stdout
                    .lines()
                    .any(|line| line == format!("test {CASE} ... ok")),
            "isolated lock test must execute its exact case: {stdout} {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("host");
    let held = acquire_host_lock(&root).unwrap();
    assert_eq!(
        acquire_host_lock(&root).unwrap_err(),
        "DESKTOP_ALREADY_RUNNING",
        "flock on a second descriptor of the same lock file conflicts within one process"
    );
    // A duplicated descriptor models the shared open-file description inherited
    // across fork. Closing one descriptor alone must not release the lease.
    let inherited = held.try_clone().unwrap();
    drop(held);
    assert_eq!(
        acquire_host_lock(&root).unwrap_err(),
        "DESKTOP_ALREADY_RUNNING"
    );
    drop(inherited);
    assert!(acquire_host_lock(&root).is_ok());
}

#[test]
fn acknowledgement_requires_the_exact_successful_registration() {
    for (id, registered, expected) in [
        ("current", true, true),
        ("older", true, false),
        ("current", false, false),
    ] {
        let bytes = format!(
            "{{\"schema\":\"hiroute.protected-apply-ack/v2\",\"registration_id\":\"{id}\",\"registered\":{registered}}}\n"
        );
        assert_eq!(
            receive_ack(&mut bytes.as_bytes(), "current", Duration::from_secs(1)).is_ok(),
            expected
        );
    }
    for bytes in [b"".as_slice(), b"{}", b"{}\n", &[b'a'; 1026]] {
        let mut reader: &[u8] = bytes;
        assert!(receive_ack(&mut reader, "current", Duration::from_secs(1)).is_err());
    }
}
#[test]
fn stalled_acknowledgement_and_registration_writes_have_absolute_deadlines() {
    struct Stalled;
    impl Read for Stalled {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::WouldBlock.into())
        }
    }
    let started = Instant::now();
    assert_eq!(
        receive_ack(&mut Stalled, "current", Duration::from_millis(20)).unwrap_err(),
        "PROTECTED_CHANNEL_TIMEOUT"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    struct Closed;
    impl Write for Closed {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Ok(0)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert_eq!(
        write_frame(&mut Closed, b"registration").unwrap_err(),
        "PROTECTED_CHANNEL_CLOSED"
    );
}
