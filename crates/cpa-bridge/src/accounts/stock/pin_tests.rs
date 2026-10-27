//! Exercise the real stock control HTTP entry, not a mocked CpaControlPlane.
use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Fixture {
    root: crate::private_test_support::TempDir,
    address: SocketAddr,
    patches: Arc<AtomicUsize>,
    reject_patch: Arc<AtomicBool>,
    patch_delay_ms: Arc<AtomicUsize>,
    fault: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn new() -> Self {
        let root = crate::private_test_support::tempdir().unwrap();
        let path = root.path().join("codex-a.json");
        #[cfg(not(windows))]
        fs::write(
            &path,
            br#"{"prefix":"hiroute-codex-current","request_retry":0,"disable_cooling":true}"#,
        )
        .unwrap();
        #[cfg(windows)]
        crate::config::private_atomic_write(
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
        let fault = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (count, reject, stopped) = (patches.clone(), reject_patch.clone(), stop.clone());
        let delay = patch_delay_ms.clone();
        let fault_value = fault.clone();
        let worker = thread::spawn(move || {
            let mut account_reads = 0;
            let mut model_reads = 0;
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
                    if stream.read_exact(&mut byte).is_err() {
                        break;
                    }
                    bytes.push(byte[0]);
                }
                // A caller may exhaust its shared budget after connect or a partial
                // write. That abandoned request must not kill the fixture server.
                if !bytes.ends_with(b"\r\n\r\n") {
                    continue;
                }
                let header = String::from_utf8(bytes).unwrap();
                let length: usize = header
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                let mut body = vec![0; length];
                if stream.read_exact(&mut body).is_err() {
                    continue;
                }
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
                    account_reads += 1;
                    let mut value: Value =
                        serde_json::from_slice(&super::tests::oauth_response("")).unwrap();
                    if account_reads > 1 && fault_value.load(Ordering::SeqCst) == 4 {
                        value["files"][0]["auth_index"] = serde_json::json!("idx-changed");
                    }
                    if fault_value.load(Ordering::SeqCst) == 5 {
                        value["files"][0]["request_retry"] = serde_json::json!(1);
                    }
                    (200, value.to_string())
                } else if header.starts_with("GET /v0/management/auth-files/models?") {
                    model_reads += 1;
                    let body = match fault_value.load(Ordering::SeqCst) {
                        1 => r#"{"models":[]}"#.to_owned(),
                        2 => r#"{"models":[{"id":"other-prefix/future-model"}]}"#.to_owned(),
                        9 => serde_json::json!({"models":[{"id":format!("hiroute-codex-current/{}", "a".repeat(257))}]}).to_string(),
                        fault @ 15..=16 if model_reads < 3 => {
                            let latest = if fault == 15 {
                                serde_json::json!({"attempt_id":1,"phase":"registration","outcome":"discarded","reason":"account_changed","elapsed_ms":50,"phase_elapsed_ms":0,"timings":[]})
                            } else {
                                serde_json::json!({"attempt_id":2,"phase":"registration","outcome":"completed","model_count":1,"elapsed_ms":60,"phase_elapsed_ms":0,"timings":[]})
                            };
                            serde_json::json!({"models":[],"hiroute_model_discovery":{
                                "schema":"hiroute.cpa-model-discovery/v1","process_id":123,
                                "evicted_accounts":0,"first_failure":null,"latest":latest,
                                "registration_pending":fault == 15
                            }}).to_string()
                        },
                        fault @ 10..=14 => {
                            let empty = fault != 11 && (fault != 12 || model_reads < 3);
                            let (phase, outcome, reason) = match fault {
                                12 => ("request", "entered", None),
                                13 => ("registration", "completed", Some("empty_after_filtering")),
                                14 => ("registration", "discarded", Some("account_changed")),
                                _ => ("request", "failed", Some("timeout")),
                            };
                            serde_json::json!({
                                "models": if empty {serde_json::json!([])} else {serde_json::json!([{"id":"hiroute-codex-current/future-model"}])},
                                "hiroute_model_discovery": {"schema":"hiroute.cpa-model-discovery/v1", "process_id":123,"evicted_accounts":0,"first_failure":null,
                                    "registration_pending":false,
                                    "latest":{"attempt_id":1,"phase":phase,"outcome":outcome,"reason":reason,"model_count":0,"elapsed_ms":50,"phase_elapsed_ms":50,"timings":[]}}
                            }).to_string()
                        },
                        _ => r#"{"models":[{"id":"hiroute-codex-current/future-model"}]}"#.to_owned(),
                    };
                    (200, body)
                } else if header.starts_with("GET /v1/models ") {
                    let body = if fault_value.load(Ordering::SeqCst) == 3 {
                        r#"{"data":[]}"#
                    } else {
                        r#"{"data":[{"id":"hiroute-codex-current/future-model"}]}"#
                    };
                    (200, body.to_owned())
                } else {
                    panic!("unexpected request: {}", header.lines().next().unwrap());
                };
                let written = write!(
                    stream,
                    "HTTP/1.1 {status} OK\r\nX-CPA-Version: 8.0.4-hiroute.4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                if let Err(error) = written {
                    assert!(matches!(
                        error.kind(),
                        std::io::ErrorKind::BrokenPipe
                            | std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::WouldBlock
                    ));
                }
            }
        });
        Self {
            root,
            address,
            patches,
            reject_patch,
            patch_delay_ms,
            fault,
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
            &hiroute_diagnostics::context::DiagnosticHandle::noop().context(),
        )
    }
}

