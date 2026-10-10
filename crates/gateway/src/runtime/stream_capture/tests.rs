use super::*;
use crate::server::core_runtime::profiles::fixed_reasoning;
use crate::server::request_plan::IngressProtocol;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

fn setup() -> PathBuf {
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).unwrap();
    let root = fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!("hiroute-capture-{:x}", u128::from_le_bytes(random)));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let session = Session {
        source_sha: "a".repeat(40),
        binary_sha256: "b".repeat(64),
        client: "fixture".into(),
        expires_at: now() + 60,
        delete_after: now() + 120,
    };
    private_file(&root.join("session.json"))
        .unwrap()
        .write_all(&serde_json::to_vec(&session).unwrap())
        .unwrap();
    root
}
fn profile() -> CandidateProtocolProfile {
    CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Messages,
        IngressProtocol::ChatCompletions,
        "physical",
        fixed_reasoning("fixed"),
    )
}

fn promoted_correlation(index: u32) -> CaptureCorrelation {
    CaptureCorrelation {
        request_token: Some(hiroute_diagnostics::identity::CorrelationToken::from_bytes(
            [1; 32],
        )),
        request_id: "private-request".into(),
        attempt_index: index,
        attempt_token: Some(hiroute_diagnostics::identity::CorrelationToken::from_bytes(
            [index as u8; 32],
        )),
    }
}

pub(super) fn correlation_checkpoint(root: &Path, stage: &str) {
    if std::env::var_os("HIROUTE_CAPTURE_INTERRUPT_ROOT").as_deref() == Some(root.as_os_str())
        && std::env::var("HIROUTE_CAPTURE_INTERRUPT_STAGE").as_deref() == Ok(stage)
    {
        private_file(&root.join("checkpoint"))
            .unwrap()
            .write_all(stage.as_bytes())
            .unwrap();
        loop {
            std::thread::park();
        }
    }
}

