//! Exercise the real stock control HTTP entry, not a mocked CpaControlPlane.
use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Fixture {
    root: tempfile::TempDir,
    address: SocketAddr,
    patches: Arc<AtomicUsize>,
    reject_patch: Arc<AtomicBool>,
    patch_delay_ms: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("codex-a.json");
        fs::write(
            &path,
            br#"{"prefix":"hiroute-codex-current","request_retry":0,"disable_cooling":true}"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let patches = Arc::new(AtomicUsize::new(0));
        let reject_patch = Arc::new(AtomicBool::new(false));
        let patch_delay_ms = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (count, reject, stopped) = (patches.clone(), reject_patch.clone(), stop.clone());
        let delay = patch_delay_ms.clone();
        let worker = thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                if stopped.load(Ordering::SeqCst) {
                    break;
                }
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut byte = [0];
                while !bytes.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    bytes.push(byte[0]);
                }
                let header = String::from_utf8(bytes).unwrap();
                let length: usize = header
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                let mut body = vec![0; length];
                stream.read_exact(&mut body).unwrap();
                let (status, body) = if header.starts_with("PATCH ") {
                    count.fetch_add(1, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(delay.load(Ordering::SeqCst) as u64));
                    let body: Value = serde_json::from_slice(&body).unwrap();
                    assert_eq!(body["prefix"], "hiroute-codex-current");
                    if reject.load(Ordering::SeqCst) {
                        (503, "{}".to_owned())
                    } else {
                        (200, "{}".to_owned())
                    }
                } else if header.starts_with("GET /v0/management/auth-files?name=") {
                    (
                        200,
                        String::from_utf8(super::tests::oauth_response("")).unwrap(),
                    )
                } else if header.starts_with("GET /v0/management/auth-files/models?") {
                    (
                        200,
                        r#"{"models":[{"id":"hiroute-codex-current/future-model"}]}"#.to_owned(),
                    )
                } else if header.starts_with("GET /v1/models ") {
                    (
                        200,
                        r#"{"data":[{"id":"hiroute-codex-current/future-model"}]}"#.to_owned(),
                    )
                } else {
                    panic!("unexpected request: {}", header.lines().next().unwrap());
                };
                let written = write!(
                    stream,
                    "HTTP/1.1 {status} OK\r\nX-CPA-Version: 8.0.4-hiroute.4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                // The legacy-deadline negative case deliberately closes this socket early.
                if delay.load(Ordering::SeqCst) == 0 {
                    written.unwrap();
                }
            }
        });
        Self {
            root,
            address,
            patches,
            reject_patch,
            patch_delay_ms,
            stop,
            worker: Some(worker),
        }
    }

    fn discover(&self, refresh: bool) -> Result<Vec<AccountSnapshotRecord>, AccountDiscoveryError> {
        self.discover_with_timeout(refresh, Duration::from_secs(2))
    }

    fn discover_with_timeout(
        &self,
        refresh: bool,
        timeout: Duration,
    ) -> Result<Vec<AccountSnapshotRecord>, AccountDiscoveryError> {
        StockCpaControlPlane.discover_and_pin(
            self.address,
            self.root.path(),
            &[ManagedAccountIdentity {
                account_kind: CpaAccountKind::Codex,
                stock_file_name: "codex-a.json".into(),
                account_digest: "a".repeat(64),
                generation: 1,
                client_version: None,
            }],
            &InstanceSecrets::generate().unwrap(),
            "8.0.4-hiroute.4",
            timeout,
            refresh,
        )
    }
}

#[test]
fn production_control_budget_covers_synchronous_refresh_and_pin_readback() {
    let fixture = Fixture::new();
    // A management update includes synchronous upstream registration hooks, plus local work.
    fixture.patch_delay_ms.store(5_200, Ordering::SeqCst);
    assert!(
        fixture
            .discover_with_timeout(true, Duration::from_secs(5))
            .is_err()
    );
    let accounts = fixture
        .discover_with_timeout(true, crate::MANAGED_CPA_CONTROL_TIMEOUT)
        .unwrap();
    assert_eq!(
        accounts[0].observed_model_ids,
        BTreeSet::from(["future-model".into()])
    );
    assert_eq!(fixture.patches.load(Ordering::SeqCst), 2);
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(self.address);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn repeated_pin_reads_do_not_refresh_remote_catalog() {
    let fixture = Fixture::new();
    fixture.discover(true).unwrap();
    assert_eq!(fixture.patches.load(Ordering::SeqCst), 1);
    // The refresh path is now unavailable. All four compilation reads and
    // subsequent preview must still use the verified, already registered models.
    fixture.reject_patch.store(true, Ordering::SeqCst);
    for _ in 0..6 {
        let accounts = fixture.discover(false).unwrap();
        assert_eq!(
            accounts[0].observed_model_ids,
            BTreeSet::from(["future-model".into()])
        );
    }
    assert_eq!(fixture.patches.load(Ordering::SeqCst), 1);
    assert!(fixture.discover(true).is_err());
    assert_eq!(fixture.patches.load(Ordering::SeqCst), 2);
}

#[test]
fn changed_controls_still_require_a_patch_and_bad_pins_are_not_accepted() {
    let fixture = Fixture::new();
    fs::write(
        fixture.root.path().join("codex-a.json"),
        br#"{"prefix":"wrong","request_retry":0,"disable_cooling":true}"#,
    )
    .unwrap();
    fixture.reject_patch.store(true, Ordering::SeqCst);
    assert!(fixture.discover(false).is_err());
    assert_eq!(fixture.patches.load(Ordering::SeqCst), 1);
}

/// Opt-in smoke against a running isolated managed CPA. The manifest contains
/// paths, not credentials. A recording proxy can reject PATCH to prove reads
/// remain usable without the directory refresh path.
#[test]
#[ignore = "requires HIROUTE_CPA_PIN_SMOKE manifest and a live isolated CPA"]
fn live_managed_cpa_pin_reads() {
    let manifest = std::env::var("HIROUTE_CPA_PIN_SMOKE").expect("smoke manifest required");
    let value: Value = serde_json::from_slice(&fs::read(manifest).unwrap()).unwrap();
    let field = |key: &str| value[key].as_str().unwrap();
    let secrets = InstanceSecrets::read(Path::new(field("capability_file"))).unwrap();
    let identity = ManagedAccountIdentity {
        account_kind: CpaAccountKind::Codex,
        stock_file_name: field("stock_file_name").into(),
        account_digest: "a".repeat(64),
        generation: 1,
        client_version: None,
    };
    for _ in 0..6 {
        let accounts = StockCpaControlPlane
            .discover_and_pin(
                field("address").parse().unwrap(),
                Path::new(field("auth_dir")),
                std::slice::from_ref(&identity),
                &secrets,
                field("version"),
                Duration::from_secs(5),
                false,
            )
            .unwrap();
        assert_eq!(accounts.len(), 1);
        assert!(
            accounts[0]
                .observed_model_ids
                .contains(field("expected_model"))
        );
    }
}
