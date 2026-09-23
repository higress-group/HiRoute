use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use hiroute_gateway_core::runtime::body::{BudgetTree, MemoryRole, StreamBudget};

use super::{ReplayConfig, ReplayError, ReplayManager, ReplayStore};

#[path = "tests/locator_literals.rs"]
mod locator_literals;

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRoot(PathBuf);

impl TestRoot {
    fn new(label: &str) -> Self {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "hiroute-replay-test-{label}-{}-{sequence}-{nanos}",
            std::process::id()
        )))
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn config(root: &Path, threshold: usize, record_bytes: usize) -> ReplayConfig {
    ReplayConfig {
        root: root.to_path_buf(),
        memory_threshold_bytes: threshold,
        record_bytes,
        orphan_ttl: Duration::from_secs(60),
    }
}

fn budget() -> StreamBudget {
    BudgetTree::new(16 * 1024 * 1024, 16 * 1024 * 1024)
        .expect("budget tree")
        .stream(16 * 1024 * 1024)
        .expect("stream budget")
}

fn read_all(store: &ReplayStore, reference: &crate::content_ref::ContentRef) -> Vec<u8> {
    let mut reader = store.reader(reference).expect("open replay reader");
    let mut output = Vec::new();
    reader
        .read_to_end(&mut output)
        .expect("bounded replay read");
    output
}

#[test]
fn production_default_retains_up_to_ten_mib() {
    let config = ReplayConfig::production_default();
    assert_eq!(config.memory_threshold_bytes, 10 * 1024 * 1024);
    assert_eq!(config.record_bytes, 16 * 1024);
}

