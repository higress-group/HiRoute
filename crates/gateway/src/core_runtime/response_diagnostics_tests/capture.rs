//! Capture readiness and complete offline replay assertions for listener fixtures.
use super::*;

pub(super) fn wait_for_captured_request_eof(root: &Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let mut kinds = Vec::new();
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|ext| ext != "capture") {
                continue;
            }
            let bytes = fs::read(path).unwrap();
            let mut remaining = bytes.as_slice();
            // A concurrent writer may leave an incomplete trailing record.
            while remaining.len() >= 9 {
                let size = u64::from_le_bytes(remaining[1..9].try_into().unwrap());
                if size > (remaining.len() - 9) as u64 {
                    break;
                }
                kinds.push(remaining[0]);
                if remaining[0] == 2 {
                    assert_eq!(size, 0);
                    return;
                }
                remaining = &remaining[9 + size as usize..];
            }
        }
        assert!(
            Instant::now() < deadline,
            "request EOF was not captured: {kinds:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

pub(super) fn create_capture_session(root: &Path) {
    fs::DirBuilder::new().mode(0o700).create(root).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(root.join("session.json"))
        .unwrap();
    file.write_all(serde_json::to_string(&json!({"source_sha":"a".repeat(40),"binary_sha256":"b".repeat(64),
        "client":"deterministic-production-listener-fixture","expires_at":now+60,"delete_after":now+120})).unwrap().as_bytes()).unwrap();
}

pub(super) fn assert_capture(
    root: &Path,
    upstream: &[u8],
    entity: &[u8],
    reason: Option<&str>,
    diagnostics: &[Value],
    facts: &[Value],
) {
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root.join("active.lock"))
        .unwrap();
    // The stable inode survives both clean exit and crashes; kernel ownership
    // must be released before cleanup can use this same lock.
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .expect("writer ownership was not released");
    let files: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "capture"))
        .collect();
    assert_eq!(files.len(), 1);
    let path = &files[0];
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let bytes = fs::read(path).unwrap();
    assert!(
        !bytes
            .windows(b"SENTINEL_UPSTREAM_CREDENTIAL".len())
            .any(|w| w == b"SENTINEL_UPSTREAM_CREDENTIAL")
    );
    let mut remaining = bytes.as_slice();
    let mut records = Vec::new();
    while !remaining.is_empty() {
        assert!(remaining.len() >= 9);
        let size = u64::from_le_bytes(remaining[1..9].try_into().unwrap()) as usize;
        assert!(size <= remaining.len() - 9);
        records.push((remaining[0], &remaining[9..9 + size]));
        remaining = &remaining[9 + size..];
    }
    assert_eq!(records.last().unwrap().0, 8);
    assert!(records.iter().any(|r| r.0 == 2));
    let captured = |kind| {
        records
            .iter()
            .filter(|r| r.0 == kind)
            .flat_map(|r| r.1.iter().copied())
            .collect::<Vec<_>>()
    };
    let body_offset = upstream.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    assert_eq!(
        captured(1),
        upstream[body_offset..],
        "capture must use actual prepared request reader"
    );
    assert_eq!(
        captured(4),
        entity,
        "ordered read blocks must reproduce the exact entity"
    );
    let context: Value = serde_json::from_slice(records[0].1).unwrap();
    assert_eq!(
        context["request_bytes"],
        (upstream.len() - body_offset) as u64
    );
    assert_eq!(context["profile"]["ingress_protocol"], "messages");
    assert_eq!(
        context["profile"]["capability"]["upstream_protocol"],
        "chat_completions"
    );
    assert!(!context["chat_tools"].is_null());
    assert_eq!(records.iter().filter(|r| r.0 == 7).count(), 1);
    let correlation: Value =
        serde_json::from_slice(records.iter().find(|r| r.0 == 7).unwrap().1).unwrap();
    let attempt = diagnostics
        .iter()
        .find_map(|r| payload(r, "attempt_begin"))
        .unwrap();
    assert_eq!(correlation["request_token"], attempt["request_token"]);
    assert_eq!(correlation["attempt_token"], attempt["attempt_token"]);
    assert_eq!(correlation["attempt_index"], attempt["attempt_index"]);
    assert_eq!(
        correlation["request_id"],
        facts
            .iter()
            .find_map(|r| r.pointer("/correlation/request_id"))
            .unwrap()
            .clone()
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    for chunks in [0, 1, 4096] {
        let replay = rt
            .block_on(crate::runtime::stream_capture::replay_capture(path, chunks))
            .unwrap();
        assert_eq!(
            replay["decoder"]["result"],
            if reason.is_some() {
                "rejected"
            } else {
                "accepted"
            }
        );
        if let Some(reason) = reason {
            assert_eq!(replay["decoder"]["reason"], reason);
        } else {
            assert_eq!(replay["decoder"]["tool_calls"], 1);
        }
        assert!(!replay.to_string().contains("SENTINEL"));
    }
}