#[tokio::test]
async fn sealed_correlation_survives_process_termination() {
    const TEST: &str =
        "runtime::stream_capture::tests::sealed_correlation_survives_process_termination";
    if let Some(root) = std::env::var_os("HIROUTE_CAPTURE_INTERRUPT_ROOT") {
        let root = PathBuf::from(root);
        let capture = Capture::open(&root, &profile(), None, 2, true).unwrap();
        capture.record(1, b"{}");
        capture.record(2, &[]);
        capture.record(3, &200u16.to_le_bytes());
        capture.record(4, b"data: {invalid\n\n");
        capture.failed();
        PendingCapture(Arc::downgrade(&capture.0)).promoted(promoted_correlation(1));
        panic!("missing interruption checkpoint");
    }
    for stage in ["partial", "ready", "published"] {
        let root = setup();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env("HIROUTE_CAPTURE_INTERRUPT_ROOT", &root)
            .env("HIROUTE_CAPTURE_INTERRUPT_STAGE", stage)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !root.join("checkpoint").exists() && Instant::now() < deadline {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let reached = root.join("checkpoint").exists();
        let stopped = root.join("stopped").exists();
        // SIGKILL cannot execute a destructor or finish an in-progress write.
        let _ = child.kill();
        let status = child.wait().unwrap();
        assert!(
            reached && stopped,
            "{stage}: checkpoint not reached: {status}"
        );
        let path = root.join("attempt-1.capture");
        let bytes = read_private(&path, MAX_FILE).unwrap();
        let records = replay::records(&bytes).unwrap();
        assert_eq!(records.last().unwrap().0, 8, "{stage}");
        assert_eq!(records.iter().filter(|r| r.0 == 8).count(), 1, "{stage}");
        assert_eq!(
            records.iter().filter(|r| r.0 == 7).count(),
            usize::from(stage == "published"),
            "{stage}"
        );
        for chunk in [0, 1, 4096] {
            let replay = replay_capture(&path, chunk).await.unwrap();
            assert_eq!(replay["decoder"]["reason"], "invalid_json", "{stage}");
            assert_eq!(replay["captured_gateway_failure"], true, "{stage}");
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn promoted_failure_appends_only_one_bounded_correlation_without_reopening_content() {
    let root = setup();
    let capture = Capture::open(&root, &profile(), None, 2, true).unwrap();
    capture.record(1, b"{}");
    capture.record(2, &[]);
    capture.record(3, &200u16.to_le_bytes());
    capture.record(4, b"original response");
    capture.failed();
    let path = root.join("attempt-1.capture");
    let initial = fs::read(&path).unwrap();
    assert!(root.join("stopped").exists());
    assert_eq!(replay::records(&initial).unwrap().last().unwrap().0, 8);
    PendingCapture(Arc::downgrade(&capture.0)).promoted(promoted_correlation(1));
    let bound = fs::read(&path).unwrap();
    assert_eq!(&initial[..initial.len() - 9], &bound[..initial.len() - 9]);
    let records = replay::records(&bound).unwrap();
    assert_eq!(records.iter().filter(|r| r.0 == 7).count(), 1);
    assert_eq!(records.iter().filter(|r| r.0 == 8).count(), 1);
    assert_eq!(records.last().unwrap().0, 8);
    let correlation: serde_json::Value =
        serde_json::from_slice(records.iter().find(|r| r.0 == 7).unwrap().1).unwrap();
    assert_eq!(correlation["attempt_index"], 1);
    assert!(!capture.0.lock().unwrap().active);
    PendingCapture(Arc::downgrade(&capture.0)).promoted(promoted_correlation(2));
    capture.record(4, b"must not capture a later response");
    capture.failed();
    assert_eq!(fs::read(&path).unwrap(), bound);
    let weak = PendingCapture(Arc::downgrade(&capture.0));
    drop(capture);
    weak.promoted(promoted_correlation(3));
    assert_eq!(fs::read(&path).unwrap(), bound);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sealed_correlation_respects_original_bounds_and_file_identity() {
    for guard in [
        "file_limit",
        "session_limit",
        "temporary_copy_limit",
        "records",
        "expiry",
        "permissions",
        "identity",
    ] {
        let root = setup();
        let capture = Capture::open(&root, &profile(), None, 2, true).unwrap();
        capture.record(1, b"{}");
        capture.record(2, &[]);
        capture.failed();
        let path = root.join("attempt-1.capture");
        let initial = fs::read(&path).unwrap();
        match guard {
            "file_limit" => {
                let mut w = capture.0.lock().unwrap();
                w.limit = w.written;
            }
            "session_limit" => {
                private_file(&root.join("other.capture"))
                    .unwrap()
                    .set_len(MAX_TOTAL - initial.len() as u64)
                    .unwrap();
            }
            "temporary_copy_limit" => {
                // The final delta fits, but both complete copies cannot coexist.
                let correlation_bytes = serde_json::to_vec(&promoted_correlation(1)).unwrap().len();
                let existing_bytes: u64 = fs::read_dir(&root)
                    .unwrap()
                    .map(|entry| entry.unwrap().metadata().unwrap().len())
                    .sum();
                private_file(&root.join("other.capture"))
                    .unwrap()
                    .set_len(MAX_TOTAL - existing_bytes - correlation_bytes as u64 - 9)
                    .unwrap();
            }
            "records" => capture.0.lock().unwrap().records = MAX_RECORDS,
            "expiry" => capture.0.lock().unwrap().expires_at = now() - 1,
            "permissions" => fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap(),
            "identity" => {
                fs::rename(&path, root.join("original.capture")).unwrap();
                private_file(&path).unwrap().write_all(&initial).unwrap();
            }
            _ => unreachable!(),
        }
        PendingCapture(Arc::downgrade(&capture.0)).promoted(promoted_correlation(1));
        assert_eq!(fs::read(&path).unwrap(), initial, "{guard}");
        assert!(!capture.0.lock().unwrap().active, "{guard}");
        drop(capture);
        fs::remove_dir_all(root).unwrap();
    }
}

// Only after the test has dropped its previous writer. A concurrent process
// spawn can inherit the open-file description until CLOEXEC closes it. Wait
// for that actual kernel ownership to end; all other errors remain failures.
fn open_after_capture_release(root: &Path, mut on_blocked: impl FnMut()) -> Capture {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match Capture::open(root, &profile(), None, 2, true) {
            Ok(capture) => return capture,
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                on_blocked();
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("capture ownership did not release: {error}"),
        }
    }
}

#[test]
fn completed_capture_waits_for_inherited_lock_descriptor_release() {
    let root = setup();
    let capture = Capture::open(&root, &profile(), None, 2, true).unwrap();
    // A descriptor duplicate models the same open-file description inherited
    // across fork, even when the originating writer has already been dropped.
    let inherited = capture.0.lock().unwrap()._lock.try_clone().unwrap();
    drop(capture);
    assert_eq!(
        Capture::open(&root, &profile(), None, 2, true)
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::WouldBlock,
        "live descriptor ownership must still reject another writer"
    );
    let (release, observed) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        observed.recv_timeout(Duration::from_secs(2)).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        drop(inherited);
    });
    let mut blocked = 0;
    let next = open_after_capture_release(&root, || {
        blocked += 1;
        if blocked == 1 {
            release.send(()).unwrap();
        }
    });
    holder.join().unwrap();
    assert!(
        blocked > 0,
        "re-admission must observe the retained kernel lock"
    );
    drop(next);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn capture_preserves_bytes_and_replays_request_bound_tools_across_chunks() {
    let root = setup();
    let request = serde_json::json!({"model":"alias", "max_tokens":64,"messages":[{"role":"user","content":"hello"}], "tools":[{"name":"known","description":"tool", "input_schema":{"type":"object"}}]});
    let ir = adapters::decode_ingress_request(IngressProtocol::Messages, &request).unwrap();
    let tools = adapters::ChatToolProjection::for_request(&ir).unwrap();
    let body = serde_json::to_vec(&request).unwrap();
    let capture = Capture::open(&root, &profile(), Some(&tools), body.len(), true).unwrap();
    capture.record(1, &body[..9]);
    capture.record(1, &body[9..]);
    capture.record(2, &[]);
    capture.record(3, &103u16.to_le_bytes());
    capture.record(3, &200u16.to_le_bytes());
    let sse = b"data: {\"id\":\"resp\",\"model\":\"physical\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call\",\"type\":\"function\",\"function\":{\"name\":\"unknown_secret_tool\",\"arguments\":\"{}\"}}]},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n";
    capture.record(4, &sse[..17]);
    capture.record(4, &sse[17..]);
    capture.failed();
    // The supervisor may stop the process as soon as this marker appears.
    // A failed sample must already be durable and sealed while other owners still exist.
    assert!(root.join("stopped").exists());
    assert!(root.join("active.lock").exists());
    let at_stop = read_private(&root.join("attempt-1.capture"), MAX_FILE).unwrap();
    assert_eq!(replay::records(&at_stop).unwrap().last().unwrap().0, 8);
    for size in [0, 1, 4096] {
        let replay = replay_capture(&root.join("attempt-1.capture"), size)
            .await
            .unwrap();
        assert_eq!(replay["decoder"]["result"], "rejected");
        assert_eq!(replay["decoder"]["reason"], "missing_tool_identity");
    }
    capture.record(4, b"cannot append after seal");
    drop(capture);
    let path = root.join("attempt-1.capture");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let bytes = read_private(&path, MAX_FILE).unwrap();
    let records = replay::records(&bytes).unwrap();
    let actual: Vec<u8> = records
        .iter()
        .filter(|r| r.0 == 1)
        .flat_map(|r| r.1.iter().copied())
        .collect();
    assert_eq!(actual, body);
    let chunks: Vec<_> = records.iter().filter(|r| r.0 == 4).map(|r| r.1).collect();
    assert_eq!(chunks, vec![&sse[..17], &sse[17..]]);
    for size in [0, 1, 4096] {
        let result = replay_capture(&path, size).await.unwrap();
        assert_eq!(result["decoder"]["result"], "rejected");
        assert!(!result.to_string().contains("unknown_secret_tool"));
    }
    assert!(
        Capture::open(&root, &profile(), None, 2, true).is_err(),
        "first failure stops further captures"
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn bounded_capture_is_fail_closed_without_mutating_business_input() {
    let root = setup();
    let capture = Capture::open(&root, &profile(), None, 2, true).unwrap();
    assert!(
        Capture::open(&root, &profile(), None, 2, true).is_err(),
        "one writer per session"
    );
    {
        let mut writer = capture.0.lock().unwrap();
        writer.limit = writer.written + 10;
    }
    capture.record(1, b"{}");
    drop(capture);
    assert!(
        replay_capture(&root.join("attempt-1.capture"), 0)
            .await
            .is_err()
    );
    fs::set_permissions(root.join("session.json"), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(Capture::open(&root, &profile(), None, 2, true).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn capture_redacts_transport_headers_and_enforces_expiry() {
    let root = setup();
    let mut profile = profile();
    if let crate::server::core_runtime::profiles::CriticalFact::Exact(headers) =
        &mut profile.connector.headers
    {
        headers
            .required_headers
            .push(("x-api-key".into(), "SECRET_CREDENTIAL".into()));
    }
    let capture = Capture::open(&root, &profile, None, 2, true).unwrap();
    {
        let mut writer = capture.0.lock().unwrap();
        writer.expires_at = now() - 1;
    }
    capture.record(1, b"{}");
    drop(capture);
    let bytes = read_private(&root.join("attempt-1.capture"), MAX_FILE).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("SECRET_CREDENTIAL"));
    assert_ne!(replay::records(&bytes).unwrap().last().unwrap().0, 8);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn capture_limits_and_partial_request_cannot_produce_replay_evidence() {
    let root = setup();
    assert!(Capture::open(&root, &profile(), None, MAX_FILE as usize, true).is_err());
    assert!(root.join("active.lock").exists());
    let capture = open_after_capture_release(&root, || {});
    capture.record(1, b"{");
    capture.record(3, &200u16.to_le_bytes());
    capture.failed();
    drop(capture);
    assert_eq!(
        replay_capture(&root.join("attempt-1.capture"), 0)
            .await
            .unwrap_err(),
        "request body capture incomplete"
    );
    fs::remove_dir_all(root).unwrap();

    let root = setup();
    for _ in 0..MAX_ATTEMPTS {
        drop(open_after_capture_release(&root, || {}));
    }
    assert!(Capture::open(&root, &profile(), None, 2, true).is_err());
    assert!(root.join("active.lock").exists());
    fs::remove_dir_all(root).unwrap();

    let root = setup();
    private_file(&root.join("existing.capture"))
        .unwrap()
        .set_len(MAX_TOTAL)
        .unwrap();
    assert!(Capture::open(&root, &profile(), None, 2, true).is_err());
    assert!(root.join("active.lock").exists());
    fs::remove_dir_all(root).unwrap();

    let root = setup();
    let capture = Capture::open(&root, &profile(), None, 2, true).unwrap();
    capture.0.lock().unwrap().records = MAX_RECORDS;
    capture.record(1, b"{}");
    drop(capture);
    assert!(
        replay_capture(&root.join("attempt-1.capture"), 0)
            .await
            .is_err()
    );
    assert!(replay::records(&[4, 2, 0, 0, 0, 0, 0, 0, 0, 1]).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn replay_uses_final_head_and_rejects_paths_without_decoder_equivalence() {
    let body = b"{}";
    let response = b"data: {\"id\":\"resp\",\"model\":\"physical\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"ok\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"resp\",\"model\":\"physical\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
    let mut mismatches = Vec::new();
    for native in [false, true] {
        for (heads, invalid) in [
            (&[200u16][..], None),
            (&[103u16, 200][..], None),
            (&[103, 200, 200][..], Some("multiple final statuses")),
            (&[429][..], Some("unsupported_capture_path")),
            (&[103, 429][..], Some("unsupported_capture_path")),
            (&[503][..], Some("unsupported_capture_path")),
            (&[103, 503][..], Some("unsupported_capture_path")),
            (&[103][..], Some("missing final status")),
            (
                &[200, 103][..],
                Some("informational head after final status"),
            ),
        ] {
            let root = setup();
            let mut profile = profile();
            if native {
                profile.ingress_protocol = profile.capability.upstream_protocol;
            }
            let capture = Capture::open(&root, &profile, None, body.len(), true).unwrap();
            capture.record(1, body);
            capture.record(2, &[]);
            for head in heads {
                capture.record(3, &head.to_le_bytes());
            }
            capture.record(4, &response[..23]);
            capture.record(4, &response[23..]);
            capture.record(5, &[]);
            drop(capture);
            for size in [0, 1, 4096] {
                let result = replay_capture(&root.join("attempt-1.capture"), size).await;
                let expected = invalid.or(native.then_some("unsupported_capture_path"));
                let matched = match (&result, expected) {
                    (Err(actual), Some(expected)) => *actual == expected,
                    (Ok(actual), None) => actual["decoder"]["result"] == "accepted",
                    _ => false,
                };
                if !matched {
                    mismatches.push(format!(
                        "heads={heads:?} native={native} chunk={size} expected={expected:?} actual={result:?}"
                    ));
                }
            }
            fs::remove_dir_all(root).unwrap();
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}