#[cfg(unix)]
#[test]
fn production_default_supports_symlinked_system_temp() {
    const CHILD: &str = "HIROUTE_REPLAY_SYMLINK_TEMP_TEST";
    if std::env::var_os(CHILD).is_some() {
        let manager = ReplayManager::from_environment().expect("default replay root");
        let store = manager.begin_request(budget()).expect("request backing");
        let mut writer = store.begin_raw().expect("writer");
        writer.append(b"native client request").expect("append");
        let reference = writer.seal().expect("seal");
        assert_eq!(read_all(&store, &reference), b"native client request");
        return;
    }
    let root = TestRoot::new("system-temp-link");
    fs::create_dir_all(root.0.join("actual")).expect("temporary directory");
    let actual = fs::canonicalize(root.0.join("actual")).expect("resolved temp");
    let link = root.0.join("system-temp");
    std::os::unix::fs::symlink(&actual, &link).expect("system temp alias");
    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "replay::tests::production_default_supports_symlinked_system_temp",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("TMPDIR", &link)
        .env_remove("HIROUTE_REPLAY_ROOT")
        .env_remove("HIROUTE_REPLAY_MEMORY_THRESHOLD")
        .env_remove("HIROUTE_REPLAY_RECORD_BYTES")
        .env_remove("HIROUTE_REPLAY_ORPHAN_TTL_MS")
        .output()
        .expect("isolated default-path check");
    assert!(
        output.status.success(),
        "default replay failed with a system temp alias: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn small_stream_retains_charged_capacity_and_has_independent_readers() {
    let root = TestRoot::new("memory");
    let manager = ReplayManager::open(config(&root.0, 64 * 1024, 4 * 1024)).expect("manager");
    let stream_budget = budget();
    let store = manager.begin_request(stream_budget.clone()).expect("store");
    let payload = b"small canonical body".repeat(257);
    let mut writer = store.begin_raw().expect("writer");
    for chunk in payload.chunks(113) {
        writer.append(chunk).expect("append");
    }
    let reference = writer.seal().expect("seal");
    let marker = reference.wire_marker();
    assert!(marker.starts_with("__hiroute_content_ref_v2_"));
    assert!(!marker.contains("sha256"));
    assert!(
        serde_json::to_value(&reference)
            .expect("serialize ContentRef")
            .get("sha256")
            .is_none()
    );
    assert_eq!(
        crate::content_ref::ContentRef::from_wire_marker(&marker),
        Some(reference.clone())
    );

    let snapshot = store.snapshot();
    assert!(!snapshot.disk_backed);
    assert!(snapshot.memory_retained_bytes >= payload.len());
    assert!(snapshot.memory_retained_bytes <= 64 * 1024);
    assert_eq!(
        stream_budget.snapshot().expect("snapshot").role_live[MemoryRole::RawRequest as usize],
        snapshot.memory_retained_bytes
    );
    assert_eq!(read_all(&store, &reference), payload);
    assert_eq!(read_all(&store, &reference), payload);

    store.release_stream(&reference).expect("release stream");
    assert_eq!(store.snapshot().memory_retained_bytes, 0);
    assert_eq!(
        stream_budget.snapshot().expect("snapshot").role_live[MemoryRole::RawRequest as usize],
        0
    );
    assert!(stream_budget.snapshot().expect("snapshot").live > 0);
    drop(store);
    drop(manager);
    assert_eq!(stream_budget.snapshot().expect("snapshot").live, 0);
}

#[test]
fn replay_stream_and_range_counts_use_owned_memory_without_fixed_quotas() {
    let root = TestRoot::new("metadata-counts");
    let manager = ReplayManager::open(config(&root.0, 64 * 1024, 4 * 1024)).unwrap();
    let stream_budget = budget();
    let store = manager.begin_request(stream_budget.clone()).unwrap();
    for _ in 0..9 {
        let mut writer = store.begin_raw().unwrap();
        writer.append(b"raw").unwrap();
        let reference = writer.seal().unwrap();
        assert_eq!(read_all(&store, &reference), b"raw");
    }
    let mut writer = store.begin_content_pool().unwrap();
    let mut last = None;
    for _ in 0..16_385 {
        last = Some(writer.append(b"range").unwrap());
    }
    writer.seal().unwrap();
    assert_eq!(read_all(&store, &last.unwrap()), b"range");
    assert!(stream_budget.snapshot().unwrap().live > 16_384 * super::RANGE_METADATA_BYTES);
    drop(store);
    assert_eq!(stream_budget.snapshot().unwrap().live, 0);
}

#[test]
fn large_stream_uses_one_plaintext_backing_for_multiple_readers() {
    let root = TestRoot::new("disk");
    let manager = ReplayManager::open(config(&root.0, 128, 31)).expect("manager");
    let store = manager.begin_request(budget()).expect("store");
    let payload = (0..2_003_u32)
        .map(|value| (value.wrapping_mul(37) & 0xff) as u8)
        .collect::<Vec<_>>();
    let mut writer = store.begin_raw().expect("writer");
    writer.append(&payload).expect("append");
    let live_paths = fs::read_dir(store.request_directory())
        .expect("request directory")
        .map(|entry| entry.expect("directory entry").path())
        .collect::<Vec<_>>();
    assert_eq!(live_paths.len(), 1, "one backing exists before seal");
    #[cfg(unix)]
    assert_eq!(fs::read(&live_paths[0]).expect("live backing"), payload);
    let reference = writer.seal().expect("seal");

    assert!(store.snapshot().disk_backed);
    assert_eq!(store.snapshot().memory_retained_bytes, 0);
    let path = store.stream_path(&reference).expect("disk path");
    assert_eq!(path, live_paths[0], "seal keeps the original backing");
    assert!(path.is_file());
    assert_eq!(
        store
            .stream_backing_bytes(&reference)
            .expect("sealed plaintext backing"),
        payload
    );
    assert_eq!(read_all(&store, &reference), payload);
    assert_eq!(read_all(&store, &reference), payload);
}

#[test]
fn plaintext_disk_reader_coalesces_single_byte_consumers() {
    let root = TestRoot::new("disk-read-ahead");
    let record_bytes = 16;
    let manager = ReplayManager::open(config(&root.0, 64, record_bytes)).expect("manager");
    let store = manager.begin_request(budget()).expect("store");
    let payload = (0..1_024_u32)
        .map(|value| (value.wrapping_mul(29) & 0xff) as u8)
        .collect::<Vec<_>>();
    let mut writer = store.begin_raw().expect("writer");
    writer.append(&payload).expect("append");
    let reference = writer.seal().expect("seal");

    let mut reader = store.reader(&reference).expect("reader");
    let mut observed = Vec::with_capacity(payload.len());
    for _ in 0..payload.len() {
        let mut byte = [0_u8; 1];
        reader.read_exact(&mut byte).expect("single-byte read");
        observed.push(byte[0]);
    }
    assert_eq!(observed, payload);
    assert_eq!(
        reader.disk_read_count(),
        payload.len().div_ceil(record_bytes),
        "single-byte consumers must use bounded disk read-ahead"
    );
    assert_eq!(reader.read(&mut [0_u8; 1]).expect("bounded eof"), 0);
}

#[test]
fn prevalidate_does_not_scan_plaintext_and_reader_rejects_truncation_at_eof() {
    let root = TestRoot::new("truncate");
    let manager = ReplayManager::open(config(&root.0, 64, 16)).expect("manager");
    let store = manager.begin_request(budget()).expect("store");
    let payload = b"plaintext replay still verifies its declared length at eof".repeat(8);
    let mut writer = store.begin_raw().expect("writer");
    writer.append(&payload).expect("append");
    let reference = writer.seal().expect("seal");
    store
        .truncate_stream_last_byte(&reference)
        .expect("truncate retained replay handle");

    store
        .prevalidate(std::slice::from_ref(&reference))
        .expect("prevalidation only checks the pinned owner handle");
    let error = store
        .reader(&reference)
        .expect("reader")
        .verify_terminal()
        .expect_err("reader must reject early eof");
    assert!(matches!(error, ReplayError::Integrity), "{error}");
}

#[test]
fn equal_length_plaintext_changes_are_inside_the_owner_trust_boundary() {
    let root = TestRoot::new("equal-length-change");
    let manager = ReplayManager::open(config(&root.0, 64, 16)).expect("manager");
    let store = manager.begin_request(budget()).expect("store");
    let payload = b"owner-only replay does not hash equal-length changes".repeat(8);
    let mut writer = store.begin_raw().expect("writer");
    writer.append(&payload).expect("append");
    let reference = writer.seal().expect("seal");
    store
        .tamper_stream_last_byte(&reference)
        .expect("change retained replay handle");

    store
        .prevalidate(std::slice::from_ref(&reference))
        .expect("prevalidation does not scan replay bytes");
    let observed = read_all(&store, &reference);
    let mut expected = payload;
    *expected.last_mut().expect("payload byte") ^= 0x80;
    assert_eq!(observed, expected);
}

#[cfg(unix)]
#[test]
fn permissions_symlinks_and_path_escape_fail_closed() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let unsafe_root = TestRoot::new("permissions-root");
    fs::create_dir(&unsafe_root.0).expect("create unsafe root");
    fs::set_permissions(&unsafe_root.0, fs::Permissions::from_mode(0o755))
        .expect("set unsafe root mode");
    assert!(matches!(
        ReplayManager::open(config(&unsafe_root.0, 64, 16)),
        Err(ReplayError::UnsafePermissions)
    ));

    let target = TestRoot::new("symlink-target");
    fs::create_dir(&target.0).expect("create target");
    fs::set_permissions(&target.0, fs::Permissions::from_mode(0o700)).expect("set target mode");
    let link = TestRoot::new("symlink-root");
    symlink(&target.0, &link.0).expect("create root symlink");
    assert!(matches!(
        ReplayManager::open(config(&link.0, 64, 16)),
        Err(ReplayError::UnsafePath)
    ));

    let root = TestRoot::new("file-mode");
    let manager = ReplayManager::open(config(&root.0, 64, 16)).expect("manager");
    let store = manager.begin_request(budget()).expect("store");
    let mut writer = store.begin_raw().expect("writer");
    writer.append(&[7_u8; 256]).expect("append");
    let reference = writer.seal().expect("seal");
    let path = store.stream_path(&reference).expect("disk path");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("loosen file mode");
    assert!(matches!(
        store.prevalidate(std::slice::from_ref(&reference)),
        Err(ReplayError::UnsafePermissions)
    ));

    let escaped = root.0.join("child").join("..").join("other");
    assert!(matches!(
        ReplayManager::open(config(&escaped, 64, 16)),
        Err(ReplayError::UnsafePath)
    ));
}