#[path = "diagnostic_tests.rs"]
mod diagnostic_tests;

#[test]
fn terminal_empty_discovery_fails_without_spending_the_control_budget() {
    for fault in [10, 13, 14] {
        let fixture = Fixture::new();
        fixture.fault.store(fault, Ordering::SeqCst);
        let started = Instant::now();
        let error = fixture.discover(false).unwrap_err();
        if fault == 10 {
            assert!(matches!(
                error,
                AccountDiscoveryError::TransientModelCatalog { process_id: 123 }
            ));
        } else {
            assert!(matches!(error, AccountDiscoveryError::PinNotApplied));
        }
        assert!(started.elapsed() < Duration::from_millis(500));
        assert_eq!(fixture.patches.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn pending_discovery_and_retained_catalog_keep_their_existing_meaning() {
    for fault in [11, 12] {
        let fixture = Fixture::new();
        fixture.fault.store(fault, Ordering::SeqCst);
        assert_eq!(
            fixture.discover(false).unwrap()[0].observed_model_ids,
            BTreeSet::from(["future-model".into()])
        );
        assert_eq!(fixture.patches.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn authorization_handoff_gap_keeps_waiting_for_the_current_catalog() {
    assert_concurrent_catalog_recovers(15);
}

#[test]
fn mixed_empty_catalog_and_success_snapshot_keeps_waiting() {
    assert_concurrent_catalog_recovers(16);
}

fn assert_concurrent_catalog_recovers(fault: usize) {
    let fixture = Fixture::new();
    fixture.fault.store(fault, Ordering::SeqCst);
    assert_eq!(
        fixture.discover(false).unwrap()[0].observed_model_ids,
        BTreeSet::from(["future-model".into()])
    );
    assert_eq!(fixture.patches.load(Ordering::SeqCst), 0);
}

#[test]
fn fixture_survives_abandoned_deadline_connections() {
    for partial in [
        b"".as_slice(),
        b"GET /v0/management/auth-files?name=codex-a.json HTTP/1.1\r\n".as_slice(),
        b"PATCH /v0/management/auth-files/fields HTTP/1.1\r\nContent-Length: 3\r\n\r\nx".as_slice(),
    ] {
        let fixture = Fixture::new();
        let mut stream = std::net::TcpStream::connect(fixture.address).unwrap();
        stream.write_all(partial).unwrap();
        drop(stream);
        assert_eq!(fixture.discover(false).unwrap().len(), 1);
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
                &hiroute_diagnostics::context::DiagnosticHandle::noop().context(),
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
