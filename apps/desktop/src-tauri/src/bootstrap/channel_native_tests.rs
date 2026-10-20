//! Real Resident/daemon exchanges; synthetic private inputs, no user state.
use super::*;
use hiroute_application_api::ClientServiceStatusV1;
use hiroute_product_session::HostAuthority;

fn daemon() -> PathBuf {
    let build = Command::new("cargo")
        .args([
            "build",
            "--locked",
            "-p",
            "hiroute-daemon",
            "--bin",
            "hirouted",
        ])
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."))
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "daemon prerequisite: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("hirouted")
}

struct Paused(nix::unistd::Pid);
impl Paused {
    fn new(pid: u32) -> Self {
        let pid = nix::unistd::Pid::from_raw(pid as i32);
        nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGSTOP).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        Self(pid)
    }
}
impl Drop for Paused {
    fn drop(&mut self) {
        let _ = nix::sys::signal::kill(self.0, nix::sys::signal::Signal::SIGCONT);
    }
}

#[test]
fn live_daemon_after_ack_timeout_does_not_report_a_healthy_native_session() {
    let root = tempfile::Builder::new()
        .prefix("hr187-")
        .tempdir_in(std::fs::canonicalize("/tmp").unwrap())
        .unwrap();
    let mut resident = Resident::open(root.path(), &daemon()).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let before = runtime
        .block_on(resident.client.query::<_, ClientServiceStatusV1>(
            "GetClientServiceStatus",
            "before-fault",
            &serde_json::json!({}),
        ))
        .unwrap()
        .data
        .unwrap();
    let paused = Paused::new(resident.owned.as_ref().unwrap().child.id());
    let started = Instant::now();
    assert!(
        resident
            .register_operation(
                &CanonicalDigest::of_bytes(b"timeout witness"),
                &before.revisions,
                "ListSessionsV2"
            )
            .is_err()
    );
    assert!(started.elapsed() >= Duration::from_secs(2));
    assert!(started.elapsed() < Duration::from_secs(4));
    drop(paused);
    let after = runtime
        .block_on(resident.client.query::<_, ClientServiceStatusV1>(
            "GetClientServiceStatus",
            "after-fault",
            &serde_json::json!({}),
        ))
        .unwrap()
        .data
        .unwrap();
    assert!(after.recovery_ready);
    assert_eq!(before.revisions, after.revisions);
    let mut session = crate::session_from_resident(resident);
    let request = || {
        serde_json::from_value(serde_json::json!({
            "schema":"hiroute.observation.query/v2", "intent":{"view":"status","query":{}}
        }))
        .unwrap()
    };
    for _ in 0..2 {
        let error = runtime
            .block_on(session.observation_read(request()))
            .unwrap_err();
        assert!(
            matches!(error, hiroute_product_session::failure::DesktopFailure::Native { ref code } if code == "TRUSTED_AUTHORITY_UNAVAILABLE")
        );
    }
    let snapshot = runtime.block_on(session.snapshot()).unwrap();
    assert!(
        !snapshot.trusted_authority,
        "a live service does not prove a usable native authority"
    );
}

#[test]
fn fresh_native_session_recovers_without_reusing_late_ack_or_saved_state_changes() {
    let root = tempfile::Builder::new()
        .prefix("hr187-")
        .tempdir_in(std::fs::canonicalize("/tmp").unwrap())
        .unwrap();
    let binary = daemon();
    let mut resident = Resident::open(root.path(), &binary).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let before = runtime
        .block_on(resident.client.query::<_, ClientServiceStatusV1>(
            "GetClientServiceStatus",
            "before-recovery",
            &serde_json::json!({}),
        ))
        .unwrap()
        .data
        .unwrap();
    let previous = resident.owned.as_ref().unwrap().child.id();
    let paused = Paused::new(previous);
    assert!(
        resident
            .register_operation(
                &CanonicalDigest::of_bytes(b"late old registration"),
                &before.revisions,
                "ListSessionsV2"
            )
            .is_err()
    );
    drop(paused);
    // The daemon writes the original success ACK after the caller's absolute timeout.
    // It stays unread in the tainted stream while independent maintenance drains/restarts.
    std::thread::sleep(Duration::from_millis(100));
    let replacement = resident
        .recover_authority(root.path(), &binary, &NativeDiagnostics::disabled())
        .unwrap();
    assert_ne!(replacement.owned.as_ref().unwrap().child.id(), previous);
    assert!(
        !resident.has_authority(),
        "captured old authority cannot authorize after replacement"
    );
    assert!(
        acquire_host_lock(root.path()).is_err(),
        "replacement retains the original host lease"
    );
    let mut session = crate::session_from_resident(replacement);
    let snapshot = runtime.block_on(session.snapshot()).unwrap();
    assert!(snapshot.trusted_authority);
    assert_eq!(snapshot.service.revisions, before.revisions);
    assert_eq!(
        snapshot.native_connection,
        Some(hiroute_product_session::NativeConnectionStatus::Ready)
    );
    let request = serde_json::from_value(serde_json::json!({
        "schema":"hiroute.observation.query/v2", "intent":{"view":"status","query":{}}
    }))
    .unwrap();
    assert!(runtime.block_on(session.observation_read(request)).is_ok());
    let candidate = session
        .authority
        .register_model_input(Zeroizing::new("synthetic recovered input".into()))
        .unwrap();
    session.authority.release_model_input(&candidate).unwrap();
    assert!(!resident.has_authority());
    assert!(
        runtime
            .block_on(resident.client.query::<_, ClientServiceStatusV1>(
                "GetClientServiceStatus",
                "old-session-callback",
                &serde_json::json!({}),
            ))
            .is_err(),
        "captured old client must reject the replacement process identity"
    );
}