#[cfg(unix)]
#[test]
fn replay_handle_relative_rename_and_symlink_race_cannot_escape() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let root = TestRoot::new("handle-race-root");
    let outside = TestRoot::new("handle-race-outside");
    fs::create_dir(&outside.0).expect("outside directory");
    fs::set_permissions(&outside.0, fs::Permissions::from_mode(0o700))
        .expect("outside permissions");
    let manager = ReplayManager::open(config(&root.0, 64, 16)).expect("manager");
    let store = manager.begin_request(budget()).expect("store");
    let original_directory = store.request_directory().to_path_buf();
    let mut writer = store.begin_raw().expect("raw writer");
    let payload = b"pinned replay inode".repeat(64);
    writer.append(&payload).expect("append raw");
    let raw = writer.seal().expect("seal raw");
    let original_file = store.stream_path(&raw).expect("disk path");
    let file_name = original_file.file_name().expect("file name");

    let moved_directory = root.0.join(format!(
        "req-renamed-{}",
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let sentinel = outside.0.join(file_name);
    fs::write(&sentinel, b"outside-sentinel").expect("outside sentinel");
    let race = std::sync::Arc::new(std::sync::Barrier::new(2));
    let attacker_race = std::sync::Arc::clone(&race);
    let attacker_original = original_directory.clone();
    let attacker_moved = moved_directory.clone();
    let attacker_outside = outside.0.clone();
    let attacker = std::thread::spawn(move || {
        attacker_race.wait();
        fs::rename(&attacker_original, &attacker_moved).expect("rename live request directory");
        symlink(&attacker_outside, &attacker_original).expect("replace pathname with symlink");
    });
    race.wait();
    let raced = store
        .store_content(b"write racing the pathname replacement stays on the pinned inode")
        .expect("concurrent handle-relative content write");
    attacker.join().expect("pathname attacker");

    let ranged = store
        .store_content(b"new content stays on the pinned directory handle")
        .expect("handle-relative content write");
    assert_eq!(
        fs::read(&sentinel).expect("sentinel readable"),
        b"outside-sentinel"
    );
    store
        .prevalidate(&[raw.clone(), raced.clone(), ranged.clone()])
        .expect("pinned backing prevalidation");
    assert_eq!(read_all(&store, &raw), payload);
    assert_eq!(
        read_all(&store, &raced),
        b"write racing the pathname replacement stays on the pinned inode"
    );
    assert_eq!(
        read_all(&store, &ranged),
        b"new content stays on the pinned directory handle"
    );
    assert_eq!(
        fs::read(&sentinel).expect("sentinel survives read/unlink"),
        b"outside-sentinel"
    );

    drop(store);
    assert!(!moved_directory.exists(), "renamed pinned inode is cleaned");
    assert_eq!(
        fs::read(&sentinel).expect("sentinel survives cleanup"),
        b"outside-sentinel"
    );
}

#[test]
fn replay_ingress_structure_is_budgeted_without_a_fixed_field_count_limit() {
    let root = TestRoot::new("structure-gate");
    let manager = ReplayManager::open(config(&root.0, 128, 64)).expect("manager");
    let store = manager.begin_request(budget()).expect("store");
    let body = serde_json::to_vec(&serde_json::json!({
        "model": "alias",
        "input": (0..10_000).map(|index| serde_json::json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": format!("part-{index}")}]
        })).collect::<Vec<_>>()
    }))
    .expect("valid fragmented JSON");
    assert!(body.len() < 1024 * 1024);
    let mut writer = store.begin_raw().expect("raw writer");
    writer.append(&body).expect("append raw");
    let raw = writer.seal().expect("seal raw");

    let stats = crate::content_ref::scan_ingress_document(store.reader(&raw).expect("raw reader"))
        .expect("many fields are not a protocol error");
    let large = manager
        .begin_request(
            BudgetTree::new(64 * 1024 * 1024, 64 * 1024 * 1024)
                .unwrap()
                .stream(64 * 1024 * 1024)
                .unwrap(),
        )
        .unwrap();
    let workspace = stats
        .reserve_workspace(&large, raw.byte_len())
        .expect("sufficient memory");
    assert!(workspace.bytes() > 16_384 * 128);
    let small = manager
        .begin_request(
            BudgetTree::new(1024 * 1024, 1024 * 1024)
                .unwrap()
                .stream(1024 * 1024)
                .unwrap(),
        )
        .unwrap();
    assert!(stats.reserve_workspace(&small, raw.byte_len()).is_err());
    assert_eq!(
        store.snapshot().live_streams,
        1,
        "the gate runs before any per-content backing is created"
    );
    assert!(
        fs::read_dir(store.request_directory())
            .expect("request directory")
            .count()
            <= 1
    );
}

