//! Real Resident/daemon exchanges; synthetic private inputs, no user state.
use super::*;
use hiroute_application_api::ClientServiceStatusV1;
use hiroute_product_session::HostAuthority;
use std::io::BufRead;

fn sql_count(root: &Path, table: &str) -> u64 {
    assert!(matches!(table, "apply_capabilities" | "operations"));
    let database = root.join("storage/live/control.db");
    assert!(
        database.is_file(),
        "owned fixture database missing at {database:?}; root entries: {:?}",
        std::fs::read_dir(root)
            .unwrap()
            .map(|v| v.unwrap().file_name())
            .collect::<Vec<_>>()
    );
    let output = Command::new("python3").args(["-c",
        "import sqlite3,sys,pathlib; p=pathlib.Path(sys.argv[1]); assert p.is_file(),str(p); d=sqlite3.connect(p); d.execute('PRAGMA query_only=ON'); print(d.execute('SELECT COUNT(*) FROM '+sys.argv[2]).fetchone()[0])"])
        .arg(database).arg(table).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

#[test]
fn busy_sql_writer_refuses_real_native_grants_without_losing_the_session_or_dispatching() {
    let root = tempfile::Builder::new()
        .prefix("hr-grant-")
        .tempdir_in(std::fs::canonicalize("/tmp").unwrap())
        .unwrap();
    let mut resident = Resident::open(root.path(), &daemon()).unwrap();
    let pid = resident.owned.as_ref().unwrap().child.id();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let before = runtime
        .block_on(resident.client.query::<_, ClientServiceStatusV1>(
            "GetClientServiceStatus",
            "before-busy",
            &serde_json::json!({}),
        ))
        .unwrap()
        .data
        .unwrap();
    let grants_before = sql_count(root.path(), "apply_capabilities");
    let operations_before = sql_count(root.path(), "operations");
    // Only a private SQLite writer reservation is injected: no DML, model,
    // content producer, paused daemon or production configuration is involved.
    let mut blocker = Command::new("python3").args(["-u","-c",
        "import sqlite3,sys; d=sqlite3.connect(sys.argv[1]); d.execute('BEGIN IMMEDIATE'); print('held',flush=True); sys.stdin.read(); d.rollback()"])
        .arg(root.path().join("storage/live/control.db"))
        .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut line = String::new();
    std::io::BufReader::new(blocker.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line.trim(), "held");
    let start = Instant::now();
    let refused = resident.register_operation(
        &CanonicalDigest::of_bytes(b"busy witness"),
        &before.revisions,
        "ListSessionsV2",
    );
    let refusal_elapsed = start.elapsed();
    assert_eq!(refused.err().as_deref(), Some("TRUSTED_AUTHORITY_BUSY"));
    assert!(refusal_elapsed < Duration::from_secs(1));
    assert!(resident.has_authority());
    assert_eq!(resident.owned.as_ref().unwrap().child.id(), pid);
    assert!(resident.owned.as_ref().unwrap().pending_ack.is_none());
    let mut session = crate::session_from_resident(resident);
    let request = || {
        serde_json::from_value(serde_json::json!({
            "schema":"hiroute.observation.query/v2","intent":{"view":"status","query":{}}
        }))
        .unwrap()
    };
    let error = runtime
        .block_on(session.observation_read(request()))
        .unwrap_err();
    assert!(
        matches!(error,hiroute_product_session::failure::DesktopFailure::Native { ref code } if code=="TRUSTED_AUTHORITY_BUSY")
    );
    assert_eq!(sql_count(root.path(), "apply_capabilities"), grants_before);
    assert_eq!(sql_count(root.path(), "operations"), operations_before);
    drop(blocker.stdin.take());
    assert!(blocker.wait().unwrap().success());
    // A new explicit call, never a replay of the refused operation, succeeds
    // on the same owned process and synchronized inherited descriptors.
    assert!(
        runtime
            .block_on(session.observation_read(request()))
            .is_ok()
    );
    let snapshot = runtime.block_on(session.snapshot()).unwrap();
    assert!(snapshot.trusted_authority);
    assert_eq!(
        snapshot.native_connection,
        Some(hiroute_product_session::NativeConnectionStatus::Ready)
    );
    let owned = session
        .authority
        .as_any()
        .downcast_ref::<std::sync::Mutex<Resident>>()
        .unwrap()
        .lock()
        .unwrap();
    assert_eq!(owned.owned.as_ref().unwrap().child.id(), pid);
    drop(owned);
    drop(session);
    assert_eq!(
        sql_count(root.path(), "apply_capabilities"),
        grants_before + 1
    );
    assert_eq!(sql_count(root.path(), "operations"), operations_before);
    println!(
        "{}",
        serde_json::json!({
            "schema":"hiroute.native-grant-refusal-test/v1",
            "busy_elapsed_us":refusal_elapsed.as_micros(),
            "grant_rows_added_on_refusal":0,"business_operations_added":0,
            "same_daemon":true,"channel_reused":true,"successful_grant_persisted":true,
            "model_calls":0,"content_events":0,
        })
    );
}

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
fn fresh_native_session_resynchronizes_without_restarting_the_daemon() {
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
    // Recovery consumes only this exact pending ACK, then creates a new host session.
    std::thread::sleep(Duration::from_millis(100));
    let replacement = resident.recover_authority().unwrap();
    assert_eq!(replacement.owned.as_ref().unwrap().child.id(), previous);
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
        resident
            .register_operation(
                &CanonicalDigest::of_bytes(b"old callback"),
                &before.revisions,
                "ListSessionsV2"
            )
            .is_err(),
        "captured old authority must not regain grant issuance"
    );
    assert!(
        runtime
            .block_on(resident.client.query::<_, ClientServiceStatusV1>(
                "GetClientServiceStatus",
                "same-daemon-still-responsive",
                &serde_json::json!({})
            ))
            .is_ok()
    );
}

#[test]
fn resynchronization_releases_an_input_whose_registration_finished_after_timeout() {
    let root = tempfile::Builder::new()
        .prefix("hr187-input-")
        .tempdir_in(std::fs::canonicalize("/tmp").unwrap())
        .unwrap();
    let mut resident = Resident::open(root.path(), &daemon()).unwrap();
    let pid = resident.owned.as_ref().unwrap().child.id();
    let candidate = ComputeCandidateRefV2 {
        candidate_ref: format!("candidate/native/{}", crate::random_id().unwrap()),
        candidate_revision: 1,
    };
    let paused = Paused::new(pid);
    assert!(
        resident
            .register_model_input_once(&candidate, "synthetic abandoned input")
            .is_err()
    );
    assert!(
        resident
            .owned
            .as_ref()
            .unwrap()
            .manual_input_candidates
            .contains(&candidate.candidate_ref)
    );
    drop(paused);
    let mut replacement = resident.recover_authority().unwrap();
    assert_eq!(replacement.owned.as_ref().unwrap().child.id(), pid);
    assert!(
        replacement
            .owned
            .as_ref()
            .unwrap()
            .manual_input_candidates
            .is_empty()
    );
    // Reusing this private test identity would fail if the late registration's
    // protected input remained in the daemon registry.
    replacement
        .register_model_input_once(&candidate, "synthetic fresh input")
        .unwrap();
    replacement.release_model_input(&candidate).unwrap();
    assert!(!resident.has_authority());
}