#[test]
fn replay_ingress_decode_workspace_is_charged_to_the_shared_budget_and_released() {
    let root = TestRoot::new("ingress-workspace-budget");
    let manager = ReplayManager::open(config(&root.0, 1024, 1024)).expect("manager");
    let tree = BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024).expect("budget tree");
    let stream_budget = tree.stream(8 * 1024 * 1024).expect("stream budget");
    let store = manager.begin_request(stream_budget.clone()).expect("store");
    let body = serde_json::to_vec(&serde_json::json!({
        "model": "alias",
        "input": "charged decode workspace ".repeat(28_000),
        "stream": false
    }))
    .expect("valid request JSON");
    assert!(body.len() < 1024 * 1024);
    let mut writer = store.begin_raw().expect("raw writer");
    writer.append(&body).expect("append raw body");
    let raw = writer.seal().expect("seal raw body");

    let stats =
        crate::content_ref::scan_ingress_document(store.reader(&raw).expect("raw scanner reader"))
            .expect("scan ingress structure");
    let workspace = stats
        .reserve_workspace(&store, raw.byte_len())
        .expect("reserve decode workspace");
    let charged = stream_budget.snapshot().expect("charged snapshot");
    assert!(
        charged.role_live[MemoryRole::ModelIrBacking as usize] >= body.len() * 2,
        "the full Value/copy transient must be charged before serde allocation"
    );

    let mut document: serde_json::Value = {
        let mut reader = store.reader(&raw).expect("raw decode reader");
        let document = serde_json::from_reader(&mut reader).expect("decode JSON");
        reader.verify_terminal().expect("raw terminal");
        document
    };
    crate::content_ref::compact_ingress_document(
        crate::server::request_plan::IngressProtocol::Responses,
        &mut document,
        &store,
    )
    .expect("compact large ingress field");
    let content = crate::content_ref::ContentRef::from_wire_marker(
        document["input"].as_str().expect("compact input marker"),
    )
    .expect("content reference");
    drop(document);
    drop(workspace);
    let after_decode = stream_budget
        .snapshot()
        .expect("released workspace snapshot");
    assert!(
        after_decode.role_live[MemoryRole::ModelIrBacking as usize] < body.len() / 8,
        "decode workspace must release, leaving only bounded replay/ref metadata"
    );
    assert!(
        after_decode.role_peak[MemoryRole::ModelIrBacking as usize] < body.len() * 3,
        "charged MVP decode path must not reserve multiple unbounded body copies"
    );
    store
        .prevalidate(&[raw, content])
        .expect("prevalidate both replay streams");

    drop(store);
    drop(manager);
    assert_eq!(stream_budget.snapshot().expect("released stream").live, 0);
    assert_eq!(tree.snapshot().process_live, 0);
}

#[cfg(windows)]
#[test]
fn windows_root_and_replay_file_have_protected_owner_only_dacls() {
    let root = TestRoot::new("windows-dacl");
    let manager = ReplayManager::open(config(&root.0, 64, 16)).expect("manager");
    super::windows_acl::verify_owner_only(manager.root(), true).expect("root owner-only DACL");
    let store = manager.begin_request(budget()).expect("store");
    let mut writer = store.begin_raw().expect("writer");
    writer.append(&[9_u8; 256]).expect("append");
    let reference = writer.seal().expect("seal");
    let path = store.stream_path(&reference).expect("disk path");
    super::windows_acl::verify_owner_only(&path, false).expect("file owner-only DACL");
}

#[test]
fn cleanup_waits_for_last_reader_and_restart_removes_orphans() {
    let root = TestRoot::new("cleanup");
    let manager = ReplayManager::open(config(&root.0, 64, 16)).expect("manager");
    let store = manager.begin_request(budget()).expect("store");
    let directory = store.request_directory().to_path_buf();
    let mut writer = store.begin_raw().expect("writer");
    writer.append(&[3_u8; 256]).expect("append");
    let reference = writer.seal().expect("seal");
    let reader = store.reader(&reference).expect("reader");
    drop(store);
    assert!(directory.is_dir(), "reader keeps request owner alive");
    drop(reader);
    assert!(!directory.exists(), "last reader removes request directory");
    drop(manager);

    let orphan = root.0.join("req-stale-restart");
    fs::create_dir(&orphan).expect("orphan directory");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&orphan, fs::Permissions::from_mode(0o700)).expect("orphan mode");
    }
    #[cfg(windows)]
    super::windows_acl::set_owner_only(&orphan, true).expect("orphan DACL");
    std::thread::sleep(Duration::from_millis(3));
    let mut restart = config(&root.0, 64, 16);
    restart.orphan_ttl = Duration::from_millis(1);
    ReplayManager::open(restart).expect("restart cleanup");
    assert!(!orphan.exists());
}

#[test]
fn abandoned_writer_closes_before_last_owner_cleanup() {
    let root = TestRoot::new("abandoned-writer");
    let manager = ReplayManager::open(config(&root.0, 64, 16)).expect("manager");
    let store = manager.begin_request(budget()).expect("store");
    let directory = store.request_directory().to_path_buf();
    let mut writer = store.begin_raw().expect("writer");
    writer.append(&[5_u8; 256]).expect("append");
    drop(store);
    drop(writer);
    assert!(!directory.exists());
}

#[test]
fn terminal_owner_rejects_new_streams() {
    let root = TestRoot::new("terminal");
    let manager = ReplayManager::open(config(&root.0, 64, 16)).expect("manager");
    let store = manager.begin_request(budget()).expect("store");
    store.mark_terminal();
    assert!(matches!(store.begin_raw(), Err(ReplayError::AfterTerminal)));
}
